---
title: Claims
description: Falsifiable public claims Asterel can currently defend with source, tests, and reproducible checks.
---

Each claim is written so it can be challenged. Evidence here is repository-local
unless otherwise stated; future paper work should add external benchmarks,
ablations, and human evaluation.

| ID | Claim | Current evidence | Verification |
|---|---|---|---|
| C1 | Asterel is a companion runtime, not a planner-first agent framework. | Public docs define the companion-centered product shape; project policy tests guard removal of planner/simulation/evolution mainline surfaces. | `cargo test --test project` |
| C2 | Gateway HTTP, gateway WebSocket, and channel handlers converge on a shared companion-turn contract. | Runtime contract fixtures compare the transport surfaces for tenant scope, session owner scope, directness, route hints, and turn evidence. | `cargo test --test runtime companion_turn_contract` |
| C3 | Continuity state is transport-independent. | Public layer docs require continuity-bearing state below transports; module-boundary tests prevent core memory/persona/session layers from importing transport owners. | `cargo test --test project module_boundaries` |
| C4 | Memory is governed continuity infrastructure, not a transcript cache. | Memory tests cover provenance, tenant recall, governance, correction/forget behavior, backend parity, and consolidation orchestration. | `cargo test --test memory` |
| C5 | Public/private exposure control is a release criterion. | Grounding exposure suppresses secret recall before prompt text, response-contract tests block private-memory exposure in public contexts, and replay fixtures track verifier reasons. | `cargo test companion_grounding_block_suppresses_secret_items_and_reports_exposure --lib`; `cargo test --test project companion_bad_turn_replay_fixture_tracks_verifier_events` |
| C6 | Persona and affect are structured runtime inputs, not only style text. | Character-runtime tests cover identity continuity, affect/appraisal context, soul-pressure posture, topology routing, and writeback injection guards. | `cargo test --test eval character_runtime`; `cargo test --lib soul_core` |
| C7 | Security is single-operator containment around privileged local edges, not SaaS isolation. | Public security docs define the threat model; runtime and gateway tests cover tool injection, per-user ACLs, admin pairing, tenant scope, replay, and secret scrubbing. | `cargo test --test runtime security_guarantees`; `cargo test --test gateway auth` |
| C8 | Pre-send verification protects relationship continuity before a turn is sent or remembered. | Naturalness/response-finalization tests cover mechanical output repair, memory/internal-state exposure, streaming suppression, fixture-backed guardrail scoring, and bad-turn replay metrics. | `cargo test --lib naturalness`; `cargo run -- eval replay --input tests/fixtures/replay/discord_companion_bad_turns.jsonl --suite discord-companion-bad-turns` |

## Evidence levels

- **Implemented invariant:** source and tests demonstrate the property for known
  paths.
- **Fixture-backed behavior:** synthetic fixtures exercise representative cases,
  but do not claim broad real-world coverage.
- **Operational gate:** CI/release commands detect drift before shipping.
- **Research gap:** requires external data, ablation, or human evaluation before
  being presented as an empirical conclusion.

## Non-claims

Asterel does not currently claim:

- benchmark superiority over existing memory-agent systems;
- validated long-term user wellbeing outcomes;
- complete multi-tenant SaaS isolation;
- complete coverage of every social, affective, or safety failure mode;
- that internal design notes are public evidence.
