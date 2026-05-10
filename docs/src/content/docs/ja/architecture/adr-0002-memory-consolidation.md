---
title: ADR 0002 — 記憶統合の境界
description: post-turn memory consolidation が事実を永続化してよい条件。
---

# ADR 0002 — 記憶統合の境界

## Status

Accepted.

## Context

ランタイムには継続性が必要です。一方で、すべてのターンが任意の長期事実を書けるようにすると、訂正や忘却が難しくなります。consolidation は rule-based でも LLM-assisted でもよいですが、どちらの経路でも provenance、privacy、confidence、運用者による訂正の流れを保つ必要があります。

## Decision

memory consolidation は post-turn job として実行し、`Memory` trait を通して書き込みます。LLM consolidation は任意です。timeout、provider failure、不正な JSON、空出力が起きた場合は、無理に失敗させず rule-based path へ戻ります。

永続化される consolidation record は、安定した slot key、semantic layer、system provenance、private を既定とする privacy を持ちます。

## Consequences

- provider や parser の失敗は turn を止めず、rule-based path へ downgrade する。
- 統合された事実は、通常の memory event と同じように確認・忘却できる。
- テストは、直接の file / database write ではなく、抽出と parsing、fallback behavior、memory trait 経由の永続化を覆う。
