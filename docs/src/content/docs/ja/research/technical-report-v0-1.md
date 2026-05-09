---
title: Technical report v0.1
description: Asterel の Discord-first companion runtime、governed memory、pre-send verification evidence をまとめた public artifact report。
---

**Asterel: Discord-first companion runtime と governed memory / pre-send verification**

## Abstract

Asterel は、planner-first な agent loop ではなく、shared companion-turn pipeline を中心にした early-stage の Discord-first AI companion runtime です。transport に依存しない continuity state、governed memory writeback、public / private exposure control、pre-send verification を組み合わせ、response が送信・記憶される前に runtime 側で確認します。

この文書は、完成した empirical paper ではありません。public artifact report として、現在の implementation evidence、再現可能な local gates、synthetic な harness-off / harness-on ablation をまとめます。5 件の injected-failure fixture では、harness によって observable constraint violations が 8 から 3 に、template findings が 4 から 0 に、public / private exposure findings が 2 から 0 に減りました。

この結果が支えるのは狭い runtime claim です。今回の synthetic failure classes では、Asterel の harness が一部の unsafe または低品質な response shape を delivery 前に検出・修正・ブロックできる、という claim です。model quality 全般、長期的な user outcome、human-level naturalness、他システムへの superiority は主張しません。

## 1. Motivation

長く続く AI companionship は、prompt だけの問題ではありません。Discord room にいる companion は、いつ黙るか、public channel でどの距離感まで許されるか、どの memory を recall してよいか、draft response を relationship history に入れてよいかを判断する必要があります。

Asterel は、それらを runtime の責任として扱います。現在の product proof はあえて狭くしています。Discord text、durable relationship memory、local operator governance、そして他の surface も再利用できる shared turn contract が中心です。

## 2. System overview

Asterel の primary runtime path は次の形です。

```text
Channel Input -> Pickup Policy -> Turn Enrichment -> Response Assembly -> Pre-send Verification -> Reply Delivery -> Post-turn Update
```

| Runtime area | Role |
|---|---|
| Pickup policy | 応答するか、黙るか、ambient context として扱うかを決める。 |
| Turn enrichment | response assembly の前に affect、memory、persona、session、policy context を加える。 |
| Governed memory | provenance、privacy levels、correction、forgetting を持つ continuity state を保存する。 |
| Pre-send verification | model draft を delivery と post-turn memory update の前に確認・finalize する。 |
| Operator surfaces | governance、diagnostics、pairing、memory review を primary loop の外に出す。 |

transport-facing execution は companion turn service に集約しています。pre/post-turn enrichment は transport owners より下の層に置きます。これにより、Discord、gateway、channel handlers がそれぞれ別の companion behavior path を持たないようにしています。

## 3. Governed memory and exposure control

Asterel は durable memory を raw transcript cache ではなく、governed continuity infrastructure として扱います。memory entries は provenance と privacy levels を持ち、memory writeback は durable state を変える前に validation されます。

public release line では、public context、private or direct context、secret or sensitive material を分けます。runtime は model generation の前後で exposure control を行います。prompt construction 前に sensitive recall を抑制し、response-contract checks で public surface に private context を漏らす draft を block または replace します。

この layer の現在の evidence は repository-local です。memory tests、governance tests、boundary tests、bad-turn replay fixtures が中心です。これは implementation evidence であり、あらゆる social / privacy failure を cover したという claim ではありません。

## 4. Companion harness and pre-send verification

companion harness は candidate response の外側にある control layer です。base model 自体が賢くなったと主張するものではありません。delivery 前に draft を評価し、finalize します。

この report の ablation では、同じ synthetic candidate responses に対して二つの path を比較します。

- **Harness off:** draft がそのまま送信されたものとして score する。
- **Harness on:** response finalization、contract checks、public / private exposure policy、naturalness checks を通してから final output を score する。

harness は public / private memory exposure、canned lead-ins、template phrasing、長すぎる reply、internal-state leakage、send 前に repair すべき response shapes を減らせます。

harness-on 後に残った failures は report に残します。それは隠すべき結果ではなく、次の fixture と policy work のための signal です。

## 5. Evaluation method

現在の evaluation method は engineering-first です。再現可能な commands、synthetic fixtures、release gates によって runtime invariants を守ります。後の empirical study に進めるための土台であり、すでに study が完了したようには書きません。

| Evidence class | Meaning |
|---|---|
| Implemented invariant | Source と tests が known code paths の property を示す。 |
| Fixture-backed behavior | Synthetic fixtures が representative failure classes を exercise する。 |
| Operational gate | Build、lint、test、docs、snapshot commands が release drift を検出する。 |
| Research gap | empirical conclusions の前に external data、ablations、human evaluation が必要なもの。 |

public artifact boundary では、private Discord logs、raw relationship memory、private context を含む provider prompts / responses、未解決の security review details、local agent handoff notes、personal workspace paths を除外します。

