<div align="center">
  <img alt="Asterel turtle mascot" src="./Asterel.png" width="360">

  <h1>Asterel</h1>

  <p>
    <strong>A Discord-first AI companion runtime for durable memory, persona, and relationship continuity.</strong>
  </p>

  <p>
    <a href="https://www.rust-lang.org"><img alt="Rust" src="https://img.shields.io/badge/rust-2024_edition-orange.svg"></a>
    <img alt="Platform" src="https://img.shields.io/badge/platform-linux%20%7C%20macos%20%7C%20windows-lightgrey.svg">
    <img alt="Status" src="https://img.shields.io/badge/status-active_development-8a6f4d.svg">
  </p>

  <p>
    <a href="https://asterel-rs.github.io/asterel/"><strong>Docs</strong></a>
    ·
    <a href="#quick-start"><strong>Quick start</strong></a>
    ·
    <a href="#current-product-proof"><strong>Product proof</strong></a>
    ·
    <a href="SECURITY.md"><strong>Security</strong></a>
  </p>
</div>

---

> [!WARNING]
> Asterel is pre-1.0 and in active development. APIs, commands, configuration, and behavior may
> change between commits.

## The shape of the project

Asterel is a text-first companion runtime built around a narrow product bet: Discord rooms and DMs
can support an AI presence that is honest about being AI, remembers over time, keeps public/private
distance calibrated, and knows when not to speak.

It is not trying to be a SaaS agent workbench, a human-passing persona, or a desktop-first chat app.
The desktop, gateway, skills, MCP, subagents, and secondary channel adapters exist to support the
same companion turn contract — not to become separate product centers.

| Promise | What Asterel optimizes for |
|---|---|
| **Quiet co-presence** | Sparse pickup, public-room restraint, and replies that do not dominate the room |
| **Durable relationship memory** | Long-term user, room, topic, and continuity signals with review/correction paths |
| **Surface-aware intimacy** | Public channels, threads, DMs, gateway turns, and operator surfaces get different exposure limits |
| **Auditable behavior** | Pre-send verification, replayable evals, diagnostics, and local operator governance |

## Current product proof

The first release line is intentionally narrow.

**Default proof path:** Discord text → shared companion turn runtime → memory-backed continuity →
response finalization → local operator governance.

Discord text is the only channel with end-to-end product coverage today. Other adapters compile and
load, but should be treated as extension-level alpha surfaces until they earn the same release-gated
coverage.

```text
Channel input
  → pickup policy
  → turn enrichment
  → response assembly
  → pre-send verification
  → reply delivery
  → post-turn update
```

## What Asterel is / is not

| Asterel is | Asterel is not |
|---|---|
| A Discord-first text companion runtime | A general hosted multi-tenant SaaS |
| Honest-about-being-AI relationship software | A human impersonation project |
| Memory, persona, and exposure-policy infrastructure | A raw prompt-only chatbot wrapper |
| A single-operator local workspace with governance tools | A public RAG endpoint or shared inference gateway |
| A base for future creative/thought-companion extensions | A planner/simulation/approval workbench as the main product |

## Quick Start

### Install (macOS/Linux)

```bash
curl -fsSL https://asterel-rs.github.io/asterel/install.sh | sh
asterel onboard --interactive
```

The installer puts `asterel` in `~/.local/bin` by default. It uses GitHub release binaries when
available and falls back to a source build otherwise. If your shell cannot find `asterel` yet, run
`~/.local/bin/asterel onboard --interactive` or add `~/.local/bin` to `PATH`.

### Run

```bash
asterel agent
asterel agent --message "Summarize my open tasks"
```

<details>
<summary>Build from source instead</summary>

Prerequisites:

- Rust stable from [`rust-toolchain.toml`](rust-toolchain.toml)
- `protoc` v29+
- Git
- A model provider credential, or a local provider selected during onboarding
- PostgreSQL for the recommended memory backend. Markdown and `none` memory modes exist for
  constrained/offline setups, but PostgreSQL is the production recommendation.

