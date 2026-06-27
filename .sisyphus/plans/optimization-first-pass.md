# Optimization First Pass

## TL;DR
> **Summary**: Run an evidence-first optimization pass for Asterel's CLI/daemon runtime, prioritizing memory/RSS and runtime hazards before binary footprint reporting. Preserve default features and documented out-of-box behavior; no broad rewrites.
> **Deliverables**:
> - Baseline and final measurement evidence for release binary size, dependency footprint, idle daemon footprint, attachment/media flow, and companion/repository recall paths where touched.
> - Retry-preserving large-buffer clone reductions in media upload paths.
> - Lock-across-await fixes in MCP connection, plus a decision-complete consolidation guardrail: preserve consolidation serialization unless a tested two-phase watermark protocol is implemented in this task.
> - Measured-only clone/snapshot trimming report for observability/repository recall hot paths.
> - Report-only binary/dependency analysis with recommendations, no default feature changes.
> **Effort**: Medium
> **Parallel**: YES - 3 waves
> **Critical Path**: Task 1 → Tasks 2/3 → Tasks 4/5 → Task 6 → Final Verification

## Context
### Original Request
- User requested: 「コードを深く解析したうえで現行より最適化を図る」.
- User scope guardrail: 「過剰な最適化は避けたうえで、明らかなボトルネック、コード改善を主とする」.
- User added binary/memory focus: 「バイナリーサイズ メモリ使用量に関しても元々そこまで気にしていなかったフシがあるので、ここらに関しても詰める」.

### Interview Summary
- Primary target: CLI/daemon Rust binary and daemon/gateway runtime first.
- Priority order: memory/runtime first, binary size second.
- Feature defaults: preserve current default features and documented out-of-box behavior.
- Test strategy: tests-after, with targeted tests around changed retry/ownership/lock behavior plus existing suites.
- Measurement policy: baseline first; no hard numeric target; every optimization claim must show no regression plus meaningful before/after evidence.

### Metis Review (gaps addressed)
- Added explicit stop rules to prevent optimization sprawl.
- Added concrete measurement scenarios and evidence files.
- Added retry semantics tests for multipart uploads so byte-sharing does not break single-use request bodies.
- Added lock invariant notes and `clippy::await_holding_lock` verification.
- Kept dependency/feature work report-only because defaults must not change in this pass.
- Avoided API credential-dependent QA; use local/unit/mocked tests only.

### Oracle Review (strategy incorporated)
- Prioritize retained allocations and accidental cloning before dependency graph churn or async architecture redesign.
- Safe sequence: baseline → media byte clones → lock-across-await → measured cache/snapshot trimming → report-only dependency/binary analysis.
- Defer subprocess pools, scheduler redesign, allocator swaps, and release-profile churn.

## Work Objectives
### Core Objective
Reduce clear memory/runtime overhead in the CLI/daemon path without changing documented behavior, default features, public integration availability, or release profile defaults.

### Deliverables
- `.sisyphus/evidence/task-1-baseline-summary.md` with commands, median values, binary size, and tool availability.
- Source changes for media byte-sharing/retry-preserving upload paths, if measurement confirms current full-buffer clone risk.
- Source changes for MCP lock guards held across `.await`; consolidation either receives a fully tested two-phase watermark fix or remains intentionally serialized with evidence and no source change.
- Report-only notes for repository recall and Prometheus snapshot clone pressure unless local measurement clearly supports a small safe change.
- `.sisyphus/evidence/task-6-binary-footprint.md` with dependency/bloat findings and future recommendations.

