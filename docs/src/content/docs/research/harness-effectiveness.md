---
title: Harness effectiveness
description: Synthetic harness-off versus harness-on evidence for the companion runtime control layer.
---

This page records what the companion harness changes when a candidate response
already contains known failure modes. It is not a live-model benchmark and it
does not use private chat logs. The fixtures are synthetic and public-safe.

## What is being tested

The ablation compares two paths for the same synthetic candidate response:

- **Harness off** — score the draft response as if it were sent directly.
- **Harness on** — run the response through Asterel's response finalization,
  contract checks, public/private exposure policy, and naturalness gate, then
  score the final response.

This isolates the external control layer described in the companion harness: the
system is not claiming that the base model became better. It is showing which
failure classes the runtime can catch, rewrite, or block before delivery.

## Reproduce the fixture run

```bash
cargo run -- eval harness \
  --fixtures tests/fixtures/harness \
  --output evidence/harness-ablation-report.json
```

The command produces a JSON report with per-fixture off/on rows, verifier reason
codes, and deterministic metrics.

To run the same fixture set with live model-generated drafts, use
`--model-backed` and pin the provider/model/temperature in the command output:

```bash
cargo run -- eval harness \
  --fixtures tests/fixtures/harness \
  --model-backed \
  --provider openrouter \
  --model anthropic/claude-sonnet-4.6 \
  --temperature 0.4 \
  --output evidence/harness-model-backed-report.json
```

This path calls the configured provider once per fixture to produce the raw
draft, then evaluates that generated draft with harness off and harness on. The
report records `methodology="model_backed_synthetic_fixture_ablation"`, provider,
model, and temperature so results can be compared or rerun later.

## Current deterministic injected-failure result

The initial public fixture set contains five synthetic turns across short
support, public/private boundaries, Discord density, and continuity drift.

| Mode | Fixtures | Constraint violations | Template findings | Lecture drift findings | Privacy exposure findings | Surface length violations |
|---|---:|---:|---:|---:|---:|---:|
| off | 5 | 8 | 4 | 1 | 2 | 1 |
| on | 5 | 3 | 0 | 1 | 0 | 1 |

Observed effect in this fixture set:

- public/private exposure violations were blocked before delivery;
- template and boilerplate findings were reduced by deterministic finalization;
- some density and continuity failures remain visible, which is useful evidence
  for the next harness work rather than a reason to hide the result.

## Example: public/private boundary

| Harness off | Harness on | Observed effect |
|---|---|---|
| `DMで話してくれた秘密の件だね...個人情報...` | `I can't share that private detail in this context.` | The public response no longer exposes private-memory markers. |

The important point is not the fallback wording itself. The evidence is that the
runtime detected an exposure violation and replaced the unsafe response before it
could be sent in a public context.

## Example: template cleanup

| Harness off | Harness on | Observed effect |
|---|---|---|
| `いい質問です。疲れているときは...非常に重要です...` | `疲れているときは...重要です...` | Canned lead-in and inflated wording are removed, while the remaining content is preserved. |

## What this does not prove

This evidence does not prove human-level naturalness, broad model quality, or
superiority over other agents. It only supports a narrower claim: for these
synthetic failure cases, the harness reduces specific observable failure classes
before response delivery.

The next stronger step is a larger frozen fixture set with model/provider
versions, seeds or generation settings, and human-readable side-by-side examples.
