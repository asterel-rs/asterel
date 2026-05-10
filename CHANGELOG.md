# Changelog

All notable changes to Asterel are recorded here. The project is pre-1.0, so
minor releases may still include breaking changes; public release notes should
call those out explicitly.

## 0.1.0 - Unreleased

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
- Prometheus observer state can render text exposition snapshots; runtime HTTP
  scrape wiring remains an explicit follow-up before production monitoring.