## 6. Results

### 6.1 Local release gates

この report の作成前に、local public-release checks は通過しました。

<p class="table-summary"><strong>要約:</strong> Rust formatting、warnings、checks、tests、docs build、desktop verification、package metadata、Docker Compose validation、snapshot patch-cleanliness はすべて pass しました。</p>

| Gate | Result | Scope |
|---|---|---|
| `cargo fmt -- --check` | Pass | Rust formatting |
| `cargo clippy -- -D warnings` | Pass | Rust warnings-as-errors |
| `cargo check-all` | Pass | Repository cargo check alias |
| `cargo test` | Pass | Default Rust test matrix。credential-gated tests は marked ignored のまま。 |
| `pnpm --dir docs build` | Pass | Public documentation build |
| `pnpm exec oxfmt` in `desktop/` | Pass | Desktop formatting |
| `pnpm exec oxlint --react-plugin src` in `desktop/` | Pass | Desktop lint。0 warnings / 0 errors。 |
| `pnpm build` in `desktop/` | Pass | Desktop TypeScript and Vite build |
| `cargo metadata --no-deps --format-version 1` in clean snapshot | Pass | Package metadata |
| `docker compose config` in clean snapshot | Pass | Compose configuration |
| `git diff --cached --check` in clean snapshot | Pass | Snapshot patch cleanliness |

desktop build では shared vendor chunk に関する Vite warning が出ました。これは build failure ではありません。

### 6.2 Clean snapshot validation

public release は private development history ではなく clean snapshot から始める前提です。snapshot helper は public tracked set を `/tmp/opencode/asterel-public-blocker-check` に copy し、local agent assets、private design archive、top-level agent notes、session context notes、author-history cleanup files などを除外しました。

一方で、public onboarding template は snapshot に残しています。

### 6.3 Harness ablation

deterministic harness ablation は、public-safe な 5 件の synthetic fixtures を使います。fixture は known failure modes を candidate responses に注入し、harness finalization の前後で score を比較します。

Reproduction command:

```bash
cargo run -- eval harness \
  --fixtures tests/fixtures/harness \
  --output evidence/harness-ablation-report.json
```

Current result:

<p class="table-summary"><strong>要約:</strong> 5 件の fixture で、harness-on scoring は total constraint violations を 8 から 3 に、template findings を 4 から 0 に、privacy exposure findings を 2 から 0 に減らしました。</p>

| Mode | Fixtures | Constraint violations | Template findings | Lecture drift findings | Privacy exposure findings | Surface length violations |
|---|---:|---:|---:|---:|---:|---:|
| harness off | 5 | 8 | 4 | 1 | 2 | 1 |
| harness on | 5 | 3 | 0 | 1 | 0 | 1 |

Observed effect:

- public / private exposure findings は 2 から 0 に減った。
- template findings は 4 から 0 に減った。
- total observable constraint violations は 8 から 3 に減った。
- lecture drift と surface-length issue は一部残った。

この結果が支えるのは、狭い fixture-backed claim です。この synthetic injected-failure set では、harness が response delivery 前に一部の observable failure classes を減らしました。

## 7. Limitations

この report は、強い empirical claims までは踏み込みません。

Known limitations:

- harness ablation は現在 5 件の synthetic fixtures に基づく。
- deterministic run は injected candidate failures を見るもので、live Discord conversations の model-generated population ではない。
- model-backed harness path は存在するが、provider-backed run を別途記録しない限り、ここでは executed result として扱わない。
- external memory、affect、social-calibration、security benchmark results はまだない。
- consented human-rating study はまだない。
- other long-term memory agents、emotional-support systems、agent-security frameworks への superiority は主張しない。

public wording として正しいのは次の形です。

> Asterel has implementation and fixture evidence for governed companion-runtime behavior, and a benchmark plan for empirical evaluation.

この report を、Asterel が他システムを上回ることや human-level companion quality を達成したことの根拠として使うべきではありません。

## 8. Reproducibility

現在の evidence は、clean public snapshot から evidence ledger と public release gate note にある commands で再現できます。

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

PostgreSQL または live provider API keys が必要な credential-gated tests は default では ignored です。その環境で実行した場合は、別の evidence として記録します。

## 9. Next steps

この artifact report から paper-style result に近づける最短の道は次です。

1. synthetic harness fixture set を 5 件から、public / private exposure、template、density、continuity、repair failures を含む frozen suite に広げる。
2. provider、model、temperature、fixture hash、output report を固定して model-backed harness path を実行する。
3. dataset license と task fit を確認してから benchmark adapters を追加する。
4. aggregate metrics、hashes、configs、public-safe examples を公開し、private transcripts や raw memory payloads は公開しない。
5. long-term social / wellbeing outcomes を主張する前に、consented human-rating studies を設計する。
