//! Policy rule definitions and rule set loading.

use serde::{Deserialize, Serialize};

/// A single policy rule that matches a tool + subject pattern and declares a decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    /// Tool name pattern. `"*"` matches all tools. `"mcp_*"` matches prefix.
    pub tool: String,
    /// Subject/argument pattern. `"*"` matches all. Path globs supported.
    #[serde(default = "default_subject")]
    pub subject: String,
    /// What to do when this rule matches.
    pub decision: PolicyDecisionRule,
    /// Human-readable reason for this rule (for audit trail).
    #[serde(default)]
    pub reason: String,
}

/// The decision a rule declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecisionRule {
    /// Hard block — tool call is denied.
    Deny,
    /// Force interactive approval — even if grants exist.
    Ask,
    /// Allow without approval prompt.
    Allow,
}

/// An ordered set of policy rules. First matching rule wins.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PolicyRuleSet {
    #[serde(default)]
    pub rules: Vec<PolicyRule>,
}

/// Tool name + subject pattern matcher.
#[derive(Debug, Clone)]
pub struct ToolPattern {
    tool: String,
    subject: String,
}

impl ToolPattern {
    #[must_use]
    pub fn new(tool: &str, subject: &str) -> Self {
        Self {
            tool: tool.to_string(),
            subject: subject.to_string(),
        }
    }

    /// Check if this pattern matches a given tool name and args summary.
    ///
    /// Patterns use glob-style matching: `*` alone matches everything, and a
    /// trailing `*` (e.g., `mcp_*`) matches any tool whose name begins with
    /// that prefix. There is no support for interior wildcards or `?` — only
    /// the exact-match and prefix-match forms are recognised. Subject patterns
    /// follow the same rules applied to the args summary string.
    #[must_use]
    pub fn matches(&self, tool_name: &str, args_summary: &str) -> bool {
        self.matches_tool(tool_name) && self.matches_subject(args_summary)
    }

    fn matches_tool(&self, tool_name: &str) -> bool {
        if self.tool == "*" {
            return true;
        }
        if let Some(prefix) = self.tool.strip_suffix('*') {
            return tool_name.starts_with(prefix);
        }
        self.tool == tool_name
    }

    fn matches_subject(&self, args_summary: &str) -> bool {
        if self.subject == "*" {
            return true;
        }
        if let Some(prefix) = self.subject.strip_suffix('*') {
            return args_summary.starts_with(prefix);
        }
        self.subject == args_summary
    }
}

impl PolicyRule {
    /// Check if this rule matches the given tool call.
    #[must_use]
    pub fn matches(&self, tool_name: &str, args_summary: &str) -> bool {
        let pattern = ToolPattern::new(&self.tool, &self.subject);
        pattern.matches(tool_name, args_summary)
    }
}

impl PolicyRuleSet {
    /// Load rules from a TOML string.
    ///
    /// # Errors
    ///
    /// Returns an error if the TOML is malformed.
    pub fn from_toml(toml_str: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(toml_str)
    }

    /// Load rules from a file path. Returns an empty rule set if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_from_file(path: &std::path::Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read policy file: {e}"))?;
        Self::from_toml(&content).map_err(|e| anyhow::anyhow!("failed to parse policy TOML: {e}"))
    }

    /// Find the first rule whose tool and subject patterns both match the given
    /// tool call, returning a reference to that rule.
    ///
    /// Rules are evaluated in **declaration order** — the order they appear in
    /// the policy file or in the [`PolicyRuleSet::rules`] vec. The first match
    /// wins and subsequent rules are not checked. This means more-specific
    /// rules must be declared before catch-all rules to take effect.
    ///
    /// Returns `None` if no rule matches, which causes the caller (typically
    /// [`crate::security::tool_policy::engine::PolicyEngine`]) to fall through
    /// to the permission-grant and autonomy-fallback stages.
    #[must_use]
    pub fn evaluate(&self, tool_name: &str, args_summary: &str) -> Option<&PolicyRule> {
        self.rules
            .iter()
            .find(|rule| rule.matches(tool_name, args_summary))
    }

    /// Number of rules in the set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Whether the rule set is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