```bash
git clone https://github.com/asterel-rs/asterel.git
cd asterel
cargo build --release
cargo run -- onboard --interactive
cargo run -- agent
```

</details>

## Core capabilities

Status legend: **Default path** = implemented and release-gated for the current proof · **Beta** =
feature-complete, API may move · **Alpha** = happy-path/extension maturity · **Stub** = compiles or
loads, but does not yet have default-path product coverage.

| Capability | Status | Summary |
|---|---|---|
| Companion runtime | Default path | Shared text companion loop with pickup, enrichment, finalization, and post-turn update |
| Discord channel | Default path | Primary delivery surface with end-to-end product coverage |
| Memory | Default path | PostgreSQL-first memory, Markdown fallback, autosave, recall, GraphRAG, review/correction/forget |
| Persona | Default path | Relationship continuity, affect-aware tone calibration, public/private distance shaping |
| Runtime harness | Default path | Response finalization, surface realization policy, safety/governance hooks |
| Gateway | Default path | Axum HTTP/WS gateway with pairing, webhook/A2A ingress, companion routes, admin API |
| Daemon | Default path | Long-running gateway + channels + scheduler + heartbeat process |
| Eval | Default path | Behavioral, replay, reliance, persona-consistency, and memory-bench harnesses |
| Security | Default path | Command allowlist, path policy, pairing flow, encrypted vault, scrubbing, writeback guards |
| Skills | Beta | Install/manage skill packs with trust-tier evaluation |
| Subagents | Beta | Inline/spawned orchestration with cancellation and status tracking |
| MCP bridge | Beta | Connect external MCP servers through the runtime boundary |
| Desktop console | Alpha | Secondary Tauri/React operator console for governance, diagnostics, and memory/admin workflows |
| Secondary channels | Mixed | See the transport matrix below; only Discord is default-path today |
| Tunnel | Alpha | Expose local services externally when configured |

### Transport maturity matrix

| Transport | Status | Notes |
|---|---|---|
| Discord | Default path | Release-gated text room/DM flow with shared companion turn coverage |
| Gateway HTTP/WS | Default path | Operator and external ingress surface into the shared runtime |
| Telegram | Alpha | Adapter surface exists; requires more end-to-end product coverage |
| Slack | Alpha | Adapter surface exists; requires more end-to-end product coverage |
| Matrix | Stub | Skeleton adapter; not a release-gated companion surface |
| iMessage | Stub | Local/experimental adapter shape only |
| Twitter/X | Stub | Experimental adapter shape only |
| WhatsApp | Stub | Experimental adapter shape only |
| IRC | Alpha | Lightweight adapter surface; not part of the default proof path |

## Runtime ownership

Every accepted companion turn converges on the same enrichment and finalization path.

```text
Pre-turn:  affect detection → memory recall → persona context → system prompt composition
Post-turn: relationship update → message autosave → memory consolidation
```

- Transport-facing turn execution: `src/runtime/services/companion_turn.rs`
- Pre/post-turn enrichment owner: `src/core/agent/turn_enrichment.rs`
- Runtime composition root: `src/runtime/services/mod.rs`

This keeps CLI, gateway, desktop/operator surfaces, and channel adapters from inventing separate
companion behavior.

## Operator surfaces

### Daemon

```bash
cargo run -- daemon --host 127.0.0.1 --port 3000
```

### Gateway

```bash
cargo run -- gateway --host 127.0.0.1 --port 3000
```

Admin routes are not public. Pair first through `POST /pair`, then send
`Authorization: Bearer <token>` and `X-Asterel-Tenant: <tenant-id>` on `/admin/v1/*` requests.

<details>
<summary><strong>Gateway route overview</strong></summary>

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

Admin API (`/admin/v1/*`): runtime, usage, mood, activity timeline, sessions, governance,
memory review, companion approvals, auth profiles, channels, skills, cron, companion admin, tenant
management, and `GET /admin/v1/openapi.json`.

