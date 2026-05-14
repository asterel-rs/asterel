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

strict release gate では、quality、supply-chain、baseline、replay checks を 1 つの source of truth に保つため、checked-in release script を使います。

```bash
./scripts/release/human_like_release_gate.sh
```

local documentation pass で full strict gate が重すぎる場合は、その制限を記録し、もっとも近い targeted subset を実行します。

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

paper-facing artifact では次を記録します。

- public commit hash
- Rust、Node、pnpm、OS versions
- command list と pass / fail result
- fixture file hashes
- non-deterministic evaluation に使った model / provider versions
- random seeds と sampling parameters
- logs または transcripts に使った redaction policy
- known ignored tests と、その除外理由
- strict release gate または documented targeted subset のどちらを実行したか

## Clean snapshot requirement

private development history には internal review notes、local agent assets、operational handoffs が含まれる可能性があります。そのため public initialization は、public tracked set の clean snapshot を使うべきです。historical private blobs は research evidence ではなく、public repository history に残すべきではありません。

ignored cache や local-only files を copy しないために、snapshot helper を使います。

```bash
scripts/release/create_public_snapshot.sh /tmp/asterel-public-snapshot --dry-run
scripts/release/create_public_snapshot.sh /tmp/asterel-public-snapshot
```
