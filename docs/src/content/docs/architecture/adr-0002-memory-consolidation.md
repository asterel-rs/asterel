---
title: ADR 0002 — Memory consolidation boundary
description: How post-turn memory consolidation is allowed to persist facts.
---

# ADR 0002 — Memory consolidation boundary

## Status

Accepted.

## Context

The runtime needs durable continuity without letting every turn write arbitrary
long-term facts. The current live post-turn consolidation path is rule-based.
LLM-assisted consolidation code exists as optional implementation work, but it is
not exported or wired into the default post-turn path. Any future LLM-assisted
path must preserve provenance, privacy, confidence, and operator correction
flows.

## Decision

Memory consolidation is a post-turn job that writes through the `Memory` trait.
The active path uses rule-based extraction and persistence. If LLM-assisted
consolidation is reconnected later, provider timeout, provider failure, invalid
JSON, or empty output must degrade to the rule-based path rather than blocking
the turn. Persisted consolidation records use a stable slot key, semantic layer,
system provenance, and private default privacy.

## Consequences

- The current rule-based path does not depend on provider calls.
- Any future provider/parser failure in the optional LLM-assisted path must not
  block the turn; it must downgrade to the rule-based path.
- Consolidated facts remain inspectable and forgettable like ordinary memory
  events.
- Tests should cover extraction/parsing, fallback behavior, and persistence
  through the memory trait rather than direct file or database writes.
