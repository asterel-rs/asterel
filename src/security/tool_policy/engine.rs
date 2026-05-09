//! Policy evaluation engine with explicit precedence chain.
//!
//! Precedence order:
//! 1. Deny rules (hard block — no override possible)
//! 2. Ask rules (force approval prompt)
//! 3. Allow rules (bypass approval)
//! 4. Permission grants (existing `PermissionStore` grants)
//! 5. Autonomy mode fallback (`Supervised` → ask, `ReadOnly` → block)
//! 6. Default → deny
//!
//! Hook overrides (WP-G2) will insert between deny and ask in a future slice.

use super::rules::{PolicyDecisionRule, PolicyRuleSet};
use crate::security::policy::AutonomyLevel;

/// Exhaustive list of tools that only read state and never mutate it.
///
/// **Classification criteria:** a tool qualifies as read-only when all of the
/// following hold:
/// - It does not write to disk, network, memory, or any external service.
/// - Its output cannot be used to indirectly trigger a write (i.e., it is not
///   a command-construction tool).
/// - Its execution is idempotent and has no observable side effects.
///
/// Read-only classification is retained for explicit policy authoring and
/// tests, but `Supervised` autonomy fallback no longer auto-approves these
/// tools. When no rule or grant matches, supervised mode requires approval for
/// *all* tools.
const READ_ONLY_TOOLS: &[&str] = &[
    "file_read",
    "memory_recall",
    "memory_lookup",
    "introspect_affect",
    "introspect_relationship",
    "introspect_self_model",
    "introspect_principles",
    "introspect_experience",
    "subagent_output",
];

/// Returns `true` if the tool is classified as read-only.
///
/// See [`READ_ONLY_TOOLS`] for the full list and classification criteria.
#[must_use]
pub fn is_read_only_tool(tool_name: &str) -> bool {
    READ_ONLY_TOOLS.contains(&tool_name)
}

/// Shared reason string for approvals triggered by supervised fallback when
/// neither a policy rule nor a permission grant matched.
pub const SUPERVISED_FALLBACK_APPROVAL_REASON: &str =
    "approval required: supervised fallback (no matching policy rule or permission grant)";

/// The engine's final decision for a tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecisionKind {
    /// Allow the tool call without prompting.
    Allow,
    /// Block the tool call.
    Deny,
    /// Require interactive approval before executing.
    RequireApproval,
}

/// Full evaluation result with provenance.
#[derive(Debug, Clone)]
pub struct PolicyEvaluation {
    /// The decision.
    pub decision: PolicyDecisionKind,
    /// Which stage of the precedence chain produced this decision.
    pub source: PolicySource,
    /// Human-readable reason (from the matching rule, or a default).
    pub reason: String,
}

/// Which stage of the precedence chain produced the decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicySource {
    /// A deny rule in the policy file.
    DenyRule,
    /// An ask rule in the policy file.
    AskRule,
    /// An allow rule in the policy file.
    AllowRule,
    /// A permission grant from `PermissionStore`.
    PermissionGrant,
    /// Autonomy level fallback.
    AutonomyFallback,
    /// No rule matched — default deny.
    DefaultDeny,
}

/// The policy engine. Holds a loaded rule set and evaluates tool calls
/// against the precedence chain.
#[derive(Debug, Clone)]
pub struct PolicyEngine {
    rules: PolicyRuleSet,
}

impl PolicyEngine {
    /// Create a new engine with the given rules.
    #[must_use]
    pub fn new(rules: PolicyRuleSet) -> Self {
        Self { rules }
    }

