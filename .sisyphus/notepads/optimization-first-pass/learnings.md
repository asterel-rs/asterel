## Task 1 baseline capture

- `cargo build --release --locked` completed successfully with exit status `0`.
- Baseline release artifact `target/release/asterel` size: `17820864` bytes.
- Captured Cargo feature tree and duplicate dependency tree under `.sisyphus/evidence/` for later optimization comparison.
- `cargo-bloat` is unavailable in this environment; recorded the required optional-tool sentinel instead of installing anything.
- `target/release/asterel --help` startup samples succeeded with elapsed times `5457000`, `5348200`, and `5467900` ns using a shell timer fallback because `/usr/bin/time` is absent.
- Reusable 8 MiB zero-filled fixture created at `.sisyphus/evidence/fixtures/large-8m.bin` with SHA256 `2daeb1f36095b44b318410b3f4e8b5d989dcc7bb023d1426c492dab0a3053e74`.

## Task 2 media clone reduction

- Discord and Telegram multipart retry loops can avoid per-attempt full-buffer clones by moving the original `Vec<u8>` into `Arc<[u8]>` once, then rebuilding each retry body with `bytes::Bytes::from_owner(arc_clone)` plus `Part::stream_with_length`.
- Local `reqwest` multipart source shows `Part::bytes` is `Into<Cow<'static, [u8]>>`, so using it with retry-safe shared bytes would still require a fresh owned byte buffer per retry attempt.
- Keep `MediaContent::Bytes(Vec<u8>)` for now: changing the enum payload to a shared-byte type would affect many channel adapters outside Task 2's scoped files.
- Telegram channel tests are behind the `telegram` feature; targeted Telegram media tests must run with `cargo test --features telegram ...` or they will be filtered out under default features.

## Task 3 MCP and consolidation locks

- `rmcp 1.5.0` exposes `RunningService` as `Deref<Target = Peer<RoleClient>>`; clone the `Peer<RoleClient>` under `McpConnection::service`'s read lock, then drop the guard before awaiting `list_all_tools()` or `call_tool()`.
- MCP disconnected behavior is easiest to cover directly in `src/plugins/mcp/client_connection.rs` with `McpConnection::disconnected_for_test`, which keeps the tests under the `mcp` feature and avoids spawning external servers.
- Rule-based memory consolidation intentionally serializes the full per-entity check -> append -> watermark sequence. Without a two-phase pending/applied watermark protocol, shortening that lock can either duplicate semantic events or skip a failed append permanently.
- `tests/memory/consolidation_orchestrator.rs::memory_consolidation_is_idempotent` now counts appends to `CONSOLIDATION_SLOT_KEY`, so duplicate checkpoint prevention is checked by append count, not only by total event count.

## Task 4 Prometheus render clone pressure

- Release measurement with 10,000 synthetic Prometheus render labels showed `render_text` improving from `1745289` ns/iter to `1414885` ns/iter after removing render-path full-map clones; output bytes/checksum stayed unchanged at `588495` bytes per render.
- The narrow safe optimization is to borrow each `HashMap<String, u64>` under its mutex while `push_labeled_counters` sorts borrowed entries and writes the same escaped label output; metric names, labels, sort order, and poisoned-lock empty-output behavior are preserved.
- Test-only snapshot helpers still clone maps because their public `#[cfg(test)]` return structs own `HashMap` values; changing those helpers would require changing test-facing data shapes, which is outside Task 4 scope.

## Task 6 binary footprint report

- `cargo build --release --locked` completed successfully with exit status `0`; final `target/release/asterel` size is `17822464` bytes, a `+1600` byte delta from the Task 1 baseline `17820864` bytes.
- Task 6 made no Cargo default, release profile, CI, dependency, or source changes; `GIT_MASTER=1 git diff -- Cargo.toml` produced no output.
- The largest dependency-family contributors visible from `cargo tree -e features` are HTTP/TLS/gateway (`axum`, `reqwest`, `tower-http`, `rustls`), PostgreSQL/vector storage (`sqlx-*`, `pgvector`), link extraction (`scraper` and HTML parser stack), i18n/config support (`rust-i18n`), and crypto/auth/RNG families.
- The duplicate-family candidates worth future investigation are `getrandom`/`rand`, `digest`/`sha2`/`crypto-common`, `chacha20`, `phf`, TOML/winnow/serde support paths, and `webpki-roots`.

## Task 5 repository recall investigation

- `search_and_merge_for_tier` is still sequential: FTS awaits first, then query embedding, then vector search (`src/core/memory/postgres/repository_recall.rs:168-184`).
- The recall search sequence is reachable from CLI/daemon turn paths via turn enrichment and working-memory materialization, so it is plausibly critical-path when a non-noop embedding provider is configured.
- A simple `tokio::join!`/`try_join!` between FTS and query embedding is not a proven semantics-preserving optimization: `get_or_compute_embedding` can touch or upsert `embedding_cache`, so parallelizing it with FTS can reorder DB side effects on FTS failure.
- Task 5 therefore left recall source unchanged; evidence is in `.sisyphus/evidence/task-5-recall-investigation.md`.

## Final verification wave lint cleanup

- Fixed the final-wave changed-file Clippy issues without changing behavior:
  - `src/plugins/mcp/client_connection.rs`: `Peer::clone(service)` instead of explicit deref cloning.
  - `src/runtime/observability/prometheus.rs`: comparator now uses `lhs.0.cmp(rhs.0)`.
  - `src/transport/channels/discord/http_client.rs`: route bucket key now passes `url` directly.
  - `src/transport/channels/attachments/load.rs`: test module now sits after all non-test items.
- `cargo fmt -- --check` passed after the edits.
- `lsp_diagnostics` reported no diagnostics on the touched Rust files after the final move.
