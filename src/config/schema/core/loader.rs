//! Config loading, initialization, persistence, and model-default
//! updates. Handles TOML read/write with same-directory atomic replacement;
//! Unix saves additionally enforce 0o600 file permissions.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::Config;

impl Config {
    fn load_from_path_internal(
        config_path: &Path,
        workspace_dir: &Path,
        validate: bool,
    ) -> Result<Self> {
        let contents = fs::read_to_string(config_path).context("Failed to read config file")?;
        let mut config: Config =
            toml::from_str(&contents).context("Failed to parse config file")?;
        config.config_path = config_path.to_path_buf();
        config.workspace_dir = workspace_dir.to_path_buf();

        let secrets_need_persist = config.decrypt_config_secrets_in_place()?;
        if secrets_need_persist {
            config.save()?;
        }

        config.apply_env_overrides();
        config.validate_model_list_registry()?;
        if validate {
            config.validate_autonomy_controls()?;
        }
        Ok(config)
    }

    /// # Errors
    /// Returns an error if file read, parse, secret decryption, or validation fails.
    pub fn load_from_path(config_path: &Path, workspace_dir: &Path) -> Result<Self> {
        Self::load_from_path_internal(config_path, workspace_dir, true)
    }

    /// # Errors
    /// Returns an error if file read, parse, or secret decryption fails.
    pub fn load_from_path_unvalidated(config_path: &Path, workspace_dir: &Path) -> Result<Self> {
        Self::load_from_path_internal(config_path, workspace_dir, false)
    }

    fn load_or_init_internal(validate: bool) -> Result<Self> {
        let asterel_dir = crate::utils::dirs::asterel_home_dir()?;
        let config_path = asterel_dir.join("config.toml");

        fs::create_dir_all(&asterel_dir).context("Failed to create .asterel directory")?;
        fs::create_dir_all(asterel_dir.join("workspace"))
            .context("Failed to create workspace directory")?;

        if config_path.exists() {
            Self::load_from_path_internal(&config_path, &asterel_dir.join("workspace"), validate)
        } else {
            let mut config = Self {
                config_path,
                workspace_dir: asterel_dir.join("workspace"),
                ..Self::default()
            };
            config.apply_env_overrides();
            if validate {
                config.validate_autonomy_controls()?;
            }
            config.save()?;
            Ok(config)
        }
    }

    /// # Errors
    /// Returns an error if config directory setup, config load, or default initialization fails.
    pub fn load_or_init() -> Result<Self> {
        Self::load_or_init_internal(true)
    }

    /// # Errors
    /// Returns an error if config directory setup, config load, or default initialization fails.
    pub fn load_or_init_unvalidated() -> Result<Self> {
        Self::load_or_init_internal(false)
    }

    /// # Errors
    /// Returns an error if config serialization or file write fails.
    pub fn save(&self) -> Result<()> {
        let persisted = self.config_for_persistence()?;
        let toml_str = toml::to_string_pretty(&persisted).context("Failed to serialize config")?;
        write_config_file_atomically(&self.config_path, &toml_str)?;
        Ok(())
    }

    /// Update the default model (and optionally provider), validate, persist, and
    /// return the effective values.
    ///
    /// # Errors
    ///
    /// Returns an error when the provider string is empty or config persistence fails.
    pub fn update_model_defaults(
        &mut self,
        model: String,
        provider: Option<&str>,
    ) -> Result<(String, Option<String>)> {
        self.default_model = Some(model);
        if let Some(name) = provider {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                anyhow::bail!("--provider cannot be empty");
            }
            self.default_provider = Some(trimmed.to_string());
        }
        self.save()?;
        Ok((
            self.default_model.clone().unwrap_or_default(),
            self.default_provider.clone(),
        ))
    }
}

fn write_config_file_atomically(config_path: &Path, contents: &str) -> Result<()> {
    let tmp_path = config_save_tmp_path(config_path);
    if let Err(error) = write_config_temp_file(&tmp_path, contents) {
        let _ = fs::remove_file(&tmp_path);
        return Err(error);
    }

    if let Err(error) = fs::rename(&tmp_path, config_path).with_context(|| {
        format!(
            "Failed to atomically replace config file: {} -> {}",
            tmp_path.display(),
            config_path.display()
        )
    }) {
        let _ = fs::remove_file(&tmp_path);
        return Err(error);
    }

    sync_config_parent_directory(config_path)?;

    Ok(())
}

fn config_save_tmp_path(config_path: &Path) -> PathBuf {
    let parent = config_parent_dir(config_path);
    let file_name = config_path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("config.toml");
    parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        uuid::Uuid::new_v4().as_hyphenated()
    ))
}

