---
title: ADR 0002 — Memory consolidation boundary
description: How post-turn memory consolidation is allowed to persist facts.
---

# ADR 0002 — Memory consolidation boundary

## Status

Accepted.

## Context

The runtime needs durable continuity without letting every turn write arbitrary
long-term facts. Consolidation can be rule-based or LLM-assisted, and both paths
must preserve provenance, privacy, confidence, and operator correction flows.

## Decision

Memory consolidation is a post-turn job that writes through the `Memory` trait.
LLM consolidation is optional and falls back gracefully on timeout, provider
failure, invalid JSON, or empty output. Persisted consolidation records use a
stable slot key, semantic layer, system provenance, and private default privacy.

## Consequences

- Provider or parser failures do not block the turn; they downgrade to the
  rule-based path.
- Consolidated facts remain inspectable and forgettable like ordinary memory
  events.
- Tests should cover extraction/parsing, fallback behavior, and persistence
  through the memory trait rather than direct file or database writes.
