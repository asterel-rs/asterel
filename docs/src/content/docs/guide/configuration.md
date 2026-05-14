---
title: Configuration
description: "The practical configuration model: where settings live, which defaults matter, and which knobs should be treated as operational boundaries."
---

Asterel is configured from a local TOML file, then selectively overridden by environment variables. The default path is:

```text
~/.asterel/config.toml
```

`onboard --interactive` creates the first usable config and initializes the workspace. Treat the generated file as the operator's local deployment record, not as a template that should be copied between machines without review.

## The important sections

Most operators only need to understand five areas first.

| Area | What it controls | Why it matters |
|---|---|---|
| Provider | default provider, model, API key | Determines what model renders and reasons inside the turn loop |
| Memory | backend, retention, recall, embeddings | Determines whether continuity survives across sessions |
| Gateway | host, port, pairing, body limits | Determines how local/admin/API surfaces enter the runtime |
| Channels | Discord and secondary adapters | Determines which inbound events can become companion turns |
| Security | trust scoring, intent classification, tool policy | Determines what untrusted ingress and tools are allowed to do |

The README remains the main orientation for command names, route inventory, and common environment variables. Source schema, `.env.example`, and generated contracts remain authoritative for exhaustive configuration details. This page explains how to reason about the config.

## Provider settings

At minimum, the runtime needs a model provider and credentials. The common environment overrides are:

| Variable | Purpose |
|---|---|
| `ASTEREL_API_KEY` | Provider API key |
| `ASTEREL_PROVIDER` | Default provider |
| `ASTEREL_MODEL` | Default model |
| `ASTEREL_TEMPERATURE` | Sampling temperature |

Use environment variables for secrets and deployment-specific values. Keep durable operator choices in TOML when they should be visible during later review.

## Memory settings

The default memory backend is PostgreSQL. Markdown exists for constrained or offline setups. The `none` selector is a compatibility/test posture for avoiding the full PostgreSQL product path; in the current implementation it routes to the Markdown fallback rather than a truly stateless store.

```toml
[memory]
backend = "postgres"
# Prefer ASTEREL_POSTGRES_URL for credentials in real deployments.
postgres_url = "postgres://asterel@localhost/asterel"
auto_save = true
hygiene_enabled = true
conversation_retention_days = 30
```

If your database URL contains a password, keep it in the environment rather than committing it to a copied config file.

Important defaults:

- `backend = "postgres"` by default.
- `auto_save = true`, so conversation context is written unless disabled.
- `hygiene_enabled = true`, so archive and retention cleanup can run.
- `working_memory_capacity = 50`, limiting the session working set.
- `recall_min_confidence = 0.3`, filtering low-confidence recall before it enters prompt context.
- `graph_retrieval_fusion_enabled = true`, so graph context can contribute to recall ranking.

If memory is disabled or pointed at a disposable backend, the runtime can still answer turns, but the companion promise is weakened. That is a useful test mode, not the product posture.

## Persona and character gates

Persona settings separate long-lived identity from short-lived runtime state. In broad terms:

- response finalization is enabled by default for user-facing text;
- the stricter naturalness gate is opt-in;
- session control, affect topology, behavior selection, trait activation, and soul-pressure posture are typed feature gates;
- core character configuration is operator-owned config, not something normal turns self-edit.

Use these knobs as rollout controls. Do not add transport-local flags for behavior that should be shared across Discord, gateway, channel, and replay paths.

## Gateway settings

The gateway defaults are local-first:

```toml
[gateway]
host = "127.0.0.1"
port = 3000
require_pairing = true
allow_public_bind = false
defense_mode = "enforce"
max_body_size_bytes = 65536
```

Keep `require_pairing = true` for normal operation. Binding to a public address is a deployment decision, not a quick-start shortcut; if the gateway must be reachable from outside the machine, put a trusted edge or tunnel in front of it and preserve the trust model.

## Channel settings

Discord is the primary product surface. The Discord adapter is configured under `channels_config.discord` and includes the bot token, optional application/guild restrictions, allowed users, thinking embed settings, and pickup policy.

The default pickup policy is conservative:

```toml
[channels_config.discord.pickup_policy]
mode = "direct_only"
max_unsummoned_replies_per_hour = 0
min_gap_seconds = 600
```

This means the companion should not drift into ambient public-channel chatter unless the operator explicitly opts into sparse ambient behavior. That is a character and boundary decision, not only a spam-prevention setting.

## Security posture

Security configuration is layered. Channel-level autonomy and tool allowlists can narrow what a channel may do, but global security controls still apply.

External knowledge and ingress are trust-scored. Built-in profiles assign different defaults for sources such as Discord, webhooks, A2A, and browser/tool-originated content. Operator overrides should be narrow and source-specific.

```toml
[security.external_knowledge_trust]
enabled = true
default_score = 0.60
min_allow_score = 0.70
min_sanitize_score = 0.30
```

Do not raise trust scores to make integrations "just work" unless you also trust the edge that produced the signal.

## Configuration checklist

Before treating a local runtime as useful:

- `onboard --interactive` has completed.
- A provider and model are configured.
- PostgreSQL is configured if you expect durable relationship continuity.
- Discord is configured if you expect the primary product surface.
- Gateway pairing is still required.
- Public ingress is behind a trusted edge or disabled.
- Any channel-level autonomy increase has an explicit reason.
