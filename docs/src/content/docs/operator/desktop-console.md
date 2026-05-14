---
title: Desktop console
description: "What the desktop app is for: operator review, diagnostics, and governance around the companion runtime."
---

The desktop app is a secondary operator console. It is not the primary place a
user meets the companion, and it is not a second runtime. It reads and manages
the daemon/gateway runtime through admin APIs.

## Run it locally

Start the daemon first:

```bash
cargo run -- daemon --host 127.0.0.1 --port 3000
```

Then start the desktop app:

```bash
pnpm --dir desktop tauri dev
```

## What it is for

Use the desktop console for:

- session review and transcript inspection;
- memory review, correction, forget, and self-amendment approval workflows;
- exposure diagnostics and governance checks;
- runtime, channel, and scheduler health, with durable cron state backed by PostgreSQL;
- auth, provider, skill, cron, and tenant/operator settings.

## What it is not for

The desktop app should not become:

- the main user-facing chat product;
- a forked owner of runtime state;
- a place where private memory is copied into unrelated notes;
- a bypass around gateway pairing, tenant scope, or memory governance.

The useful mental model is a desk for the operator, not a second companion.

## Verification for desktop changes

For desktop source changes, use the project-defined checks:

```bash
pnpm --dir desktop exec oxfmt
pnpm --dir desktop exec oxlint --react-plugin src
pnpm --dir desktop build
```

The build may warn about a large shared vendor chunk; that warning is not by
itself a failed build.
