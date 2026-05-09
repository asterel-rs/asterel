//! Attachment helpers: download, size-gate, and convert media attachments
//! from channel messages into LLM-consumable content blocks.
mod describe;
mod load;
mod store;

use std::sync::Arc;

use futures_util::stream::{self, StreamExt};

use super::traits::{MediaAttachment, MediaContent};
use crate::core::providers::response::ContentBlock;
use crate::media::{MediaProcessor, MediaStore, describe_media_for_prompt};

use self::store::persist_attachment;

const ATTACHMENT_PREPARE_CONCURRENCY: usize = 4;

#[cfg(any(feature = "discord", feature = "matrix", feature = "slack"))]
pub(crate) fn media_attachment_url(
    url: String,
    mime_type: Option<&str>,
    filename: Option<String>,
) -> MediaAttachment {
    MediaAttachment {
        mime_type: mime_type.unwrap_or("application/octet-stream").to_string(),
        data: MediaContent::Url(url),
        filename,
    }
}

/// Filters image attachments and converts them to LLM content blocks.
pub(crate) fn convert_attachments_to_images(attachments: &[MediaAttachment]) -> Vec<ContentBlock> {
    attachments
        .iter()
        .filter_map(attachment_to_image_block)
        .collect()
}

pub(crate) use crate::utils::encoding::encode_base64;
pub(crate) use describe::fallback_attachment_description;
#[cfg(feature = "telegram")]
pub(crate) async fn collect_attachment_response_body(
    response: reqwest::Response,
) -> anyhow::Result<Vec<u8>> {
    load::collect_attachment_body(response).await
}
#[cfg(test)]
pub(crate) use load::MAX_ATTACHMENT_BYTES;
#[cfg(test)]
pub(crate) use load::collect_attachment_chunks_for_test;
#[cfg(test)]
pub(crate) use load::is_redirect_status;
pub(crate) use load::{load_attachment_bytes, output_attachment_to_media_attachment};

/// Converts a single image attachment to a content block, returning
/// `None` for non-image MIME types.
pub(crate) fn attachment_to_image_block(attachment: &MediaAttachment) -> Option<ContentBlock> {
    use crate::core::providers::response::ImageSource;

    if !attachment.mime_type.starts_with("image/") {
        return None;
    }

    let MediaContent::Bytes(bytes) = &attachment.data else {
        return None;
    };
    let source = ImageSource::base64(&attachment.mime_type, encode_base64(bytes));
    Some(ContentBlock::Image { source })
}

struct PreparedAttachment {
    index: usize,
    description: Option<String>,
    image_block: Option<ContentBlock>,
}

async fn prepare_single_attachment(
    index: usize,
    attachment: MediaAttachment,
    store: &Arc<MediaStore>,
    processor: &MediaProcessor,
) -> PreparedAttachment {
    if attachment.mime_type.starts_with("image/") {
        return prepare_image_attachment(index, attachment, store).await;
    }

    let description = prepare_attachment_description(&attachment, store, processor).await;

    PreparedAttachment {
        index,
        description,
        image_block: None,
    }
}

async fn prepare_image_attachment(
    index: usize,
    attachment: MediaAttachment,
    store: &Arc<MediaStore>,
) -> PreparedAttachment {
    let bytes = match &attachment.data {
        MediaContent::Bytes(bytes) => bytes.clone(),
        MediaContent::Url(_) => match load_attachment_bytes(&attachment).await {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(
                    channel_attachment = ?attachment.filename,
                    error = %error,
                    "failed to load image attachment bytes"
                );
                return PreparedAttachment {
                    index,
                    description: Some(fallback_attachment_description(&attachment, None)),
                    image_block: None,
                };
            }
        },
    };

    if let Err(error) = persist_attachment(store, bytes.clone(), attachment.filename.clone()).await
    {
        tracing::warn!(
            channel_attachment = ?attachment.filename,
            error = %error,
            "failed to persist image attachment"
        );
    }

    let source = crate::core::providers::response::ImageSource::base64(
        &attachment.mime_type,
        encode_base64(&bytes),
    );
    PreparedAttachment {
        index,
        description: None,
        image_block: Some(ContentBlock::Image { source }),
    }
}