fn config_parent_dir(config_path: &Path) -> &Path {
    match config_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

#[cfg(unix)]
fn write_config_temp_file(tmp_path: &Path, contents: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(tmp_path)
        .with_context(|| {
            format!(
                "Failed to open temporary config file with restricted permissions: {}",
                tmp_path.display()
            )
        })?;
    fs::set_permissions(tmp_path, fs::Permissions::from_mode(0o600)).with_context(|| {
        format!(
            "Failed to set temporary config file permissions on '{}': expected 0600",
            tmp_path.display()
        )
    })?;
    file.write_all(contents.as_bytes()).with_context(|| {
        format!(
            "Failed to write temporary config file: {}",
            tmp_path.display()
        )
    })?;
    file.sync_all().with_context(|| {
        format!(
            "Failed to sync temporary config file: {}",
            tmp_path.display()
        )
    })?;
    Ok(())
}

#[cfg(unix)]
fn sync_config_parent_directory(config_path: &Path) -> Result<()> {
    let parent = config_parent_dir(config_path);
    let dir = fs::File::open(parent).with_context(|| {
        format!(
            "Failed to open config directory for sync: {}",
            parent.display()
        )
    })?;
    dir.sync_all()
        .with_context(|| format!("Failed to sync config directory: {}", parent.display()))
}

#[cfg(not(unix))]
fn sync_config_parent_directory(_config_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(not(unix))]
fn write_config_temp_file(tmp_path: &Path, contents: &str) -> Result<()> {
    use std::io::Write;

    // NOTE: On Windows, file permissions are not restricted here. The config
    // file may be world-readable until the user manually adjusts ACLs. Windows
    // ACL manipulation requires platform-specific APIs (e.g., icacls or
    // SetNamedSecurityInfo) that are out of scope for the current implementation.
    // Users on Windows should ensure their home directory has appropriate permissions.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(tmp_path)
        .with_context(|| {
            format!(
                "Failed to open temporary config file: {}",
                tmp_path.display()
            )
        })?;
    file.write_all(contents.as_bytes()).with_context(|| {
        format!(
            "Failed to write temporary config file: {}",
            tmp_path.display()
        )
    })?;
    file.sync_all().with_context(|| {
        format!(
            "Failed to sync temporary config file: {}",
            tmp_path.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn test_config(config_path: PathBuf, workspace_dir: PathBuf) -> Config {
        Config {
            config_path,
            workspace_dir,
            secrets: crate::config::SecretsConfig { encrypt: false },
            ..Config::default()
        }
    }

    #[cfg(unix)]
    #[test]
    fn save_creates_config_with_restricted_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let config = test_config(config_path.clone(), temp.path().join("workspace"));

        config.save().expect("config save should succeed");

        let mode = fs::metadata(&config_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn config_parent_dir_handles_bare_relative_paths() {
        assert_eq!(config_parent_dir(Path::new("config.toml")), Path::new("."));
        let tmp_path = config_save_tmp_path(Path::new("config.toml"));
        assert_eq!(tmp_path.parent().unwrap(), Path::new("."));
    }

    #[test]
    fn unvalidated_config_load_still_rejects_invalid_model_aliases() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let mut config = test_config(config_path.clone(), temp.path().join("workspace"));
        config.default_model = Some("broken-alias".to_string());
        config.model_list = vec![crate::config::ModelListEntry {
            model_name: "broken-alias".to_string(),
            model: "not-a-provider-model-ref".to_string(),
            api_key: None,
            api_base: None,
        }];
        fs::write(
            &config_path,
            toml::to_string_pretty(&config).expect("serialize config"),
        )
        .expect("write config");

        let error =
            Config::load_from_path_unvalidated(&config_path, &temp.path().join("workspace"))
                .expect_err("invalid model aliases should fail even on unvalidated load");

        let message = format!("{error:#}");
        assert!(
            message.contains("model_list model must use provider/model format"),
            "unexpected error: {message}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn failed_temp_write_preserves_existing_config_file() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let original = "default_provider = \"old\"\n";
        fs::write(&config_path, original).expect("write original config");
        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600))
            .expect("set original permissions");
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o500))
            .expect("make temp dir read-only");

        let mut config = test_config(config_path.clone(), temp.path().join("workspace"));
        config.default_provider = Some("new".to_string());
        let save_result = config.save();

        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700))
            .expect("restore temp dir permissions");

        assert!(save_result.is_err());
        assert_eq!(
            fs::read_to_string(&config_path).expect("read original after failed save"),
            original
        );
    }

    #[test]
    fn failed_rename_cleans_up_temporary_config_file() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        fs::create_dir(&config_path).expect("create directory at config path");

        let config = test_config(config_path.clone(), temp.path().join("workspace"));
        let save_result = config.save();

        assert!(save_result.is_err());
        assert!(config_path.is_dir());
        let leaked_temps = fs::read_dir(temp.path())
            .expect("read temp dir")
            .filter_map(std::result::Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".config.toml.")
            })
            .collect::<Vec<_>>();
        assert!(
            leaked_temps.is_empty(),
            "temporary config files should be cleaned after rename failure"
        );
    }
}
