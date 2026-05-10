---
title: ADR 0003 — GraphRAG パイプライン
description: graph memory の抽出、解決、永続化の境界。
---

# ADR 0003 — GraphRAG パイプライン

## Status

Accepted.

## Context

GraphRAG は通常の recall の上に、型付き entity、relation、evidence ID、時間的な有効期間を重ねます。この pipeline には壊れやすい継ぎ目が三つあります。LLM JSON extraction、entity resolution、memory storage への persistence / projection です。

## Decision

graph extraction は制約された ontology JSON を返す必要があります。entity、relation、endpoint、evidence ID、confidence、validity window は、resolution や persistence の前にすべて検証します。

entity resolution は別ステップです。relation endpoint を remap し、alias を記録します。persistence は memory backend の write path を通して行います。これにより graph projection は provenance、privacy、forget behavior を継承します。

## Consequences

- 不正な extraction output は graph state を変更する前に fail closed する。
- 最小の happy path は extraction → resolution → serializable / persistable result であり、PostgreSQL projection は memory integration tests で覆う。
- 将来 ontology を変えるときは、migration、extraction、resolution のテストをまとめて更新する必要がある。