pub(crate) async fn prepare_channel_input_and_images(
    model_input: &str,
    attachments: &[MediaAttachment],
    media_store: Option<&Arc<MediaStore>>,
    processor: &MediaProcessor,
) -> (String, Vec<ContentBlock>) {
    let Some(store) = media_store else {
        return (
            model_input.to_string(),
            convert_attachments_to_images(attachments),
        );
    };
    let mut prepared = stream::iter(attachments.iter().cloned().enumerate())
        .map(|(index, attachment)| prepare_single_attachment(index, attachment, store, processor))
        .buffer_unordered(ATTACHMENT_PREPARE_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    prepared.sort_by_key(|item| item.index);

    let mut attach_desc = String::new();
    let mut image_blocks = Vec::new();
    for item in prepared {
        if let Some(description) = item.description {
            if !attach_desc.is_empty() {
                attach_desc.push('\n');
            }
            attach_desc.push_str(&description);
        }
        if let Some(block) = item.image_block {
            image_blocks.push(block);
        }
    }

    if attach_desc.is_empty() {
        (model_input.to_string(), image_blocks)
    } else {
        (format!("{attach_desc}\n\n{model_input}"), image_blocks)
    }
}

async fn prepare_attachment_description(
    attachment: &MediaAttachment,
    store: &Arc<MediaStore>,
    processor: &MediaProcessor,
) -> Option<String> {
    let bytes = match load_attachment_bytes(attachment).await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(
                channel_attachment = ?attachment.filename,
                error = %error,
                "failed to load attachment bytes"
            );
            return Some(fallback_attachment_description(attachment, None));
        }
    };

    match persist_attachment(store, bytes, attachment.filename.clone()).await {
        Ok(stored) => {
            match describe_media_for_prompt(processor, stored.file(), stored.bytes()).await {
                Ok(description) => Some(description),
                Err(error) => {
                    tracing::warn!(
                        channel_attachment = ?attachment.filename,
                        error = %error,
                        "failed to describe non-image attachment"
                    );
                    Some(fallback_attachment_description(
                        attachment,
                        Some(stored.size_bytes()),
                    ))
                }
            }
        }
        Err(error) => {
            tracing::warn!(
                channel_attachment = ?attachment.filename,
                error = %error,
                "failed to persist attachment"
            );
            Some(fallback_attachment_description(
                attachment,
                Some(error.size_bytes()),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use tempfile::TempDir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::providers::response::ContentBlock;
    use crate::core::tools::OutputAttachment;
    use crate::media::{MediaProcessor, MediaStore};

    /// RAII guard that wraps an `Arc<MediaStore>` and drops the inner store
    /// on a blocking-allowed thread via `tokio::task::block_in_place`.
    ///
    /// `MediaStore` owns a dedicated `tokio::runtime::Runtime` so its sync
    /// API can run async sqlx queries via `block_on_sync`. If the store is
    /// constructed and then dropped inside an async test body, the nested
    /// runtime drop panics with "Cannot drop a runtime in a context where
    /// blocking is not allowed". This guard's Drop impl sidesteps that by
    /// yielding the current worker before dropping the `Arc`.
    struct TestMediaStore(Option<Arc<MediaStore>>);

    impl TestMediaStore {
        fn inner(&self) -> &Arc<MediaStore> {
            self.0.as_ref().expect("media store should still be alive")
        }
    }

    impl Drop for TestMediaStore {
        fn drop(&mut self) {
            if let Some(store) = self.0.take() {
                tokio::task::block_in_place(move || drop(store));
            }
        }
    }

    fn test_media_store(temp_dir: &TempDir, max_file_size_mb: u64) -> TestMediaStore {
        let database_url = crate::utils::test_env::postgres_url()
            .expect("test requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL");
        let workspace = temp_dir.path().to_string_lossy().into_owned();
        crate::utils::test_env::write_workspace_postgres_config(temp_dir.path(), &database_url)
            .expect("test config should be written");
        let config = crate::media::types::MediaConfig {
            enabled: true,
            storage_dir: None,
            max_file_size_mb,
            ..crate::media::types::MediaConfig::default()
        };
        TestMediaStore(Some(Arc::new(
            MediaStore::new(&config, &workspace).expect("media store should be created"),
        )))
    }

    fn stored_file_count(temp_dir: &TempDir) -> usize {
        fs::read_dir(temp_dir.path().join("media"))
            .unwrap()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy() != "media.db")
            .count()
    }

    #[test]
    fn encode_base64_empty() {
        assert_eq!(encode_base64(&[]), "");
    }

    #[test]
    fn encode_base64_hello() {
        assert_eq!(encode_base64(b"Hello"), "SGVsbG8=");
    }

    #[test]
    fn encode_base64_three_byte_aligned() {
        assert_eq!(encode_base64(b"abc"), "YWJj");
    }

    #[test]
    fn convert_attachments_filters_non_images() {
        let attachments = vec![
            MediaAttachment {
                mime_type: "image/png".to_string(),
                data: MediaContent::Bytes(vec![0x89, 0x50, 0x4E, 0x47]),
                filename: Some("img.png".to_string()),
            },
            MediaAttachment {
                mime_type: "audio/mpeg".to_string(),
                data: MediaContent::Url("https://example.com/audio.mp3".to_string()),
                filename: Some("audio.mp3".to_string()),
            },
        ];
        let blocks = convert_attachments_to_images(&attachments);
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0], ContentBlock::Image { .. }));
    }

    #[test]
    fn convert_attachments_does_not_forward_unvalidated_url_source() {
        let attachments = vec![MediaAttachment {
            mime_type: "image/jpeg".to_string(),
            data: MediaContent::Url("https://example.com/photo.jpg".to_string()),
            filename: None,
        }];
        let blocks = convert_attachments_to_images(&attachments);
        assert!(blocks.is_empty());
    }

    #[test]
    fn convert_attachments_bytes_source() {
        let attachments = vec![MediaAttachment {
            mime_type: "image/png".to_string(),
            data: MediaContent::Bytes(vec![0x89, 0x50, 0x4E, 0x47]),
            filename: Some("test.png".to_string()),
        }];
        let blocks = convert_attachments_to_images(&attachments);
        assert_eq!(blocks.len(), 1);
        if let ContentBlock::Image { source } = &blocks[0] {
            let json = serde_json::to_value(source).unwrap();
            assert_eq!(json["type"], "base64");
            assert_eq!(json["media_type"], "image/png");
            assert!(!json["data"].as_str().unwrap().is_empty());
        } else {
            panic!("expected Image block");
        }
    }

    #[test]
    fn convert_attachments_empty() {
        let blocks = convert_attachments_to_images(&[]);
        assert!(blocks.is_empty());
    }

    #[test]
    fn attachment_to_image_block_returns_none_for_non_images() {
        let attachment = MediaAttachment {
            mime_type: "audio/mpeg".to_string(),
            data: MediaContent::Bytes(vec![1, 2, 3]),
            filename: Some("clip.mp3".to_string()),
        };

        assert!(attachment_to_image_block(&attachment).is_none());
    }

    #[test]
    fn attachment_to_image_block_rejects_unvalidated_url_variant() {
        let attachment = MediaAttachment {
            mime_type: "image/png".to_string(),
            data: MediaContent::Url("https://example.com/a.png".to_string()),
            filename: Some("a.png".to_string()),
        };

        assert!(attachment_to_image_block(&attachment).is_none());
    }

    #[test]
    fn fallback_attachment_description_includes_size_when_known() {
        let attachment = MediaAttachment {
            mime_type: "application/pdf".to_string(),
            data: MediaContent::Bytes(vec![0_u8; 2048]),
            filename: Some("doc.pdf".to_string()),
        };

        let description = fallback_attachment_description(&attachment, Some(2048));
        assert_eq!(description, "[Attachment: doc.pdf (application/pdf, 2KB)]");
    }

    #[test]
    fn fallback_attachment_description_omits_size_when_unknown() {
        let attachment = MediaAttachment {
            mime_type: "application/octet-stream".to_string(),
            data: MediaContent::Url("https://example.com/blob".to_string()),
            filename: None,
        };

        let description = fallback_attachment_description(&attachment, None);
        assert_eq!(
            description,
            "[Attachment: unnamed (application/octet-stream)]"
        );
    }

    #[tokio::test]
    async fn load_attachment_bytes_returns_raw_bytes_variant() {
        let attachment = MediaAttachment {
            mime_type: "text/plain".to_string(),
            data: MediaContent::Bytes(vec![7, 8, 9]),
            filename: Some("note.txt".to_string()),
        };

        let loaded = load_attachment_bytes(&attachment).await.unwrap();
        assert_eq!(loaded, vec![7, 8, 9]);
    }

    #[tokio::test]
    async fn load_attachment_bytes_rejects_private_url_data() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/file.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1, 3, 3, 7]))
            .mount(&server)
            .await;

        let attachment = MediaAttachment {
            mime_type: "application/octet-stream".to_string(),
            data: MediaContent::Url(format!("{}/file.bin", server.uri())),
            filename: Some("file.bin".to_string()),
        };

        let err = load_attachment_bytes(&attachment)
            .await
            .expect_err("private loopback URL should be blocked by SSRF guard");
        assert!(err.to_string().contains("SSRF blocked"));
    }

    #[tokio::test]
    async fn prepare_channel_input_media_disabled_keeps_behavior() {
        let processor = MediaProcessor::new();
        let attachments = vec![
            MediaAttachment {
                mime_type: "image/png".to_string(),
                data: MediaContent::Bytes(vec![0x89, 0x50, 0x4E, 0x47]),
                filename: Some("inline.png".to_string()),
            },
            MediaAttachment {
                mime_type: "audio/mpeg".to_string(),
                data: MediaContent::Bytes(vec![1, 2, 3]),
                filename: Some("sound.mp3".to_string()),
            },
        ];

        let (input, images) =
            prepare_channel_input_and_images("hello", &attachments, None, &processor).await;

        assert_eq!(input, "hello");
        assert_eq!(images.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn prepare_channel_input_non_image_adds_description_and_stores() {
        let _db_guard = crate::utils::test_env::acquire_test_db().await;
        let processor = MediaProcessor::new();
        let temp_dir = TempDir::new().unwrap();
        let store = test_media_store(&temp_dir, 25);
        let attachments = vec![MediaAttachment {
            mime_type: "audio/mpeg".to_string(),
            data: MediaContent::Bytes(vec![1, 2, 3, 4]),
            filename: Some("sound.mp3".to_string()),
        }];

        let (input, images) = prepare_channel_input_and_images(
            "hello",
            &attachments,
            Some(store.inner()),
            &processor,
        )
        .await;

        assert!(input.starts_with("[Audio: sound.mp3 (audio/mpeg, 4 bytes"));
        assert!(input.ends_with("\n\nhello"));
        assert!(images.is_empty());
        assert_eq!(stored_file_count(&temp_dir), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn prepare_channel_input_image_bytes_remains_inline_and_is_stored() {
        let _db_guard = crate::utils::test_env::acquire_test_db().await;
        let processor = MediaProcessor::new();
        let temp_dir = TempDir::new().unwrap();
        let store = test_media_store(&temp_dir, 25);
        let attachments = vec![MediaAttachment {
            mime_type: "image/png".to_string(),
            data: MediaContent::Bytes(vec![0x89, 0x50, 0x4E, 0x47]),
            filename: Some("img.png".to_string()),
        }];

        let (input, images) = prepare_channel_input_and_images(
            "hello",
            &attachments,
            Some(store.inner()),
            &processor,
        )
        .await;

        assert_eq!(input, "hello");
        assert_eq!(images.len(), 1);
        assert_eq!(stored_file_count(&temp_dir), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn prepare_channel_input_image_url_is_blocked_and_not_forwarded_as_url() {
        let _db_guard = crate::utils::test_env::acquire_test_db().await;
        let processor = MediaProcessor::new();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/img.png"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "image/png")
                    .set_body_bytes(vec![0x89, 0x50, 0x4E, 0x47]),
            )
            .mount(&server)
            .await;

        let temp_dir = TempDir::new().unwrap();
        let store = test_media_store(&temp_dir, 25);
        let attachments = vec![MediaAttachment {
            mime_type: "image/png".to_string(),
            data: MediaContent::Url(format!("{}/img.png", server.uri())),
            filename: Some("img.png".to_string()),
        }];

        let (input, images) = prepare_channel_input_and_images(
            "hello",
            &attachments,
            Some(store.inner()),
            &processor,
        )
        .await;

        assert_eq!(input, "[Attachment: img.png (image/png)]\n\nhello");
        assert!(images.is_empty());
        assert_eq!(stored_file_count(&temp_dir), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn prepare_channel_input_non_image_url_blocked_falls_back_to_attachment_description() {
        let _db_guard = crate::utils::test_env::acquire_test_db().await;
        let processor = MediaProcessor::new();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/voice.mp3"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "audio/mpeg")
                    .set_body_bytes(vec![0x49, 0x44, 0x33, 0x00]),
            )
            .mount(&server)
            .await;

        let temp_dir = TempDir::new().unwrap();
        let store = test_media_store(&temp_dir, 25);
        let attachments = vec![MediaAttachment {
            mime_type: "audio/mpeg".to_string(),
            data: MediaContent::Url(format!("{}/voice.mp3", server.uri())),
            filename: Some("voice.mp3".to_string()),
        }];

        let (input, images) = prepare_channel_input_and_images(
            "hello",
            &attachments,
            Some(store.inner()),
            &processor,
        )
        .await;

        assert_eq!(input, "[Attachment: voice.mp3 (audio/mpeg)]\n\nhello");
        assert!(images.is_empty());
        assert_eq!(stored_file_count(&temp_dir), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn prepare_channel_input_non_image_url_download_failure_falls_back() {
        let _db_guard = crate::utils::test_env::acquire_test_db().await;
        let processor = MediaProcessor::new();
        let attachments = vec![MediaAttachment {
            mime_type: "application/pdf".to_string(),
            data: MediaContent::Url("http://127.0.0.1:9/missing.pdf".to_string()),
            filename: Some("missing.pdf".to_string()),
        }];

        let temp_dir = TempDir::new().unwrap();
        let store = test_media_store(&temp_dir, 25);

        let (input, images) = prepare_channel_input_and_images(
            "hello",
            &attachments,
            Some(store.inner()),
            &processor,
        )
        .await;

        assert_eq!(
            input,
            "[Attachment: missing.pdf (application/pdf)]\n\nhello"
        );
        assert!(images.is_empty());
        assert_eq!(stored_file_count(&temp_dir), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn prepare_channel_input_store_failure_falls_back_for_non_image() {
        let _db_guard = crate::utils::test_env::acquire_test_db().await;
        let processor = MediaProcessor::new();
        let temp_dir = TempDir::new().unwrap();
        let store = test_media_store(&temp_dir, 0);
        let attachments = vec![MediaAttachment {
            mime_type: "application/pdf".to_string(),
            data: MediaContent::Bytes(vec![1]),
            filename: Some("doc.pdf".to_string()),
        }];

        let (input, images) = prepare_channel_input_and_images(
            "hello",
            &attachments,
            Some(store.inner()),
            &processor,
        )
        .await;

        assert_eq!(
            input,
            "[Attachment: doc.pdf (application/pdf, 1KB)]\n\nhello"
        );
        assert!(images.is_empty());
        assert_eq!(stored_file_count(&temp_dir), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn prepare_channel_input_mixed_attachments_preserves_images_and_adds_text_prefix() {
        let _db_guard = crate::utils::test_env::acquire_test_db().await;
        let processor = MediaProcessor::new();
        let temp_dir = TempDir::new().unwrap();
        let store = test_media_store(&temp_dir, 25);
        let attachments = vec![
            MediaAttachment {
                mime_type: "audio/mpeg".to_string(),
                data: MediaContent::Bytes(vec![1, 2, 3]),
                filename: Some("clip.mp3".to_string()),
            },
            MediaAttachment {
                mime_type: "image/png".to_string(),
                data: MediaContent::Bytes(vec![0x89, 0x50, 0x4E, 0x47]),
                filename: Some("img.png".to_string()),
            },
        ];

        let (input, images) = prepare_channel_input_and_images(
            "hello",
            &attachments,
            Some(store.inner()),
            &processor,
        )
        .await;

        assert!(input.starts_with("[Audio: clip.mp3 (audio/mpeg, 3 bytes"));
        assert!(input.ends_with("\n\nhello"));
        assert_eq!(images.len(), 1);
        assert_eq!(stored_file_count(&temp_dir), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn prepare_channel_input_parallel_work_preserves_description_order() {
        let _db_guard = crate::utils::test_env::acquire_test_db().await;
        let processor = MediaProcessor::new();
        let temp_dir = TempDir::new().unwrap();
        let store = test_media_store(&temp_dir, 25);
        let attachments = vec![
            MediaAttachment {
                mime_type: "audio/mpeg".to_string(),
                data: MediaContent::Bytes(vec![1, 2, 3]),
                filename: Some("first.mp3".to_string()),
            },
            MediaAttachment {
                mime_type: "application/pdf".to_string(),
                data: MediaContent::Bytes(b"pdf".to_vec()),
                filename: Some("second.pdf".to_string()),
            },
        ];

        let (input, images) = prepare_channel_input_and_images(
            "hello",
            &attachments,
            Some(store.inner()),
            &processor,
        )
        .await;

        let first_index = input.find("first.mp3").expect("first description");
        let second_index = input.find("second.pdf").expect("second description");
        assert!(first_index < second_index);
        assert!(images.is_empty());
        assert_eq!(stored_file_count(&temp_dir), 2);
    }

    #[tokio::test]
    async fn output_attachment_to_media_attachment_reads_bytes_from_path() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("generated.bin");
        fs::write(&path, [1_u8, 2, 3, 4]).unwrap();
        let attachment = OutputAttachment::from_path(
            "application/octet-stream",
            path.to_string_lossy().to_string(),
            Some("generated.bin".to_string()),
        );

        let media = output_attachment_to_media_attachment(&attachment)
            .await
            .unwrap();
        match media.data {
            MediaContent::Bytes(bytes) => assert_eq!(bytes, vec![1, 2, 3, 4]),
            MediaContent::Url(_) => panic!("expected bytes media data"),
        }
        assert_eq!(media.mime_type, "application/octet-stream");
        assert_eq!(media.filename.as_deref(), Some("generated.bin"));
    }

    #[tokio::test]
    async fn output_attachment_to_media_attachment_does_not_preserve_url_variant() {
        let attachment = OutputAttachment::from_url(
            "image/png",
            "http://127.0.0.1/a.png",
            Some("a.png".to_string()),
        );

        let media = output_attachment_to_media_attachment(&attachment).await;
        assert!(media.is_none());
    }

    #[tokio::test]
    async fn output_attachment_to_media_attachment_rejects_private_url() {
        let attachment = OutputAttachment::from_url(
            "image/png",
            "http://127.0.0.1/internal.png",
            Some("internal.png".to_string()),
        );

        let media = output_attachment_to_media_attachment(&attachment).await;
        assert!(media.is_none());
    }

    #[tokio::test]
    async fn output_attachment_to_media_attachment_rejects_rfc1918_10_address() {
        let attachment = OutputAttachment::from_url(
            "image/png",
            "http://10.1.2.3/internal.png",
            Some("internal.png".to_string()),
        );

        let media = output_attachment_to_media_attachment(&attachment).await;
        assert!(media.is_none());
    }

    #[tokio::test]
    async fn output_attachment_to_media_attachment_rejects_rfc1918_172_address() {
        let attachment = OutputAttachment::from_url(
            "image/png",
            "http://172.16.5.10/internal.png",
            Some("internal.png".to_string()),
        );

        let media = output_attachment_to_media_attachment(&attachment).await;
        assert!(media.is_none());
    }

    #[tokio::test]
    async fn output_attachment_to_media_attachment_rejects_rfc1918_192_address() {
        let attachment = OutputAttachment::from_url(
            "image/png",
            "http://192.168.1.20/internal.png",
            Some("internal.png".to_string()),
        );

        let media = output_attachment_to_media_attachment(&attachment).await;
        assert!(media.is_none());
    }

    #[tokio::test]
    async fn output_attachment_to_media_attachment_rejects_loopback_v6_address() {
        let attachment = OutputAttachment::from_url(
            "image/png",
            "http://[::1]/internal.png",
            Some("internal.png".to_string()),
        );

        let media = output_attachment_to_media_attachment(&attachment).await;
        assert!(media.is_none());
    }

    #[tokio::test]
    async fn output_attachment_to_media_attachment_rejects_file_scheme() {
        let attachment = OutputAttachment::from_url(
            "application/octet-stream",
            "file:///tmp/secret.txt",
            Some("secret.txt".to_string()),
        );

        let media = output_attachment_to_media_attachment(&attachment).await;
        assert!(media.is_none());
    }

    #[tokio::test]
    async fn output_attachment_to_media_attachment_rejects_ftp_scheme() {
        let attachment = OutputAttachment::from_url(
            "application/octet-stream",
            "ftp://example.com/public.bin",
            Some("public.bin".to_string()),
        );

        let media = output_attachment_to_media_attachment(&attachment).await;
        assert!(media.is_none());
    }

    #[tokio::test]
    async fn output_attachment_to_media_attachment_rejects_empty_url() {
        let attachment = OutputAttachment::from_url("image/png", "", Some("img.png".to_string()));

        let media = output_attachment_to_media_attachment(&attachment).await;
        assert!(media.is_none());
    }

    #[tokio::test]
    async fn output_attachment_to_media_attachment_rejects_malformed_url() {
        let attachment =
            OutputAttachment::from_url("image/png", "https://", Some("img.png".to_string()));

        let media = output_attachment_to_media_attachment(&attachment).await;
        assert!(media.is_none());
    }

    #[tokio::test]
    async fn output_attachment_to_media_attachment_rejects_url_with_userinfo() {
        let attachment = OutputAttachment::from_url(
            "image/png",
            "https://user:pass@example.com/a.png",
            Some("a.png".to_string()),
        );

        let media = output_attachment_to_media_attachment(&attachment).await;
        assert!(media.is_none());
    }

    #[tokio::test]
    async fn output_attachment_to_media_attachment_rejects_private_url_with_port() {
        let attachment = OutputAttachment::from_url(
            "image/png",
            "http://127.0.0.1:8080/internal.png",
            Some("internal.png".to_string()),
        );

        let media = output_attachment_to_media_attachment(&attachment).await;
        assert!(media.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn prepare_channel_input_non_image_auth_url_falls_back_to_attachment_description() {
        let _db_guard = crate::utils::test_env::acquire_test_db().await;
        let processor = MediaProcessor::new();
        let temp_dir = TempDir::new().unwrap();
        let store = test_media_store(&temp_dir, 25);
        let attachments = vec![MediaAttachment {
            mime_type: "application/pdf".to_string(),
            data: MediaContent::Url("https://user:pass@example.com/doc.pdf".to_string()),
            filename: Some("doc.pdf".to_string()),
        }];

        let (input, images) = prepare_channel_input_and_images(
            "hello",
            &attachments,
            Some(store.inner()),
            &processor,
        )
        .await;

        assert_eq!(input, "[Attachment: doc.pdf (application/pdf)]\n\nhello");
        assert!(images.is_empty());
        assert_eq!(stored_file_count(&temp_dir), 0);
    }

    #[tokio::test]
    async fn output_attachment_to_media_attachment_missing_path_returns_none() {
        let attachment = OutputAttachment::from_path(
            "image/png",
            "/tmp/does-not-exist.png",
            Some("missing.png".to_string()),
        );

        let media = output_attachment_to_media_attachment(&attachment).await;
        assert!(media.is_none());
    }

    // NOTE: The old test `output_attachment_to_media_attachment_without_location_returns_none`
    // is no longer needed — the AttachmentSource enum makes the "neither path nor url" state
    // unrepresentable at compile time.

    #[tokio::test]
    async fn load_attachment_bytes_rejects_file_scheme_url() {
        let attachment = MediaAttachment {
            mime_type: "application/octet-stream".to_string(),
            data: MediaContent::Url("file:///tmp/private.bin".to_string()),
            filename: Some("private.bin".to_string()),
        };

        let error = load_attachment_bytes(&attachment)
            .await
            .expect_err("file scheme should be blocked by fetch validation");
        assert!(error.to_string().contains("http:// and https://"));
    }

    #[tokio::test]
    async fn load_attachment_bytes_rejects_ftp_scheme_url() {
        let attachment = MediaAttachment {
            mime_type: "application/octet-stream".to_string(),
            data: MediaContent::Url("ftp://example.com/public.bin".to_string()),
            filename: Some("public.bin".to_string()),
        };

        let error = load_attachment_bytes(&attachment)
            .await
            .expect_err("ftp scheme should be blocked by fetch validation");
        assert!(error.to_string().contains("http:// and https://"));
    }

    #[tokio::test]
    async fn load_attachment_bytes_rejects_empty_url() {
        let attachment = MediaAttachment {
            mime_type: "application/octet-stream".to_string(),
            data: MediaContent::Url(String::new()),
            filename: Some("missing.bin".to_string()),
        };

        let error = load_attachment_bytes(&attachment)
            .await
            .expect_err("empty URL should fail parsing");
        assert!(error.to_string().contains("invalid URL"));
    }

    #[tokio::test]
    async fn load_attachment_bytes_rejects_malformed_url() {
        let attachment = MediaAttachment {
            mime_type: "application/octet-stream".to_string(),
            data: MediaContent::Url("https://".to_string()),
            filename: Some("broken.bin".to_string()),
        };

        let error = load_attachment_bytes(&attachment)
            .await
            .expect_err("malformed URL should fail parsing");
        assert!(error.to_string().contains("invalid URL"));
    }

    #[tokio::test]
    async fn load_attachment_bytes_rejects_url_with_credentials() {
        let attachment = MediaAttachment {
            mime_type: "application/octet-stream".to_string(),
            data: MediaContent::Url("https://user:pass@example.com/secret.bin".to_string()),
            filename: Some("secret.bin".to_string()),
        };

        let error = load_attachment_bytes(&attachment)
            .await
            .expect_err("URL userinfo should be rejected");
        assert!(error.to_string().contains("userinfo"));
    }

    #[test]
    fn load_attachment_redirect_statuses_are_rejected_before_body_read() {
        assert!(is_redirect_status(reqwest::StatusCode::MOVED_PERMANENTLY));
        assert!(is_redirect_status(reqwest::StatusCode::FOUND));
        assert!(is_redirect_status(reqwest::StatusCode::TEMPORARY_REDIRECT));
        assert!(!is_redirect_status(reqwest::StatusCode::OK));
    }

    #[tokio::test]
    async fn load_attachment_body_rejects_stream_without_content_length_over_limit() {
        let chunks = [vec![0_u8; MAX_ATTACHMENT_BYTES as usize], vec![1_u8; 1]];

        let error = collect_attachment_chunks_for_test(chunks)
            .await
            .expect_err("streaming body gate should reject as soon as the limit is exceeded");

        assert!(error.to_string().contains("attachment too large"));
    }

    #[tokio::test]
    async fn load_attachment_bytes_rejects_private_rfc1918_10_address() {
        let attachment = MediaAttachment {
            mime_type: "application/octet-stream".to_string(),
            data: MediaContent::Url("http://10.0.0.5/data.bin".to_string()),
            filename: Some("data.bin".to_string()),
        };

        let error = load_attachment_bytes(&attachment)
            .await
            .expect_err("RFC1918 10/8 addresses should be blocked");
        assert!(error.to_string().contains("SSRF blocked"));
    }

    #[tokio::test]
    async fn load_attachment_bytes_rejects_private_rfc1918_172_address() {
        let attachment = MediaAttachment {
            mime_type: "application/octet-stream".to_string(),
            data: MediaContent::Url("http://172.16.0.9/data.bin".to_string()),
            filename: Some("data.bin".to_string()),
        };

        let error = load_attachment_bytes(&attachment)
            .await
            .expect_err("RFC1918 172.16/12 addresses should be blocked");
        assert!(error.to_string().contains("SSRF blocked"));
    }

    #[tokio::test]
    async fn load_attachment_bytes_rejects_private_rfc1918_192_address() {
        let attachment = MediaAttachment {
            mime_type: "application/octet-stream".to_string(),
            data: MediaContent::Url("http://192.168.2.4/data.bin".to_string()),
            filename: Some("data.bin".to_string()),
        };

        let error = load_attachment_bytes(&attachment)
            .await
            .expect_err("RFC1918 192.168/16 addresses should be blocked");
        assert!(error.to_string().contains("SSRF blocked"));
    }

    #[tokio::test]
    async fn output_attachment_to_media_attachment_rejects_oversized_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("huge.bin");
        // Create a file that exceeds MAX_ATTACHMENT_BYTES (25 MiB)
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(MAX_ATTACHMENT_BYTES + 1).unwrap();

        let attachment = OutputAttachment::from_path(
            "application/octet-stream",
            path.to_string_lossy().to_string(),
            Some("huge.bin".to_string()),
        );

        let media = output_attachment_to_media_attachment(&attachment).await;
        assert!(media.is_none(), "oversized file should be rejected");
    }
}
