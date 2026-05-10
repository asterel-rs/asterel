---
title: ADR 0001 — Companion turn contract
description: Why all surfaces must converge on one companion turn path.
---

# ADR 0001 — Companion turn contract

## Status

Accepted.

## Context

Asterel exposes CLI, gateway, Discord, desktop/operator, and extension channel
surfaces. If each surface assembles prompts, memory context, safety checks, and
post-turn updates independently, product behavior drifts and tests no longer
prove the default companion experience.

## Decision

All accepted text turns converge on the shared companion turn contract:

1. surface admission and pickup policy;
2. turn enrichment with affect, memory, persona, and governance context;
3. provider/tool loop execution;
4. response finalization and exposure checks;
5. post-turn update for autosave, relationship continuity, and memory work.

Transport adapters may normalize input and deliver output, but they must not
fork companion behavior.

## Consequences

- Integration tests can target the shared contract instead of every adapter.
- Discord remains the release-gated default path until another adapter proves
  the same lifecycle coverage.
- New surfaces need adapter tests plus a contract test showing they enter the
  shared runtime path.
