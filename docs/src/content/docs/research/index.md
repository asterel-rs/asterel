---
title: Research packet
description: "Public research framing for Asterel: claims, methods, evidence, references, and what remains private."
---

This packet is the deeper reading track. The general docs explain what Asterel is
and how to run it; this section explains what can be claimed, what evidence
supports those claims, and what still needs benchmarks or human evaluation.

It is intentionally narrower than the private design archive: it keeps claims,
evidence classes, and references that a reader can inspect without publishing raw
review logs, private transcripts, or operator notes.

Asterel is not presented here as a finished empirical paper. The current packet
separates:

- **implemented claims** backed by code and regression tests;
- **method claims** backed by reproducible checks and fixture design;
- **research gaps** that need external benchmarks, ablations, or human studies
  before paper-level conclusions are justified.

## Packet structure

- [Claims](./claims/) — falsifiable design claims and their current evidence.
- [Methodology](./methodology/) — how claims are evaluated and what counts as
  evidence.
- [Evidence ledger](./evidence-ledger/) — publishable evidence classes and
  commands that reproduce them.
- [Technical report v0.1](./technical-report-v0-1/) — artifact-report summary of
  the current runtime design, local gates, and harness ablation result.
- [Public release roadmap](./public-release-roadmap/) — clean-snapshot release
  phases for publishing the repository without private history.
- [Benchmark roadmap](./benchmark-roadmap/) — external and local benchmark tracks
  required before empirical paper claims.
- [Ablation plan](./ablation-plan/) — planned feature-removal studies for memory,
  affect, exposure, naturalness, and security controls.
- [Experimental protocol](./experimental-protocol/) — clean-snapshot runbook for
  benchmark, ablation, human-rating, and artifact publication.
- [Reproducibility](./reproducibility/) — environment, commands, and snapshot
  discipline for public release.
- [Publication boundary](./publication-boundary/) — what can be public, what must
  stay private, and how internal material should be distilled.
- [Research references](../reference/references/) — bibliography mapped to
  concepts and modules.

## Current thesis

Asterel treats long-running companionship as a runtime problem rather than a
prompting problem. The central thesis is that durable companion behavior requires
transport-independent continuity state, governed memory writeback, surface-aware
exposure control, and pre-send verification before a response becomes part of a
relationship history.

The repository currently supports that thesis with architecture checks, boundary
tests, memory/governance tests, response-verifier fixtures, and replay-based
quality gates. It does **not** yet claim statistically validated superiority over
external systems.
