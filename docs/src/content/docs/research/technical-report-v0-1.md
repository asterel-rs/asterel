---
title: Technical report v0.1
description: A public artifact report for Asterel's Discord-first companion runtime, governed memory, and pre-send verification evidence.
---

**Asterel: A Discord-First Companion Runtime with Governed Memory and Pre-Send Verification**

## Abstract

Asterel is an early-stage, Discord-first AI companion runtime built around a
shared companion-turn pipeline rather than a planner-first agent loop. The system
combines transport-independent continuity state, governed memory writeback,
surface-aware public/private exposure control, and pre-send verification before a
response is delivered or remembered.

This report is a public artifact report, not a finished empirical paper. It
summarizes the current implementation evidence, reproducible local gates, and a
synthetic harness-off versus harness-on ablation. In a five-fixture injected
failure set, the harness reduced observable constraint violations from 8 to 3,
template findings from 4 to 0, and public/private exposure findings from 2 to 0.
These results support a narrow runtime claim: for the tested synthetic failure
classes, Asterel's harness can catch, rewrite, or block specific unsafe or
low-quality response shapes before delivery. They do not show broad model
superiority, long-term user outcomes, or human-level naturalness.

## 1. Motivation

Long-running AI companionship is not only a prompting problem. A companion that
appears in a Discord room must decide when to stay quiet, how much intimacy is
appropriate for a public channel, which memories may be recalled, and whether a
draft response should become part of the relationship history.

Asterel treats those decisions as runtime responsibilities. The product proof is
intentionally narrow: Discord text, durable relationship memory, local operator
governance, and a shared turn contract that other surfaces can reuse without
becoming separate product centers.

## 2. System overview

Asterel's primary runtime path is:

```text
Channel Input -> Pickup Policy -> Turn Enrichment -> Response Assembly -> Pre-send Verification -> Reply Delivery -> Post-turn Update
```

| Runtime area | Role |
|---|---|
| Pickup policy | Decides whether a turn should be answered, ignored, or treated as ambient context. |
| Turn enrichment | Adds affect, memory, persona, session, and policy context before response assembly. |
| Governed memory | Stores continuity state with provenance, privacy levels, correction, and forgetting. |
| Pre-send verification | Checks and finalizes model drafts before delivery and post-turn memory updates. |
| Operator surfaces | Expose governance, diagnostics, pairing, and memory review outside the primary loop. |

Transport-facing execution is centralized in the companion turn service, while
pre/post-turn enrichment remains below transport owners. This keeps Discord,
gateway, and channel handlers thin and prevents each adapter from inventing a
separate companion behavior path.

## 3. Governed memory and exposure control

Asterel treats durable memory as governed continuity infrastructure, not as a raw
transcript cache. Memory entries carry provenance and privacy levels, and memory
writeback is validated before it can alter durable state.

The public release line distinguishes public context, private or direct context,
and secret or sensitive material. Grounding can suppress sensitive recall before
prompt construction, while response-contract checks can block or replace drafts
that would leak private context in a public surface.

The current evidence for this layer is repository-local: memory tests,
governance tests, boundary tests, and bad-turn replay fixtures. This is
implementation evidence, not a claim that all possible social or privacy failures
are covered.

## 4. Companion harness and pre-send verification

The companion harness is the external control layer around a candidate response.
It does not make a base model intrinsically better. It evaluates and finalizes the
draft before delivery.

For this report, the harness ablation compares two paths over the same synthetic
candidate responses:

- **Harness off:** score the draft as if it were sent directly.
- **Harness on:** pass the draft through response finalization, contract checks,
  public/private exposure policy, and naturalness checks before scoring the final
  output.

The harness can reduce specific observable failure classes: public/private memory
exposure, canned lead-ins, template phrasing, overlong replies, internal-state
leakage, and other response shapes that should be repaired before send.

Failures that remain after harness-on scoring are preserved in the report. They
are useful signals for the next fixture and policy work, not results to hide.

## 5. Evaluation method

The current evaluation method is engineering-first. It uses reproducible commands,
synthetic fixtures, and release gates to defend runtime invariants. The method is
designed to make a later empirical study possible without pretending that the
study has already been run.

| Evidence class | Meaning |
|---|---|
| Implemented invariant | Source and tests demonstrate a property for known code paths. |
| Fixture-backed behavior | Synthetic fixtures exercise representative failure classes. |
| Operational gate | Build, lint, test, docs, and snapshot commands catch release drift. |
| Research gap | External data, ablations, or human evaluation are still needed before empirical conclusions. |

The public artifact boundary excludes private Discord logs, raw relationship
memory, provider prompts or responses that include private context, unresolved
security review details, local agent handoff notes, and personal workspace paths.

## 6. Results

### 6.1 Local release gates

The local public-release checks passed before this report was drafted.

<p class="table-summary"><strong>Compact summary:</strong> Rust formatting,
warnings, checks, tests, docs build, desktop verification, package metadata,
Docker Compose validation, and snapshot patch-cleanliness all passed.</p>

