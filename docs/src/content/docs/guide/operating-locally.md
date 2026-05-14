---
title: Operating locally
description: How to run Asterel as a single-operator local deployment, what to check first, and how to avoid confusing test modes with product modes.
---

Asterel is designed to be operated by one person, or a small trusted team, on a machine they control. The local daemon is the center of that deployment. It combines the gateway, channels, scheduler, heartbeat, and companion turn runtime.

```bash
cargo run -- daemon --host 127.0.0.1 --port 3000
```

Use `agent` for a quick direct conversation loop. Use `daemon` when you are exercising the product shape: Discord text, gateway routes, background tasks, and operator surfaces all around the same runtime.

If you are setting up for the first time, read [Getting started](../getting-started/) before this page. If Discord is the next step, use [Run Discord](../discord-setup/) after the daemon passes health checks.

## First checks

After onboarding, these commands are the fastest way to separate configuration problems from runtime problems:

```bash
cargo run -- config validate
cargo run -- doctor
cargo run -- status
cargo run -- channel list
```

Use `doctor --repair` only for safe local repairs. If the issue is an external service, credential, or database, fix that dependency rather than treating the runtime as broken.

Cron and scheduler operations that persist durable job state require PostgreSQL
configuration through `ASTEREL_POSTGRES_URL` or `memory.postgres_url`. Without
that state, read-only cron status can report unavailable/degraded instead of
pretending scheduled jobs are active.

## Local gateway model

The gateway defaults to `127.0.0.1:3000`. That is intentional. Admin routes are not public routes, and `/admin/v1/*` requires pairing plus tenant scope:

```text
Authorization: Bearer <token>
X-Asterel-Tenant: <tenant-id>
```

Pairing gives a client permission to talk to the daemon. Tenant scope tells admin routes which operator workspace context they are acting on. This is not a SaaS tenant boundary; it is local operator scoping.

## Discord operation

Discord text is the primary product surface. The operational path is:

```text
Discord event
  -> channel adapter
  -> pickup / ingress policy
  -> shared companion turn
  -> reply delivery
  -> post-turn memory and relationship update
```

If a Discord message does not produce a reply, check the decision points in that order:

- The Discord bot token and guild/application restrictions are correct.
- The channel is enabled and not disabled by operator state.
- The message passes the pickup policy.
- The user is allowed if `allowed_users` is configured.
- The provider can complete the turn.
- The pre-send verifier allows the response.

Do not loosen pickup or allowlists just to get visible activity. A companion that speaks too eagerly in a public room breaks the product promise.

## Desktop operation

The desktop app is a secondary operator console. Run it against a daemon:

```bash
cargo run -- daemon --host 127.0.0.1 --port 3000
pnpm --dir desktop tauri dev
```

Use desktop for governance, diagnostics, session review, memory review/correction/forget, exposure diagnostics, channel health, and companion admin workflows. It is not the primary user-facing companion surface, and it should not become the owner of runtime state. See [Desktop console](../../operator/desktop-console/) for the operator model.

## Memory review loop

When behavior feels wrong, avoid jumping straight to prompt edits. First ask
whether the runtime remembered the wrong thing, recalled it in the wrong context,
or exposed it on the wrong surface. The operator loop is:

```text
inspect compact memory -> check provenance -> correct / forget / approve
  -> verify the next turn uses the updated projection
```

See [Memory review](../../operator/memory-review/) for the public-safe model.

## Useful local gates

For code changes, the local Rust gate is:

```bash
cargo fmt -- --check
cargo clippy -- -D warnings
cargo check-all
cargo test
```

For docs changes:

```bash
pnpm --dir docs build
```

For desktop changes:

```bash
pnpm --dir desktop exec oxfmt
pnpm --dir desktop exec oxlint --react-plugin src
pnpm --dir desktop build
```

Use the smaller focused gate that matches the change first. Run the full gate before release-quality handoff.

## What healthy operation looks like

A healthy local deployment has these properties:

- The daemon starts without falling back to a surprise config.
- The provider, memory backend, gateway, and primary channel are all explicit.
- Discord messages become turns only when pickup policy accepts them.
- Every accepted turn passes through enrichment, pre-send verification, and post-turn update.
- Memory consolidation and hygiene can run without blocking the live turn path.
- Admin access requires pairing and tenant scope.
- Desktop reads runtime/admin state; it does not fork a second runtime truth.
