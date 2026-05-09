//! On-disk media file storage with `PostgreSQL` metadata tracking.
//!
//! Stores media files in the workspace directory and maintains a
//! `PostgreSQL` table for metadata lookup, deduplication, and listing.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sqlx_core::pool::{Pool, PoolOptions};
use sqlx_core::query::query;
use sqlx_core::row::Row;
use sqlx_postgres::Postgres;

use super::types::{MediaConfig, MediaFile, MediaType, StoredMedia};

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn verify_write_enabled() -> bool {
    std::env::var("ASTEREL_MEDIA_VERIFY_WRITE")
        .ok()
        .is_some_and(|v| {
            let normalized = v.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        })
}

/// On-disk media file store backed by `PostgreSQL` metadata.
pub(crate) struct MediaStore {
    storage_dir: PathBuf,
    max_file_size: u64,
    pool: Pool<Postgres>,
    runtime: tokio::runtime::Runtime,
}

impl MediaStore {
    /// Create a new media store rooted in the workspace directory.
    ///
    /// # Errors
    ///
    /// Returns an error if storage directory setup or metadata table
    /// initialization fails.
    pub(crate) fn new(config: &MediaConfig, workspace_dir: &str) -> Result<Self> {
        let storage_dir = config
            .storage_dir
            .as_deref()
            .map_or_else(|| PathBuf::from(workspace_dir).join("media"), PathBuf::from);

        // Validate that storage_dir is contained within workspace to prevent
        // path traversal via malicious config values.
        let canonical_workspace = std::path::Path::new(workspace_dir)
            .canonicalize()
            .with_context(|| format!("workspace directory not accessible: {workspace_dir}"))?;
        std::fs::create_dir_all(&storage_dir)?;
        let canonical_storage = storage_dir.canonicalize()?;
        anyhow::ensure!(
            canonical_storage.starts_with(&canonical_workspace),
            "media storage_dir must be within workspace: {} is outside {}",
            canonical_storage.display(),
            canonical_workspace.display()
        );

        let database_url = crate::utils::postgres::require_postgres_url(
            None,
            Some(Path::new(workspace_dir)),
            "media store",
        )?;

        let runtime = tokio::runtime::Runtime::new().context("create media runtime")?;
        let pool = crate::utils::postgres::block_on_sync(&runtime, async {
            PoolOptions::<Postgres>::new()
                .max_connections(5)
                .connect(&database_url)
                .await
                .context("connect postgres for media store")
        })?;

        crate::utils::postgres::block_on_sync(&runtime, async {
            query(
                "CREATE TABLE IF NOT EXISTS media_files (
                    id TEXT PRIMARY KEY,
                    mime_type TEXT NOT NULL,
                    media_type TEXT NOT NULL,
                    filename TEXT,
                    size_bytes BIGINT NOT NULL,
                    storage_path TEXT NOT NULL,
                    created_at TEXT NOT NULL
                )",
            )
            .execute(&pool)
            .await
            .context("create media_files table")?;
            Ok::<(), anyhow::Error>(())
        })?;

        Ok(Self {
            storage_dir,
            max_file_size: config.max_file_size_mb * 1_024 * 1_024,
            pool,
            runtime,
        })
    }

    /// Store a media file on disk and persist its metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exceeds the size limit, the
    /// write fails, or metadata persistence fails.
    pub(crate) fn store(&self, data: &[u8], filename: Option<&str>) -> Result<MediaFile> {
        let size_bytes = usize_to_u64(data.len());
        if size_bytes > self.max_file_size {
            anyhow::bail!(
                "file size {} exceeds maximum {} bytes",
                data.len(),
                self.max_file_size
            );
        }

        let id = uuid::Uuid::new_v4().to_string();
        let (mime_type, media_type) = super::detection::detect_media_type(data, filename);

        let ext = extension_from_mime(&mime_type);
        let storage_filename = format!("{id}.{ext}");
        let storage_path = self.storage_dir.join(storage_filename);

        std::fs::write(&storage_path, data)?;

        let created_at = chrono::Utc::now().to_rfc3339();
        let media_file = MediaFile {
            id,
            mime_type,
            media_type,
            filename: filename.map(String::from),
            size_bytes,
            storage_path: storage_path.to_string_lossy().into_owned(),
            created_at,
        };
        if let Err(error) = self.persist_metadata(&media_file) {
            let _ = std::fs::remove_file(&storage_path);
            return Err(error);
        }

        if verify_write_enabled()
            && let Ok(stored_media) = self.retrieve(&media_file.id)
        {
            tracing::debug!(
                media_id = %stored_media.file.id,
                stored_bytes = stored_media.file.size_bytes,
                readback_bytes = stored_media.data.len(),
                "media write verification readback completed"
            );
        }

        Ok(media_file)
    }

    /// Retrieve a stored media file by its unique ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the metadata lookup or file read fails.
    pub(crate) fn retrieve(&self, id: &str) -> Result<StoredMedia> {
        let media_file = self.load_metadata(id)?;
        let storage_path =
            validated_stored_media_path(&self.storage_dir, &media_file.storage_path)?;
        let data = std::fs::read(storage_path)?;
        Ok(StoredMedia {
            file: media_file,
            data,
        })
    }

    /// Return the root directory where media files are stored.
    #[must_use]
    pub(crate) fn storage_dir(&self) -> &Path {
        &self.storage_dir
    }

    fn persist_metadata(&self, media_file: &MediaFile) -> Result<()> {
        let size_bytes = i64::try_from(media_file.size_bytes)?;
        crate::utils::postgres::block_on_sync(&self.runtime, async {
            query(
                "INSERT INTO media_files (
                    id, mime_type, media_type, filename, size_bytes, storage_path, created_at
                ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(&media_file.id)
            .bind(&media_file.mime_type)
            .bind(media_file.media_type.as_str())
            .bind(&media_file.filename)
            .bind(size_bytes)
            .bind(&media_file.storage_path)
            .bind(&media_file.created_at)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(Into::into)
        })
    }

    fn load_metadata(&self, id: &str) -> Result<MediaFile> {
        crate::utils::postgres::block_on_sync(&self.runtime, async {
            let row = query(
                "SELECT id, mime_type, media_type, filename, size_bytes, storage_path, created_at
                 FROM media_files
                 WHERE id = $1",
            )
            .bind(id)
            .fetch_one(&self.pool)
            .await?;

            let media_type: String = row.get("media_type");
            let size_bytes_i64: i64 = row.get("size_bytes");
            let size_bytes = u64::try_from(size_bytes_i64)
                .map_err(|error| anyhow::anyhow!("invalid media size in DB: {error}"))?;

            Ok(MediaFile {
                id: row.get("id"),
                mime_type: row.get("mime_type"),
                media_type: MediaType::from_kind(&media_type),
                filename: row.get("filename"),
                size_bytes,
                storage_path: row.get("storage_path"),
                created_at: row.get("created_at"),
            })
        })
    }
}

