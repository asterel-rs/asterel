---
title: 記憶レビュー
description: コンパニオンの記憶をレビュー、訂正、忘却、承認するときに運用者が考えるべきこと。
---

記憶レビューは、Asterel のコンパニオンとしての約束を点検できる場所です。目的は、すべての raw event を既定で露出することではありません。運用者が、なぜその記憶があるのかを確認し、間違っていれば訂正し、私的文脈が不適切な表面へ漏れないようにすることです。

## レビューモデル

記憶はレイヤーで読みます。

```text
compact view -> timeline / provenance -> full evidence recovery
```

通常の live operation では compact view で足ります。provenance と timeline は「なぜコンパニオンはこれを信じているのか」を見るために使います。full evidence recovery は、レビュー、訂正、削除、incident analysis のためのものです。

## Correct / forget / delete

意図に応じて操作を分けます。

| 意図 | 運用者にとっての意味 |
|---|---|
| Correct | 古い事実が間違っている、または古くなっている。lineage を残し、現在の view を正しくする |
| Forget | backend と policy semantics が許す範囲で、その事実を再利用しない |
| Delete / hard removal | support と policy が許す場合の、より強い削除経路 |

過去の振る舞いに影響した事実を黙って上書きしないでください。訂正 lineage は信頼の一部です。

## 公開 / 私的な露出

コンパニオンは、私的文脈では private memory を潜在的な接地情報として使うことがあります。ただし、それは公開チャネルでその事実を言ってよいという意味ではありません。露出レビューでは次を見ます。

- この記憶はどこから来たか。
- 現在のターンは公開、スレッド、DM、gateway のどれか。
- その事実は訂正済み、または sensitive とされたものか。
- 応答の最終化が draft を止めたか、修正したか。

## Self-amendment review

有用な記憶がユーザーについての事実ではない場合があります。次にこの人やこの表面へどう向き合うべきか、というコンパニオン側の lesson です。これが self-amendment candidate です。

これはガバナンスされたままにします。

- reviewable candidate として生成する。
- raw transcript をコピーせず、bounded かつ redacted にする。
- durable persistence の前に運用者が承認する。
- character-core mutation ではなく、private procedural memory として保存する。

ユーザーの訂正を、自動的な親密度の上昇に変えないでください。修復は、ユーザーが訂正、reset、離脱、忘却を求める自由を保ったまま、次の振る舞いをよくするためにあります。
