//! Model download and SHA-256 verification.
//!
//! Only compiled when the `intent-classifier` feature is enabled.

use std::path::Path;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

/// Manifest entry for a model file.
struct ModelFile {
    /// Relative filename inside the models directory.
    filename: &'static str,
    /// `HuggingFace` Hub download URL.
    url: &'static str,
    /// Expected SHA-256 hex digest.
    ///
    /// When `None`, this digest must be supplied via `sha256_env`.
    sha256: Option<&'static str>,
    /// Optional environment variable for runtime SHA-256 override.
    sha256_env: Option<&'static str>,
    /// Integrity workflow required for this upstream artifact.
    integrity_policy: IntegrityPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntegrityPolicy {
    /// The repository owns a static digest. Updating the upstream artifact must
    /// change this manifest entry in source control with review evidence.
    RepositoryPinnedSha,
    /// The operator owns the digest. Auto-download is refused until the
    /// operator supplies an independently verified digest through `sha256_env`.
    OperatorSuppliedSha,
}

const CLASSIFIER_SHA_ENV: &str = "ASTEREL_INTENT_CLASSIFIER_XGB_SHA256";

const MODEL_MANIFEST: &[ModelFile] = &[
    ModelFile {
        filename: "all-MiniLM-L6-v2.onnx",
        url: "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/onnx/model.onnx",
        sha256: Some("6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452"),
        sha256_env: None,
        integrity_policy: IntegrityPolicy::RepositoryPinnedSha,
    },
    ModelFile {
        filename: "tokenizer.json",
        url: "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/tokenizer.json",
        sha256: Some("be50c3628f2bf5bb5e3a7f17b1f74611b2561a3a27eeab05e5aa30f411572037"),
        sha256_env: None,
        integrity_policy: IntegrityPolicy::RepositoryPinnedSha,
    },
    ModelFile {
        filename: "intent-classifier-xgb.onnx",
        url: "https://huggingface.co/asterel/intent-classifier-xgb/resolve/main/model.onnx",
        sha256: None,
        sha256_env: Some(CLASSIFIER_SHA_ENV),
        integrity_policy: IntegrityPolicy::OperatorSuppliedSha,
    },
];

/// Compile-time guard: release builds must not ship malformed static hashes.
#[cfg(not(debug_assertions))]
const _: () = {
    let mut i = 0;
    while i < MODEL_MANIFEST.len() {
        if let Some(sha) = MODEL_MANIFEST[i].sha256 {
            assert!(
                is_valid_sha256_hex_const(sha),
                "MODEL_MANIFEST contains invalid static SHA-256 hash"
            );
        }
        i += 1;
    }
};

#[cfg(not(debug_assertions))]
const fn is_valid_sha256_hex_const(s: &str) -> bool {
    if s.len() != 64 {
        return false;
    }
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if !b.is_ascii_hexdigit() {
            return false;
        }
        i += 1;
    }
    true
}

/// Ensure all model files are present in `models_dir`.
///
/// Downloads missing files from `HuggingFace` Hub and verifies SHA-256 digests.
/// Returns `Ok(true)` if all models are ready, `Ok(false)` if download was
/// skipped because `auto_download` is false.
pub(super) async fn ensure_models(models_dir: &Path, auto_download: bool) -> Result<bool> {
    for entry in MODEL_MANIFEST {
        verify_manifest_integrity_policy(entry)?;
    }

    let missing_files: Vec<&ModelFile> = MODEL_MANIFEST
        .iter()
        .filter(|entry| !models_dir.join(entry.filename).exists())
        .collect();

    if !missing_files.is_empty() {
        if !auto_download {
            tracing::info!(
                dir = %models_dir.display(),
                missing = missing_files.len(),
                "intent classifier models missing and auto_download is disabled"
            );
            return Ok(false);
        }

        tokio::fs::create_dir_all(models_dir)
            .await
            .with_context(|| {
                format!(
                    "failed to create models directory '{}'",
                    models_dir.display()
                )
            })?;

        for entry in missing_files {
            let expected_sha = resolve_expected_sha256(entry)?
                .ok_or_else(|| anyhow::anyhow!(missing_sha_msg(entry)))?;
            let path = models_dir.join(entry.filename);
            download_file(entry.url, &path).await?;
            verify_sha256(&path, &expected_sha)?;
        }
    }

    // Verify all present files, including those that already existed before
    // this run. This catches stale/corrupt model files in partially populated
    // directories.
    for entry in MODEL_MANIFEST {
        let expected_sha = resolve_expected_sha256(entry)?
            .ok_or_else(|| anyhow::anyhow!(missing_sha_msg(entry)))?;
        let path = models_dir.join(entry.filename);
        verify_sha256(&path, &expected_sha)?;
    }

    Ok(true)
}

