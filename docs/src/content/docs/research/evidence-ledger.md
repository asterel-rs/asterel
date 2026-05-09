---
title: Evidence ledger
description: Public-safe evidence categories and the commands that reproduce the current implementation claims.
---

The ledger lists evidence that can be reproduced from the public repository. It
does not include private operator logs, real chat transcripts, secret-bearing
configs, or raw internal review notes.

## Core gates

| Evidence | Command | Supports |
|---|---|---|
| Formatting invariant | `cargo fmt -- --check` | Release hygiene |
| Rust warnings-as-errors | `cargo clippy -- -D warnings` | Baseline code quality |
| Full default test matrix | `cargo test` | Broad regression coverage |
| Project policy tests | `cargo test --test project` | Architecture, release policy, fixture inventory |
| Docs build | `pnpm --dir docs build` | Public documentation coherence |
| Cargo metadata | `cargo metadata --no-deps --format-version 1` | Package license/repository/readme metadata |

## Claim-focused checks

| Claim area | Command |
|---|---|
| Transport turn contract | `cargo test --test runtime companion_turn_contract` |
| Architecture boundaries | `cargo test --test project module_boundaries` |
| Memory behavior | `cargo test --test memory` |
| Runtime security guarantees | `cargo test --test runtime security_guarantees` |
| Gateway auth/scope | `cargo test --test gateway auth` |
| Naturalness/pre-send fixtures | `cargo test --lib naturalness` |
| Companion harness OFF/ON ablation | `cargo run -- eval harness --fixtures tests/fixtures/harness --output evidence/harness-ablation-report.json` |
| Model-backed companion harness ablation | `cargo run -- eval harness --fixtures tests/fixtures/harness --model-backed --provider <provider> --model <model> --temperature 0.4 --output evidence/harness-model-backed-report.json` |
| Bad-turn replay fixture | `cargo run -- eval replay --input tests/fixtures/replay/discord_companion_bad_turns.jsonl --suite discord-companion-bad-turns` |
| Architecture helper scripts | `./scripts/dev/generate_module_map.sh && ./scripts/dev/check_architecture.sh` |

## Publishable result forms

Publish these as evidence:

- command name, commit hash, toolchain version, and pass/fail result;
- aggregate counts, fixture IDs, verifier reason names, and failure taxonomy;
- synthetic fixture text when it contains no real user/private memory;
- benchmark prompts and scoring rubrics when they are synthetic or licensed for
  redistribution;
- redacted logs that remove tokens, tenant/person IDs, raw memory payloads, and
  provider responses.

Do not publish:

- private Discord logs or real relationship memory;
- provider prompts/responses that include private context;
- OAuth/client secrets, provider keys, pairing tokens, webhook signatures;
- unresolved security review findings or exploit details;
- local machine paths that identify an operator environment;
- internal handoff prompts or raw design-debate notes.
