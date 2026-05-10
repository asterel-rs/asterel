---
title: ADR 0004 — ターン後更新の所有権
description: autosave、relationship continuity、遅延 memory work をどこに置くか。
---

# ADR 0004 — ターン後更新の所有権

## Status

Accepted.

## Context

長く動くコンパニオンの振る舞いは、応答が選ばれた後の作業に依存します。message autosave、relationship state、memory consolidation、metrics です。adapter がそれぞれ独自に書き込むと、replay や rollback の意味が表面ごとにずれます。

## Decision

post-turn update は、response finalization の後に runtime service layer が所有します。adapter は正規化済みの turn result を渡し、配送可能な出力を受け取ります。永続的な relationship や memory write は adapter の責務ではありません。

## Consequences

- 失敗した post-turn hook は、adapter logic を複製せずに観測・再試行できる。
- CLI、gateway、Discord、将来の transport surface に、同じ memory と persona の副作用が適用される。
- observability は hook status metrics を公開し、継続性の作業が配送に遅れていないか運用者が検知できるようにする。
