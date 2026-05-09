//! Data types for the codespace subsystem: projects, test results,
//! and the action enum dispatched by the codespace tool.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A codespace project with its metadata and last test state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodespaceProject {
    /// Human-readable project name.
    pub name: String,
    /// Programming language used by the project.
    pub language: String,
    /// Absolute path to the project root directory.
    pub root: PathBuf,
    /// Timestamp when the project was created.
    pub created_at: DateTime<Utc>,
    /// Shell command used to run tests, if configured.
    pub test_command: Option<String>,
    /// Shell command for the project entry point, if configured.
    pub entry_point: Option<String>,
    /// Whether this project has been promoted to a reusable skill.
    pub promoted: bool,
    /// Most recent test result, if any.
    pub last_test_result: Option<TestResult>,
}

/// Result of running a command or test suite in a codespace project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    /// Whether the command exited successfully.
    pub success: bool,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
    /// Process exit code, if available.
    pub exit_code: Option<i32>,
    /// Wall-clock execution time in milliseconds.
    pub duration_ms: u64,
    /// Timestamp when the command was executed.
    pub ran_at: DateTime<Utc>,
}

/// Actions the agent can perform on the codespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum CodespaceAction {
    /// Create a new project with the given language scaffold.
    CreateProject {
        /// Project name.
        name: String,
        /// Programming language.
        language: String,
        /// Optional test command.
        #[serde(default)]
        test_command: Option<String>,
        /// Optional entry point command.
        #[serde(default)]
        entry_point: Option<String>,
    },
    /// List all projects in the workspace.
    ListProjects,
    /// Write content to a file in a project.
    WriteFile {
        /// Target project name.
        project: String,
        /// Relative file path within the project.
        path: String,
        /// File content to write.
        content: String,
    },
    /// Read a file from a project.
    ReadFile {
        /// Target project name.
        project: String,
        /// Relative file path within the project.
        path: String,
    },
    /// Run the project's configured test suite.
    RunTests {
        /// Target project name.
        project: String,
    },
    /// Execute an arbitrary command inside a project directory.
    Exec {
        /// Target project name.
        project: String,
        /// Shell command to execute.
        command: String,
    },
    /// Run the project's configured entry point.
    Run {
        /// Target project name.
        project: String,
        /// Optional arguments to pass to the entry point.
        #[serde(default)]
        args: Option<String>,
    },
    /// Promote a project to a reusable skill.
    Promote {
        /// Target project name.
        project: String,
        /// Name for the promoted skill.
        tool_name: String,
        /// Description for the promoted skill.
        tool_description: String,
    },
    /// Initialize a git repository in a project.
    GitInit {
        /// Target project name.
        project: String,
    },
    /// Run a git sub-command in a project directory.
    Git {
        /// Target project name.
        project: String,
        /// Git sub-command to execute.
        command: String,
    },
    /// Delete a project and its directory.
    DeleteProject {
        /// Target project name.
        project: String,
    },
    /// Get the status of a project.
    Status {
        /// Target project name.
        project: String,
    },
}

#[cfg(test)]
mod tests {
    use super::CodespaceAction;

    #[test]
    fn deserialize_create_project() {
        let json = r#"{"action":"create_project","name":"demo","language":"python"}"#;
        let action: CodespaceAction = serde_json::from_str(json).expect("deserialize");
        match action {
            CodespaceAction::CreateProject { name, language, .. } => {
                assert_eq!(name, "demo");
                assert_eq!(language, "python");
            }
            other => panic!("expected CreateProject, got {other:?}"),
        }
    }

    #[test]
    fn deserialize_write_file() {
        let json =
            r#"{"action":"write_file","project":"demo","path":"src/main.py","content":"print(1)"}"#;
        let action: CodespaceAction = serde_json::from_str(json).expect("deserialize");
        assert!(matches!(action, CodespaceAction::WriteFile { .. }));
    }

    #[test]
    fn deserialize_run_tests() {
        let json = r#"{"action":"run_tests","project":"demo"}"#;
        let action: CodespaceAction = serde_json::from_str(json).expect("deserialize");
        assert!(matches!(action, CodespaceAction::RunTests { .. }));
    }

    #[test]
    fn deserialize_promote() {
        let json = r#"{"action":"promote","project":"demo","tool_name":"my_tool","tool_description":"desc"}"#;
        let action: CodespaceAction = serde_json::from_str(json).expect("deserialize");
        assert!(matches!(action, CodespaceAction::Promote { .. }));
    }
}
