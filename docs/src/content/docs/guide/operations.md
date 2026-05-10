---
title: Operations
description: Backup, restore, secret rotation, and monitoring runbooks for local operators.
---

# Operations

Asterel is a single-operator runtime. Treat the workspace, database, and local
secret vault as production state even when running on a personal machine.

## Backup

Back up these artifacts together so memory, persona, and configuration remain
consistent:

- `~/.asterel/config.toml` and any workspace-local config overrides.
- The encrypted secret vault and secret-encryption metadata.
- PostgreSQL database when using the `postgres` memory backend.
- Markdown memory directory when using the fallback backend.
- Release artifact or exact commit SHA used by the daemon.

For PostgreSQL, use a logical dump before upgrades:

```bash
pg_dump --format=custom --file=asterel.dump "$ASTEREL_POSTGRES_URL"
```

Keep backups encrypted at rest. Do not copy provider keys into unencrypted bug
reports, CI artifacts, or shared logs.

## Restore

1. Stop the daemon and channel workers.
2. Restore config and the secret vault first.
3. Restore PostgreSQL into an empty database:

   ```bash
   pg_restore --clean --if-exists --dbname "$ASTEREL_POSTGRES_URL" asterel.dump
   ```

4. Start `asterel doctor` and confirm memory, gateway, channel, and
   observability status.
5. Start the daemon and watch post-turn hook and memory metrics for the first
   few turns.

## Secret rotation

Rotate secrets after operator handoff, suspected exposure, provider dashboard
changes, or public ingress misconfiguration.

1. Revoke the old provider/channel/tunnel token at the upstream service.
2. Write the new secret through the configured secret store or encrypted config
   path; avoid editing plaintext files when secret encryption is enabled.
3. Restart the daemon or reload the affected channel worker.
4. Run a minimal smoke test for the affected provider or transport.
5. Check logs and metrics for authentication failures.

## Monitoring

Use the `prometheus` observability backend when running long-lived daemons. The
observer can render Prometheus text exposition snapshots; expose them only on a
localhost or trusted-admin scrape endpoint. Track at minimum:

- observer event/error totals;
- post-turn hook status totals;
- memory lifecycle and SLO violation totals;
- signal ingestion and deduplication labels;
- channel worker heartbeat status.

If metrics stop changing while the daemon is still receiving traffic, treat that
as an incident: capture logs, stop channel delivery if needed, and verify that
post-turn update is not stuck behind a failing memory or provider call.
