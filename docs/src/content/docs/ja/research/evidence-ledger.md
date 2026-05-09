---
title: Evidence ledger
description: 現在の implementation claims を再現する public-safe evidence categories と commands。
---

この ledger は、公開 repository から再現できる evidence を列挙します。private operator logs、real chat transcripts、secret-bearing configs、raw internal review notes は含めません。

## Core gates

| Evidence | Command | Supports |
|---|---|---|
| Formatting invariant | `cargo fmt -- --check` | Release hygiene |
| Rust warnings-as-errors | `cargo clippy -- -D warnings` | Baseline code quality |
| Full default test matrix | `cargo test` | Broad regression coverage |
| Project policy tests | `cargo test --test project` | Architecture, release policy, fixture inventory |
| Docs build | `pnpm --dir docs build` | Public documentation coherence |
| Cargo metadata | `cargo metadata --no-deps --format-version 1` | Package license / repository / readme metadata |

## Claim-focused checks

| Claim area | Command |
|---|---|
| Transport turn contract | `cargo test --test runtime companion_turn_contract` |
| Architecture boundaries | `cargo test --test project module_boundaries` |
| Memory behavior | `cargo test --test memory` |
| Runtime security guarantees | `cargo test --test runtime security_guarantees` |
| Gateway auth / scope | `cargo test --test gateway auth` |
| Naturalness / pre-send fixtures | `cargo test --lib naturalness` |
| Companion harness OFF / ON ablation | `cargo run -- eval harness --fixtures tests/fixtures/harness --output evidence/harness-ablation-report.json` |
| Model-backed companion harness ablation | `cargo run -- eval harness --fixtures tests/fixtures/harness --model-backed --provider <provider> --model <model> --temperature 0.4 --output evidence/harness-model-backed-report.json` |
| Bad-turn replay fixture | `cargo run -- eval replay --input tests/fixtures/replay/discord_companion_bad_turns.jsonl --suite discord-companion-bad-turns` |
| Architecture helper scripts | `./scripts/dev/generate_module_map.sh && ./scripts/dev/check_architecture.sh` |

## Publishable result forms

Evidence として公開してよいもの:

- command name、commit hash、toolchain version、pass / fail result
- aggregate counts、fixture IDs、verifier reason names、failure taxonomy
- real user / private memory を含まない synthetic fixture text
- 再配布可能な synthetic または licensed benchmark prompts / scoring rubrics
- tokens、tenant / person IDs、raw memory payloads、provider responses を除いた redacted logs

公開しないもの:

- private Discord logs や real relationship memory
- private context を含む provider prompts / responses
- OAuth / client secrets、provider keys、pairing tokens、webhook signatures
- unresolved security review findings や exploit details
- operator environment を特定する local machine paths
- internal handoff prompts や raw design-debate notes
