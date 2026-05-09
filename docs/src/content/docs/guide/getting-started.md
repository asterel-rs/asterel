---
title: Getting started
description: The shortest useful path from clone to a running companion. For full reference, the repository README is canonical.
---

This page is the shortest path to a useful local run. It stays practical on
purpose; the [repository README](https://github.com/asterel-rs/asterel/blob/main/README.md)
remains the full reference for every command, route, and configuration key.

## Prerequisites

- Rust stable — pinned via `rust-toolchain.toml`
- `protoc` v29 or newer
- Git
- A model provider credential or a local provider configured during onboarding
- For the recommended memory backend: PostgreSQL available to the runtime.
  Markdown fallback is useful for constrained testing, but PostgreSQL is the
  product posture for durable relationship continuity.

## Build

```bash
git clone https://github.com/asterel-rs/asterel.git
cd asterel
cargo build --release
```

## First run

The first run is ordered. `onboard --interactive` **must** complete before `agent` can start — it writes `~/.asterel/config.toml` and initializes the workspace.

```bash
# Interactive onboarding wizard (run first on a fresh install)
cargo run -- onboard --interactive

# Start the interactive agent loop (requires completed onboarding)
cargo run -- agent

# One-shot message
cargo run -- agent --message "Summarize my open tasks"
```

Use `agent` to confirm the local provider and config are usable. Use the daemon
when you want the real product shape.

## Run the full daemon

For Discord, gateway, scheduler, and heartbeat to run together:

```bash
cargo run -- daemon --host 127.0.0.1 --port 3000
```

This is the mode the [turn pipeline](../../architecture/turn-pipeline/) was built for. Discord text is the primary product surface; connect it through the daemon using the configuration written by onboarding and the channel settings described in the README. Once a Discord message is accepted as a companion turn, it uses the shared transport path and the same enrichment and verification contract used by other accepted companion turns.

Check the local runtime before connecting Discord:

```bash
cargo run -- config validate
cargo run -- doctor
cargo run -- status
```

Then follow [Run Discord](../discord-setup/) for the primary channel setup.

## Where to go next

- **Connect the primary surface** — [Run Discord](../discord-setup/).
- **Operate the daemon** — [Operating locally](../operating-locally/) and [Gateway](../../operator/gateway/).
- **Review memory and governance** — [Memory review](../../operator/memory-review/) and [Security and governance](../../architecture/security-governance/).
- **Understand the design** — [Overview](../../overview/) and [Companion runtime](../../concepts/companion/).
- **Fix setup issues** — [Troubleshooting](../troubleshooting/).
