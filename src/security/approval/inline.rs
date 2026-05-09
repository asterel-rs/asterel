//! Shared inline approval polling loop.
//!
//! Provides the timeout-aware polling mechanism used by channel-based
//! approval brokers (Discord, Slack, Telegram, Matrix).

use std::future::Future;
use std::time::Duration;

use anyhow::Result;

use crate::security::approval::ApprovalDecision;

const INLINE_APPROVAL_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Build a standard "approval timed out" denial decision.
#[must_use]
pub fn timed_out_decision() -> ApprovalDecision {
    ApprovalDecision::Denied {
        reason: "approval timed out".to_string(),
    }
}

/// # Errors
///
/// Returns an error when the polling callback returns an error.
pub async fn run_inline_approval<F, Fut>(
    timeout: Duration,
    mut poll_once: F,
) -> Result<ApprovalDecision>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Option<ApprovalDecision>>>,
{
    if timeout.is_zero() {
        return Ok(timed_out_decision());
    }

    let result = tokio::time::timeout(timeout, async {
        loop {
            if let Some(decision) = poll_once().await? {
                return Ok(decision);
            }
            tokio::time::sleep(INLINE_APPROVAL_POLL_INTERVAL).await;
        }
    })
    .await;

    match result {
        Ok(decision) => decision,
        Err(_elapsed) => Ok(timed_out_decision()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::{run_inline_approval, timed_out_decision};
    use crate::security::approval::ApprovalDecision;

    #[test]
    fn timeout_decision_reason_is_stable() {
        assert_eq!(
            timed_out_decision(),
            ApprovalDecision::Denied {
                reason: "approval timed out".to_string()
            }
        );
    }

    #[tokio::test]
    async fn inline_approval_returns_timeout_when_disabled() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        let decision = run_inline_approval(Duration::ZERO, move || {
            observed.fetch_add(1, Ordering::SeqCst);
            async { Ok(Some(ApprovalDecision::Approved)) }
        })
        .await
        .expect("inline approval should return decision");

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(decision, timed_out_decision());
    }

    #[tokio::test]
    async fn inline_approval_returns_first_available_decision() {
        let decision = run_inline_approval(Duration::from_secs(1), || async {
            Ok(Some(ApprovalDecision::Approved))
        })
        .await
        .expect("inline approval should return decision");

        assert_eq!(decision, ApprovalDecision::Approved);
    }

    #[tokio::test]
    async fn inline_approval_times_out_when_no_decision_arrives() {
        let decision = run_inline_approval(Duration::from_millis(5), || async { Ok(None) })
            .await
            .expect("inline approval should return decision");

        assert_eq!(decision, timed_out_decision());
    }
}
