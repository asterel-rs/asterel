---
title: Experimental protocol
description: Runbook for producing paper-facing Asterel evaluation artifacts from a clean public snapshot.
---

This protocol describes the minimum bar for a paper-facing evaluation run. It is
stricter than a normal release check because it must let readers reproduce the
claim, inspect the artifact boundary, and distinguish implementation evidence
from empirical evidence.

## 1. Freeze the snapshot

Record:

- public commit hash;
- clean working tree status;
- Rust, Node, pnpm, Python, OS, and Docker versions where relevant;
- `Cargo.lock`, `pnpm-lock.yaml`, fixture hashes, and benchmark adapter hashes;
- exact config files with secrets removed;
- model/provider names, API versions when available, sampling parameters, and
  random seeds.

The public paper artifact should be based on a clean snapshot repository, not on
private development history containing local agent notes or internal reviews.

## 2. Pre-register claims and metrics

Before running full benchmarks, write the claim in falsifiable form:

| Claim type | Example metric |
|---|---|
| Memory continuity | recall accuracy, correction latency, stale-recall rate |
| Exposure control | private-memory leak rate, safe-block rate, false-block rate |
| Naturalness | verifier reason counts, human naturalness ratings, over-explanation rate |
| Affect calibration | context-sensitivity score, inappropriate-tone rate |
| Security containment | attack success rate, unsafe action completion rate, false-positive block rate |

Do not change the primary metric after seeing the result. Exploratory analyses
may be included, but they must be labeled exploratory.

## 3. Prepare data safely

Allowed inputs:

- public benchmark datasets with confirmed licenses;
- synthetic fixtures committed to the repository;
- consented human-study transcripts collected under the study protocol;
- redacted logs that remove raw private memory, tenant/person IDs, secrets, and
  provider payloads.

Forbidden inputs:

- private Discord logs without explicit consent;
- raw relationship memory or private grounding context;
- unresolved security findings with exploit details;
- provider credentials, OAuth tokens, webhook signatures, pairing tokens;
- local handoff prompts or operator notes.

## 4. Run local implementation gates

Run the implementation evidence before empirical benchmarks:

```bash
cargo fmt -- --check
cargo clippy -- -D warnings
cargo check-all
cargo test
pnpm --dir docs build
cargo metadata --no-deps --format-version 1
```

Then run claim-focused checks from the [evidence ledger](./evidence-ledger/),
including replay fixtures and architecture checks relevant to the paper claim.

## 5. Run benchmark and ablation suites

For each benchmark suite:

1. create an isolated workspace;
2. load only the benchmark's public or synthetic input;
3. run the full-runtime baseline;
4. run planned ablation conditions from the [ablation plan](./ablation-plan/);
5. preserve raw machine-readable results in a private review area;
6. publish only aggregate metrics, public-safe examples, hashes, and redacted
   failure taxonomies.

Provider-backed runs should include retry policy and failure handling. A failed
provider call should be recorded as a run artifact, not silently dropped.

## 6. Human evaluation protocol

When using human raters:

- obtain consent for the study scope and publication boundary;
- avoid real private memories unless the participant explicitly supplied them for
  the study;
- blind raters to condition names where possible;
- use a written rubric, not free-form vibes;
- collect at least two independent ratings per item when feasible;
- report inter-rater agreement or explain why it was not computed;
- separate safety-critical violations from aesthetic preferences.

Suggested rubric dimensions for companion dialogue:

- connection to the preceding turn;
- appropriate response density;
- AI identity honesty;
- public/private distance calibration;
- memory relevance without overexposure;
- repair behavior after correction;
- emotional attunement without overclaiming human experience.

## 7. Security evaluation protocol

Security benchmarks must run in a contained environment:

- no real credentials;
- no production workspaces;
- no real external side effects;
- mocked or disposable tools only;
- explicit allowlist of network targets;
- preserved policy decisions and blocked-action reason codes.

Report both attack success and false-positive safe-blocking. A system that blocks
everything is not a useful companion runtime.

## 8. Artifact layout

Recommended public artifact structure:

```text
artifacts/
  README.md
  environment.json
  configs/
  fixtures-hashes.txt
  benchmark-adapters/
  results/
    aggregate.csv
    aggregate.json
    ablations.csv
    failure-taxonomy.md
  redaction-policy.md
```

Private raw logs may exist during review, but they are not part of the public
artifact unless they are synthetic or fully redacted.

## 9. Report results conservatively

Every result section should include:

- what was run;
- what was not run;
- known excluded tests or datasets;
- model/provider sensitivity caveats;
- whether the result supports an implementation invariant, a fixture-backed
  behavior, or an empirical benchmark conclusion.

If the benchmark data or adapter cannot be redistributed, publish enough hashes,
schema, and commands for a licensed reader to reproduce the run independently.
