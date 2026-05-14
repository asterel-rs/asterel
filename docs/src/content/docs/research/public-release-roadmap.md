---
title: Public release roadmap
description: Roadmap for keeping Asterel's public repository and clean release snapshots safe without carrying private development history.
---

This roadmap turns the publicization work into release-maintenance phases. It is
operational rather than aspirational: each phase has an exit gate and keeps the
same publication boundary used by the rest of the research packet.

The key constraint is simple: public releases should be refreshed from a clean
snapshot of the public tracked set. Private development history, local agent
assets, internal review notes, and personal workspace context are not part of the
public artifact.

## Phase 0 — Freeze the public tracked set

Goal: know exactly what can be published before refreshing the public artifact.

Tasks:

- Confirm that local/private material is not in the public tracked set.
- Confirm that public docs do not depend on private documentation.
- Confirm that repository, docs, package, and license metadata point at the
  intended organization repository.
- Re-run the smallest checks that prove the public docs and metadata are coherent.

Gate:

```bash
git diff --check
pnpm --dir docs build
cargo metadata --no-deps --format-version 1
cargo test --test project
```

Do not proceed if public files still reference private docs, local agent notes,
old personal repository URLs, or private-license wording.

## Phase 1 — Finalize the public narrative

Goal: make the repository understandable without private design notes.

Tasks:

- Final-pass the README maturity language.
- Keep implementation claims separate from benchmark or paper claims.
- Ensure the research packet explains claims, evidence classes, gaps,
  reproducibility, and publication boundaries.
- Keep Japanese overview pages aligned with the public status language.

Exit criteria:

- A new reader can tell that Asterel is a Discord-first, text-first companion
  runtime with local operator governance.
- A new reader can tell what is implemented, what is alpha, and what still needs
  empirical evaluation.
- No public page requires private internal context to understand the project.

## Phase 2 — Refresh the clean snapshot

Goal: refresh the public release tree without preserving private historical
blobs.

Tasks:

- Create or replace a separate clean-snapshot working directory.
- Copy only the public tracked file set into that directory, preferably through
  `scripts/release/create_public_snapshot.sh` rather than a raw workspace copy.
- Verify the copied tree before using it as release evidence or publication
  input.
- If initializing a repository from the snapshot, add the intended remote only
  after verification.
- Commit or tag only from the verified public-safe tree.

Non-goals:

- Do not push the existing private repository history.
- Do not mirror refs.
- Do not preserve closed PR refs, local branches, or historical blobs.
- Do not copy ignored local files just because they are useful in development.

Snapshot gate:

```bash
scripts/release/create_public_snapshot.sh /tmp/asterel-public-snapshot --dry-run
pnpm --dir docs build
cargo metadata --no-deps --format-version 1
```

Run the strict release gate before publication when time permits.

## Phase 3 — Configure the GitHub repository

Goal: keep the organization repository safe to receive issues, docs, and
security reports.

Repository settings:

- Confirm `asterel-rs/asterel` remains the intended public repository.
- Enable GitHub Pages for the docs path used by the site config.
- Enable private vulnerability reporting, or keep the fallback address in
  `SECURITY.md` accurate until it is enabled.
- Review Actions permissions and branch protection.
- Add repository description and topics that match the companion-first scope.

Deferred until org details exist:

- Add `CODEOWNERS` only after the owning team name, visibility, and write
  permission are confirmed.
- Add an organization profile README in the separate `asterel-rs/.github`
  repository if the organization needs one.

## Phase 4 — Validate from a clean checkout

Goal: prove the public repository can reproduce the current evidence from a fresh
clone.

Run from the clean repository:

```bash
cargo fmt -- --check
cargo clippy -- -D warnings
cargo check-all
cargo test
docker compose config
pnpm --dir docs build
cargo metadata --no-deps --format-version 1
./scripts/dev/generate_module_map.sh && ./scripts/dev/check_architecture.sh
./scripts/release/human_like_release_gate.sh
```

If a full gate cannot be run, record why, run the closest targeted substitutes,
and do not present the release as fully validated.

## Phase 5 — Harden the research artifact

Goal: move from implementation evidence toward paper-level empirical evidence
without overstating results.

Tasks:

- Confirm benchmark dataset licenses and task fit before using external names as
  evidence.
- Build benchmark adapters for memory, affect calibration, public/private room
  behavior, and security containment.
- Run the ablation conditions listed in the [ablation plan](./ablation-plan/).
- Record model/provider versions, seeds, fixture hashes, config, and redaction
  policy.
- Design consented human-rating studies before using real human transcript data.

Exit criteria:

- Implementation claims and empirical claims remain visibly separate.
- No superiority claim appears without a frozen benchmark or human-evaluation
  artifact.
- Public artifacts contain aggregate metrics, public-safe examples, and hashes,
  not private transcripts or raw memory payloads.

## Phase 6 — Cut the first public release

Goal: publish an alpha release that reflects the narrow product proof rather than
claiming broad platform maturity.

Tasks:

- Run final README, docs, security, support, and license review.
- Confirm GitHub Pages resolves to the public docs URL.
- Confirm issue templates warn against posting secrets or private memory.
- Prepare release notes around the Discord-first companion-runtime snapshot.
- Tag only after the clean repository validation gate passes.

Suggested first release label:

```text
v0.1.0-alpha — Discord-first companion runtime snapshot
```

## Public no-go conditions

Do not publish if any of the following are true:

- the existing private `.git` history is about to be pushed;
- public tracked files reference private docs or local agent/session files;
- old personal repository URLs remain in public docs or package metadata;
- license metadata disagrees across Rust, Node, README, and license files;
- private vulnerability reporting is promised but not enabled or explained with a
  fallback;
- benchmark or paper claims are stronger than the evidence ledger supports.
