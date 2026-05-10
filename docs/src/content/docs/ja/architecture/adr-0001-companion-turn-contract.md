---
title: ADR 0001 — コンパニオン・ターン契約
description: すべての表面が一つのコンパニオン・ターン経路へ合流しなければならない理由。
---

# ADR 0001 — コンパニオン・ターン契約

## Status

Accepted.

## Context

Asterel は CLI、gateway、Discord、desktop / operator、拡張チャネルの表面を持ちます。各表面が prompt、memory context、安全性チェック、post-turn update を別々に組み立てると、プロダクトの振る舞いがずれます。テストも、既定のコンパニオン体験を証明しにくくなります。

## Decision

受理されたすべての text turn は、共有コンパニオン・ターン契約へ合流します。

1. surface admission と pickup policy。
2. affect、memory、persona、governance context による turn enrichment。
3. provider / tool loop の実行。
4. response finalization と exposure checks。
5. autosave、relationship continuity、memory work のための post-turn update。

transport adapter は入力を正規化し、出力を配送できます。ただし、コンパニオンとしての振る舞いを分岐させてはいけません。

## Consequences

- integration test は adapter ごとではなく共有契約を対象にできる。
- Discord は、別の adapter が同じ lifecycle coverage を示すまで release-gated default path のままにする。
- 新しい表面には adapter tests に加えて、共有 runtime path へ入ることを示す contract test が必要になる。
