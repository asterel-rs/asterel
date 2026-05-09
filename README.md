<h1 align="center">Asterel</h1>

<p align="center">
  <strong>Discord-first AI companion runtime built in Rust</strong>
</p>

<p align="center">
  <a href="https://www.rust-lang.org"><img alt="Rust" src="https://img.shields.io/badge/rust-2024_edition-orange.svg"></a>
  <img alt="Platform" src="https://img.shields.io/badge/platform-linux%20%7C%20macos%20%7C%20windows-lightgrey.svg">
</p>

> [!WARNING]
> Active development. APIs and behavior can change between commits.

> [!TIP]
> For the conceptual story — why this runtime is shaped the way it is —
> see the docs site: **<https://asterel-rs.github.io/asterel/>**.
> Japanese docs are available at **<https://asterel-rs.github.io/asterel/ja/>**.
> This README is the "how"; the site is the "why".

## Project Status

Asterel is in active development. The default companion path (Discord text, shared turn
execution, memory-backed continuity, response finalization, and local operator governance) is the
current product proof. Discord text is the only channel with end-to-end product coverage today;
non-Discord adapters compile and load but are extension-level alpha surfaces.

## Product Thesis

Asterel treats natural Discord co-presence as the product, not a side effect of model quality.
It should be explicit about being AI, remember across sessions, stay quiet when a room does not
need it, and calibrate public, thread, and DM distance without leaking private memory.

Release readiness is gated by observable companion behavior: sparse pickup, public/private
exposure control, anti-template response style, continuity, and replayable verifier evidence.

## What is Asterel?

Asterel is a text-first AI companion runtime built for durable conversation quality on Discord.
It is open about being an AI, keeps relationship memory across sessions, and calibrates tone and
distance for public rooms, threads, and DMs without becoming noisy.

Primary loop: `Channel Input -> Pickup Policy -> Turn Enrichment -> Response Assembly -> Pre-send Verification -> Reply Delivery -> Post-turn Update`.

Desktop and other channels are secondary operator/adapter surfaces around this loop, not separate
product centers.

## Initial Product Proof

The first release line is intentionally narrow. Asterel should prove that Discord text can
support a companion that remembers, keeps public/private distance calibrated, stays quiet when it
should, and exposes memory review/correction/forget controls through operator surfaces.

For this release line, the project does **not** try to become a voice-first product, desktop-first
chat app, multi-tenant SaaS, or general agent workbench. Skills, MCP, subagents, and secondary
adapters remain useful infrastructure only when they preserve the shared companion turn contract.

## Core Capabilities

Status legend: **Default path** (implemented and release-gated for the current product proof) ·
**Beta** (feature-complete, API may move) · **Alpha** (works on the happy path). Asterel is still
pre-1.0; do not read any status label as broad SaaS or all-channel production maturity.

| Capability | Status | Summary |
|---|---|---|
| Companion | Default path | Discord-first text companion with relationship continuity and public/private calibration |
| Channels (Discord) | Beta | Primary delivery surface with end-to-end coverage from gateway through turn pipeline |
| Channels (others) | Alpha | Secondary adapters (Telegram/Slack/Matrix/etc.); not primary product proof surface |
| Memory | Default path | Recommended `postgres` backend plus Markdown compatibility fallback; autosave, recall, GraphRAG, ingestion pipeline |
| Persona | Default path | Relationship memory, affect-aware tone calibration, continuity checks, and public/private distance shaping |
| Runtime Harness | Default path | Shared turn loop, response finalization, pickup controls, and safety/governance hooks |
| Gateway | Default path | Axum HTTP/WS gateway with pairing, A2A messaging, webhook ingress, companion surface routes, and admin API |
| Daemon | Default path | Long-running runtime: gateway + channels + scheduler + heartbeat |
| Turn Enrichment | Default path | Shared pre/post-turn pipeline: affect detection, memory recall, session control, persona context, prompt composition, relationship update |
| Skills | Beta | Install/manage skill packs with trust-tier evaluation |
| Desktop | Alpha | Secondary operator console for governance, diagnostics, and memory/admin workflows |
| Subagents | Beta | Multi-agent orchestration: inline/spawned execution, cancellation, status tracking |
| Eval | Default path | Behavioral, replay, reliance, persona-consistency, and memory-bench harnesses |
| Security | Default path | Command allowlist, path policy, pairing flow, encrypted vault, secret scrubbing, writeback guards |
| Tunnel | Alpha | Expose local services externally |
| MCP Bridge | Beta | Connect to external MCP servers |

