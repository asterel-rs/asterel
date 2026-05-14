---
title: ADR 0002 — 記憶統合の境界
description: post-turn memory consolidation が事実を永続化してよい条件。
---

# ADR 0002 — 記憶統合の境界

## Status

Accepted.

## Context

ランタイムには継続性が必要です。一方で、すべてのターンが任意の長期事実を書けるようにすると、訂正や忘却が難しくなります。現在の live post-turn consolidation path は rule-based です。

LLM-assisted consolidation の実装コードは存在しますが、default の post-turn path には export / 接続されていません。将来 LLM-assisted path を再接続する場合も、provenance、privacy、confidence、運用者による訂正の流れを保つ必要があります。

## Decision

memory consolidation は post-turn job として実行し、`Memory` trait を通して書き込みます。現在有効な経路は rule-based extraction / persistence です。

将来 LLM-assisted consolidation を再接続する場合、timeout、provider failure、不正な JSON、空出力が起きたときは、turn を止めず rule-based path へ戻す必要があります。

永続化される consolidation record は、安定した slot key、semantic layer、system provenance、private を既定とする privacy を持ちます。

## Consequences

- 現在の rule-based path は provider call に依存しない。
- 将来の LLM-assisted path で provider や parser が失敗しても turn を止めず、rule-based path へ downgrade する。
- 統合された事実は、通常の memory event と同じように確認・忘却できる。
- テストは、直接の file / database write ではなく、抽出と parsing、fallback behavior、memory trait 経由の永続化を覆う。
