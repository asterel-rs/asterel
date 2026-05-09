//! Distillation update pipeline — experience-to-principle capture.
//!
//! Called once per turn by [`super::post_answer_capture::run_post_answer`] after
//! memory updates have completed. Runs the experience-to-principle path:
//!
//! | Path | Guard | What happens |
//! |------|-------|-------------|
//! | Experience-to-principle trigger | always enabled | Calls [`super::distillation_trigger::maybe_run_distillation`] which checks whether the experience atom count crosses the distillation threshold and, if so, runs the experience clustering + principle synthesis pipeline |
//!
//! Neither path blocks the caller; failures are logged and swallowed.

use super::post_answer_capture::PostAnswerContext;

/// Run the distillation path for the current turn.
pub(super) async fn run_distillation_updates(ctx: &PostAnswerContext<'_>) {
    if let Err(error) = super::distillation_trigger::maybe_run_distillation(
        ctx.mem,
        ctx.entity_id,
        ctx.persona_config,
    )
    .await
    {
        tracing::warn!(%error, "distillation trigger failed");
    }
}
