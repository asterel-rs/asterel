---
title: Gateway
description: How the local HTTP/WebSocket gateway fits into a single-operator Asterel deployment.
---

The gateway is the local HTTP/WebSocket edge around the companion runtime. It is
useful for health checks, pairing, webhooks, A2A-style messages, companion
surface routes, and admin APIs. It is not a public SaaS boundary.

## Start locally

```bash
cargo run -- gateway --host 127.0.0.1 --port 3000
```

For normal operation, prefer the daemon because it runs gateway, channels,
scheduler, heartbeat, and runtime services together:

```bash
cargo run -- daemon --host 127.0.0.1 --port 3000
```

## Local-first posture

The safe default is local binding with pairing required:

```toml
[gateway]
host = "127.0.0.1"
port = 3000
require_pairing = true
allow_public_bind = false
```

If you need public reachability, put a trusted edge or tunnel in front of the
runtime. Do not turn a local admin API into an unauthenticated public service.

## Public routes vs admin routes

Public routes include health, readiness, pairing, gateway OpenAPI, webhook, A2A,
companion surface, and WebSocket entrypoints. Admin routes live under
`/admin/v1/*` and require pairing plus explicit tenant scope.

```text
Authorization: Bearer <token>
X-Asterel-Tenant: <tenant-id>
```

The tenant header scopes local operator state. It is not a claim that Asterel is
a hosted multi-tenant SaaS.

## What the gateway should not own

The gateway should normalize transport input and expose routes. It should not be
the owner of memory, persona, session continuity, prompt policy, or response
verification. Those belong to shared runtime/core services so Discord, gateway,
and channel turns do not drift into separate products.