Optional trust-signal headers for external ingress (`POST /webhook`, `POST /a2a/v1/messages`):

- `X-Signature-Verified` (`true`/`1`)
- `X-Signature-Status`
- `X-Webhook-Signature-Status`
- `X-Source-Url` / `X-External-Source-Url`
- `X-Forwarded-Proto`, `Origin`, `Referer`

These headers influence trust scoring only when injected by a trusted edge verifier or reverse
proxy. Untrusted clients should not set them directly.

</details>

### Desktop console

The `desktop/` directory contains a Tauri 2 + React 19 + Tailwind 4 operator console for local
governance and diagnostics.

- Session review and transcript inspection
- Memory review, correction, forget, and self-amendment approval workflows
- Channel, runtime, exposure, and diagnostics visibility
- Companion admin surfaces and secondary tooling
- Context ingress for text, clipboard, files, and screenshots where supported

Run it against a local daemon:

```bash
cargo run -- daemon --host 127.0.0.1 --port 3000
# then in desktop/
pnpm tauri dev
```

## Security model

Asterel is designed for a **single-operator workspace** — one person, or a small trusted team,
running the daemon on a machine they control. It is **not** a multi-tenant SaaS, a public RAG
endpoint, or a shared inference gateway.

The threat model focuses on three containment boundaries:

1. The LLM and its tools execute under a deny-by-default command allowlist and path policy.
2. Externally reachable surfaces trust only edge-verified ingress signals.
3. Runtime secrets live in an encrypted local vault and are scrubbed from outbound surfaces.

Security highlights:

- Command allowlist + path policy enforcement
- Workspace-scoped execution and path writeback guards
- Gateway pairing flow with token hashing and lockout handling
- Encrypted local secret vault (ChaCha20-Poly1305)
- Secret scrubbing on outbound surfaces
- Trust-scored external ingress via edge-verifier headers

Full threat model and disclosure policy: [`SECURITY.md`](SECURITY.md).

## Configuration

Default paths:

```text
~/.asterel/config.toml
~/.asterel/workspace
```

Reference template: [`.env.example`](.env.example)

<details>
<summary><strong>Common environment overrides and ingress tuning</strong></summary>

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

</details>

## Command surface

<details>
<summary><strong>Top-level commands</strong></summary>

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

</details>

## Architecture map

<details>
<summary><strong>Repository layout</strong></summary>

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
├── plugins/       # Companion surfaces, extensions, integrations, MCP, skills
├── runtime/       # Diagnostics, environment, observability, services, tunnel, usage
├── security/      # Policy, pairing, auth, secrets, guards
├── transport/     # Channel adapters and HTTP/WS gateway
├── ui/            # Terminal UI building blocks
└── utils/         # Shared utilities

desktop/           # Tauri/React companion operator console
tests/             # Integration tests by domain
migrations/        # PostgreSQL schema migrations
```

</details>

## Development

Baseline checks:

```bash
cargo fmt -- --check
cargo clippy -- -D warnings
cargo check-all
cargo test
```

Strict release gate:

```bash
./scripts/release/human_like_release_gate.sh
```

Useful aliases:

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

Enable the repo hook locally:

```bash
git config core.hooksPath .githooks
```

## Documentation & policies

- Public docs site: <https://asterel-rs.github.io/asterel/>
- Japanese public docs: <https://asterel-rs.github.io/asterel/ja/>
- Contribution guide: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- Security policy: [`SECURITY.md`](SECURITY.md)
- Support guide: [`SUPPORT.md`](SUPPORT.md)
- Code of conduct: [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md)

## Legal notice

Asterel is dual-licensed under MIT or Apache-2.0, at your option. See [`LICENSE`](LICENSE),
[`LICENSE-MIT`](LICENSE-MIT), and [`LICENSE-APACHE`](LICENSE-APACHE).
