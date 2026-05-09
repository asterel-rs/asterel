//! Legacy agent-origin cron execution guard.
//!
//! Packet D removes planner-backed cron execution from the primary runtime.
//! Agent-origin jobs are therefore rejected on the mainline scheduler path.

use crate::config::Config;
use crate::contracts::strings::verdicts::SECURITY_POLICY_BLOCK_PREFIX;
use crate::platform::cron::{CronJob, is_legacy_planner_command};
use crate::security::SecurityPolicy;

use super::ROUTE_MARKER_AGENT_BLOCKED;

/// Reject agent-origin cron commands on the primary runtime surface.
pub(super) async fn run_agent_job_command(
    _config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (bool, String) {
    if is_legacy_planner_command(&job.command) {
        return (
            false,
            format!(
                "{ROUTE_MARKER_AGENT_BLOCKED}\n{SECURITY_POLICY_BLOCK_PREFIX}legacy planner cron commands are no longer accepted on the primary runtime"
            ),
        );
    }

    if let Err(output) =
        super::policy::enforce_policy_invariants(security, &job.command, ROUTE_MARKER_AGENT_BLOCKED)
    {
        return (false, output);
    }

    (
        false,
        format!(
            "{ROUTE_MARKER_AGENT_BLOCKED}\n{SECURITY_POLICY_BLOCK_PREFIX}agent-origin cron commands are quarantined from the primary runtime"
        ),
    )
}
