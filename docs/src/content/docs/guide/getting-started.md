---
title: Getting started
description: The shortest useful path from install to a running companion. For full reference, the repository README is canonical.
---

This page is the shortest path to a useful local run. It stays practical on
purpose; the [repository README](https://github.com/asterel-rs/asterel/blob/main/README.md)
remains the full reference for every command, route, and configuration key.

## Install (macOS/Linux)

```bash
curl -fsSL https://asterel-rs.github.io/asterel/install.sh | sh
asterel onboard --interactive
```

The installer uses GitHub release binaries when available, falls back to a
source build otherwise, and installs to `~/.local/bin` by default. If your shell
cannot find `asterel` yet, run `~/.local/bin/asterel onboard --interactive` or
add `~/.local/bin` to `PATH`.

## First run

```bash
asterel agent
asterel agent --message "Summarize my open tasks"
```

`agent` is useful for confirming the local provider and config. Use the daemon
when you want the real product shape.

## Build from source

```bash
git clone https://github.com/asterel-rs/asterel.git
cd asterel
cargo build --release
cargo run -- onboard --interactive
cargo run -- agent
```

Source builds need Rust stable, `protoc` v29 or newer, Git, and a provider
credential or local provider selected during onboarding. PostgreSQL is the
recommended memory backend; Markdown and `none` are available for constrained
local testing.

## Run the full daemon

For Discord, gateway, scheduler, and heartbeat to run together:

```bash
asterel daemon --host 127.0.0.1 --port 3000
```

This is the mode the [turn pipeline](../../architecture/turn-pipeline/) was built for. Discord text is the primary product surface; connect it through the daemon using the configuration written by onboarding and the channel settings described in the README. Once a Discord message is accepted as a companion turn, it uses the shared transport path and the same enrichment and verification contract used by other accepted companion turns.

Check the local runtime before connecting Discord:

```bash
asterel config validate
asterel doctor
asterel status
```

Then follow [Run Discord](../discord-setup/) for the primary channel setup.

## Where to go next

- **Connect the primary surface** — [Run Discord](../discord-setup/).
- **Operate the daemon** — [Operating locally](../operating-locally/) and [Gateway](../../operator/gateway/).
- **Review memory and governance** — [Memory review](../../operator/memory-review/) and [Security and governance](../../architecture/security-governance/).
- **Understand the design** — [Overview](../../overview/) and [Companion runtime](../../concepts/companion/).
- **Fix setup issues** — [Troubleshooting](../troubleshooting/).
