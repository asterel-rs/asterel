---
title: Ablation plan
description: Planned feature-removal studies for isolating which Asterel runtime components contribute to companion quality and safety.
---

The ablation plan isolates one runtime component at a time while keeping the
dataset, provider, model, seed, prompt policy, and workspace hygiene fixed. Its
purpose is to show which parts of the companion runtime matter, not to tune one
condition until it wins.

No ablation result is currently published. This page defines the intended design
so future reports can be checked against a preregistered structure.

## Common protocol

For every condition:

1. use the same public commit hash and clean snapshot;
2. pin model/provider versions, sampling parameters, and random seeds;
3. start from an isolated workspace with no real user memory;
4. run the same benchmark records in the same order;
5. collect aggregate metrics and failure taxonomies;
6. report paired deltas against the full-runtime condition;
7. disclose any condition that needed a code patch rather than a typed config
   switch.

The full-runtime condition should use the release-intended safety and exposure
settings. Risky ablations that disable safety checks must run only in isolated
benchmark workspaces and must not become shipped defaults.

## Planned ablations

| ID | Intervention | Implementation sketch | Validates | Primary metrics |
|---|---|---|---|---|
| A0 | Full runtime | Release-intended config with memory, exposure control, response finalization, and selected character/runtime features enabled | Reference condition | All metrics |
| A1 | No durable memory writeback | Disable autosave/consolidation paths and use a fresh empty workspace per run. Do not treat `MemoryBackend::None` as stateless, because it currently routes to the Markdown fallback. | Whether durable memory improves recall and relationship continuity | recall accuracy, stale recall, continuity violations |
| A2 | No graph retrieval fusion | Set `memory.graph_retrieval_fusion_enabled = false` while keeping non-graph recall available. | Whether graph-ranked context improves memory relevance and context economy | recall precision, prompt budget, latency |
| A3 | No vector retrieval | Set `memory.embedding_provider = "none"` and rebalance retrieval metrics only if the benchmark adapter supports keyword-only recall. | Whether embeddings contribute beyond keyword/graph context | recall precision/recall, cost, latency |
| A4 | No public/private exposure projection | Benchmark-only unsafe condition that bypasses exposure projection or response-contract blocking. Requires an explicit patch and must never be a release config. | Whether exposure controls prevent private-memory leaks | exposure leaks, blocked unsafe output, false blocks |
| A5 | No response finalization / naturalness gate | Disable `persona.enable_response_finalization` and `persona.enable_naturalness_gate` separately, then together. | Whether pre-send checks reduce template tone, over-explanation, and memory/internal-state exposure | verifier reasons, human naturalness ratings, leak rate |
| A6 | No session control state | Disable `persona.enable_session_control_state`. | Whether thin session state improves density, mode continuity, and topic follow-through | density mismatch, unresolved-loop rate, continuity rubric |
| A7 | No affect topology | Disable `persona.enable_affect_topology` while keeping basic affect inputs if used by the run. | Whether topology routing improves context-sensitive expression over flat affect labels | TRACE/CULEMO-style scores, inappropriate-tone rate |
| A8 | No behavior selector / trait activation | Disable `persona.enable_behavior_selector` and `persona.enable_trait_activation` separately. | Whether structured behavior selection preserves persona coherence better than prompt-only style | identity continuity, trait drift, tone jump rate |
| A9 | No soul pressure / self-amendment path | Disable `persona.enable_soul_pressure` and self-amendment candidate/review features. | Whether repair/privacy/autonomy pressure improves bounded correction behavior | repair rubric, defensiveness, private-memory discretion |

## Local config surfaces

The planned ablations should prefer typed configuration over ad hoc code edits:

- memory: `auto_save`, `embedding_provider`, `graph_retrieval_fusion_enabled`,
  retrieval weights, and isolated workspace setup;
- persona/character: `enable_response_finalization`, `enable_naturalness_gate`,
  `enable_session_control_state`, `enable_affect_topology`,
  `enable_behavior_selector`, `enable_trait_activation`, `enable_soul_pressure`;
- release safety: public/private response contracts and exposure verification must
  remain enabled outside explicitly unsafe benchmark-only conditions.

Some ablations may still require a benchmark-only patch because the shipped
runtime intentionally does not expose a kill switch for unsafe behavior. Those
patches must be published as separate diffs or recorded in the artifact bundle.

## Reporting format

Each ablation table should include:

- condition ID and exact config/diff;
- dataset or fixture hash;
- model/provider/version and sampling parameters;
- run seed and workspace initialization procedure;
- aggregate score with confidence interval or bootstrap interval where applicable;
- paired delta from A0;
- failure taxonomy and representative public-safe examples;
- note on whether the condition is release-safe or benchmark-only unsafe.

## Interpretation rules

- A feature "helps" only when it improves the target metric without worsening a
  release-critical guardrail such as private-memory exposure.
- A feature "hurts" only when degradation is reproducible across seeds or paired
  samples, not because of one provider run.
- If disabling a feature improves a benchmark but increases public leakage,
  over-participation, or identity drift, the result is a tradeoff, not a win.
