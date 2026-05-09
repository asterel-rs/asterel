---
title: Reproducibility
description: public research-quality Asterel release のための最小 commands と snapshot discipline。
---

research-quality release は clean checkout から再現できるべきです。そのため public repository は、internal notes を含む private development history ではなく、clean snapshot を公開します。

## Environment

- Rust toolchain: `1.88.0`
- Docs package manager: `pnpm`
- Desktop package manager: `pnpm`
- 既定の database-backed tests は、`TEST_DATABASE_URL` または `ASTEREL_POSTGRES_URL` が設定されている場合だけ PostgreSQL を使います。

## Minimal release evidence

```bash
cargo fmt -- --check
cargo clippy -- -D warnings
cargo check-all
cargo test
pnpm --dir docs build
cargo metadata --no-deps --format-version 1
```

## Research evidence snapshot

paper-facing artifact では次を記録します。

- public commit hash
- Rust、Node、pnpm、OS versions
- command list と pass / fail result
- fixture file hashes
- non-deterministic evaluation に使った model / provider versions
- random seeds と sampling parameters
- logs または transcripts に使った redaction policy
- known ignored tests と、その除外理由

## Clean snapshot requirement

private development history には internal review notes、local agent assets、operational handoffs が含まれる可能性があります。そのため public initialization は、public tracked set の clean snapshot を使うべきです。historical private blobs は research evidence ではなく、public repository history に残すべきではありません。

ignored cache や local-only files を copy しないために、snapshot helper を使います。

```bash
scripts/release/create_public_snapshot.sh /tmp/asterel-public-snapshot --dry-run
scripts/release/create_public_snapshot.sh /tmp/asterel-public-snapshot
```
