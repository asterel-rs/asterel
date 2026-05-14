---
title: Release checklist
description: Build profile, artifacts, checks, and signature expectations for tagged releases.
---

# Release checklist

Tagged releases are built by `.github/workflows/release.yml` for Linux, macOS
Intel, macOS Apple Silicon, and Windows.

## Build profile

The root `Cargo.toml` explicitly configures `[profile.release]` with size-focused
optimization, LTO, one codegen unit, symbol stripping, and `panic = "abort"`.
Do not rely on implicit Cargo defaults for release artifacts.

## Artifact signing

Release archives are signed in the publish job with Cosign keyless blob signing.
Each archive should have a sibling `*.bundle.sigstore.json` bundle uploaded to
the GitHub release. The bundle contains the signature, certificate, and
transparency-log proof needed for verification.

Verification example:

```bash
cosign verify-blob \
  --bundle asterel-x86_64-unknown-linux-gnu.tar.gz.bundle.sigstore.json \
  asterel-x86_64-unknown-linux-gnu.tar.gz
```

## Before tagging

Use `scripts/release/human_like_release_gate.sh` as the source of truth for a
release-quality local gate. It currently runs:

- `cargo fmt -- --check`
- `cargo clippy -- -D warnings`
- `cargo check-all`
- `cargo test`
- `cargo fuzz-smoke`
- `cargo audit`
- deterministic `eval baseline` pre/post evidence comparison
- bad-turn `eval replay` evidence checks

Also run the docs build if documentation changed and confirm `CHANGELOG.md` has
release notes for the tag.
