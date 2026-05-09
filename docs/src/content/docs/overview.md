---
title: Overview
description: What Asterel optimizes for, why the shape of the runtime follows from that, and what reading this site will teach you.
---

Most AI products optimize **the turn** — the quality of the single response the model produces when you hit send. Asterel optimizes **what persists between turns**.

That sentence is the entire thesis. Everything downstream — the memory backends, the affect topology, the shared turn pipeline, the persona layer, the refusal to use approval-gated planners as the product centerpiece — follows from it.

The character target is deliberately quiet: it does not lead by default, but when it speaks, the words should carry weight. It notices contours the user has not quite named yet. It is soft in manner, not vague in judgment; emotional, but not consumed by emotion.

## Current status

Asterel is an early-stage companion runtime. The current center is Discord text, the shared companion turn pipeline, memory-backed continuity, and local operator governance. Other channels and some deeper character mechanisms should be read as alpha or design trajectory unless the repository README marks them as part of the current default path.

## The companion loop

The runtime is built around a loop, not a request-response:

```
conversation → context captured → memory consolidated → distance calibrated
             → enters again when it fits → relationship accrues
             → widens into creative / reflective support
```

Every design decision is graded against that loop. If something accelerates a single turn but erodes the continuity underneath, it loses.

## Current product posture

Asterel is Discord-first and text-first. The companion runtime, gateway, memory, persona, and shared enrichment path are the current default product-proof center. Discord text is the current primary proof channel; other channel adapters are secondary and should be treated as alpha unless the README says otherwise. The desktop app is an operator console for governance, diagnostics, and memory review, not the primary place users meet the companion.

## What you will find on this site

- **Start here** — [Getting started](../guide/getting-started/), [Run Discord](../guide/discord-setup/), [Operating locally](../guide/operating-locally/), [Configuration](../guide/configuration/), and [Troubleshooting](../guide/troubleshooting/) are the practical path.
- **Core concepts** — [Companion runtime](../concepts/companion/), [Continuity over conversation](../concepts/continuity/), [Memory model](../concepts/memory-model/), [Character and persona](../concepts/character-persona/), and [What Asterel is not](../concepts/boundaries/) explain the model without requiring research context.
- **Operator guide** — [Gateway](../operator/gateway/), [Desktop console](../operator/desktop-console/), [Memory review](../operator/memory-review/), and [Security and governance](../architecture/security-governance/) are for running and inspecting a local deployment.
- **Architecture** — [Turn pipeline](../architecture/turn-pipeline/) and [Layered dependencies](../architecture/layers/) describe the implementation shape after you know what the product is.
- **Research packet** — [Claims](../research/claims/), [Evidence ledger](../research/evidence-ledger/), benchmark plans, ablations, and reproducibility docs are available as a deeper track. They are not required for a first run.

## What you will not find (yet)

Full reference for every CLI subcommand, gateway route, and config key lives in the [repository README](https://github.com/asterel-rs/asterel/blob/main/README.md). This site keeps the public guide readable first, while preserving research-quality evidence in a separate track.
