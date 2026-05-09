//! Subagent spawn limits with lineage walk (WP-J1).
//!
//! Enforces maximum depth and total descendant count for sub-agent trees.
//! Prevents runaway agent spawning. Fail-safe: if lineage cannot be
//! resolved, the spawn is blocked.
//!
//! Design source: ecosystem survey 2026-04-03 (oh-my-openagent
//! subagent spawn limits with lineage walk).
//!
//! ## Wiring status — phase-J
//!
//! **Blocked by:** `SubagentRunner::spawn` integration.
//! **Entry point:** `SubagentRunner::spawn` builds `LineageNode` slices from
//! `SubagentSession::lineage()`, calls `check_spawn_allowed` before creating the
//! child session, and reads `SpawnLimits` from the session config.
//! `SpawnLimits`, `LineageNode`, and `check_spawn_allowed` carry
//! `#[allow(dead_code)]` until that wiring lands.

use std::collections::HashSet;

/// Configuration for spawn limits.
#[derive(Debug, Clone)]
pub(crate) struct SpawnLimits {
    /// Maximum depth of nested sub-agents.
    pub max_depth: usize,
    /// Maximum total descendants across all depths.
    pub max_descendants: usize,
}

impl Default for SpawnLimits {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_descendants: 10,
        }
    }
}

/// A node in the agent lineage tree.
#[derive(Debug, Clone)]
pub(crate) struct LineageNode {
    pub agent_id: String,
    pub parent_id: Option<String>,
    pub depth: usize,
}

/// Check if a new sub-agent spawn is allowed given the current lineage.
///
/// Returns `Ok(depth)` if allowed, or `Err(reason)` if blocked.
pub(crate) fn check_spawn_allowed(
    parent_id: &str,
    lineage: &[LineageNode],
    limits: &SpawnLimits,
) -> Result<usize, String> {
    // Find the parent node
    let parent = lineage
        .iter()
        .find(|n| n.agent_id == parent_id)
        .ok_or_else(|| {
            "parent agent not found in lineage (fail-safe: spawn blocked)".to_string()
        })?;

    // Check for cycles first (prevents infinite loops in descendant counting)
    if has_cycle(parent_id, lineage) {
        return Err("spawn blocked: cycle detected in agent lineage".to_string());
    }

    let new_depth = parent.depth + 1;

    // Check depth limit
    if new_depth > limits.max_depth {
        return Err(format!(
            "spawn blocked: depth {new_depth} exceeds max_depth {}",
            limits.max_depth
        ));
    }

    // Count total descendants
    let descendant_count = count_descendants(parent_id, lineage);
    if descendant_count >= limits.max_descendants {
        return Err(format!(
            "spawn blocked: {descendant_count} descendants reaches max_descendants {}",
            limits.max_descendants
        ));
    }

    Ok(new_depth)
}

/// Count all descendants of a given agent (iterative BFS to avoid stack overflow on cycles).
fn count_descendants(agent_id: &str, lineage: &[LineageNode]) -> usize {
    let mut visited = HashSet::new();
    let mut queue = vec![agent_id.to_string()];
    let mut count = 0;

    while let Some(current) = queue.pop() {
        for node in lineage {
            if node.parent_id.as_deref() == Some(current.as_str())
                && visited.insert(node.agent_id.clone())
            {
                count += 1;
                queue.push(node.agent_id.clone());
            }
        }
    }
    count
}

/// Check if following parent links from the given agent leads to a cycle.
fn has_cycle(start_id: &str, lineage: &[LineageNode]) -> bool {
    let mut visited = HashSet::new();
    let mut current_id = Some(start_id);

    while let Some(id) = current_id {
        if !visited.insert(id.to_string()) {
            return true; // cycle detected
        }
        current_id = lineage
            .iter()
            .find(|n| n.agent_id == id)
            .and_then(|n| n.parent_id.as_deref());
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lineage_chain(depth: usize) -> Vec<LineageNode> {
        let mut nodes = vec![LineageNode {
            agent_id: "root".to_string(),
            parent_id: None,
            depth: 0,
        }];
        for i in 1..=depth {
            nodes.push(LineageNode {
                agent_id: format!("agent-{i}"),
                parent_id: Some(if i == 1 {
                    "root".to_string()
                } else {
                    format!("agent-{}", i - 1)
                }),
                depth: i,
            });
        }
        nodes
    }

    #[test]
    fn allows_spawn_within_limits() {
        let lineage = lineage_chain(1);
        let limits = SpawnLimits::default(); // depth 3, descendants 10
        let result = check_spawn_allowed("agent-1", &lineage, &limits);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 2);
    }

    #[test]
    fn blocks_spawn_at_max_depth() {
        let lineage = lineage_chain(3);
        let limits = SpawnLimits {
            max_depth: 3,
            max_descendants: 100,
        };
        let result = check_spawn_allowed("agent-3", &lineage, &limits);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("max_depth"));
    }

    #[test]
    fn blocks_spawn_at_max_descendants() {
        let mut lineage = vec![LineageNode {
            agent_id: "root".to_string(),
            parent_id: None,
            depth: 0,
        }];
        for i in 1..=5 {
            lineage.push(LineageNode {
                agent_id: format!("child-{i}"),
                parent_id: Some("root".to_string()),
                depth: 1,
            });
        }
        let limits = SpawnLimits {
            max_depth: 10,
            max_descendants: 5,
        };
        let result = check_spawn_allowed("root", &lineage, &limits);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("max_descendants"));
    }

    #[test]
    fn blocks_unknown_parent() {
        let lineage = lineage_chain(1);
        let result = check_spawn_allowed("nonexistent", &lineage, &SpawnLimits::default());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("fail-safe"));
    }

    #[test]
    fn detects_cycle() {
        // Cyclic lineage: a→b→a (both at depth 0 since cycle makes depth meaningless)
        let lineage = vec![
            LineageNode {
                agent_id: "a".to_string(),
                parent_id: Some("b".to_string()),
                depth: 0,
            },
            LineageNode {
                agent_id: "b".to_string(),
                parent_id: Some("a".to_string()),
                depth: 0,
            },
        ];
        let result = check_spawn_allowed("a", &lineage, &SpawnLimits::default());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cycle"));
    }
}
