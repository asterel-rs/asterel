---
title: Methodology
description: How Asterel turns design claims into reproducible evidence without exposing private runtime data.
---

Asterel's current methodology is engineering-first: define a falsifiable runtime
property, attach it to code boundaries, then guard it with deterministic tests or
fixtures. Paper-level work will add external benchmarks and human evaluation on
top of this base.

## Claim-to-evidence loop

1. **State the runtime property.** Example: public channels must not quote
   private memory.
2. **Identify the owner.** Example: grounding exposure projection and response
   contract verification, not individual channel handlers.
3. **Add a regression.** The test should fail if the property is bypassed.
4. **Add a release gate when the property is product-critical.** Examples include
   project policy tests, replay fixtures, and architecture checks.
5. **Record only public-safe evidence.** Commands, fixture names, aggregate
   counts, and source paths are publishable; raw user messages and private memory
   are not.

## Evidence classes

| Class | Use | Examples |
|---|---|---|
| Boundary tests | Prove ownership and dependency direction do not drift. | `tests/project/module_boundaries.rs`, architecture scripts |
| Contract tests | Prove multiple surfaces preserve the same turn semantics. | `tests/runtime/companion_turn_contract.rs` |
| Governance tests | Prove memory/tool/security state changes are scoped and auditable. | `tests/memory/*`, `tests/runtime/security_guarantees.rs` |
| Fixture evals | Prove deterministic quality gates catch known bad output shapes. | `tests/fixtures/replay/discord_companion_bad_turns.jsonl` |
| Build metadata checks | Prove release packaging and docs remain coherent. | `cargo metadata`, docs build, Docker/Pages checks |

## Current limitations

The current packet is not a complete experimental study. Missing paper-level
components include:

- external benchmark runs such as LongMemEval-style memory evaluation;
- ablations for memory, GraphRAG, affect topology, exposure projection, and
  naturalness verification;
- provider/model sensitivity analysis;
- consented human ratings with inter-rater agreement;
- confidence intervals and failure taxonomies;
- a frozen artifact bundle with fixture hashes and exact commit ID.

These are research tasks, not public-release blockers for the repository itself.
