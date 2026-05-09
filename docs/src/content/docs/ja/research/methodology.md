---
title: Methodology
description: private runtime data を露出せず、Asterel が design claims を reproducible evidence に変える方法。
---

Asterel の現在の methodology は engineering-first です。反証可能な runtime property を定義し、それを code boundaries に結びつけ、deterministic tests または fixtures で守ります。paper-level work では、この土台の上に external benchmarks と human evaluation を追加します。

## Claim-to-evidence loop

1. **runtime property を書く。** 例: public channels must not quote private memory.
2. **owner を特定する。** 例: individual channel handlers ではなく、grounding exposure projection と response contract verification。
3. **regression を追加する。** property が bypass されたら test が失敗するようにする。
4. **product-critical な property には release gate を追加する。** 例: project policy tests、replay fixtures、architecture checks。
5. **public-safe evidence だけを記録する。** commands、fixture names、aggregate counts、source paths は公開可能。raw user messages と private memory は公開しない。

## Evidence classes

| Class | Use | Examples |
|---|---|---|
| Boundary tests | ownership と dependency direction が drift しないことを示す。 | `tests/project/module_boundaries.rs`, architecture scripts |
| Contract tests | 複数の surface が同じ turn semantics を保つことを示す。 | `tests/runtime/companion_turn_contract.rs` |
| Governance tests | memory / tool / security state changes が scope され auditable であることを示す。 | `tests/memory/*`, `tests/runtime/security_guarantees.rs` |
| Fixture evals | deterministic quality gates が既知の bad output shapes を捕まえることを示す。 | `tests/fixtures/replay/discord_companion_bad_turns.jsonl` |
| Build metadata checks | release packaging と docs が coherent であることを示す。 | `cargo metadata`, docs build, Docker / Pages checks |

## Current limitations

現在の packet は完全な experimental study ではありません。paper-level components として、次がまだ不足しています。

- LongMemEval-style memory evaluation などの external benchmark runs
- memory、GraphRAG、affect topology、exposure projection、naturalness verification の ablations
- provider / model sensitivity analysis
- inter-rater agreement を伴う consented human ratings
- confidence intervals と failure taxonomies
- fixture hashes と exact commit ID を含む frozen artifact bundle

これらは research tasks であり、repository の public-release blockers ではありません。
