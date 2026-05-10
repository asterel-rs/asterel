---
title: ADR 0004 — Post-turn update ownership
description: Where autosave, relationship continuity, and deferred memory work belong.
---

# ADR 0004 — Post-turn update ownership

## Status

Accepted.

## Context

Long-running companion behavior depends on work that happens after a response is
chosen: message autosave, relationship state, memory consolidation, and metrics.
If adapters perform those writes independently, replay and rollback semantics
become inconsistent.

## Decision

Post-turn update belongs to the runtime service layer after response finalization.
Adapters hand off normalized turn results and receive delivery-ready output; they
do not own durable relationship or memory writes.

## Consequences

- Failed post-turn hooks can be observed and retried without duplicating adapter
  logic.
- The same memory and persona side effects apply to CLI, gateway, Discord, and
  future transport surfaces.
- Observability should expose hook status metrics so operators can detect when
  continuity work is lagging behind delivery.