## Quick Start

### Prerequisites

- Rust stable (`rust-toolchain.toml`)
- `protoc` v29+
- Git
- A model provider credential, or a local provider configured during onboarding
- PostgreSQL for the recommended memory backend. Markdown and `none` memory modes exist for
  constrained or offline setups, but PostgreSQL is the production recommendation.

### Build

```bash
git clone https://github.com/asterel-rs/asterel.git
cd asterel
cargo build --release
```

### First Run

First-run steps are ordered. `onboard --interactive` is required before `agent` can start; it
writes `~/.asterel/config.toml` and initializes the workspace.

```bash
# Interactive onboarding wizard (run first on a fresh install)
cargo run -- onboard --interactive

# Start interactive agent (requires completed onboarding)
cargo run -- agent

# One-shot message (requires completed onboarding)
cargo run -- agent --message "Summarize my open tasks"
```

## Turn Enrichment Pipeline

Every accepted companion turn converges on the same enrichment pipeline. Discord text is the
product-proven channel; CLI, gateway, desktop/operator surfaces, and secondary channel adapters
reuse the same turn contract where they create or replay companion turns, but they are not all
equally mature product surfaces.

```text
Pre-turn:  affect detection → memory recall → persona context → system prompt composition
Post-turn: relationship update → message autosave → memory consolidation
```

The transport-facing path is centralized in `src/runtime/services/companion_turn.rs`, while
`src/core/agent/turn_enrichment.rs` remains the canonical owner of pre/post-turn enrichment.

## Desktop Console

The `desktop/` directory contains a Tauri 2 + React 19 + Tailwind 4 operator console.

- Session review and transcript inspection
- Memory review, correction, forget, and self-amendment approval workflows
- Channel, runtime, exposure, and diagnostics visibility
- Companion admin surfaces and secondary tooling
- Context ingress for text, clipboard, files, and screenshots where supported

Run the desktop app against a local daemon:

```bash
cargo run -- daemon --host 127.0.0.1 --port 3000
# then in desktop/
pnpm tauri dev
```

## Gateway

```bash
cargo run -- gateway --host 127.0.0.1 --port 3000
```

Public routes:

- `GET /health`, `GET /healthz`
- `GET /ready`, `GET /readyz`
- `GET /openapi/v1.json`
- `GET /.well-known/agent.json`
- `POST /pair`
- `POST /a2a/v1/messages`
- `GET /a2a/v1/tasks`
- `GET /a2a/v1/tasks/{task_id}`
- `POST /a2a/v1/tasks/{task_id}/cancel`
- `POST /webhook`
- `POST /companion/context/ingest`
- `POST /companion/multimodal/ingest`
- `GET /ws`

Companion surface routes:

- `POST /companion/surface/caption`
- `POST /companion/surface/widget`
- `POST /companion/surface/request-window/open`
- `GET /companion/surface/request-window/{window_id}`
- `POST /companion/surface/request-window/{window_id}/confirm`
- `POST /companion/surface/request-window/{window_id}/cancel`

Admin API (`/admin/v1/*`):

- `GET /admin/v1/openapi.json`
- Runtime, usage, mood, activity timeline, agent list
- Session CRUD and message history
- Governance, memory review, and companion approval windows
- Auth profile management
- Channel management
- Skill management
- Cron management
- Companion admin
- Tenant management

