---
title: Layered dependencies
description: How src/ is organized so that continuity-bearing modules cannot accidentally depend on transports, and why the layering is enforced rather than suggested.
---

`src/` is split into six layers (L0–L5). Dependencies flow upward only: higher layers may depend on lower layers, but lower layers must not import higher layers. Violations are caught at review time.

| Layer | Modules | Role |
|---|---|---|
| L0 | `contracts/`, `config/`, `utils/` | Cross-boundary types, IDs, TOML schema |
| L1 | `core/memory/`, `core/persona/`, `core/providers/`, `core/sessions/`, `core/subagents/`, `core/experience/`, `core/eval/` | Durable state, identity, provider abstraction, shared runtime domain |
| L2 | `core/tools/`, `security/` | Tool system, approval, policy, taint, governance |
| L3 | `core/agent/`, `core/affect/`, `media/` | Turn executor, affect detection, multimodal processing |
| L4 | composition-facing `runtime/services/`, `runtime/diagnostics/`, `runtime/observability/` | Composition root, DI, telemetry |
| L5 | `transport/`, `cli/`, `platform/`, `plugins/`, `ui/`, `onboard/` | Gateway, channels, CLI, desktop plugins |

## What the layering buys

The layering exists to protect one property: **continuity state (L1) does not know what surface is reading it.**

- Memory, persona, and sessions never import from `transport/` or `cli/`. They cannot be accidentally coupled to "the shape of a Discord message" or "the shape of an HTTP request".
- Core agent logic (L3) cannot skip past the security / tool layer (L2) to talk directly to a channel.
- Transports (L5) are free to change — a new channel, a desktop panel, a new gateway route — without touching the state they display.

If any of those barriers were porous, the runtime would be one refactor away from Discord-shaped memory or gateway-flavored persona. The layering is what lets the [shared turn pipeline](../turn-pipeline/) stay honest.

This is the public-facing map. The internal architecture canon is more precise: some `runtime/services/` modules are application services that own shared turn/control-plane behavior, while others are composition services that wire providers, memory, sessions, and surfaces together. The rule for external readers is simpler: continuity-bearing state stays below transports, and surface-specific code should not become the source of truth.

## The composition root

Everything is wired together in `src/runtime/services/`. That module:

- Initializes auth, security policy, memory, the rate limiter
- Builds the provider (wrapped in reliability and OAuth recovery layers)
- Assembles the tool registry
- Constructs the `ExecutionContext` passed into every tool call

The composition root is the only place that knows about all the pieces at once. Every other module sees a narrow slice.

## Key trait surfaces

Three traits carry most of the weight at module boundaries:

- **`Memory`** — a supertrait combining `MemoryWriter` (`append_event`), `MemoryReader` (`recall_scoped`, `resolve_slot`), and `MemoryGovernance` (`health_check`, `forget_slot`). Backends: PostgreSQL (default) or Markdown fallback.
- **`Tool`** — `name()`, `description()`, `parameters_schema()`, `execute(args, ctx)`. Registered in `core/tools/registry.rs`.
- **`Provider`** — single required method `chat_with_system()`. Wrapped by `ReliableProvider` (retry + circuit-breaker) and `OAuthRecoveryProvider`. Implementations include Anthropic, OpenAI, OpenRouter, Ollama, Gemini / Gemini Vertex, and MiniMax.

These three are the contract across which most cross-layer communication happens. If you are tracing a change and you land on one of these, you have found the right seam.

## Experimental code

Experimental or retired code is not part of the production layering contract unless it is exported by the active module tree and covered by the architecture checks. The current companion runtime is owned by the layers above; old planner, simulation, and evolution surfaces are not product-bearing entrypoints.
