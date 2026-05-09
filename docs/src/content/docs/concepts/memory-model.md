---
title: Memory model
description: How Asterel treats memory as continuity infrastructure rather than chat history, and what each layer is responsible for.
---

Memory in Asterel is not a transcript cache. It is one of the substrates that lets a companion stay recognizable across time.

The runtime separates "what happened", "what it means", "how useful it was", and "what must remain true about identity". Those are different memory jobs, so they are represented as different layers and projections.

## The five layers

| Layer | Role | Typical content |
|---|---|---|
| Working | Short-lived session context | recent turns, active topic, local continuity cues |
| Episodic | Event-level observations | what was said, when, by whom, in what room |
| Semantic | Consolidated facts | stable user facts, preferences, relationship facts |
| Procedural | Learned ways of acting | principles, successful interaction patterns, tool-use lessons |
| Identity | Durable self/relationship invariants | character identity, protected relationship commitments |

Working memory helps the current turn stay coherent. Episodic memory preserves evidence. Semantic memory makes evidence usable without replaying every event. Procedural memory records "how to do better next time". Identity memory protects what should not drift casually.

## Source of truth and projections

The source of truth is event-oriented. Derived views exist so the runtime can retrieve, inspect, and correct state efficiently:

- append-only memory events
- belief slots for current resolved facts
- retrieval units for search
- graph entities and edges for GraphRAG-style continuity
- deletion and correction lineage for governance

Operator-facing tables, cards, and review screens are projections. They can make correction humane, but they are not the canonical substrate.

## Recall is not dumping memory into the prompt

Asterel does not aim to paste raw memory into every turn. Recall is budgeted and filtered:

```text
turn need
  -> scoped recall query
  -> hybrid retrieval and graph activation
  -> confidence / safety / exposure filtering
  -> compact context block
```

Good recall is selective. It brings in the evidence needed for the current turn, not everything the runtime knows.

Recall quality is judged on separate axes, not only relevance. A useful memory may still be wrong for the turn if it is stale, corrected, too private for the room, or too intimate for the current relationship distance. Ranking and projection therefore consider correction freshness, exposure safety, long-term trust, and public/private fit separately from semantic match.

## Progressive disclosure

Operator and runtime views should reveal memory in layers:

```text
compact view -> timeline / provenance -> full evidence recovery
```

The compact view is what usually belongs near a live turn. Timeline and provenance help an operator inspect why a fact exists. Full evidence recovery is for review, correction, or incident analysis, not for routine prompt stuffing.

## Writeback is governed

Post-turn writeback decides what should be saved after a response is sent. Not every sentence becomes memory. The write path distinguishes:

- transient context that should stay in working memory
- events that should remain as evidence
- facts that should be promoted after enough support
- corrections that should invalidate earlier facts with history
- sensitive material that should not be exposed casually later

This is why pre-send verification matters. A bad turn is not only a bad message; it can become bad evidence if it is allowed to flow into memory.

## Public and private exposure

The companion may know something without being allowed to say it in every room. Exposure rails separate useful grounding from indiscriminate disclosure.

For example, a private user fact may help the companion maintain empathy in a DM, but it should not be surfaced in a public Discord channel unless policy and context make that safe. The memory model is therefore tied to room type, relationship distance, and trust.

## Backend posture

PostgreSQL is the recommended backend because durable continuity needs real storage, queryability, and operational inspection. Markdown is a fallback for constrained or offline use. The `none` selector is a compatibility/test posture for avoiding the full PostgreSQL product path; in the current implementation it routes to the Markdown fallback rather than a truly stateless memory store.

If you run without durable memory, Asterel can still produce text. It is no longer exercising the full companion product, and any claim about relationship continuity should be read as reduced accordingly.

## Self-amendment is governed memory, not identity mutation

When the companion learns that it should approach a person or surface differently next time, that lesson is treated as a governed memory event. Dry-run candidates can be reviewed by an operator, and approved persistence goes through memory governance. They are not user facts, automatic intimacy upgrades, character-core edits, or direct changes to affect runtime state.

## What operators should watch

Memory failures often show up as behavior before they show up as hard errors:

- The companion repeats facts the user corrected.
- It overuses a recent detail outside its proper context.
- It forgets relationship commitments between sessions.
- It discloses private grounding in a public channel.
- It sounds plausible but thin because recall is not contributing evidence.

Those are continuity failures, not just retrieval misses.
