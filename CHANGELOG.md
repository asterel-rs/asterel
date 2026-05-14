# Changelog

All notable changes to Asterel are recorded here. The project is pre-1.0, so
minor releases may still include breaking changes; public release notes should
call those out explicitly.

## 0.1.1 - Unreleased

### Added

- Added the one-line installer script for GitHub release binaries with source
  build fallback.
- Added the GraphRAG extraction pipeline for memory graph projection.
- Added Prometheus text exposition formatting for runtime observer snapshots.
- Added a tool introspection helper and tightened semantic compaction
  formatter behavior.

### Fixed

- Made the gateway defense kill switch fail closed, so an invalid or disabled
  defense configuration no longer silently opens the external ingress boundary.
- Hardened gateway tenant isolation by binding uploaded content to tenant context
  and pairing tenant scopes with their authenticated principals.
- Serialized memory consolidation state writes to prevent concurrent autosave and
  consolidation paths from racing each other.
- Bounded cron shell jobs during daemon reload so scheduler refreshes cannot
  leave old jobs running outside the intended supervisor lifecycle.
- Avoided awaiting while holding runtime locks, reducing deadlock risk and
  improving fairness around shared runtime state.
- Enabled optional ONNX Runtime artifact downloads needed by the intent
  classifier build path.
- Stripped HTML-entity-decoded invisible characters during security scrubbing.
- Bumped vulnerable docs and desktop lockfile dependencies, including the docs
  Astro runtime update to `6.1.10`.

### Performance

- Shared media bytes across transport upload retries instead of cloning the
  payload for each retry attempt.
- Batched memory recall reinforcement updates to reduce write amplification.
- Avoided cloning observability metric maps while rendering Prometheus text
  exposition.

### Tests and CI

- Added docs guards against stale public references and expanded feature-matrix
  CI coverage for the PostgreSQL backend.
- Avoided caching Cargo-installed binaries in build jobs so release and CI jobs
  do not restore stale tool artifacts.
- Hardened memory, Codex provider, and release workflow checks, including
  unwrap-use lint pressure and required release build target installation.

### Documentation

- Aligned runtime reference docs with the shared companion turn surface and
  clarified memory consolidation wiring.
- Updated the public release roadmap, transport matrix, security contact,
  localization, operations, release, and architecture documentation.
- Fixed Starlight layout overflow in the docs site.

## 0.1.0

### Added

- Discord-first companion runtime proof path: pickup, turn enrichment,
  response finalization, post-turn update, and local governance.
- PostgreSQL memory backend with Markdown fallback, recall, review/correction,
  forget paths, and GraphRAG projection support.
- Gateway, daemon, desktop/operator, skills, subagent, MCP, and secondary
  channel surfaces as extension points around the shared turn contract.

### Operational notes

- Discord is the only default release-gated transport. Secondary channel
  adapters are marked separately as alpha or stub surfaces in the README.
