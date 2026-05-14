---
title: What Asterel is not
description: Non-goals that keep the runtime honest. Reading this is often the fastest way to decide whether Asterel fits a given use case.
---

Listing non-goals is more useful than listing capabilities. Capabilities drift; non-goals define the project's spine.

Asterel is also not finished. APIs and behavior can change between commits. Treat the README's status matrix as current truth for maturity labels; this site explains the direction and boundaries rather than tracking every minor maturity shift.

## It is not a human-pretender

Asterel never denies being an AI. Prompts, persona definitions, and the pre-send verifier all enforce this. A companion whose first move is to claim humanity cannot build a trust relationship on anything real; the project refuses that trade.

Being openly AI is the *enabling* constraint, not a limitation. Everything else — memory, affect, persona continuity — becomes meaningful because the user is not being tricked about what they are talking to.

## It is not a multi-tenant SaaS

Asterel is a **single-operator runtime**. One person, or a small trusted team, runs the daemon on a machine they control. The threat model assumes the operator is trusted. The security boundaries protect:

- The LLM and its tools from doing things outside the operator's consent
- External ingress (webhooks, A2A) from forging trust signals the edge did not verify
- Secrets from leaking through outbound surfaces

That is a different shape than "a cloud product serving arbitrary tenants". The runtime has tenant-scoped admin and operator concepts so local workspaces and bindings stay separated, but those are not a public SaaS isolation model. Arbitrary-tenant hosting is not supported and is not a near-term direction.

## It is not an agent framework

The runtime has tools, a tool registry, subagents, and a governed tool loop. None of that is the product. They exist because a companion that cannot look anything up or take small grounded actions drifts into vague small talk; they are continuity infrastructure, not agentic showpieces.

If the question is "how many autonomous steps can this plan and execute", Asterel is not the answer. If the question is "how much grounding can this runtime hold across a month of conversation without losing who it is", it is.

## It is not a prompt-engineering surface

The system does not expose a user-editable system prompt as the main configuration path. Character behavior is shaped through:

- Persona definitions (durable identity)
- Affect topology (how emotions route through character-specific geography)
- Memory ingestion and recall policy
- Pre-send verifier rules
- Tool allowlists and policy

Prompt text is generated from those. Tweaking prompt text directly is possible but is treated as a short-term lever, not a design interface.

## It is not a voice-first product

Voice adapters exist. They are not where the project is proven. Text is the primary modality because:

- Memory and continuity are easier to verify in text
- The companion's character has to read right in the most scrutinized channel first
- Voice layers cleanly on top of a working text runtime; the inverse is not true

A voice-first companion can be valuable, but it proves a different product shape than the one Asterel is trying to prove first.

## It is not a desktop product

The `desktop/` console exists as an **operator surface** — governance, diagnostics, admin workflows, memory review. It is not where users meet the companion. Users meet the companion in Discord (primary), or in other channels that feed the shared companion-turn contract.

## It is not an AI VTuber

AI VTubers — Neuro-sama, 紡ネン, and the wider streaming-AI scene — optimise around what makes a stream alive: high utterance rate, immediate response, accident-friendliness, clippable moments, fan participation. The aliveness lives in the next minute. Asterel optimises in the opposite direction: non-speaking time, relational continuity, memory as accumulating responsibility, distance as a feature rather than a problem to overcome. The aliveness, if any, lives in the next month.

Adapters, surfaces, and operator infrastructure could in principle support streaming. Asterel may borrow a streaming body when an operator chooses to lend one, but the product centre is not streaming and the silhouette is not a stage voice. What holds the character together is what stays the same when the camera is off.
