---
title: Troubleshooting
description: Common local operation failures and the first checks to run before widening configuration.
---

Most Asterel failures are boundary problems: config did not load, a channel did
not accept a message, a provider could not answer, memory is unavailable, or the
gateway is not paired. Start with the smallest check that isolates the boundary.

## Quick health pass

Run these before changing behavior flags:

```bash
cargo run -- config validate
cargo run -- doctor
cargo run -- status
cargo run -- channel list
```

If you changed docs or source locally, also run the relevant gate:

```bash
cargo fmt -- --check
cargo clippy -- -D warnings
cargo test
pnpm --dir docs build
```

## Daemon starts, but Discord does not reply

Check in this order:

1. **Channel enabled** — `cargo run -- channel list` shows Discord as configured.
2. **Token and server access** — the bot is present in the intended server or DM.
3. **Scope filters** — `guild_id` and `allowed_users` do not exclude the message.
4. **Pickup policy** — `direct_only` means ambient public chatter is ignored.
5. **Provider** — the configured model provider can complete a basic agent turn.
6. **Verifier** — response finalization may block an unsafe or leaking draft.

Do not switch to broad ambient pickup just to prove the bot is alive. Use a direct
mention or DM first.

## Gateway pairing fails

The gateway is local-first and pairing is required by default.

- Confirm the daemon or gateway is listening on the expected host and port.
- Use `127.0.0.1` for local operation unless you deliberately placed a trusted
  edge in front of the gateway.
- Pair before calling `/admin/v1/*` routes.
- Send the returned bearer token and an explicit tenant header for admin calls.

```text
Authorization: Bearer <token>
X-Asterel-Tenant: <tenant-id>
```

Tenant scope is local operator context. It is not a public SaaS isolation model.

## PostgreSQL is unavailable

PostgreSQL is the recommended memory backend. If it is unavailable:

- verify `ASTEREL_POSTGRES_URL` or the configured `postgres_url`;
- verify the database is reachable from the daemon process;
- use Markdown fallback only when you accept reduced product evidence;
- remember that `backend = "none"` currently routes to the Markdown compatibility
  fallback, not a true stateless store.

If the goal is a public release or paper artifact, record which backend was used.

## Provider key missing or wrong

Provider errors usually mean one of these is missing or mismatched:

- `ASTEREL_API_KEY` or provider-specific credentials;
- `ASTEREL_PROVIDER` and `ASTEREL_MODEL`;
- auth profile selected by the local config;
- provider base URL for compatible providers.

Keep provider secrets in environment or the runtime's secret path. Do not paste
keys into issue reports.

## A response looks too long, too eager, or too private

Treat this as a companion-quality issue, not only a prompt issue.

- Check whether the turn was public, thread, or DM.
- Check pickup policy and public/private exposure posture.
- Check whether memory recall surfaced a private fact into a public context.
- Keep response finalization enabled on the default path.
- Use memory review/correction if a remembered fact is wrong.

If the issue could include private memory or a real user transcript, do not post
the raw content publicly. Redact or use the security/private reporting path.
