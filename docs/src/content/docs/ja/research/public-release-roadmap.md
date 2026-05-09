---
title: Public release roadmap
description: private development history を持ち込まず、Asterel を clean public snapshot として公開するための roadmap。
---

この roadmap は、現在の publicization work を release phases に分けます。願望の一覧ではなく運用手順です。各 phase には exit gate があり、research packet 全体と同じ publication boundary を保ちます。

重要な制約は単純です。public repository は、public tracked set の clean snapshot から初期化するべきです。private development history、local agent assets、internal review notes、personal workspace context は public artifact に含めません。

## Phase 0 — public tracked set を凍結する

Goal: 新しい repository を作る前に、何を公開できるかを正確に知る。

Tasks:

- local / private material が public tracked set に入っていないことを確認する。
- public docs が private documentation に依存していないことを確認する。
- repository、docs、package、license metadata が intended organization repository を指していることを確認する。
- public docs と metadata の coherence を示す最小 checks を再実行する。

Gate:

```bash
git diff --check
pnpm --dir docs build
cargo metadata --no-deps --format-version 1
cargo test --test project
```

public files が private docs、local agent notes、old personal repository URLs、private-license wording をまだ参照している場合は進めません。

## Phase 1 — public narrative を仕上げる

Goal: private design notes がなくても repository を理解できるようにする。

Tasks:

- README の maturity language を final-pass する。
- implementation claims と benchmark / paper claims を分けておく。
- research packet が claims、evidence classes、gaps、reproducibility、publication boundaries を説明していることを確認する。
- Japanese overview pages を public status language と揃える。

Exit criteria:

- 新しい読者が、Asterel は Discord-first、text-first の companion runtime であり、local operator governance を持つと理解できる。
- 新しい読者が、何が implemented で、何が alpha で、何に empirical evaluation がまだ必要かを理解できる。
- public page が private internal context なしで読める。

## Phase 2 — clean snapshot を準備する

Goal: private historical blobs を残さず public repository を作る。

Tasks:

- separate clean-snapshot working directory を作る。
- public tracked file set だけをその directory に copy する。raw workspace copy ではなく、できれば `scripts/release/create_public_snapshot.sh` を使う。
- そこで新しい `.git` repository を initialize する。
- copied tree を検証した後にだけ intended remote を追加する。
- public-safe snapshot として first commit を作る。

Non-goals:

- 既存の private repository history を push しない。
- refs を mirror しない。
- closed PR refs、local branches、historical blobs を残さない。
- development に便利だからといって ignored local files を copy しない。

Snapshot gate:

```bash
scripts/release/create_public_snapshot.sh /tmp/asterel-public-snapshot --dry-run
pnpm --dir docs build
cargo metadata --no-deps --format-version 1
```

時間が許す場合は、公開前に broader Rust gates も実行します。

## Phase 3 — GitHub repository を設定する

Goal: organization repository が issues、docs、security reports を安全に受けられるようにする。

Repository settings:

- `asterel-rs/asterel` を作る。
- site config が使う docs path に GitHub Pages を有効化する。
- private vulnerability reporting を有効化する。まだ有効化していない場合は、`SECURITY.md` の fallback address が正しいことを保つ。
- Actions permissions と branch protection を確認する。
- companion-first scope に合う repository description と topics を追加する。

org details が決まるまで deferred:

- owning team name、visibility、write permission が確認できるまで `CODEOWNERS` は追加しない。
- organization profile README が必要なら、別 repository `asterel-rs/.github` に追加する。

## Phase 4 — clean checkout から検証する

Goal: fresh clone した public repository から、現在の evidence を再現できることを示す。

clean repository で実行します。

```bash
cargo fmt -- --check
cargo clippy -- -D warnings
cargo check-all
cargo test
pnpm --dir docs build
cargo metadata --no-deps --format-version 1
./scripts/dev/generate_module_map.sh && ./scripts/dev/check_architecture.sh
```

full gate を実行できない場合は理由を記録し、もっとも近い targeted substitutes を実行します。その場合、release を fully validated と表現しません。

## Phase 5 — research artifact を固める

Goal: implementation evidence から paper-level empirical evidence へ進む。ただし結果を強く言いすぎない。

Tasks:

- external names を evidence として使う前に、benchmark dataset licenses と task fit を確認する。
- memory、affect calibration、public / private room behavior、security containment 用の benchmark adapters を作る。
- [ablation plan](./ablation-plan/) にある ablation conditions を実行する。
- model / provider versions、seeds、fixture hashes、config、redaction policy を記録する。
- real human transcript data を使う前に、consented human-rating studies を設計する。

Exit criteria:

- implementation claims と empirical claims が見た目にも分かれている。
- frozen benchmark または human-evaluation artifact なしに superiority claim が出てこない。
- public artifacts には aggregate metrics、public-safe examples、hashes を置き、private transcripts や raw memory payloads は置かない。

## Phase 6 — 最初の public release を切る

Goal: broad platform maturity ではなく、狭い product proof を反映する alpha release を公開する。

Tasks:

- README、docs、security、support、license を final review する。
- GitHub Pages が public docs URL へ解決することを確認する。
- issue templates が secrets や private memory を投稿しないよう警告していることを確認する。
- Discord-first companion-runtime snapshot として release notes を準備する。
- clean repository validation gate が通った後にだけ tag を打つ。

Suggested first release label:

```text
v0.1.0-alpha — Discord-first companion runtime snapshot
```

## Public no-go conditions

次のどれかが true なら公開しません。

- 既存の private `.git` history を push しようとしている。
- public tracked files が private docs や local agent / session files を参照している。
- old personal repository URLs が public docs または package metadata に残っている。
- Rust、Node、README、license files の license metadata が食い違っている。
- private vulnerability reporting を約束しているのに、有効化または fallback の説明がない。
- benchmark または paper claims が evidence ledger の支持を超えている。