| Gate | Result | Scope |
|---|---|---|
| `cargo fmt -- --check` | Pass | Rust formatting |
| `cargo clippy -- -D warnings` | Pass | Rust warnings-as-errors |
| `cargo check-all` | Pass | Repository cargo check alias |
| `cargo test` | Pass | Default Rust test matrix; credential-gated tests remained ignored |
| `pnpm --dir docs build` | Pass | Public documentation build |
| `pnpm exec oxfmt` in `desktop/` | Pass | Desktop formatting |
| `pnpm exec oxlint --react-plugin src` in `desktop/` | Pass | Desktop lint, 0 warnings and 0 errors |
| `pnpm build` in `desktop/` | Pass | Desktop TypeScript and Vite build |
| `cargo metadata --no-deps --format-version 1` in clean snapshot | Pass | Package metadata |
| `docker compose config` in clean snapshot | Pass | Compose configuration |
| `git diff --cached --check` in clean snapshot | Pass | Snapshot patch cleanliness |

The desktop build emitted a non-blocking Vite chunk-size warning for the shared
vendor chunk. The build still completed successfully.

### 6.2 Clean snapshot validation

The public release is intended to start from a clean snapshot rather than the
private development history. The snapshot helper copied the public tracked set to
`/tmp/opencode/asterel-public-blocker-check` and excluded local/internal material
such as agent assets, private design archives, top-level agent notes, session
context notes, and author-history cleanup files.

Top-level local agent notes are excluded, while the public onboarding template is
preserved.

### 6.3 Harness ablation

The deterministic harness ablation uses five public-safe synthetic fixtures. The
fixtures inject known failure modes into candidate responses and compare scoring
before and after harness finalization.

Reproduction command:

```bash
cargo run -- eval harness \
  --fixtures tests/fixtures/harness \
  --output evidence/harness-ablation-report.json
```

Current result:

<p class="table-summary"><strong>Compact summary:</strong> harness-on scoring
reduced total constraint violations from 8 to 3, template findings from 4 to 0,
and privacy exposure findings from 2 to 0 across five fixtures.</p>

| Mode | Fixtures | Constraint violations | Template findings | Lecture drift findings | Privacy exposure findings | Surface length violations |
|---|---:|---:|---:|---:|---:|---:|
| harness off | 5 | 8 | 4 | 1 | 2 | 1 |
| harness on | 5 | 3 | 0 | 1 | 0 | 1 |

Observed effect:

- public/private exposure findings were reduced from 2 to 0;
- template findings were reduced from 4 to 0;
- total observable constraint violations were reduced from 8 to 3;
- lecture drift and one surface-length issue remained visible after harness-on
  scoring.

The result supports a narrow fixture-backed claim: on this synthetic injected
failure set, the harness reduced specific observable failure classes before
response delivery.

## 7. Limitations

This report intentionally stops short of stronger empirical claims.

Known limitations:

- the harness ablation currently uses five synthetic fixtures;
- the deterministic run tests injected candidate failures, not a live population
  of model-generated Discord conversations;
- the model-backed harness path exists but is not treated here as an executed
  result unless a provider-backed run is recorded separately;
- no external memory, affect, social-calibration, or security benchmark result is
  reported here;
- no consented human-rating study is reported here;
- the report does not claim superiority over other long-term memory agents,
  emotional-support systems, or agent-security frameworks.

The correct public wording is therefore:

> Asterel has implementation and fixture evidence for governed companion-runtime
> behavior, and a benchmark plan for empirical evaluation.

The report should not be cited as evidence that Asterel outperforms other systems
or achieves human-level companion quality.

## 8. Reproducibility

The current evidence can be reproduced from a clean public snapshot with the
commands listed in the evidence ledger and the public release gate note.

Core commands:

```bash
cargo fmt -- --check
cargo clippy -- -D warnings
cargo check-all
cargo test
pnpm --dir docs build
cargo metadata --no-deps --format-version 1
docker compose config
cargo run -- eval harness --fixtures tests/fixtures/harness --output evidence/harness-ablation-report.json
```

Desktop commands:

```bash
pnpm exec oxfmt
pnpm exec oxlint --react-plugin src
pnpm build
```

Credential-gated tests that require PostgreSQL or live provider API keys are
ignored by default and should be recorded separately when those environments are
available.

## 9. Next steps

The fastest path from this artifact report toward a stronger paper-style result
is:

1. expand the synthetic harness fixture set from 5 cases to a frozen suite of
   public/private exposure, template, density, continuity, and repair failures;
2. run the existing model-backed harness path with a pinned provider, model,
   temperature, fixture hash, and output report;
3. add benchmark adapters only after dataset licenses and task fit are confirmed;
4. report aggregate metrics, hashes, configs, and public-safe examples without
   publishing private transcripts or raw memory payloads;
5. design consented human-rating studies before making claims about long-term
   social or wellbeing outcomes.