> [!NOTE]
> Admin routes are not public. In practice, pair first via `POST /pair`, then send
> `Authorization: Bearer <token>` and `X-Asterel-Tenant: <tenant-id>` on `/admin/v1/*`
> requests.
>
> Some routes are feature/config dependent (for example WhatsApp routes).

Optional trust-signal headers for external ingress (`POST /webhook`, `POST /a2a/v1/messages`):

- `X-Signature-Verified` (`true`/`1`)
- `X-Signature-Status`
- `X-Webhook-Signature-Status`
- `X-Source-Url` / `X-External-Source-Url`
- `X-Forwarded-Proto`, `Origin`, `Referer`

These headers influence trust-scoring only when injected by a trusted edge verifier or reverse
proxy. Untrusted clients should not set them directly.

## Command Surface

Top-level commands:

- `onboard` — initialize workspace and configuration
- `agent` — run companion loop
- `gateway` — start HTTP/WebSocket gateway
- `daemon` — start long-running runtime
- `service` — manage launchd/systemd user service
- `doctor` — run diagnostics (`--repair` for safe local repairs)
- `config` — validate configuration
- `status` — show runtime/system status
- `eval` — run evaluation suites (baseline, replay, memory-bench)
- `model` — update default model/provider
- `cron` — manage scheduled tasks
- `channel` — list/start/doctor channels
- `integrations` — inspect integrations
- `auth` — manage auth profiles and OAuth import/status
- `skills` — manage installed skills

## Configuration

Default paths:

```text
~/.asterel/config.toml
~/.asterel/workspace
```

Common environment overrides:

| Variable | Purpose |
|---|---|
| `ASTEREL_API_KEY` | Provider API key |
| `ASTEREL_PROVIDER` | Default provider |
| `ASTEREL_MODEL` | Default model |
| `ASTEREL_TEMPERATURE` | Sampling temperature |
| `ASTEREL_WORKSPACE` | Workspace path override |
| `ASTEREL_GATEWAY_HOST` | Gateway host override |
| `ASTEREL_GATEWAY_PORT` | Gateway port override |
| `ASTEREL_GATEWAY_MAX_BODY_SIZE_BYTES` | Max request body size for the gateway |

Reference template: [`.env.example`](.env.example)

Security/ingress tuning example (`~/.asterel/config.toml`):

```toml
[channels_config]
# Removes per-channel autonomy/tool restrictions.
# Global security policy is still enforced.
high_freedom_all_channels = true

[security.perimeter]
enforce_uniform_inner_freedom = true
supported_targets = ["host", "docker", "kubernetes"]

[security.external_knowledge_trust]
enabled = true
default_score = 0.60
min_allow_score = 0.70
min_sanitize_score = 0.30

[security.external_knowledge_trust.source_overrides]
# Explicitly trusted signed ingress.
"gateway:webhook:signature=verified" = 0.90
# Explicitly untrusted/anonymous relay ingress.
"gateway:webhook:anonymous:relay" = 0.15
```

Built-in source profiles are applied for common prefixes (`gateway:*`, `channel:*`, `tool:web*`,
etc.) when no explicit override matches.

## Architecture

