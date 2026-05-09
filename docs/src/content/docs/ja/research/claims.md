---
title: Claims
description: Asterel が現在 source、tests、reproducible checks で支えられる、反証可能な公開 claims。
---

各 claim は、反論できる形で書いています。ここでの evidence は、特に明記しない限り repository-local です。将来の paper work では、external benchmarks、ablations、human evaluation を追加する必要があります。

| ID | Claim | Current evidence | Verification |
|---|---|---|---|
| C1 | Asterel は planner-first agent framework ではなく companion runtime である。 | Public docs が companion-centered な product shape を定義し、project policy tests が planner / simulation / evolution mainline surfaces の削除を守る。 | `cargo test --test project` |
| C2 | Gateway HTTP、gateway WebSocket、channel handlers は共有 companion-turn contract に合流する。 | Runtime contract fixtures が tenant scope、session owner scope、directness、route hints、turn evidence について transport surfaces を比較する。 | `cargo test --test runtime companion_turn_contract` |
| C3 | Continuity state は transport-independent である。 | Public layer docs は continuity-bearing state を transports より下に置くことを求め、module-boundary tests は core memory / persona / session layers が transport owners を import することを防ぐ。 | `cargo test --test project module_boundaries` |
| C4 | Memory は transcript cache ではなく、governed continuity infrastructure である。 | Memory tests が provenance、tenant recall、governance、correction / forget behavior、backend parity、consolidation orchestration を cover する。 | `cargo test --test memory` |
| C5 | Public/private exposure control は release criterion である。 | Grounding exposure が prompt text の前に secret recall を抑制し、response-contract tests が public contexts での private-memory exposure を block し、replay fixtures が verifier reasons を追跡する。 | `cargo test companion_grounding_block_suppresses_secret_items_and_reports_exposure --lib`; `cargo test --test project companion_bad_turn_replay_fixture_tracks_verifier_events` |
| C6 | Persona と affect は style text だけではなく、structured runtime inputs である。 | Character-runtime tests が identity continuity、affect / appraisal context、soul-pressure posture、topology routing、writeback injection guards を cover する。 | `cargo test --test eval character_runtime`; `cargo test --lib soul_core` |
| C7 | Security は SaaS isolation ではなく、privileged local edges を囲う single-operator containment である。 | Public security docs が threat model を定義し、runtime / gateway tests が tool injection、per-user ACLs、admin pairing、tenant scope、replay、secret scrubbing を cover する。 | `cargo test --test runtime security_guarantees`; `cargo test --test gateway auth` |
| C8 | Pre-send verification は、turn が送信または記憶される前に relationship continuity を守る。 | Naturalness / response-finalization tests が mechanical output repair、memory / internal-state exposure、streaming suppression、fixture-backed guardrail scoring、bad-turn replay metrics を cover する。 | `cargo test --lib naturalness`; `cargo run -- eval replay --input tests/fixtures/replay/discord_companion_bad_turns.jsonl --suite discord-companion-bad-turns` |

## Evidence levels

- **Implemented invariant:** source と tests が、既知の path について property を示す。
- **Fixture-backed behavior:** synthetic fixtures が代表的な cases を動かす。ただし、広い real-world coverage は主張しない。
- **Operational gate:** CI / release commands が shipping 前に drift を検出する。
- **Research gap:** empirical conclusion として提示する前に、external data、ablation、human evaluation が必要。

## Non-claims

Asterel は現在、次を主張しません。

- 既存の memory-agent systems に対する benchmark superiority
- 検証済みの long-term user wellbeing outcomes
- 完全な multi-tenant SaaS isolation
- すべての social / affective / safety failure mode の完全 coverage
- internal design notes が public evidence であること
