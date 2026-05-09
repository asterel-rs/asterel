#!/usr/bin/env bash
set -euo pipefail

# Create a public-safe clean snapshot from the current working tree.
#
# This script copies Git's tracked public file set by default. Untracked files
# can be included only with an explicit opt-in flag, and symlinks are never
# copied because release snapshots must not follow local filesystem pointers.
#
# It also applies an explicit denylist for private, local-only, and generated
# paths. Do not use a raw workspace copy for public repository initialization.

usage() {
    cat <<'USAGE'
Usage: scripts/release/create_public_snapshot.sh <destination> [--dry-run] [--include-untracked]

Creates <destination> as a public-safe snapshot directory. The destination must
not be inside this repository and must not already exist unless it is empty.

Options:
  --dry-run             Validate and print the file count without copying.
  --include-untracked   Also include untracked, non-ignored regular files.
  --help                Show this help.
USAGE
}

DEST=""
DRY_RUN=false
INCLUDE_UNTRACKED=false

while (($#)); do
    case "$1" in
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --include-untracked)
            INCLUDE_UNTRACKED=true
            shift
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        --*)
            echo "error: unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
        *)
            if [[ -n "$DEST" ]]; then
                echo "error: destination specified more than once" >&2
                usage >&2
                exit 2
            fi
            DEST="$1"
            shift
            ;;
    esac
done

if [[ -z "$DEST" ]]; then
    usage >&2
    exit 2
fi

REPO_ROOT="$(git rev-parse --show-toplevel)"
DEST_ABS="$(python -c 'import pathlib, sys; print(pathlib.Path(sys.argv[1]).expanduser().resolve())' "$DEST")"

case "$DEST_ABS" in
    "$REPO_ROOT"|"$REPO_ROOT"/*)
        echo "error: destination must be outside the repository: $DEST_ABS" >&2
        exit 1
        ;;
esac

export REPO_ROOT DEST_ABS DRY_RUN INCLUDE_UNTRACKED

python - <<'PY'
import fnmatch
import os
import shutil
import subprocess
import sys
from pathlib import Path

repo = Path(os.environ["REPO_ROOT"]).resolve()
dest = Path(os.environ["DEST_ABS"]).resolve()
dry_run = os.environ["DRY_RUN"] == "true"
include_untracked = os.environ["INCLUDE_UNTRACKED"] == "true"

deny_patterns = (
    ".git",
    ".git/**",
    ".claude",
    ".claude/**",
    ".agents",
    ".agents/**",
    ".agent-state",
    ".agent-state/**",
    "internal-docs",
    "internal-docs/**",
    "AGENTS.md",
    "CLAUDE.md",
    "CONTEXT.md",
    ".mailmap",
    "docs/.astro",
    "docs/.astro/**",
    "docs/dist",
    "docs/dist/**",
    "desktop/dist",
    "desktop/dist/**",
    "target",
    "target/**",
    "desktop/src-tauri/target",
    "desktop/src-tauri/target/**",
)

for parent in (dest, *dest.parents):
    if parent == repo:
        sys.exit(f"error: destination must be outside repository: {dest}")

if dest.exists() and any(dest.iterdir()):
    sys.exit(f"error: destination exists and is not empty: {dest}")


def git_paths(*args: str) -> list[str]:
    result = subprocess.run(
        ["git", *args],
        cwd=repo,
        check=True,
        stdout=subprocess.PIPE,
    )
    return [part.decode() for part in result.stdout.split(b"\0") if part]


def denied(path: str) -> bool:
    return any(fnmatch.fnmatchcase(path, pattern) for pattern in deny_patterns)


candidate_paths = set(git_paths("ls-files", "-z", "--cached"))
if include_untracked:
    candidate_paths.update(git_paths("ls-files", "-z", "--others", "--exclude-standard"))

copy_paths: list[str] = []
denied_candidates: list[str] = []
for rel in sorted(candidate_paths):
    src = repo / rel
    if src.is_symlink():
        denied_candidates.append(rel)
        continue
    if not src.is_file():
        continue
    if denied(rel):
        denied_candidates.append(rel)
        continue
    copy_paths.append(rel)

if denied_candidates:
    print("info: skipped denied paths:")
    for rel in denied_candidates:
        print(f"  {rel}")

if dry_run:
    print(f"dry-run: would copy {len(copy_paths)} files to {dest}")
    return_code = 0
else:
    dest.mkdir(parents=True, exist_ok=True)
    for rel in copy_paths:
        src = repo / rel
        dst = dest / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dst)
    print(f"created public snapshot: {dest}")
    print(f"copied files: {len(copy_paths)}")
    return_code = 0

sys.exit(return_code)
PY