fn resolve_expected_sha256(entry: &ModelFile) -> Result<Option<String>> {
    if let Some(env_key) = entry.sha256_env
        && let Ok(raw) = std::env::var(env_key)
    {
        let normalized = raw.trim().to_ascii_lowercase();
        validate_sha256_hex(&normalized)
            .with_context(|| format!("{env_key} must be a 64-char hex SHA-256 digest"))?;
        return Ok(Some(normalized));
    }

    Ok(entry.sha256.map(str::to_string))
}

fn validate_sha256_hex(value: &str) -> Result<()> {
    if value.len() != 64 {
        bail!("digest length must be exactly 64 hex characters");
    }
    if !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("digest contains non-hex characters");
    }
    Ok(())
}

fn missing_sha_msg(entry: &ModelFile) -> String {
    if let Some(env_key) = entry.sha256_env {
        return format!(
            "missing checksum for '{}' (set {} to an independently verified SHA-256 before enabling auto-download)",
            entry.filename, env_key
        );
    }
    format!("missing checksum for '{}'", entry.filename)
}

fn verify_manifest_integrity_policy(entry: &ModelFile) -> Result<()> {
    match entry.integrity_policy {
        IntegrityPolicy::RepositoryPinnedSha => {
            if entry.sha256.is_none() || entry.sha256_env.is_some() {
                bail!(
                    "model manifest entry '{}' claims repository-pinned integrity without a static SHA-256",
                    entry.filename
                );
            }
        }
        IntegrityPolicy::OperatorSuppliedSha => {
            if entry.sha256_env.is_none() {
                bail!(
                    "model manifest entry '{}' claims operator-supplied integrity without an environment checksum variable",
                    entry.filename
                );
            }
        }
    }
    Ok(())
}

async fn download_file(url: &str, dest: &Path) -> Result<()> {
    tracing::info!(url, dest = %dest.display(), "downloading model file");

    let response = reqwest::get(url)
        .await
        .with_context(|| format!("HTTP request failed for '{url}'"))?;

    if !response.status().is_success() {
        bail!("download failed for '{}': HTTP {}", url, response.status());
    }

    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("failed to read response body from '{url}'"))?;

    // Atomic write: write to temp then rename
    let tmp_path = dest.with_extension("tmp");
    tokio::fs::write(&tmp_path, &bytes)
        .await
        .with_context(|| format!("failed to write temp file '{}'", tmp_path.display()))?;
    tokio::fs::rename(&tmp_path, dest)
        .await
        .with_context(|| format!("failed to rename temp file to '{}'", dest.display()))?;

    tracing::info!(dest = %dest.display(), bytes = bytes.len(), "model file downloaded");
    Ok(())
}

fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let data = std::fs::read(path)
        .with_context(|| format!("failed to read '{}' for checksum", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let actual = hex::encode(hasher.finalize());

    if actual != expected {
        bail!(
            "SHA-256 mismatch for '{}': expected {expected}, got {actual}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_entries_declare_enforced_integrity_workflow() {
        for entry in MODEL_MANIFEST {
            verify_manifest_integrity_policy(entry).unwrap();
        }
    }

    #[test]
    fn operator_supplied_classifier_checksum_message_requires_independent_verification() {
        let classifier = MODEL_MANIFEST
            .iter()
            .find(|entry| entry.filename == "intent-classifier-xgb.onnx")
            .expect("classifier manifest entry");

        let message = missing_sha_msg(classifier);
        assert!(message.contains("independently verified SHA-256"));
    }
}
