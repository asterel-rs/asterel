---
title: Benchmark roadmap
description: Planned benchmark tracks for turning Asterel's implementation claims into paper-level empirical evidence.
---

This page is a roadmap, not a result report. It names the external and
repository-local evaluation tracks needed before Asterel can make paper-level
empirical claims about long-term companion memory, affect-aware conversation, and
governed agent security.

The current public repository already has deterministic tests, replay fixtures,
and release gates. Those defend implementation invariants. They do **not** prove
superiority over external systems, broad social robustness, or long-term human
outcomes.

## Evaluation tracks

| Track | Question | Candidate external anchors | Current local anchor | Paper-ready output |
|---|---|---|---|---|
| Long-term memory | Does Asterel recall, update, and suppress relationship memory over long horizons? | LongMemEval-style long-term chat memory; PersonaMem-style persona memory if licensing and task fit are confirmed | `tests/memory/*`, memory correction/forget tests, replay fixtures | Per-task accuracy, stale-recall rate, correction latency, exposure-leak rate |
| Relationship continuity | Does the same companion identity stay coherent across turns, surfaces, and repair events? | Consented human study plus synthetic multi-turn persona scenarios | `scripts/eval/persona_eval.py`, `src/core/eval/*` deterministic baseline suites | GCR, identity-continuity violations, repair-quality rubric scores |
| Emotional and social calibration | Does affect/topology improve context-sensitive tone without overfitting to labels? | TRACE for emotion-context sensitivity; CULEMO for cross-cultural emotion understanding; HEART for emotional-support dialogue | affect topology, behavior selector, naturalness gate fixtures | Context-sensitivity score, cultural error taxonomy, support-dialogue human ratings |
| Public/private room behavior | Does the companion remain sparse, safe, and non-invasive in public rooms while still being warmer in private contexts? | Custom Discord-style synthetic corpus plus consented human ratings | public-safe response contracts and bad-turn replay fixtures | Ambient-interruption rate, public intimacy violation rate, private-memory leak rate |
| Agent/security containment | Do tool, gateway, A2A, and external-content paths resist prompt-injection and scope-confusion attacks? | WASP-style web-agent prompt-injection tasks; A2ASecBench-style protocol-aware multi-agent security tasks after task fit is confirmed | `tests/runtime/security_guarantees.rs`, gateway auth/scope tests, replay guard tests | Attack success rate, blocked unsafe action rate, false-positive block rate |

External anchors must be re-grounded against their official paper, dataset,
license, and evaluation code before a run is published. A benchmark name in this
roadmap is not evidence by itself.

## Integration stages

1. **Inventory and license check** — record source URL, version, dataset license,
   redistribution constraints, task schema, and whether private user data is
   excluded.
2. **Adapter design** — map benchmark inputs into Asterel turn records without
   changing the benchmark's ground truth labels or leaking private memory.
3. **Dry run** — execute a small subset with fixed seeds and an isolated workspace.
4. **Frozen run** — run the full suite on a clean public commit with pinned model,
   provider, toolchain, config, and fixture hashes.
5. **Error audit** — classify failures by memory, exposure, affect, pickup,
   security, or provider/model sensitivity.
6. **Public artifact** — publish aggregate metrics, config, adapter code, hashes,
   and redacted logs. Do not publish private transcripts or raw memory payloads.

## Metrics to report

Use separate metrics instead of one broad "quality" score:

- memory precision/recall and stale-recall rate;
- correction/forget propagation latency;
- private-memory exposure and secret-grounding suppression rate;
- public ambient pickup and over-participation rate;
- response naturalness verifier reason counts;
- affect-context sensitivity and inappropriate-tone rate;
- relationship/identity continuity violations;
- prompt-injection attack success, safe-block, and false-block rates;
- cost, latency, retry count, and provider/model version.

## Current repository-local starting points

- Deterministic synthetic baseline suites in `src/core/eval/harness.rs`.
- Multi-turn persona scenario runner in `scripts/eval/persona_eval.py`.
- Bad-turn replay fixture at
  `tests/fixtures/replay/discord_companion_bad_turns.jsonl`.
- Claim-focused checks listed in the [evidence ledger](./evidence-ledger/).

## Non-claim rule

Until the frozen runs exist, public writing should say:

> Asterel has implementation and fixture evidence for governed companion-runtime
> behavior, and a benchmark plan for empirical evaluation.

It should not say:

> Asterel outperforms other long-term memory agents or emotional-support systems.
