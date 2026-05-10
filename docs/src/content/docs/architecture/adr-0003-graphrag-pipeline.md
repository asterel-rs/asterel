---
title: ADR 0003 — GraphRAG pipeline
description: Extraction, resolution, and persistence boundaries for graph memory.
---

# ADR 0003 — GraphRAG pipeline

## Status

Accepted.

## Context

GraphRAG adds typed entities, relations, evidence IDs, and temporal validity on
top of ordinary recall. The pipeline has three failure-prone seams: LLM JSON
extraction, entity resolution, and persistence/projection into memory storage.

## Decision

Graph extraction must return constrained ontology JSON and validate every entity,
relation, endpoint, evidence ID, confidence, and validity window before it can be
resolved or persisted. Entity resolution is a separate step that remaps relation
endpoints and records aliases. Persistence happens through memory backend write
paths so graph projections inherit provenance, privacy, and forget behavior.

## Consequences

- Invalid extraction output fails closed before graph state is mutated.
- The minimal happy path is extraction → resolution → serializable/persistable
  result, with PostgreSQL projection covered by memory integration tests.
- Future ontology changes need migration, extraction, and resolution tests
  together.
