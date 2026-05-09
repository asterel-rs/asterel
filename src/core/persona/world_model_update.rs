//! Incremental update functions for the world model (tool
//! reliability records, project context inference).

use std::path::Path;

use super::world_model::{ProjectContext, ToolReliabilityRecord, WorldModel};

const MAX_TOOL_RECORDS: usize = 20;

/// Update tool reliability records from a batch of tool call outcomes.
///
/// Each tuple is `(tool_name, success, duration_ms)`. Existing records are
/// updated in-place; new tools create fresh entries. When the record list
/// exceeds [`MAX_TOOL_RECORDS`], the least-used tools are dropped.
pub(crate) fn update_tool_reliability(model: &mut WorldModel, tool_calls: &[(String, bool, u64)]) {
    for (name, success, duration_ms) in tool_calls {
        if let Some(rec) = model
            .tool_reliability
            .iter_mut()
            .find(|r| r.tool_name == *name)
        {
            let old_total = rec.success_count + rec.failure_count;
            if *success {
                rec.success_count = rec.success_count.saturating_add(1);
            } else {
                rec.failure_count = rec.failure_count.saturating_add(1);
            }
            let new_total = u64::from(old_total) + 1;
            rec.avg_duration_ms =
                (rec.avg_duration_ms * u64::from(old_total) + duration_ms) / new_total;
        } else {
            model.tool_reliability.push(ToolReliabilityRecord {
                tool_name: name.clone(),
                success_count: u32::from(*success),
                failure_count: u32::from(!*success),
                avg_duration_ms: *duration_ms,
            });
        }
    }
    if model.tool_reliability.len() > MAX_TOOL_RECORDS {
        model.tool_reliability.sort_by(|a, b| {
            let ta = a.success_count + a.failure_count;
            let tb = b.success_count + b.failure_count;
            tb.cmp(&ta)
        });
        model.tool_reliability.truncate(MAX_TOOL_RECORDS);
    }
}

/// Infer project context from common project manifest files.
///
/// Returns `None` if no recognizable project file is found.
pub(crate) fn infer_project_context(workspace_dir: &Path) -> Option<ProjectContext> {
    if workspace_dir.join("Cargo.toml").exists() {
        return Some(ProjectContext {
            language: "Rust".into(),
            framework: None,
            project_type: "cargo".into(),
        });
    }
    if workspace_dir.join("package.json").exists() {
        let lang = if workspace_dir.join("tsconfig.json").exists() {
            "TypeScript"
        } else {
            "JavaScript"
        };
        return Some(ProjectContext {
            language: lang.into(),
            framework: None,
            project_type: "npm".into(),
        });
    }
    if workspace_dir.join("pyproject.toml").exists() {
        return Some(ProjectContext {
            language: "Python".into(),
            framework: None,
            project_type: "python".into(),
        });
    }
    if workspace_dir.join("go.mod").exists() {
        return Some(ProjectContext {
            language: "Go".into(),
            framework: None,
            project_type: "go-module".into(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rec(name: &str, ok: u32, fail: u32, ms: u64) -> ToolReliabilityRecord {
        ToolReliabilityRecord {
            tool_name: name.into(),
            success_count: ok,
            failure_count: fail,
            avg_duration_ms: ms,
        }
    }

    #[test]
    fn new_tool_creates_record() {
        let mut m = WorldModel::default();
        update_tool_reliability(&mut m, &[("shell".into(), true, 100)]);
        assert_eq!(m.tool_reliability.len(), 1);
        let r = &m.tool_reliability[0];
        assert_eq!(
            (r.success_count, r.failure_count, r.avg_duration_ms),
            (1, 0, 100)
        );
    }

    #[test]
    fn existing_tool_updates_counts_and_avg() {
        let mut m = WorldModel {
            tool_reliability: vec![make_rec("shell", 3, 1, 80)],
            ..WorldModel::default()
        };
        update_tool_reliability(&mut m, &[("shell".into(), false, 200)]);
        let r = &m.tool_reliability[0];
        assert_eq!((r.success_count, r.failure_count), (3, 2));
        assert_eq!(r.avg_duration_ms, 104); // (80*4+200)/5
    }

    #[test]
    fn cap_keeps_most_used() {
        let mut m = WorldModel::default();
        m.tool_reliability.push(make_rec("heavy", 100, 0, 50));
        let calls: Vec<_> = (0..20).map(|i| (format!("light_{i}"), true, 10)).collect();
        update_tool_reliability(&mut m, &calls);
        assert!(m.tool_reliability.len() <= MAX_TOOL_RECORDS);
        assert!(m.tool_reliability.iter().any(|r| r.tool_name == "heavy"));
    }

    #[test]
    fn failure_records_zero_successes() {
        let mut m = WorldModel::default();
        update_tool_reliability(&mut m, &[("broken".into(), false, 500)]);
        assert_eq!(m.tool_reliability[0].success_count, 0);
        assert_eq!(m.tool_reliability[0].failure_count, 1);
    }

    #[test]
    fn infer_rust_project() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let ctx = infer_project_context(dir.path()).unwrap();
        assert_eq!(ctx.language, "Rust");
    }

    #[test]
    fn infer_typescript_over_javascript() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        std::fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();
        assert_eq!(
            infer_project_context(dir.path()).unwrap().language,
            "TypeScript"
        );
    }

    #[test]
    fn infer_python_and_go() {
        let py = tempfile::tempdir().expect("temp dir");
        std::fs::write(py.path().join("pyproject.toml"), "").unwrap();
        assert_eq!(infer_project_context(py.path()).unwrap().language, "Python");

        let go = tempfile::tempdir().expect("temp dir");
        std::fs::write(go.path().join("go.mod"), "module example").unwrap();
        assert_eq!(infer_project_context(go.path()).unwrap().language, "Go");
    }

    #[test]
    fn unknown_project_returns_none() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(infer_project_context(dir.path()).is_none());
    }
}
