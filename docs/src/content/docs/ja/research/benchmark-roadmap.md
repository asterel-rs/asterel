---
title: Benchmark roadmap
description: Asterel の implementation claims を paper-level empirical evidence にするための planned benchmark tracks。
---

このページは roadmap であり、result report ではありません。Asterel が long-term companion memory、affect-aware conversation、governed agent security について paper-level empirical claims をする前に必要な、external と repository-local の evaluation tracks を示します。

現在の public repository には deterministic tests、replay fixtures、release gates があります。それらは implementation invariants を守ります。ただし、external systems に対する superiority、広い social robustness、long-term human outcomes を証明するものではありません。

## Evaluation tracks

| Track | Question | Candidate external anchors | Current local anchor | Paper-ready output |
|---|---|---|---|---|
| Long-term memory | Asterel は long horizons で relationship memory を recall、update、suppress できるか。 | LongMemEval-style long-term chat memory; PersonaMem-style persona memory if licensing and task fit are confirmed | `tests/memory/*`, memory correction / forget tests, replay fixtures | Per-task accuracy, stale-recall rate, correction latency, exposure-leak rate |
| Relationship continuity | 同じ companion identity が turns、surfaces、repair events をまたいで coherent に保たれるか。 | Consented human study plus synthetic multi-turn persona scenarios | `scripts/eval/persona_eval.py`, `src/core/eval/*` deterministic baseline suites | GCR, identity-continuity violations, repair-quality rubric scores |
| Emotional and social calibration | affect / topology は label へ過剰適合せず、context-sensitive tone を改善するか。 | TRACE for emotion-context sensitivity; CULEMO for cross-cultural emotion understanding; HEART for emotional-support dialogue | affect topology, behavior selector, naturalness gate fixtures | Context-sensitivity score, cultural error taxonomy, support-dialogue human ratings |
| Public/private room behavior | companion は public rooms で sparse / safe / non-invasive のまま、private contexts ではより温かく振る舞えるか。 | Custom Discord-style synthetic corpus plus consented human ratings | public-safe response contracts and bad-turn replay fixtures | Ambient-interruption rate, public intimacy violation rate, private-memory leak rate |
| Agent/security containment | tool、gateway、A2A、external-content paths は prompt-injection と scope-confusion attacks に耐えるか。 | WASP-style web-agent prompt-injection tasks; A2ASecBench-style protocol-aware multi-agent security tasks after task fit is confirmed | `tests/runtime/security_guarantees.rs`, gateway auth / scope tests, replay guard tests | Attack success rate, blocked unsafe action rate, false-positive block rate |

External anchors は、run を公開する前に official paper、dataset、license、evaluation code に対して再確認する必要があります。この roadmap に benchmark 名があるだけでは evidence になりません。

## Integration stages

1. **Inventory and license check** — source URL、version、dataset license、redistribution constraints、task schema、private user data が除外されているかを記録する。
2. **Adapter design** — benchmark の ground truth labels を変えず、private memory を漏らさず、benchmark inputs を Asterel turn records に mapping する。
3. **Dry run** — fixed seeds と isolated workspace で小さな subset を実行する。
4. **Frozen run** — clean public commit 上で、pinned model、provider、toolchain、config、fixture hashes を使い full suite を実行する。
5. **Error audit** — failure を memory、exposure、affect、pickup、security、provider / model sensitivity に分類する。
6. **Public artifact** — aggregate metrics、config、adapter code、hashes、redacted logs を公開する。private transcripts や raw memory payloads は公開しない。

## Metrics to report

一つの広い “quality” score ではなく、metrics を分けて報告します。

- memory precision / recall と stale-recall rate
- correction / forget propagation latency
- private-memory exposure と secret-grounding suppression rate
- public ambient pickup と over-participation rate
- response naturalness verifier reason counts
- affect-context sensitivity と inappropriate-tone rate
- relationship / identity continuity violations
- prompt-injection attack success、safe-block、false-block rates
- cost、latency、retry count、provider / model version

## Current repository-local starting points

- `src/core/eval/harness.rs` の deterministic synthetic baseline suites
- `scripts/eval/persona_eval.py` の multi-turn persona scenario runner
- `tests/fixtures/replay/discord_companion_bad_turns.jsonl` の bad-turn replay fixture
- [evidence ledger](./evidence-ledger/) にある claim-focused checks

## Non-claim rule

frozen runs が存在するまでは、public writing は次のように書きます。

> Asterel has implementation and fixture evidence for governed companion-runtime behavior, and a benchmark plan for empirical evaluation.

次のようには書きません。

> Asterel outperforms other long-term memory agents or emotional-support systems.
