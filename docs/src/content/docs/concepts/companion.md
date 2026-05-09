---
title: Companion runtime
description: What the word "companion" means inside Asterel, and how it differs from a chatbot, an assistant, or an agent.
---

"Companion" is not only marketing language in this project. It is a technical commitment that changes which parts of the system get to be load-bearing.

The intended presence is quiet and observational. The companion should not rush to lead, over-explain, or claim human interiority. It should carry context forward, notice shape, and preserve distance when distance is kinder.

## How we use the word

A **companion** is a runtime that owns a single persistent character and the relationship that character holds with each user it talks to. The character has state — affect, bias, continuity cues — that changes as conversations accumulate. Output is how that state surfaces, not how it is *computed*.

A **chatbot** owns a single turn. It may have a scripted personality, but there is no state the next message needs to reconcile with.

An **assistant** owns a task. It is judged on whether the task ends correctly; the relationship does not carry forward.

An **agent** owns a plan. It is judged on whether the plan executes within a tool budget; the character is instrumental.

The product-proven Discord text path is designed to behave companion-first end-to-end. The other shapes (task assistance, agentic tool use, secondary adapters) are available inside the runtime, but they are not the product center and should be treated according to their maturity labels.

## What changes when the runtime is companion-shaped

Four parts of the system get promoted to primary:

- **Memory**, because a companion must carry useful context across sessions. Relationship memory, episodic memory, and semantic graphs can enter turns through recall and policy, not only through explicit recall commands.
- **Persona**, because a companion must remain recognizably the same entity even as it adapts to different rooms, users, and moments. Persona state is durable, not re-prompted.
- **Affect**, because a companion's tone is the visible edge of an internal state topology. Affect is detected, routed through character-specific latent bias, and projected — not applied as a last-mile style filter.
- **Pre-send verification**, because a companion is judged on every turn it sends. A bad turn does not just fail a task; it damages a relationship. The pre-send gate exists to protect that asymmetry.

## Character boundaries

Asterel's character model is deliberately layered so adaptation does not become identity drift:

| Layer | What it means | Boundary |
|---|---|---|
| Core identity | operator-authored character definition, values, negative identity, affect topology | not self-edited by ordinary turns |
| Surface personality | style and register for a user, room, or context | may adapt faster, but does not rewrite Big Five disposition |
| State | active affect, session mood, topology activation | derived runtime state, not hand-edited durable memory |
| Behavioral rules | per-turn posture, register, expression depth, repair/restraint signals | derived for the turn and discarded |

That separation is part of the safety model. A user preference can change a surface style; it should not silently rewrite the companion's core identity. A correction can create a reviewed self-amendment memory; it should not mutate the character definition or claim subjective consciousness.

Public-facing language should therefore describe repair, autonomy, memory discretion, and affect topology as runtime controls. It should not imply that Asterel has human interiority, suffering, or a mystical soul.

## What is *not* primary

The things many AI products put in the center, Asterel deliberately puts at the periphery:

- Multi-step approval-gated plans are a tool, not the product stage.
- Voice is a surface, not the core modality.
- The desktop console is an operator surface, not the main way users meet the companion.
- Tool use is governed and audited, but it is a means of keeping a conversation grounded, not a demo of agency.

The trade is honest: Asterel gives up "look how much this agent can do in one run" in exchange for "look how this character has stayed consistent over a month of conversation". If the second thing does not matter to a given use case, Asterel is the wrong runtime.