### Definition of Done (verifiable conditions with commands)
- `cargo fmt -- --check` passes.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings -D clippy::await_holding_lock` passes, or any false positive is documented in `.sisyphus/evidence/final-clippy-await-holding-lock.txt` with file/line and justification.
- `cargo test --workspace --all-features` passes.
- `cargo build --release --locked` passes and final `target/release/asterel` size is recorded.
- Before/after evidence exists for every task that claims a performance or memory improvement.
- No changes to `Cargo.toml` `[features] default`, `[profile.release]`, allocator choice, panic strategy, or public command behavior.

### Must Have
- Baseline measurements before optimization changes.
- Tests proving upload retry can rebuild valid request bodies after a transient 429/failure.
- Tests proving empty, small, and 8 MiB attachment byte paths still work.
- Lock fixes must avoid holding `tokio::sync` guards across external `.await` calls.
- All evidence files stored under `.sisyphus/evidence/`.

### Must NOT Have
- No unsafe code.
- No allocator swaps.
- No default feature removals or dependency feature pruning in this pass.
- No broad daemon/runtime/subprocess-pool/desktop architecture rewrite.
- No real Discord/Telegram credential-dependent tests.
- No changing release profile flags without a separate plan.
- No unrelated cleanup refactors.
- No `Co-authored-by: Sisyphus <clio-agent@sisyphuslabs.ai>` or `Ultraworked with ...` trailers in commits.

## Verification Strategy
> ZERO HUMAN INTERVENTION - all verification is agent-executed.
- Test decision: tests-after; add/update targeted tests around changed retry/ownership/lock behavior.
- QA policy: Every task has agent-executed scenarios.
- Evidence: `.sisyphus/evidence/task-{N}-{slug}.{ext}`.
- Optional tools policy: `cargo bloat`, `/usr/bin/time -v`, `hyperfine`, and `cargo llvm-lines` are useful but not required. If unavailable, record `tool unavailable` in the evidence file and continue with required Cargo checks.

## Execution Strategy
### Parallel Execution Waves
> Target: 5-8 tasks per wave. <3 per wave (except final) = under-splitting.
> Extract shared dependencies as Wave-1 tasks for max parallelism.

Wave 1: Task 1 baseline and fixtures (quick).
Wave 2: Task 2 media byte-sharing and Task 3 lock-across-await fixes (parallel after baseline).
Wave 3: Task 4 measured clone/snapshot trimming, Task 5 repository recall investigation, Task 6 binary footprint report (parallel after baseline; Task 4/5 only patch if local evidence supports a narrow safe change).
Final Wave: F1-F4 review agents in parallel after all implementation tasks.

### Dependency Matrix (full, all tasks)
| Task | Depends On | Blocks |
|---|---|---|
| 1. Baseline measurement harness | none | 2, 3, 4, 5, 6 |
| 2. Media byte sharing and retry tests | 1 | Final Verification |
| 3. Lock-across-await fixes | 1 | Final Verification |
| 4. Prometheus/cache snapshot trimming | 1 | Final Verification |
| 5. Repository recall investigation | 1 | Final Verification |
| 6. Binary/dependency report-only analysis | 1 | Final Verification |

### Agent Dispatch Summary (wave → task count → categories)
- Wave 1 → 1 task → `deep`.
- Wave 2 → 2 tasks → `deep`, `rust` skill recommended.
- Wave 3 → 3 tasks → `deep` or `unspecified-high`, `rust` skill recommended.
- Final Verification → 4 review tasks → `oracle`, `unspecified-high`, `deep`.

## TODOs
> Implementation + Test = ONE task. Never separate.
> EVERY task MUST have: Agent Profile + Parallelization + QA Scenarios.

- [x] 1. Capture baseline measurements and create reusable fixtures

  **What to do**: Create `.sisyphus/evidence/` if missing. Capture current release build size, dependency/features tree, duplicate deps, and baseline runtime/memory evidence before source changes. Use a generated 8 MiB local fixture for media-path tests under a temp directory or `.sisyphus/evidence/fixtures/large-8m.bin`; do not commit large generated binary fixtures. Record exact commands, tool versions where available, and output paths. If optional tools are unavailable, record that and proceed.
  **Must NOT do**: Do not change source code, Cargo features, release profile, or CI config in this task.

  **Recommended Agent Profile**:
  - Category: `deep` - Reason: needs careful measurement setup and evidence discipline.
  - Skills: [`rust`] - Rust/Cargo command and benchmark hygiene.
  - Omitted: [`optimize`] - This is backend Rust measurement, not UI performance.

  **Parallelization**: Can Parallel: NO | Wave 1 | Blocks: tasks 2, 3, 4, 5, 6 | Blocked By: none

  **References** (executor has NO interview context - be exhaustive):
  - Release profile: `Cargo.toml:158-163` - existing size-oriented release settings; record, do not change.
  - Default features: `Cargo.toml:124-143` - preserve defaults; report only.
  - CI checks: `.github/workflows/ci.yml:52-67` - fmt/clippy/check-all/nextest patterns.
  - CI release build: `.github/workflows/ci.yml:179-204` - release build command pattern.
  - Memory throughput evidence precedent: `.github/workflows/ci.yml:300-304` - existing artifact naming style.

  **Acceptance Criteria** (agent-executable only):
  - [ ] Run `cargo build --release --locked` and record final command status plus `target/release/asterel` byte size in `.sisyphus/evidence/task-1-baseline-summary.md`.
  - [ ] Run `cargo tree -e features > .sisyphus/evidence/task-1-cargo-tree-features.txt`.
  - [ ] Run `cargo tree --duplicates > .sisyphus/evidence/task-1-cargo-tree-duplicates.txt`.
  - [ ] Run `cargo bloat --release --crates --bin asterel > .sisyphus/evidence/task-1-cargo-bloat-crates.txt` if installed; otherwise write `cargo-bloat unavailable` to that file.
  - [ ] Record at least 3 repeated runs for `cargo run --release -- --help` startup timing using available shell timing in `.sisyphus/evidence/task-1-startup-help.txt`.
  - [ ] Generate an 8 MiB fixture and record its SHA256 in `.sisyphus/evidence/task-1-fixtures.txt`.

  **QA Scenarios** (MANDATORY - task incomplete without these):
  ```
  Scenario: Baseline release artifact captured
    Tool: Bash
    Steps: Run cargo build --release --locked; record byte size with stat -c%s target/release/asterel; append command and status to .sisyphus/evidence/task-1-baseline-summary.md
    Expected: Build exits 0 and byte size is a positive integer.
    Evidence: .sisyphus/evidence/task-1-baseline-summary.md

  Scenario: Optional bloat tooling absent
    Tool: Bash
    Steps: If cargo bloat is not installed, write exactly 'cargo-bloat unavailable' to .sisyphus/evidence/task-1-cargo-bloat-crates.txt
    Expected: Missing optional tool does not fail the task; evidence file exists.
    Evidence: .sisyphus/evidence/task-1-cargo-bloat-crates.txt
  ```

  **Commit**: NO | Message: `n/a` | Files: `.sisyphus/evidence/*` only

- [x] 2. Reduce media byte cloning without breaking multipart retry semantics

  **What to do**: Inspect current ownership boundaries for media bytes. Prefer a narrow byte-sharing type such as `bytes::Bytes` or `Arc<[u8]>` only if it avoids repeated full-buffer clones and lets every retry reconstruct a fresh multipart `Part`. Update media structs and call sites minimally. Add tests for empty, small, 8 MiB, and retry-after-429 behavior using local/mocked HTTP; no real Discord/Telegram credentials.
  **Must NOT do**: Do not introduce broad media abstraction, streaming upload redesign, unsafe code, or real API integration tests. Do not change max attachment size semantics.

  **Recommended Agent Profile**:
  - Category: `deep` - Reason: cross-file ownership changes can silently break retries.
  - Skills: [`rust`] - Ownership and async test correctness.
  - Omitted: [`api-security`] - This task must preserve existing SSRF/size checks but is not a security audit.

  **Parallelization**: Can Parallel: YES | Wave 2 | Blocks: Final Verification | Blocked By: task 1

  **References** (executor has NO interview context - be exhaustive):
  - Media attachment type: `src/transport/channels/traits.rs:10-21` - `MediaAttachment` derives `Clone`; `MediaContent::Bytes(Vec<u8>)` is the first full-buffer clone source.
  - Attachment load clone: `src/transport/channels/attachments/load.rs:29-32` - `load_attachment_bytes` clones in-memory bytes.
  - Attachment size guard: `src/transport/channels/attachments/load.rs:9-12` and `62-83` - preserve 25 MiB limit and streaming collection behavior.
  - Output attachment mapping: `src/transport/channels/attachments/load.rs:104-183` - file/URL conversion points.
  - Discord retry clone: `src/transport/channels/discord/http_client.rs:223-242` - `bytes.clone()` inside retry loop; each retry must get a fresh `Part`.
  - Telegram retry clone: `src/transport/channels/telegram/api.rs:16-27` - `file_bytes.clone()` inside retry loop.
  - Telegram byte send call sites: `src/transport/channels/telegram/api.rs:152-167` and `216-231` - preserve public method behavior.

  **Acceptance Criteria** (agent-executable only):
  - [ ] Add/update unit tests proving empty byte attachment remains valid.
  - [ ] Add/update unit tests proving an 8 MiB byte attachment can be loaded/sent through the prepared multipart path without cloning per retry where the new representation is shared.
  - [ ] Add/update mocked retry test proving first transient 429/failure then success sends identical payload bytes on the retry.
  - [ ] Run `cargo test --workspace --all-features attachment -- --nocapture` or the closest exact targeted test filter created for this task, and save output to `.sisyphus/evidence/task-2-media-tests.txt`.
  - [ ] Record before/after code-level clone sites in `.sisyphus/evidence/task-2-media-clone-delta.md`.

  **QA Scenarios** (MANDATORY - task incomplete without these):
  ```
  Scenario: Multipart retry preserves payload
    Tool: Bash
    Steps: Run the new mocked Discord/Telegram media retry test that returns 429 once and 200 on retry, with body capture enabled.
    Expected: Test exits 0 and asserts both captured payloads contain the same fixture bytes and filename.
    Evidence: .sisyphus/evidence/task-2-media-tests.txt

  Scenario: Empty and large payload edge cases
    Tool: Bash
    Steps: Run the new media byte edge-case tests for empty Vec/Bytes and generated 8 MiB payload.
    Expected: Empty payload does not panic; 8 MiB payload stays under the existing 25 MiB limit and succeeds through the local preparation path.
    Evidence: .sisyphus/evidence/task-2-media-edge-cases.txt
  ```

  **Commit**: YES | Message: `perf(transport): share media bytes across upload retries` | Files: `src/transport/channels/traits.rs`, `src/transport/channels/attachments/load.rs`, `src/transport/channels/discord/http_client.rs`, `src/transport/channels/telegram/api.rs`, relevant tests only

- [x] 3. Remove MCP await-under-lock and classify consolidation serialization explicitly

  **What to do**: Fix confirmed `.await` under the MCP `RwLock` read guard. For MCP, change storage/handle ownership so `list_tools()` and `call_tool()` obtain the active service state without keeping `RwLockReadGuard` alive across `list_all_tools().await` or `service.call_tool(...).await`; preserve shutdown's ability to mark the connection inactive and cancel the service. For consolidation, use this explicit decision tree: (1) first add/confirm duplicate-check tests for same entity/checkpoint; (2) if those tests show full-lock serialization is required to prevent duplicate semantic events, do not change consolidation locking and write `.sisyphus/evidence/task-3-consolidation-intentional-serialization.md`; (3) only if the executor implements a two-phase protocol that passes the duplicate tests may they release the entity guard before `memory.append_event(...).await`. The default decision is **no consolidation source change** unless the two-phase tests prove correctness.
  **Must NOT do**: Do not weaken duplicate-check/watermark correctness. Do not allow two consolidations for the same entity/checkpoint to append duplicate semantic events. Do not redesign MCP lifecycle broadly.

  **Recommended Agent Profile**:
  - Category: `deep` - Reason: concurrency correctness and lock invariants require careful proof.
  - Skills: [`rust`] - Async lock and clippy lint handling.
  - Omitted: [`mcp-builder`] - This is not new MCP server implementation.

  **Parallelization**: Can Parallel: YES | Wave 2 | Blocks: Final Verification | Blocked By: task 1

  **References** (executor has NO interview context - be exhaustive):
  - MCP service lock field: `src/plugins/mcp/client_connection.rs:23-28` - `Arc<RwLock<Option<McpService>>>`.
  - MCP list_tools await under read guard: `src/plugins/mcp/client_connection.rs:71-81`.
  - MCP call_tool await under read guard: `src/plugins/mcp/client_connection.rs:104-129`.
  - MCP shutdown write/take pattern: `src/plugins/mcp/client_connection.rs:132-142` - preserve ability to cancel.
  - Consolidation lock registry: `src/core/memory/consolidation.rs:40-42` and `354-370`.
  - Consolidation lock held across append: `src/core/memory/consolidation.rs:409-470`, especially `422-459`.
  - Background scheduling: `src/core/memory/consolidation.rs:477-518` - answer path must remain preserved.

  **Acceptance Criteria** (agent-executable only):
  - [ ] Add/update tests proving MCP disconnected path still returns inactive-connection errors.
  - [ ] Add/update tests proving consolidation skips duplicate checkpoint and does not append twice for same entity/checkpoint.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features -- -D warnings -D clippy::await_holding_lock` and save output to `.sisyphus/evidence/task-3-clippy-await-holding-lock.txt`; if consolidation intentionally remains serialized and triggers this lint, record the exact lint output plus `.sisyphus/evidence/task-3-consolidation-intentional-serialization.md` and do not suppress globally.
  - [ ] Write `.sisyphus/evidence/task-3-lock-invariants.md` listing each touched lock, data copied under lock, awaited operation, and invariant preserved; include `consolidation: unchanged intentionally` or the two-phase protocol details.

  **QA Scenarios** (MANDATORY - task incomplete without these):
  ```
  Scenario: MCP inactive connection behavior preserved
    Tool: Bash
    Steps: Run the targeted MCP client connection tests including disconnected_for_test inactive path.
    Expected: Test exits 0 and inactive connection still returns an error containing 'not active'.
    Evidence: .sisyphus/evidence/task-3-mcp-tests.txt

  Scenario: Consolidation duplicate checkpoint prevented
    Tool: Bash
    Steps: Run the targeted consolidation tests that call run_consolidation twice with the same entity/checkpoint against a test memory backend.
    Expected: First call returns Consolidated; second returns SkippedCheckpoint; append count remains 1.
    Evidence: .sisyphus/evidence/task-3-consolidation-tests.txt
  ```

  **Commit**: YES | Message: `fix(runtime): avoid awaiting while holding runtime locks` | Files: `src/plugins/mcp/client_connection.rs`, `src/core/memory/consolidation.rs`, relevant tests only

- [x] 4. Measure and narrow Prometheus/cache snapshot clone pressure only if material

  **What to do**: Measure `PrometheusObserver::render_text` and snapshot helper clone behavior with empty and large synthetic maps. If clone overhead is material and a narrow safe change exists, replace full-map clones in render path with lock-bounded iteration helpers that preserve metric text output and avoid holding locks across external calls. If not material, produce a no-change evidence report and leave source untouched.
  **Must NOT do**: Do not change metric names, labels, output format, test structs, or scrape semantics. Do not optimize every `.clone()` in this file.

  **Recommended Agent Profile**:
  - Category: `unspecified-high` - Reason: evidence-gated local optimization with strict compatibility.
  - Skills: [`rust`] - Mutex/map iteration and tests.
  - Omitted: [`optimize`] - Not frontend/UI optimization.

  **Parallelization**: Can Parallel: YES | Wave 3 | Blocks: Final Verification | Blocked By: task 1

  **References** (executor has NO interview context - be exhaustive):
  - Prometheus map fields: `src/runtime/observability/prometheus.rs:40-50`.
  - Render path map clones: `src/runtime/observability/prometheus.rs:221-308`, especially `261-304`.
  - Test snapshot structs and helpers: `src/runtime/observability/prometheus.rs:83-107` and grep findings around `snapshot_*` methods at `449-515`.

  **Acceptance Criteria** (agent-executable only):
  - [ ] Add benchmark-like unit test or ignored test that populates at least 10,000 synthetic metric labels and records render time/RSS proxy before/after to `.sisyphus/evidence/task-4-prometheus-render.txt`.
  - [ ] If source changes are made, run existing Prometheus observer tests and save output to `.sisyphus/evidence/task-4-prometheus-tests.txt`.
  - [ ] If no source changes are made, write `.sisyphus/evidence/task-4-no-change.md` explaining measurement result and stop.

  **QA Scenarios** (MANDATORY - task incomplete without these):
  ```
  Scenario: Metrics output compatibility
    Tool: Bash
    Steps: Run Prometheus observer tests after any render_text change.
    Expected: Tests exit 0 and expected metric names such as asterel_signal_ingest_total and asterel_post_turn_hook_total remain present.
    Evidence: .sisyphus/evidence/task-4-prometheus-tests.txt

  Scenario: Large map render measurement
    Tool: Bash
    Steps: Run the synthetic large-label render measurement with 10,000 labels.
    Expected: Evidence records elapsed time and output length; no panic or deadlock.
    Evidence: .sisyphus/evidence/task-4-prometheus-render.txt
  ```

  **Commit**: YES if source changes, otherwise NO | Message: `perf(observability): avoid cloning metric maps during render` | Files: `src/runtime/observability/prometheus.rs`, relevant tests only

- [x] 5. Investigate repository recall sequential work and patch only a proven narrow bottleneck

  **What to do**: Verify whether FTS, query embedding, and vector search sequencing is actually material for the CLI/daemon memory path. The code comment says FTS/vector run in parallel, but current implementation runs FTS, then embedding, then vector. Add timing evidence around `search_and_merge_for_tier`. Patch only if a narrow safe parallelization is possible without changing result ordering/scoring, database semantics, or fallback behavior.
  **Must NOT do**: Do not rewrite recall scoring, graph activation, metadata loading, deletion-ledger filtering, or result ranking. Do not require a live external embedding service unless existing tests already provide one.

  **Recommended Agent Profile**:
  - Category: `deep` - Reason: memory backend performance and semantic ranking correctness.
  - Skills: [`rust`] - Async futures and Postgres-backed tests.
  - Omitted: [`api-security`] - No auth/API surface change.

  **Parallelization**: Can Parallel: YES | Wave 3 | Blocks: Final Verification | Blocked By: task 1

  **References** (executor has NO interview context - be exhaustive):
  - Pipeline comment promising parallel search: `src/core/memory/postgres/repository_recall.rs:1-18`.
  - Main recall flow: `src/core/memory/postgres/repository_recall.rs:77-137`.
  - Tier fallback: `src/core/memory/postgres/repository_recall.rs:139-158` - preserve note-first, episode fallback behavior.
  - Sequential FTS/embedding/vector area: `src/core/memory/postgres/repository_recall.rs:160-219`.
  - Metadata loading query: `src/core/memory/postgres/repository_recall.rs:221-260` - do not alter unless evidence specifically points here.

  **Acceptance Criteria** (agent-executable only):
  - [ ] Record whether `search_and_merge_for_tier` remains sequential or is safely parallelized in `.sisyphus/evidence/task-5-recall-investigation.md`.
  - [ ] If changed, add/update a test proving note-tier fallback to episode-tier still works when note results are empty.
  - [ ] If changed, add/update a test proving merged result ordering is unchanged for deterministic FTS/vector fixture inputs.
  - [ ] Run targeted Postgres memory recall tests available in the repo and save output to `.sisyphus/evidence/task-5-recall-tests.txt`.

  **QA Scenarios** (MANDATORY - task incomplete without these):
  ```
  Scenario: Recall fallback preserved
    Tool: Bash
    Steps: Run the targeted recall test where note-tier search returns no results and episode-tier returns candidates.
    Expected: Test exits 0 and returned entries come from the episode fallback without changing limit handling.
    Evidence: .sisyphus/evidence/task-5-recall-tests.txt

  Scenario: No-change investigation allowed
    Tool: Bash
    Steps: If measurement does not justify a patch, write investigation findings with exact line references and no source changes.
    Expected: .sisyphus/evidence/task-5-recall-investigation.md exists and states 'no source change' with rationale.
    Evidence: .sisyphus/evidence/task-5-recall-investigation.md
  ```

  **Commit**: YES if source changes, otherwise NO | Message: `perf(memory): parallelize recall search where safe` | Files: `src/core/memory/postgres/repository_recall.rs`, relevant tests only

- [x] 6. Produce binary/dependency footprint report without changing defaults

  **What to do**: Analyze release binary size contributors, duplicate dependencies, default features, and desktop/Tauri notes as secondary context. Produce a report with actionable future recommendations, but do not alter `Cargo.toml` defaults, release profiles, or CI gates in this pass. Compare final release size to task 1 baseline and explain any change caused by source work.
  **Must NOT do**: Do not remove default features (`discord`, `postgres`, `media`, `link-extraction`, `taste`). Do not change `[profile.release]`. Do not add failing binary-size gates to CI.

  **Recommended Agent Profile**:
  - Category: `unspecified-high` - Reason: analysis-heavy, low mutation.
  - Skills: [`rust`] - Cargo tree/bloat interpretation.
  - Omitted: [`supply-chain-risk-auditor`] - Dependency security/health is not the scope.

  **Parallelization**: Can Parallel: YES | Wave 3 | Blocks: Final Verification | Blocked By: task 1

  **References** (executor has NO interview context - be exhaustive):
  - Dependency list: `Cargo.toml:15-123`.
  - Default features: `Cargo.toml:124-143` - report only; do not change.
  - Release profiles: `Cargo.toml:158-168` - already size-focused; do not change.
  - CI build command: `.github/workflows/ci.yml:179-204`.
  - Mutation/coverage context: `.github/workflows/ci.yml:398-455` - do not touch in this plan.

  **Acceptance Criteria** (agent-executable only):
  - [ ] Produce `.sisyphus/evidence/task-6-binary-footprint.md` with top contributors, duplicate dependency families, final-vs-baseline size delta, and future recommendations.
  - [ ] Run `cargo tree -e features` and `cargo tree --duplicates` after source changes and compare to task 1 outputs.
  - [ ] Run `cargo build --release --locked` after source changes and record final `target/release/asterel` byte size.
  - [ ] Confirm `Cargo.toml:124-143` default feature list is unchanged in the final diff.

  **QA Scenarios** (MANDATORY - task incomplete without these):
  ```
  Scenario: Defaults preserved
    Tool: Bash
    Steps: Compare final Cargo.toml default feature block against baseline block lines 124-143.
    Expected: No changes to default feature names or membership.
    Evidence: .sisyphus/evidence/task-6-default-features-preserved.txt

  Scenario: Binary report complete
    Tool: Bash
    Steps: Re-run release build and append final byte size plus comparison to task 1 baseline.
    Expected: Report includes final size, baseline size, delta, and explanation for any regression.
    Evidence: .sisyphus/evidence/task-6-binary-footprint.md
  ```

  **Commit**: NO unless only evidence/report artifacts are committed by user preference | Message: `n/a` | Files: `.sisyphus/evidence/*` only

## Final Verification Wave (MANDATORY — after ALL implementation tasks)
> 4 review agents run in PARALLEL. ALL must APPROVE. Present consolidated results to user and get explicit "okay" before completing.
> **Do NOT auto-proceed after verification. Wait for user's explicit approval before marking work complete.**
> **Never mark F1-F4 as checked before getting user's okay.** Rejection or user feedback -> fix -> re-run -> present again -> wait for okay.
- [x] F1. Plan Compliance Audit — oracle
- [x] F2. Code Quality Review — unspecified-high
- [x] F3. Real Manual QA — unspecified-high
- [x] F4. Scope Fidelity Check — deep

## Commit Strategy
- Commit source-changing tasks separately when their tests pass.
- Suggested commits:
  - `perf(transport): share media bytes across upload retries`
  - `fix(runtime): avoid awaiting while holding runtime locks`
  - Optional: `perf(observability): avoid cloning metric maps during render`
  - Optional: `perf(memory): parallelize recall search where safe`
- Do not commit `.sisyphus/evidence/*` unless the user asks for evidence artifacts to be versioned.
- Do not add attribution trailers: no `Co-authored-by: Sisyphus <clio-agent@sisyphuslabs.ai>` and no `Ultraworked with ...`.

## Success Criteria
- Baseline and final evidence exist and are internally consistent.
- Memory/runtime optimizations are limited to measured high-confidence paths.
- Upload retry tests prove identical payload reconstruction after transient failure.
- Lock guard evidence proves no touched standard/tokio lock is accidentally held across `.await`, or documents a necessary per-entity serialization invariant.
- Default features and release profiles remain unchanged.
- Final Cargo checks pass: fmt, clippy with `await_holding_lock`, all-feature tests, release build.