fn validated_stored_media_path(storage_dir: &Path, stored_path: &str) -> Result<PathBuf> {
    let canonical_storage = storage_dir
        .canonicalize()
        .context("canonicalize media storage directory")?;
    let canonical_file = Path::new(stored_path)
        .canonicalize()
        .context("canonicalize stored media path")?;
    anyhow::ensure!(
        canonical_file.starts_with(&canonical_storage),
        "stored media path is outside media storage directory"
    );
    Ok(canonical_file)
}

fn extension_from_mime(mime_type: &str) -> &'static str {
    match mime_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "audio/mpeg" => "mp3",
        "audio/wav" => "wav",
        "audio/ogg" => "ogg",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "application/pdf" => "pdf",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::TempDir;

    use super::MediaStore;
    use super::validated_stored_media_path;
    use crate::media::types::MediaConfig;

    fn configured_store(tmp: &TempDir) -> MediaStore {
        let database_url = crate::utils::test_env::postgres_url()
            .expect("test requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL");
        let workspace_dir = tmp.path().to_string_lossy().into_owned();
        crate::utils::test_env::write_workspace_postgres_config(tmp.path(), &database_url)
            .expect("test config should be written");
        MediaStore::new(&MediaConfig::default(), &workspace_dir)
            .expect("media store should be created")
    }

    #[test]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    fn store_and_retrieve_roundtrip() {
        let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
        let temp_dir = TempDir::new().unwrap();
        let store = configured_store(&temp_dir);

        let data = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00];
        let stored = store.store(&data, Some("sample.png")).unwrap();
        let retrieved = store.retrieve(&stored.id).unwrap();

        assert_eq!(retrieved.data, data);
        assert_eq!(retrieved.file.id, stored.id);
        assert_eq!(retrieved.file.mime_type, "image/png");
        assert_eq!(retrieved.file.filename.as_deref(), Some("sample.png"));
    }

    #[test]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    fn store_rejects_oversized_file() {
        let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
        let temp_dir = TempDir::new().unwrap();
        // Override the default 25 MB limit to 1 MB so we don't allocate a
        // 25+ MB buffer just to exercise the size gate.
        let database_url = crate::utils::test_env::postgres_url()
            .expect("test requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL");
        let workspace_dir = temp_dir.path().to_string_lossy().into_owned();
        crate::utils::test_env::write_workspace_postgres_config(temp_dir.path(), &database_url)
            .expect("test config should be written");
        let config = MediaConfig {
            max_file_size_mb: 1,
            ..MediaConfig::default()
        };
        let store = MediaStore::new(&config, &workspace_dir).expect("store should be created");

        let oversized = vec![0_u8; (1_024 * 1_024) + 1];
        let result = store.store(&oversized, Some("too_large.bin"));
        assert!(result.is_err(), "1 MB + 1 byte should exceed 1 MB limit");
    }

    #[test]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    fn store_creates_file_on_disk() {
        let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
        let temp_dir = TempDir::new().unwrap();
        let store = configured_store(&temp_dir);

        let data = b"hello";
        let stored = store.store(data, Some("hello.txt")).unwrap();
        assert!(Path::new(&stored.storage_path).exists());
    }

    #[test]
    fn validated_stored_media_path_rejects_metadata_escape() {
        let storage = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let outside_file = outside.path().join("escape.bin");
        std::fs::write(&outside_file, b"escape").unwrap();

        let error = validated_stored_media_path(storage.path(), &outside_file.to_string_lossy())
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("stored media path is outside media storage directory")
        );
    }
}
