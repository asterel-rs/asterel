//! Evaluation subsystem: deterministic harnesses, behavioral suites, and replay.
//!
//! ## Methodology
//!
//! All eval runs use **deterministic seeds** so results are reproducible and comparable across
//! commits (see [`harness::EvalHarness`] and [`rng`]).  Live model calls are avoided in baseline
//! tracking; LLM providers are only used in provider-backed comparison runs.
//!
//! ## Harness types
//!
//! | Harness | Purpose |
//! |---------|---------|
//! | [`harness::EvalHarness`] | Synthetic scenario simulation; trend-tracks key quality dimensions. |
//! | [`behavioral`] | Behavioural rule compliance suites (e.g. tone, refusal, grounding). |
//! | [`replay_harness`] | Replays recorded conversation transcripts to detect regressions. |
//! | [`reliance_drill`] | Injects forced-reliance scenarios and scores escalation behaviour. |
//! | [`memory_bench`] | Memory retrieval precision/recall benchmarks. |
//! | [`persona_consistency`] | Cross-turn persona coherence scoring. |
//!
//! Evidence files (JSON + Markdown) are written by the `write_*_evidence_files` helpers and
//! consumed by the CI release-gate pipeline.

pub mod appraisal_context;
#[allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::missing_errors_doc
)]
pub mod behavioral;
pub mod harness;
pub mod harness_ablation;
pub mod human_grounded;
pub mod memory_bench;
pub mod memory_bench_adapter;
pub mod persona_consistency;
pub(crate) mod presenter;
pub mod reliance_drill;
pub mod replay_harness;
pub mod replay_types;
mod rng;
pub mod rubrics;
pub mod types;

pub use appraisal_context::{
    AppraisalContextCase, AppraisalContextEvalReport, evaluate_appraisal_context,
    validate_appraisal_context_cases,
};
pub use harness::{
    EvalHarness, default_baseline_suites, detect_seed_change_warning, write_evidence_files,
};
pub use harness_ablation::{
    HarnessAblationReport, HarnessAblationRun, HarnessMode, ModelBackedHarnessAblationRequest,
    run_harness_ablation, run_model_backed_harness_ablation, write_harness_ablation_evidence,
};
pub use human_grounded::{
    HumanGroundedEvalCase, HumanGroundedEvalSuite, HumanGroundedRubricItem,
    default_human_grounded_rubric, validate_human_grounded_suite,
};
pub use memory_bench_adapter::{
    MemoryBenchReport, MemoryBenchTrial, evaluate_memory_bench, write_memory_bench_evidence,
};
pub use persona_consistency::{
    PersonaConsistencyReport, evaluate_persona_consistency, score_line_to_line,
    score_prompt_to_line, score_qa_consistency, write_persona_consistency_evidence_files,
};
pub use reliance_drill::{
    RelianceDrillConfig, RelianceDrillResult, evaluate_reliance_drill, should_run_reliance_drill,
};
pub use replay_harness::{run_replay, write_replay_evidence_files};
pub use replay_types::{ReplayEvalReport, ReplayRecord, ReplaySuiteReport};
pub use types::{
    EvalReport, EvalScenarioSpec, EvalSuiteSpec, EvalSuiteSummary, validate_baseline_report_columns,
};
