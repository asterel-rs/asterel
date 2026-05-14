# Changelog

All notable changes to Asterel are recorded here. The project is pre-1.0, so
minor releases may still include breaking changes; public release notes should
call those out explicitly.

## 0.1.2 - Unreleased

### Added

- Added the "Asterel's silhouette" concepts page (English and Japanese)
  capturing the cartographer-led identity outline and linking it to the
  existing persona model, companion-runtime, and boundaries pages.
- Added GPT-5.5 and GPT-5.5 Codex to the OpenAI onboarding catalog so
  operators can opt into the newer model from the wizard. Defaults still
  resolve to GPT-5.4 (flagship) and GPT-5.3 Codex (most capable Codex)
  to avoid an involuntary personality change for existing setups.

### Changed

- Rewrote the default persona prompt and the `CHARACTER.md` / `SOUL.md`
  onboarding templates around the cartographer silhouette — listens for
  the shape of what someone is trying to say, before deciding what to
  say back. Existing operator workspaces are not touched; new onboards
  scaffold the new templates.
- Switched the default stock identity emoji from 🦀 to 🐢 to match the
  documented turtle mascot, including the persona compiler's
  stock-identity detection and the interactive-mode banner.

### Fixed

- Exempted DM events from the Discord `guild_id` allowlist so configuring
  `guild_id` for public-channel restraint no longer silently suppresses
  the direct-message surface. (#11)
- Preserved the existing `channels_config` when running
  `asterel onboard --channels-only` so the repair wizard no longer
  wipes channels the operator did not explicitly re-enter; the channel
  menu also reports correct `[connected]` status for channels already
  on disk. (#10)
- Started injecting the operator's `CHARACTER.md` into the compiled
  prompt. Four sections are now read at runtime: `## Voice`, `## Avoids`,
  `## Asking Back`, and `## Voice Examples`. Untouched workspaces still
  short-circuit to `DEFAULT_PERSONA_GUIDANCE` so eval and judge
  stability are preserved; any operator edit to those sections now
  reaches the LLM as intended. (#12)

## 0.1.1

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
