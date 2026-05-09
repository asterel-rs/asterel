//! Workspace directory resolution helpers.
//!
//! Locates `~/.asterel` using the platform home directory,
//! with a local-directory fallback when home is unavailable.

use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::UserDirs;

/// Return the path to `~/.asterel`.
///
/// # Errors
///
/// Returns an error when the home directory cannot be determined.
pub(crate) fn asterel_home_dir() -> Result<PathBuf> {
    let home = UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .context("Could not find home directory")?;
    Ok(home.join(".asterel"))
}

/// Return `~/.asterel`, falling back to `.asterel` in the current
/// directory when the home directory cannot be determined.
#[must_use]
pub(crate) fn asterel_home_dir_or_local() -> PathBuf {
    asterel_home_dir().unwrap_or_else(|_| PathBuf::from(".asterel"))
}