```text
src/
├── cli/           # CLI command surface
├── config/        # TOML schema + env overrides
├── contracts/     # Cross-boundary types and shared contracts
├── core/
│   ├── affect/       # Rule-based affect detection, empathy signals
│   ├── agent/        # Turn executor, turn enrichment, tool loop
│   ├── eval/         # Evaluation harnesses and baseline/replay/memory suites
│   ├── experience/   # Experience capture and recall
│   ├── memory/       # Memory backends, ingestion, GraphRAG
│   ├── persona/      # Relationship, empathy, user model, embodied state, continuity
│   ├── providers/    # LLM provider abstraction + reliability
│   ├── sessions/     # Session state and history
│   ├── subagents/    # Multi-agent orchestration
│   ├── taste/        # Preference and taste modeling
│   └── tools/        # Tool registry, middleware, built-in tools
├── media/         # Media processing and speech config
├── onboard/       # Setup wizard and scaffolding
├── platform/      # Daemon, service, cron
├── plugins/
│   ├── companion/    # Companion surface, context, multimodal, rhythm
│   ├── extensions/   # Extension loader
│   ├── integrations/ # External service integrations
│   ├── mcp/          # MCP bridge
│   └── skills/       # Skill loading, catalog, trust tiers
├── runtime/
│   ├── diagnostics/   # Runtime diagnostics
│   ├── environment/   # Environment detection
│   ├── observability/ # Metrics, tracing, logs
│   ├── services/      # Shared runtime services (composition root)
│   ├── tunnel/        # Tunnel management
│   └── usage/         # Usage accounting
├── security/      # Policy, pairing, auth, secrets, guards
├── transport/
│   ├── channels/  # Channel adapters + shared message handler
│   └── gateway/   # HTTP/WS gateway + admin API
├── ui/            # Terminal UI building blocks
└── utils/         # Shared utilities

desktop/           # Tauri/React companion operator console
tests/             # Integration tests by domain
migrations/        # PostgreSQL schema migrations
```

## Security

Asterel is designed for a **single-operator workspace** — one person, or a small trusted team,
running the daemon on a machine they control. It is **not** a multi-tenant SaaS, a public RAG
endpoint, or a shared inference gateway. The threat model assumes the operator is trusted and
focuses on three containment boundaries: (1) the LLM and its tools execute under a deny-by-default
command allowlist and path policy; (2) externally reachable surfaces (webhooks, A2A) trust only
edge-verified ingress signals; (3) secrets the runtime must hold live in an encrypted local vault
with scrubbing on all outbound surfaces.

Highlights:

- Command allowlist + path policy enforcement (deny-by-default)
- Workspace-scoped execution; writeback guard on paths outside workspace
- Gateway pairing flow with token hashing and lockout handling
- Encrypted local secret vault (ChaCha20-Poly1305)
- Secret scrubbing on all outbound surfaces
- Trust-scored external ingress via edge-verifier headers (see Gateway section above)

Full threat model and disclosure policy: [`SECURITY.md`](SECURITY.md).

## Development

### Baseline checks

```bash
cargo fmt -- --check
cargo clippy -- -D warnings
cargo check-all
cargo test
```

Strict release gate (quality + fuzz + audit + perf compare):

```bash
./scripts/release/human_like_release_gate.sh
```

The strict gate also replays `tests/fixtures/replay/discord_companion_bad_turns.jsonl`
to keep Discord companion verifier metrics visible during release checks.

### Useful aliases

```bash
cargo test-dev
cargo test-dev-tests
cargo build-minimal
cargo check-all
cargo coverage
cargo coverage-tarpaulin
cargo ntest
cargo ntest-ci
```

### Enable pre-push hook

```bash
git config core.hooksPath .githooks
```

Pre-push runs:

1. `cargo fmt -- --check`
2. `cargo clippy -- -D warnings`
3. `cargo check-all`
4. `cargo test`

## Documentation & Policies

- Public docs site: <https://asterel-rs.github.io/asterel/>
- Japanese public docs: <https://asterel-rs.github.io/asterel/ja/>
- Contribution guide: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- Security policy: [`SECURITY.md`](SECURITY.md)
- Support guide: [`SUPPORT.md`](SUPPORT.md)
- Code of conduct: [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md)

## Legal notice

Asterel is dual-licensed under MIT or Apache-2.0, at your option. See
[`LICENSE`](LICENSE), [`LICENSE-MIT`](LICENSE-MIT), and
[`LICENSE-APACHE`](LICENSE-APACHE).
