//! CLI-based interactive approval broker.
//!
//! Prompts the operator in the terminal for approve/deny decisions
//! on tool executions that require explicit consent.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;

use crate::security::approval::{
    ApprovalBroker, ApprovalDecision, ApprovalRequest, GrantScope, PermissionGrant,
};

/// Interactive CLI approval broker prompting the operator in the terminal.
pub struct CliApprovalBroker {
    timeout: Duration,
}

impl CliApprovalBroker {
    /// Create a CLI approval broker with the given timeout.
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Create a CLI approval broker with the default 30-second timeout.
    #[must_use]
    pub fn default_timeout() -> Self {
        Self::new(Duration::from_secs(30))
    }
}

impl ApprovalBroker for CliApprovalBroker {
    fn request_approval<'a>(
        &'a self,
        request: &'a ApprovalRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ApprovalDecision>> + Send + 'a>> {
        Box::pin(async move {
            // Print formatted box to stderr
            eprintln!();
            eprintln!("┌─ Tool Approval Required ─────────────────────────");
            eprintln!("│ Tool:    {}", request.tool_name);
            eprintln!("│ Args:    {}", request.args_summary);
            eprintln!("│ Risk:    {:?}", request.risk_level);
            eprintln!("│ Entity:  {}", request.entity_id);
            eprintln!("├──────────────────────────────────────────────────");
            eprintln!("│ [A]llow  [D]eny  Allow [S]ession  Allow [P]ermanent");
            eprintln!("└──────────────────────────────────────────────────");
            eprint!("  > ");

            // Read input with timeout
            let decision = tokio::time::timeout(self.timeout, read_single_char()).await;

            match decision {
                Ok(Ok(ch)) => match ch.to_ascii_lowercase() {
                    'a' => Ok(ApprovalDecision::Approved),
                    'd' => Ok(ApprovalDecision::Denied {
                        reason: "denied by user".to_string(),
                    }),
                    's' => Ok(ApprovalDecision::ApprovedWithGrant(PermissionGrant {
                        tool: request.tool_name.clone(),
                        pattern: request.args_summary.clone(),
                        scope: GrantScope::Session,
                    })),
                    'p' => Ok(ApprovalDecision::ApprovedWithGrant(PermissionGrant {
                        tool: request.tool_name.clone(),
                        pattern: request.args_summary.clone(),
                        scope: GrantScope::Permanent,
                    })),
                    _ => Ok(ApprovalDecision::Denied {
                        reason: format!("unrecognized input: '{ch}'"),
                    }),
                },
                Ok(Err(e)) => Ok(ApprovalDecision::Denied {
                    reason: format!("input error: {e}"),
                }),
                Err(_) => Ok(ApprovalDecision::Denied {
                    reason: "approval timed out".to_string(),
                }),
            }
        })
    }
}

async fn read_single_char() -> Result<char> {
    // Use tokio::task::spawn_blocking since stdin is blocking
    let ch = tokio::task::spawn_blocking(|| {
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        input
            .trim()
            .chars()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no input received"))
    })
    .await??;
    Ok(ch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_approval_broker_new_with_timeout() {
        let broker = CliApprovalBroker::new(Duration::from_secs(60));
        assert_eq!(broker.timeout, Duration::from_secs(60));
    }

    #[test]
    fn cli_approval_broker_default_timeout() {
        let broker = CliApprovalBroker::default_timeout();
        assert_eq!(broker.timeout, Duration::from_secs(30));
    }

    #[test]
    fn cli_approval_broker_has_timeout_field() {
        let broker = CliApprovalBroker::new(Duration::from_secs(45));
        // Verify the broker can be constructed and has the timeout field
        assert_eq!(broker.timeout.as_secs(), 45);
    }
}
