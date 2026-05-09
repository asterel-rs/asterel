//! Cross-platform best-effort hardening for sensitive local files.

use std::path::Path;

use anyhow::{Context, Result};

/// Restrict a sensitive file to the current local operator account.
///
/// On Unix this applies `0600`. On Windows it removes inherited ACLs and grants
/// full control to the current `%USERNAME%` before the temp file is renamed into
/// place, matching the existing secret-key-file pattern. On other platforms the
/// file write continues with an explicit warning because there is no portable
/// standard-library ACL API.
pub(crate) fn restrict_private_file(path: &Path, description: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).with_context(|| {
            format!(
                "failed to set {description} permissions on '{}': expected 0600",
                path.display()
            )
        })?;
    }

    #[cfg(windows)]
    {
        let username = std::env::var("USERNAME").unwrap_or_default();
        let Some(grant_arg) = windows_icacls_grant_arg(&username) else {
            tracing::warn!(
                path = %path.display(),
                description,
                "USERNAME environment variable is empty; cannot restrict private file permissions via icacls"
            );
            return Ok(());
        };

        let output = std::process::Command::new("icacls")
            .arg(path)
            .args(["/inheritance:r", "/grant:r"])
            .arg(grant_arg)
            .output()
            .with_context(|| {
                format!(
                    "failed to invoke icacls for {description} permissions on '{}'",
                    path.display()
                )
            })?;

        if !output.status.success() {
            anyhow::bail!(
                "failed to set {description} permissions via icacls for '{}': exit code {:?}",
                path.display(),
                output.status.code()
            );
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        tracing::warn!(
            path = %path.display(),
            description,
            "no private-file permission hardening is implemented for this platform"
        );
    }

    Ok(())
}

#[cfg(any(test, windows))]
fn windows_icacls_grant_arg(username: &str) -> Option<String> {
    let normalized = username.trim();
    if normalized.is_empty() {
        return None;
    }
    Some(format!("{normalized}:F"))
}

#[cfg(test)]
mod tests {
    use super::windows_icacls_grant_arg;

    #[test]
    fn windows_icacls_grant_arg_rejects_empty_usernames() {
        assert_eq!(windows_icacls_grant_arg(""), None);
        assert_eq!(windows_icacls_grant_arg("   "), None);
    }

    #[test]
    fn windows_icacls_grant_arg_grants_current_user_full_control() {
        assert_eq!(
            windows_icacls_grant_arg(" alice ").as_deref(),
            Some("alice:F")
        );
    }
}
