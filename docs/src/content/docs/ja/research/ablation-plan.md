---
title: Ablation plan
description: Asterel runtime components が companion quality と safety にどう寄与するかを切り分ける planned feature-removal studies。
---

ablation plan は、dataset、provider、model、seed、prompt policy、workspace hygiene を固定したまま、runtime component を一つずつ切り離します。目的は companion runtime のどの部分が効いているかを示すことであり、ある condition が勝つまで調整することではありません。

現在、ablation result は公開していません。このページは intended design を定義し、将来の reports を事前登録された構造に照らして確認できるようにします。

## Common protocol

すべての condition で次を行います。

1. 同じ public commit hash と clean snapshot を使う。
2. model / provider versions、sampling parameters、random seeds を固定する。
3. real user memory を含まない isolated workspace から始める。
4. 同じ benchmark records を同じ順序で実行する。
5. aggregate metrics と failure taxonomies を収集する。
6. full-runtime condition との差分を paired deltas として報告する。
7. typed config switch ではなく code patch が必要だった condition を開示する。

full-runtime condition は release-intended safety と exposure settings を使います。safety checks を無効化する risky ablations は isolated benchmark workspaces の中だけで実行し、shipped defaults にしてはいけません。

## Planned ablations

| ID | Intervention | Implementation sketch | Validates | Primary metrics |
|---|---|---|---|---|
| A0 | Full runtime | memory、exposure control、response finalization、selected character / runtime features を有効にした release-intended config | Reference condition | All metrics |
| A1 | No durable memory writeback | autosave / consolidation paths を無効化し、run ごとに fresh empty workspace を使う。`MemoryBackend::None` は現在 Markdown fallback に流れるため stateless と扱わない。 | durable memory が recall と relationship continuity を改善するか | recall accuracy, stale recall, continuity violations |
| A2 | No graph retrieval fusion | non-graph recall は残し、`memory.graph_retrieval_fusion_enabled = false` にする。 | graph-ranked context が memory relevance と context economy を改善するか | recall precision, prompt budget, latency |
| A3 | No vector retrieval | `memory.embedding_provider = "none"` にし、benchmark adapter が keyword-only recall を支える場合だけ retrieval metrics を再調整する。 | embeddings が keyword / graph context を超えて寄与するか | recall precision / recall, cost, latency |
| A4 | No public/private exposure projection | exposure projection または response-contract blocking を bypass する benchmark-only unsafe condition。明示的な patch が必要で、release config にしてはいけない。 | exposure controls が private-memory leaks を防ぐか | exposure leaks, blocked unsafe output, false blocks |
| A5 | No response finalization / naturalness gate | `persona.enable_response_finalization` と `persona.enable_naturalness_gate` を別々に、さらに同時に無効化する。 | pre-send checks が template tone、over-explanation、memory / internal-state exposure を減らすか | verifier reasons, human naturalness ratings, leak rate |
| A6 | No session control state | `persona.enable_session_control_state` を無効化する。 | thin session state が density、mode continuity、topic follow-through を改善するか | density mismatch, unresolved-loop rate, continuity rubric |
| A7 | No affect topology | `persona.enable_affect_topology` を無効化し、run が使う場合は basic affect inputs を残す。 | topology routing が flat affect labels より context-sensitive expression を改善するか | TRACE / CULEMO-style scores, inappropriate-tone rate |
| A8 | No behavior selector / trait activation | `persona.enable_behavior_selector` と `persona.enable_trait_activation` を別々に無効化する。 | structured behavior selection が prompt-only style より persona coherence を保つか | identity continuity, trait drift, tone jump rate |
| A9 | No soul pressure / self-amendment path | `persona.enable_soul_pressure` と self-amendment candidate / review features を無効化する。 | repair / privacy / autonomy pressure が bounded correction behavior を改善するか | repair rubric, defensiveness, private-memory discretion |

## Local config surfaces

planned ablations は ad hoc code edits より typed configuration を優先します。

- memory: `auto_save`, `embedding_provider`, `graph_retrieval_fusion_enabled`, retrieval weights, isolated workspace setup
- persona / character: `enable_response_finalization`, `enable_naturalness_gate`, `enable_session_control_state`, `enable_affect_topology`, `enable_behavior_selector`, `enable_trait_activation`, `enable_soul_pressure`
- release safety: public / private response contracts と exposure verification は、明示的に unsafe な benchmark-only condition の外では有効に保つ

一部の ablation は benchmark-only patch を必要とするかもしれません。shipped runtime が、危険な挙動の kill switch を意図的に公開していないためです。そのような patch は separate diffs として公開するか、artifact bundle に記録します。

## Reporting format

各 ablation table には次を含めます。

- condition ID と exact config / diff
- dataset または fixture hash
- model / provider / version と sampling parameters
- run seed と workspace initialization procedure
- aggregate score と、可能なら confidence interval または bootstrap interval
- A0 からの paired delta
- failure taxonomy と representative public-safe examples
- condition が release-safe か benchmark-only unsafe かの note

## Interpretation rules

- feature が “helps” と言えるのは、private-memory exposure のような release-critical guardrail を悪化させず target metric を改善した場合だけです。
- feature が “hurts” と言えるのは、一つの provider run ではなく、seeds または paired samples をまたいで degradation が再現した場合だけです。
- feature を無効化して benchmark が良くなっても、public leakage、over-participation、identity drift が増えるなら、それは win ではなく tradeoff です。
