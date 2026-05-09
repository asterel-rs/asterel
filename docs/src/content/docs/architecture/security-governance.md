---
title: Security and governance
description: The containment model around tools, external ingress, secrets, admin access, and memory writeback.
---

Asterel assumes a trusted local operator. It does not assume that every message, webhook, tool result, or model-suggested action is trustworthy.

The security model is therefore not "multi-tenant SaaS isolation". It is containment for a single-operator companion runtime with external edges.

## Containment boundaries

The main boundaries are:

| Boundary | What it protects |
|---|---|
| Tool execution | Filesystem, shell, network, and other local capabilities |
| External ingress | Webhooks, A2A, channel events, and trust signals |
| Secrets | Provider keys, channel tokens, OAuth material |
| Memory writeback | Durable relationship state and correction history |
| Admin access | Runtime state, governance actions, tenant-scoped operator context |

Each boundary exists because the companion loop is stateful. A bad action or bad memory can persist past the turn that caused it.

## Tool governance

Tool calls run through middleware before they execute:

```text
SecurityMiddleware -> HookMiddleware -> EntityRateLimitMiddleware -> AuditMiddleware
  -> SemanticCompactionMiddleware -> ToolOutputCompactionMiddleware
  -> OutputSizeLimitMiddleware -> ToolResultSanitizationMiddleware
  -> SecretScrubMiddleware -> TaintMiddleware
```

This gives the runtime a choke point for command policy, hooks, rate limiting, auditing, output compaction, sanitization, secret scrubbing, and taint tracking. Tool use is allowed because grounded action is useful; it is not allowed to bypass the companion's safety posture.

Channel-level autonomy and tool allowlists can restrict what a surface may do. Global policy still applies even when a channel is configured more freely.

## External ingress trust

External content is scored by source. Built-in profiles distinguish sources such as Discord, Slack, webhooks, A2A, browser/tool-originated content, and generic gateway input. Operators can add source-prefix overrides.

Trust-signal headers such as signature verification are meaningful only when a trusted edge sets them. An arbitrary client setting `X-Signature-Verified: true` is not proof of trust.

The practical rule is simple: trust the verifier, not the header text.

## Pairing and admin scope

Gateway pairing is required by default. Admin routes also require tenant scope:

```text
Authorization: Bearer <token>
X-Asterel-Tenant: <tenant-id>
```

The tenant value is an operator/workspace context for local admin APIs. It should not be described as public SaaS tenant isolation. That distinction matters: the product is single-operator, but the admin API still needs an explicit scope for the state it is reading or mutating.

## Secrets

Provider keys, channel tokens, and OAuth material should be treated as local secrets. Prefer environment variables or the runtime's secret-management path for credentials, and avoid committing generated configs that contain tokens.

Outbound surfaces are scrubbed, but scrubbing is a last line of defense. The safer pattern is to keep secrets out of memory, prompts, logs, and operator-visible notes in the first place.

## Memory governance

Memory is a security boundary because it changes future behavior. Correction, forgetting, and writeback must preserve enough lineage to avoid silent mutation.

Good governance means:

- durable facts can be corrected without pretending the old evidence never existed
- deletion and forgetting leave an auditable outcome where policy allows it
- public/private exposure is enforced before recall becomes text
- identity and affect runtime state are not casually editable by model output
- operator review surfaces expose projections, not a second write model
- companion-generated self-amendment lessons persist only through reviewed memory governance, not direct persona mutation

Memory exposure is also progressive. The live turn should normally receive compact grounding. Operators can inspect provenance and lineage when needed. Raw evidence recovery is a review path, not a default prompt ingredient.

## Pre-send verification

Pre-send verification is part of governance, not a cosmetic final pass. It protects both the user-facing message and the state that may be updated after the turn.

If the verifier blocks or revises a response, that is a continuity-preserving action. The runtime is preventing a bad turn from becoming part of the relationship history.

## Operational stance

For a normal local deployment:

- keep the gateway local unless a trusted edge is configured
- keep pairing enabled
- keep channel pickup conservative in public rooms
- use narrow tool allowlists where a channel does not need broad capability
- treat memory correction as append/correct/invalidate, not silent overwrite
- use desktop/admin surfaces for review, not as an alternate runtime owner
