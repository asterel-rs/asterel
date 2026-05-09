use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

struct FileCleanup(Vec<PathBuf>);

impl Drop for FileCleanup {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn collect_docs(root: &Path, prefix: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).unwrap_or_else(|error| {
        panic!("failed to read docs dir {}: {error}", root.display());
    }) {
        let entry = entry.expect("read docs entry");
        let path = entry.path();
        let file_type = entry.file_type().expect("read docs entry type");

        if file_type.is_dir() {
            if path == prefix.join("ja") {
                continue;
            }
            collect_docs(&path, prefix, out);
            continue;
        }

        let is_doc = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext == "md" || ext == "mdx");
        if is_doc {
            out.push(path);
        }
    }
}

#[test]
fn japanese_docs_cover_public_english_docs() {
    let docs_root = repo_root().join("docs/src/content/docs");
    let ja_root = docs_root.join("ja");
    let mut english_docs = Vec::new();
    collect_docs(&docs_root, &docs_root, &mut english_docs);

    let mut missing = Vec::new();
    for english_path in english_docs {
        let relative = english_path
            .strip_prefix(&docs_root)
            .expect("english path under docs root");
        let ja_path = ja_root.join(relative);
        if !ja_path.exists() {
            missing.push(relative.display().to_string());
        }
    }

    assert!(
        missing.is_empty(),
        "Japanese docs must cover all public English docs; missing:\n{}",
        missing.join("\n")
    );
}

fn collect_text_files(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", root.display());
    }) {
        let entry = entry.expect("read public file entry");
        let path = entry.path();
        let file_type = entry.file_type().expect("read public file entry type");
        if file_type.is_dir() {
            collect_text_files(&path, out);
            continue;
        }

        let should_scan = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                matches!(
                    ext,
                    "md" | "mdx" | "mjs" | "js" | "json" | "yml" | "yaml" | "toml"
                )
            });
        if should_scan {
            out.push(path);
        }
    }
}

#[test]
fn public_docs_and_repo_front_door_do_not_reference_private_material() {
    let root = repo_root();
    let mut files = Vec::new();
    collect_text_files(&root.join("docs/src/content/docs"), &mut files);
    collect_text_files(&root.join(".github"), &mut files);
    files.extend([
        root.join("README.md"),
        root.join("SECURITY.md"),
        root.join("SUPPORT.md"),
        root.join("CONTRIBUTING.md"),
        root.join("docs/astro.config.mjs"),
        root.join("Cargo.toml"),
        root.join("docs/package.json"),
        root.join("desktop/package.json"),
        root.join("desktop/src-tauri/Cargo.toml"),
    ]);

    let forbidden = [
        "internal-docs",
        "CLAUDE.md",
        "CONTEXT.md",
        ".claude",
        "haru0416-dev",
        "docs.asterel.dev",
        "proprietary",
        "AsteronIris",
    ];

    let mut violations = Vec::new();
    for path in files {
        let content = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "failed to read public release scan file {}: {error}",
                path.display()
            )
        });
        for marker in forbidden {
            if content.contains(marker) {
                violations.push(format!("{} contains {marker}", path.display()));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "public-facing files must not reference private/stale release material:\n{}",
        violations.join("\n")
    );
}

#[test]
fn public_snapshot_script_guards_private_and_generated_paths() {
    let script =
        std::fs::read_to_string(repo_root().join("scripts/release/create_public_snapshot.sh"))
            .expect("read public snapshot script");

    for required in [
        ".git",
        ".claude",
        ".agents",
        ".agent-state",
        "internal-docs",
        "AGENTS.md",
        "CLAUDE.md",
        "CONTEXT.md",
        ".mailmap",
        "docs/.astro",
        "docs/dist",
        "desktop/dist",
        "target",
    ] {
        assert!(
            script.contains(required),
            "public snapshot script must explicitly guard {required}"
        );
    }
}

#[test]
#[cfg(unix)]
fn public_snapshot_excludes_untracked_by_default_and_never_follows_symlinks() {
    let root = repo_root();
    let unique = format!(
        ".public-snapshot-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos()
    );
    let untracked_rel = format!("{unique}.txt");
    let symlink_rel = format!("{unique}-link.txt");
    let untracked_path = root.join(&untracked_rel);
    let symlink_path = root.join(&symlink_rel);
    let _cleanup = FileCleanup(vec![untracked_path.clone(), symlink_path.clone()]);

    std::fs::write(&untracked_path, "untracked public snapshot probe")
        .expect("write untracked probe");
    let secret_dir = tempfile::tempdir().expect("create secret tempdir");
    let secret_path = secret_dir.path().join("outside-secret.txt");
    std::fs::write(&secret_path, "must not be copied through symlink")
        .expect("write external symlink target");
    std::os::unix::fs::symlink(&secret_path, &symlink_path).expect("create untracked symlink");

    let snapshot_root = tempfile::tempdir().expect("create snapshot parent");
    let default_dest = snapshot_root.path().join("default-snapshot");
    let default_output = Command::new(root.join("scripts/release/create_public_snapshot.sh"))
        .arg(&default_dest)
        .current_dir(&root)
        .output()
        .expect("run public snapshot script without untracked opt-in");
    assert!(
        default_output.status.success(),
        "default public snapshot should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&default_output.stdout),
        String::from_utf8_lossy(&default_output.stderr)
    );
    assert!(
        !default_dest.join(&untracked_rel).exists(),
        "public snapshot must not include untracked files by default"
    );
    assert!(
        !default_dest.join(&symlink_rel).exists(),
        "public snapshot must not include untracked symlinks by default"
    );

    let opt_in_dest = snapshot_root.path().join("include-untracked-snapshot");
    let opt_in_output = Command::new(root.join("scripts/release/create_public_snapshot.sh"))
        .arg(&opt_in_dest)
        .arg("--include-untracked")
        .current_dir(&root)
        .output()
        .expect("run public snapshot script with untracked opt-in");
    assert!(
        opt_in_output.status.success(),
        "opt-in public snapshot should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&opt_in_output.stdout),
        String::from_utf8_lossy(&opt_in_output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(opt_in_dest.join(&untracked_rel))
            .expect("untracked regular file should be copied with opt-in"),
        "untracked public snapshot probe"
    );
    assert!(
        !opt_in_dest.join(&symlink_rel).exists(),
        "public snapshot must reject symlinks even when untracked files are opted in"
    );
}

#[test]
fn public_snapshot_git_add_keeps_onboard_templates() {
    let root = repo_root();
    let template = Path::new("src/onboard/templates/AGENTS.md");
    assert!(
        root.join(template).exists(),
        "onboard AGENTS template must be present in the public snapshot"
    );

    let status = Command::new("git")
        .args(["check-ignore", "--no-index", "--quiet"])
        .arg(template)
        .current_dir(root)
        .status()
        .expect("run git check-ignore for onboard AGENTS template");

    assert_eq!(
        status.code(),
        Some(1),
        "onboard AGENTS template must not be ignored by clean public git add"
    );
}
