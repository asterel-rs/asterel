//! Codespace subsystem — sandboxed development environment for the agent.
//!
//! # What it does
//!
//! The `codespace` tool gives the agent a private multi-project workspace
//! where it can write code, run tests, execute arbitrary commands, and promote
//! successful projects to reusable skills — all without touching the host
//! file system outside the workspace root.
//!
//! # Sandbox model
//!
//! Security is enforced at multiple levels:
//!
//! * **Path confinement** — all file operations resolve paths to a canonical
//!   form and verify they remain within the project directory. Traversal
//!   attempts (`..`) and symlink escapes are rejected before any I/O.
//! * **Shell injection prevention** — commands are tokenized with `shlex`
//!   and validated against a `SHELL_METACHAR` blocklist (`;`, `|`, `&`, `` ` ``,
//!   `$`, newlines, NUL) before being passed to `tokio::process::Command`.
//!   Commands may not start with an environment variable assignment.
//! * **Environment isolation** — child processes inherit only a small
//!   allowlist of environment variables (`PATH`, `HOME`, `TERM`, `LANG`,
//!   `LC_ALL`, `LC_CTYPE`, `USER`, `SHELL`) and have `TMPDIR` redirected
//!   to a project-local `.asterel-tmp` directory.
//! * **Output size caps** — both `stdout` and `stderr` are capped at 1 MB;
//!   excess output is truncated with a warning suffix.
//! * **Project name validation** — names may not be empty, start with `.`,
//!   contain path separators or NUL, or exceed 64 characters.
//! * **Language allowlist** — only languages listed in `CodespaceConfig`
//!   can be used when creating a project.
//! * **Project limit** — the total number of projects in the workspace is
//!   capped at `CodespaceConfig::max_projects`.
//! * **Disk quota** — `write_file` enforces `max_project_size_mb` per project.
//!
//! # Promotion
//!
//! A project can be promoted to a reusable skill via the `promote` action,
//! but only after a successful test run. The promotion writes an
//! `extension.toml` manifest and a `SKILL.md` body to
//! `<workspace>/skills/<skill-id>/`.

mod project;
pub(crate) mod promotion;
pub(super) mod runner;
mod tool;
pub(crate) mod types;

pub use tool::CodespaceTool;