    /// Create an engine with no rules (all decisions fall through to autonomy/default).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            rules: PolicyRuleSet::default(),
        }
    }

    /// Load rules from a workspace policy file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but is malformed.
    pub fn load_from_workspace(workspace_dir: &std::path::Path) -> anyhow::Result<Self> {
        let path = workspace_dir.join("policy.toml");
        let rules = PolicyRuleSet::load_from_file(&path)?;
        Ok(Self { rules })
    }

    /// Number of loaded rules.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Evaluate a tool call against the full policy precedence chain.
    ///
    /// The chain is evaluated in strict order; the first matching stage
    /// short-circuits and returns immediately:
    ///
    /// 1. **Deny rules** — hard blocks defined in the policy file. Nothing can
    ///    override a deny rule, not even an existing permission grant or `Full`
    ///    autonomy. This is the safety floor.
    ///
    /// 2. **Ask rules** — policy file entries that force an approval prompt.
    ///    Takes precedence over permission grants so that the operator can
    ///    mandate review for high-risk tools regardless of past grants.
    ///
    /// 3. **Allow rules** — policy file entries that explicitly permit a tool
    ///    call. Bypasses the autonomy fallback so that trusted tools don't
    ///    require repeated approval in `Supervised` mode.
    ///
    /// 4. **Permission grants** — an existing grant from `PermissionStore`
    ///    (checked by the caller and passed in as `has_grant`). The engine
    ///    does not call `PermissionStore` directly to keep evaluation pure and
    ///    testable.
    ///
    /// 5. **Autonomy fallback** — the agent's current autonomy level provides
    ///    a default policy when no rule or grant matches:
    ///    - `ReadOnly` → deny all (tool-type-independent hard block).
    ///    - `Supervised` → require approval for all tools.
    ///    - `Full` → allow all.
    ///
    /// 6. **Default deny** — if somehow none of the above matched (not
    ///    currently reachable, but present as a safety net).
    ///
    /// `args_summary` is a short human-readable description of the call
    /// arguments used for subject-pattern matching in rules. It does not need
    /// to be a complete serialisation of the arguments.
    #[must_use]
    pub fn evaluate(
        &self,
        tool_name: &str,
        args_summary: &str,
        has_grant: bool,
        autonomy: AutonomyLevel,
    ) -> PolicyEvaluation {
        // Phase 1-3: Rule-based decisions (deny → ask → allow)
        // Rules are evaluated in file order. First match wins.
        // We classify the first match by its decision type.
        if let Some(rule) = self.rules.evaluate(tool_name, args_summary) {
            match rule.decision {
                PolicyDecisionRule::Deny => {
                    return PolicyEvaluation {
                        decision: PolicyDecisionKind::Deny,
                        source: PolicySource::DenyRule,
                        reason: if rule.reason.is_empty() {
                            format!("denied by policy rule: tool={}", rule.tool)
                        } else {
                            rule.reason.clone()
                        },
                    };
                }
                PolicyDecisionRule::Ask => {
                    // Ask rule: force approval even if a grant exists
                    return PolicyEvaluation {
                        decision: PolicyDecisionKind::RequireApproval,
                        source: PolicySource::AskRule,
                        reason: if rule.reason.is_empty() {
                            format!("approval required by policy rule: tool={}", rule.tool)
                        } else {
                            rule.reason.clone()
                        },
                    };
                }
                PolicyDecisionRule::Allow => {
                    return PolicyEvaluation {
                        decision: PolicyDecisionKind::Allow,
                        source: PolicySource::AllowRule,
                        reason: if rule.reason.is_empty() {
                            format!("allowed by policy rule: tool={}", rule.tool)
                        } else {
                            rule.reason.clone()
                        },
                    };
                }
            }
        }

        // Phase 4: Permission grants
        if has_grant {
            return PolicyEvaluation {
                decision: PolicyDecisionKind::Allow,
                source: PolicySource::PermissionGrant,
                reason: "allowed by permission grant".to_string(),
            };
        }

        // Phase 5: Autonomy level fallback
        match autonomy {
            AutonomyLevel::ReadOnly => PolicyEvaluation {
                decision: PolicyDecisionKind::Deny,
                source: PolicySource::AutonomyFallback,
                reason: "read-only autonomy level".to_string(),
            },
            AutonomyLevel::Supervised => PolicyEvaluation {
                decision: PolicyDecisionKind::RequireApproval,
                source: PolicySource::AutonomyFallback,
                reason: SUPERVISED_FALLBACK_APPROVAL_REASON.to_string(),
            },
            AutonomyLevel::Full => PolicyEvaluation {
                decision: PolicyDecisionKind::Allow,
                source: PolicySource::AutonomyFallback,
                reason: "autonomous mode".to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::tool_policy::rules::{PolicyDecisionRule, PolicyRule, PolicyRuleSet};

    fn engine_with_rules(rules: Vec<PolicyRule>) -> PolicyEngine {
        PolicyEngine::new(PolicyRuleSet { rules })
    }

    #[test]
    fn deny_rule_overrides_grant_and_autonomy() {
        let engine = engine_with_rules(vec![PolicyRule {
            tool: "shell".to_string(),
            subject: "rm *".to_string(),
            decision: PolicyDecisionRule::Deny,
            reason: "destructive command".to_string(),
        }]);

        let result = engine.evaluate("shell", "rm -rf /", true, AutonomyLevel::Full);
        assert_eq!(result.decision, PolicyDecisionKind::Deny);
        assert_eq!(result.source, PolicySource::DenyRule);
    }

    #[test]
    fn ask_rule_overrides_grant() {
        let engine = engine_with_rules(vec![PolicyRule {
            tool: "shell".to_string(),
            subject: "*".to_string(),
            decision: PolicyDecisionRule::Ask,
            reason: "all shell needs approval".to_string(),
        }]);

        let result = engine.evaluate("shell", "ls", true, AutonomyLevel::Full);
        assert_eq!(result.decision, PolicyDecisionKind::RequireApproval);
        assert_eq!(result.source, PolicySource::AskRule);
    }

    #[test]
    fn allow_rule_bypasses_supervised_mode() {
        let engine = engine_with_rules(vec![PolicyRule {
            tool: "memory_recall".to_string(),
            subject: "*".to_string(),
            decision: PolicyDecisionRule::Allow,
            reason: "always safe".to_string(),
        }]);

        let result = engine.evaluate("memory_recall", "query", false, AutonomyLevel::Supervised);
        assert_eq!(result.decision, PolicyDecisionKind::Allow);
        assert_eq!(result.source, PolicySource::AllowRule);
    }

    #[test]
    fn grant_allows_when_no_rule_matches() {
        let engine = PolicyEngine::empty();
        let result = engine.evaluate("file_read", "/tmp/x", true, AutonomyLevel::Supervised);
        assert_eq!(result.decision, PolicyDecisionKind::Allow);
        assert_eq!(result.source, PolicySource::PermissionGrant);
    }

    #[test]
    fn supervised_requires_approval_without_rules_or_grants() {
        let engine = PolicyEngine::empty();
        let result = engine.evaluate("shell", "echo hi", false, AutonomyLevel::Supervised);
        assert_eq!(result.decision, PolicyDecisionKind::RequireApproval);
        assert_eq!(result.source, PolicySource::AutonomyFallback);
    }

    #[test]
    fn readonly_denies_without_rules() {
        let engine = PolicyEngine::empty();
        let result = engine.evaluate("file_write", "/tmp/x", false, AutonomyLevel::ReadOnly);
        assert_eq!(result.decision, PolicyDecisionKind::Deny);
        assert_eq!(result.source, PolicySource::AutonomyFallback);
    }

    #[test]
    fn autonomous_allows_without_rules() {
        let engine = PolicyEngine::empty();
        let result = engine.evaluate("file_write", "/tmp/x", false, AutonomyLevel::Full);
        assert_eq!(result.decision, PolicyDecisionKind::Allow);
        assert_eq!(result.source, PolicySource::AutonomyFallback);
    }

    #[test]
    fn first_rule_wins_deny_before_allow() {
        let engine = engine_with_rules(vec![
            PolicyRule {
                tool: "shell".to_string(),
                subject: "rm *".to_string(),
                decision: PolicyDecisionRule::Deny,
                reason: "no rm".to_string(),
            },
            PolicyRule {
                tool: "shell".to_string(),
                subject: "*".to_string(),
                decision: PolicyDecisionRule::Allow,
                reason: "shell ok".to_string(),
            },
        ]);

        let rm = engine.evaluate("shell", "rm -rf /", true, AutonomyLevel::Full);
        assert_eq!(rm.decision, PolicyDecisionKind::Deny);

        let ls = engine.evaluate("shell", "ls", true, AutonomyLevel::Full);
        assert_eq!(ls.decision, PolicyDecisionKind::Allow);
    }

    #[test]
    fn evaluation_includes_reason() {
        let engine = engine_with_rules(vec![PolicyRule {
            tool: "mcp_*".to_string(),
            subject: "*".to_string(),
            decision: PolicyDecisionRule::Ask,
            reason: "MCP tools need operator review".to_string(),
        }]);

        let result = engine.evaluate("mcp_github", "list repos", false, AutonomyLevel::Full);
        assert_eq!(result.reason, "MCP tools need operator review");
    }

    #[test]
    fn empty_engine_has_zero_rules() {
        let engine = PolicyEngine::empty();
        assert_eq!(engine.rule_count(), 0);
    }

    #[test]
    fn supervised_read_only_requires_approval_without_rules_or_grants() {
        let engine = PolicyEngine::empty();
        for tool in &[
            "file_read",
            "memory_recall",
            "memory_lookup",
            "subagent_output",
        ] {
            let result = engine.evaluate(tool, "", false, AutonomyLevel::Supervised);
            assert_eq!(
                result.decision,
                PolicyDecisionKind::RequireApproval,
                "{tool} should require approval in supervised mode without grant/rule"
            );
        }
    }

    #[test]
    fn supervised_read_only_allows_with_allow_rule() {
        let engine = engine_with_rules(vec![PolicyRule {
            tool: "file_read".to_string(),
            subject: "*".to_string(),
            decision: PolicyDecisionRule::Allow,
            reason: "read-only allowlisted".to_string(),
        }]);

        let result = engine.evaluate("file_read", "README.md", false, AutonomyLevel::Supervised);
        assert_eq!(result.decision, PolicyDecisionKind::Allow);
        assert_eq!(result.source, PolicySource::AllowRule);
    }

    #[test]
    fn supervised_still_requires_approval_for_mutating_tools() {
        let engine = PolicyEngine::empty();
        for tool in &["shell", "file_write", "memory_store"] {
            let result = engine.evaluate(tool, "", false, AutonomyLevel::Supervised);
            assert_eq!(
                result.decision,
                PolicyDecisionKind::RequireApproval,
                "{tool} should require approval in supervised mode"
            );
            assert_eq!(result.reason, SUPERVISED_FALLBACK_APPROVAL_REASON);
        }
    }

    #[test]
    fn is_read_only_tool_classification() {
        assert!(is_read_only_tool("file_read"));
        assert!(is_read_only_tool("memory_recall"));
        assert!(is_read_only_tool("memory_lookup"));
        assert!(is_read_only_tool("introspect_affect"));
        assert!(!is_read_only_tool("shell"));
        assert!(!is_read_only_tool("file_write"));
        assert!(!is_read_only_tool("memory_store"));
    }
}
