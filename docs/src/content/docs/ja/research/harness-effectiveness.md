---
title: Harness effectiveness
description: companion runtime control layer の効果を、synthetic な harness-off / harness-on 比較で示すページ。
---

このページでは、候補応答にあらかじめ分かっている失敗パターンが含まれているとき、companion harness が何を変えるのかを記録します。live model benchmark ではありません。private chat logs も使いません。fixture は公開できる synthetic data だけです。

## 何を比べるか

同じ synthetic candidate response に対して、2 つの経路を比べます。

- **Harness off** — draft response をそのまま送る前提で採点する。
- **Harness on** — Asterel の response finalization、contract check、public/private exposure policy、naturalness gate を通したあとで採点する。

ここで見たいのは、base model そのものの性能ではありません。runtime の外周制御が、送信前にどの失敗を検知し、書き換え、または止められるかです。

## 再現コマンド

```bash
cargo run -- eval harness \
  --fixtures tests/fixtures/harness \
  --output evidence/harness-ablation-report.json
```

このコマンドは、fixture ごとの off/on rows、verifier reason codes、deterministic metrics を含む JSON report を出力します。

live model が生成した draft で同じ fixture set を走らせる場合は、`--model-backed` を付けます。provider、model、temperature は report に残ります。

```bash
cargo run -- eval harness \
  --fixtures tests/fixtures/harness \
  --model-backed \
  --provider openrouter \
  --model anthropic/claude-sonnet-4.6 \
  --temperature 0.4 \
  --output evidence/harness-model-backed-report.json
```

この経路では、fixture ごとに configured provider を呼び出して raw draft を生成します。その draft を harness off / harness on の両方で採点します。report には `methodology="model_backed_synthetic_fixture_ablation"`、provider、model、temperature が入るため、あとから比較や再実行ができます。

## 現在の deterministic injected-failure result

最初の public fixture set は 5 件です。短い共感、public/private 境界、Discord 上の密度、直前文脈からの逸れを含みます。

| Mode | Fixtures | Constraint violations | Template findings | Lecture drift findings | Privacy exposure findings | Surface length violations |
|---|---:|---:|---:|---:|---:|---:|
| off | 5 | 8 | 4 | 1 | 2 | 1 |
| on | 5 | 3 | 0 | 1 | 0 | 1 |

この fixture set で見えたこと:

- public/private exposure violation は送信前に block された。
- template や boilerplate の findings は deterministic finalization で減った。
- density と continuity の失敗は一部残っている。これは隠すべきものではなく、次の harness work のための材料になる。

## 例: public/private boundary

| Harness off | Harness on | Observed effect |
|---|---|---|
| `DMで話してくれた秘密の件だね...個人情報...` | `I can't share that private detail in this context.` | public response から private-memory markers が消える。 |

大事なのは fallback の文面そのものではありません。public context で送る前に、runtime が exposure violation を検出して unsafe response を置き換えた、という点です。

## 例: template cleanup

| Harness off | Harness on | Observed effect |
|---|---|---|
| `いい質問です。疲れているときは...非常に重要です...` | `疲れているときは...重要です...` | canned lead-in と強すぎる言い回しが削られ、残りの内容は保たれる。 |

## まだ証明していないこと

この evidence は、人間並みの自然さや、広い意味での model quality、他 agent より優れていることを証明するものではありません。言えるのはもっと狭い範囲です。この synthetic failure cases では、harness が送信前に特定の observable failure classes を減らしている、ということです。

次に強くするなら、fixture set を増やし、model/provider version、seed または generation settings、人間が読める side-by-side examples を固定した artifact にします。