fn default_subject() -> String {
    "*".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_tool_matches_everything() {
        let rule = PolicyRule {
            tool: "*".to_string(),
            subject: "*".to_string(),
            decision: PolicyDecisionRule::Deny,
            reason: "block all".to_string(),
        };
        assert!(rule.matches("shell", "rm -rf /"));
        assert!(rule.matches("file_read", "/etc/passwd"));
    }

    #[test]
    fn prefix_tool_matches() {
        let rule = PolicyRule {
            tool: "mcp_*".to_string(),
            subject: "*".to_string(),
            decision: PolicyDecisionRule::Ask,
            reason: "MCP tools need approval".to_string(),
        };
        assert!(rule.matches("mcp_github", "list repos"));
        assert!(rule.matches("mcp_slack", "send message"));
        assert!(!rule.matches("shell", "echo hello"));
    }

    #[test]
    fn exact_tool_matches() {
        let rule = PolicyRule {
            tool: "shell".to_string(),
            subject: "*".to_string(),
            decision: PolicyDecisionRule::Ask,
            reason: String::new(),
        };
        assert!(rule.matches("shell", "ls"));
        assert!(!rule.matches("file_read", "ls"));
    }

    #[test]
    fn subject_prefix_matches() {
        let rule = PolicyRule {
            tool: "file_read".to_string(),
            subject: "/etc/*".to_string(),
            decision: PolicyDecisionRule::Deny,
            reason: "no system files".to_string(),
        };
        assert!(rule.matches("file_read", "/etc/passwd"));
        assert!(rule.matches("file_read", "/etc/shadow"));
        assert!(!rule.matches("file_read", "/home/user/file.txt"));
    }

    #[test]
    fn first_matching_rule_wins() {
        let rules = PolicyRuleSet {
            rules: vec![
                PolicyRule {
                    tool: "shell".to_string(),
                    subject: "rm *".to_string(),
                    decision: PolicyDecisionRule::Deny,
                    reason: "no rm".to_string(),
                },
                PolicyRule {
                    tool: "shell".to_string(),
                    subject: "*".to_string(),
                    decision: PolicyDecisionRule::Ask,
                    reason: "other shell needs approval".to_string(),
                },
            ],
        };

        let rm_result = rules.evaluate("shell", "rm -rf /tmp");
        assert_eq!(rm_result.unwrap().decision, PolicyDecisionRule::Deny);

        let ls_result = rules.evaluate("shell", "ls -la");
        assert_eq!(ls_result.unwrap().decision, PolicyDecisionRule::Ask);
    }

    #[test]
    fn no_match_returns_none() {
        let rules = PolicyRuleSet {
            rules: vec![PolicyRule {
                tool: "shell".to_string(),
                subject: "*".to_string(),
                decision: PolicyDecisionRule::Deny,
                reason: String::new(),
            }],
        };
        assert!(rules.evaluate("file_read", "/tmp/x").is_none());
    }

    #[test]
    fn toml_roundtrip() {
        let toml_str = r#"
[[rules]]
tool = "shell"
subject = "*"
decision = "ask"
reason = "shell commands require approval"

[[rules]]
tool = "file_read"
subject = "/etc/*"
decision = "deny"
reason = "system config off-limits"

[[rules]]
tool = "memory_recall"
decision = "allow"
reason = "memory recall is always safe"
"#;
        let ruleset = PolicyRuleSet::from_toml(toml_str).expect("valid TOML");
        assert_eq!(ruleset.len(), 3);
        assert_eq!(ruleset.rules[0].tool, "shell");
        assert_eq!(ruleset.rules[0].subject, "*");
        assert_eq!(ruleset.rules[0].decision, PolicyDecisionRule::Ask);
        assert_eq!(ruleset.rules[0].reason, "shell commands require approval");
        assert_eq!(ruleset.rules[1].tool, "file_read");
        assert_eq!(ruleset.rules[1].subject, "/etc/*");
        assert_eq!(ruleset.rules[1].decision, PolicyDecisionRule::Deny);
        assert_eq!(ruleset.rules[2].tool, "memory_recall");
        assert_eq!(ruleset.rules[2].decision, PolicyDecisionRule::Allow);
        assert_eq!(ruleset.rules[2].subject, "*"); // default
    }

    #[test]
    fn empty_file_returns_empty_ruleset() {
        let path = std::path::Path::new("/nonexistent/policy.toml");
        let ruleset = PolicyRuleSet::load_from_file(path).expect("missing file is ok");
        assert!(ruleset.is_empty());
    }
}
