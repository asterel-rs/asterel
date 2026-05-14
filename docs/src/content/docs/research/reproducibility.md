---
title: Reproducibility
description: Minimum commands and snapshot discipline for a public research-quality Asterel release.
---

A research-quality release should be reproducible from a clean checkout. The
public repository should therefore publish a clean snapshot, not a private
development history containing internal notes.

## Environment

- Rust toolchain: `1.88.0`
- Docs package manager: `pnpm`
- Desktop package manager: `pnpm`
- Default database-backed tests use PostgreSQL only when `TEST_DATABASE_URL` or
  `ASTEREL_POSTGRES_URL` is configured.

## Minimal release evidence

For the strict release gate, use the checked-in release script so quality,
supply-chain, baseline, and replay checks stay in one source of truth:

```bash
./scripts/release/human_like_release_gate.sh
```

When the full strict gate is too expensive for a local documentation pass, record
that limitation and run the closest targeted subset:

```bash
cargo fmt -- --check
cargo clippy -- -D warnings
cargo check-all
cargo test
docker compose config
pnpm --dir docs build
cargo metadata --no-deps --format-version 1
```

## Research evidence snapshot

For a paper-facing artifact, record:

- public commit hash;
- Rust, Node, pnpm, and OS versions;
- command list and pass/fail result;
- fixture file hashes;
- model/provider versions for any non-deterministic evaluation;
- random seeds and sampling parameters;
- redaction policy used for any logs or transcripts;
- known ignored tests and why they were excluded.
- whether the strict release gate or a documented targeted subset was run.

## Clean snapshot requirement

Because the private development history may include internal review notes, local
agent assets, and operational handoffs, public initialization should use a clean
snapshot of the public tracked set. Historical private blobs are not research
evidence and should not be preserved in the public repository history.

Use the snapshot helper to avoid copying ignored caches or local-only files:

```bash
scripts/release/create_public_snapshot.sh /tmp/asterel-public-snapshot --dry-run
scripts/release/create_public_snapshot.sh /tmp/asterel-public-snapshot
```
