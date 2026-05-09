#!/usr/bin/env bash
set -euo pipefail

# Strict 2026 release gate:
# - quality gates
# - supply-chain audit
# - deterministic baseline comparison

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$(dirname "$SCRIPT_DIR")")"

PRE_SLUG="${PRE_SLUG:-strict-2026-pre}"
POST_SLUG="${POST_SLUG:-strict-2026-post-release-gate}"
REPLAY_SLUG="${REPLAY_SLUG:-strict-2026-discord-companion-bad-turns}"
REPLAY_SUITE="${REPLAY_SUITE:-discord-companion-bad-turns}"
REPLAY_FIXTURE="${REPLAY_FIXTURE:-$PROJECT_ROOT/tests/fixtures/replay/discord_companion_bad_turns.jsonl}"
SEED="${SEED:-42}"
EVIDENCE_DIR="${EVIDENCE_DIR:-$PROJECT_ROOT/evidence/strict-2026-overhaul}"
WORKSPACE_DIR="${ASTEREL_WORKSPACE:-$HOME/.asterel/workspace}"
WORKSPACE_EVIDENCE_DIR="${WORKSPACE_DIR}/evidence"

run_gate() {
    local label="$1"
    shift
    printf '\n== %s ==\n' "$label"
    "$@"
}

need_cmd() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "missing required command: $1" >&2
        exit 1
    }
}

need_cmd cargo
need_cmd jq
need_cmd diff

mkdir -p "$EVIDENCE_DIR" "$WORKSPACE_EVIDENCE_DIR"

run_gate "fmt" cargo fmt -- --check
run_gate "clippy" cargo clippy -- -D warnings
run_gate "check-all" cargo check-all
run_gate "test" cargo test
run_gate "fuzz-smoke" cargo fuzz-smoke
run_gate "audit" cargo audit

PRE_CSV="$EVIDENCE_DIR/${PRE_SLUG}-baseline-report.csv"
PRE_JSON="$EVIDENCE_DIR/${PRE_SLUG}-baseline-report.json"
if [[ ! -f "$PRE_CSV" || ! -f "$PRE_JSON" ]]; then
    run_gate "baseline-pre" cargo run -- eval baseline --seed "$SEED" --evidence-slug "$PRE_SLUG"
    cp "${WORKSPACE_EVIDENCE_DIR}/${PRE_SLUG}.txt" "$EVIDENCE_DIR/${PRE_SLUG}.txt"
    cp "${WORKSPACE_EVIDENCE_DIR}/${PRE_SLUG}-baseline-report.csv" "$PRE_CSV"
    cp "${WORKSPACE_EVIDENCE_DIR}/${PRE_SLUG}-baseline-report.json" "$PRE_JSON"
fi

run_gate "baseline-post" cargo run -- eval baseline --seed "$SEED" --evidence-slug "$POST_SLUG"
POST_CSV="${WORKSPACE_EVIDENCE_DIR}/${POST_SLUG}-baseline-report.csv"
POST_JSON="${WORKSPACE_EVIDENCE_DIR}/${POST_SLUG}-baseline-report.json"
cp "${WORKSPACE_EVIDENCE_DIR}/${POST_SLUG}.txt" "$EVIDENCE_DIR/${POST_SLUG}.txt"
cp "$POST_CSV" "$EVIDENCE_DIR/${POST_SLUG}-baseline-report.csv"
cp "$POST_JSON" "$EVIDENCE_DIR/${POST_SLUG}-baseline-report.json"

PRE_FINGERPRINT="$(jq -r '.summary_fingerprint' "$PRE_JSON")"
POST_FINGERPRINT="$(jq -r '.summary_fingerprint' "$POST_JSON")"
if [[ "$PRE_FINGERPRINT" != "$POST_FINGERPRINT" ]]; then
    echo "performance fingerprint mismatch: pre=${PRE_FINGERPRINT}, post=${POST_FINGERPRINT}" >&2
    diff -u "$PRE_CSV" "$POST_CSV" || true
    exit 1
fi

if ! diff -u "$PRE_CSV" "$POST_CSV" >/dev/null; then
    echo "performance metrics changed from baseline" >&2
    diff -u "$PRE_CSV" "$POST_CSV" || true
    exit 1
fi

run_gate "replay-companion-bad-turns" cargo run -- eval replay --input "$REPLAY_FIXTURE" --suite "$REPLAY_SUITE" --evidence-slug "$REPLAY_SLUG"
REPLAY_JSON="${WORKSPACE_EVIDENCE_DIR}/${REPLAY_SLUG}-replay-report.json"
cp "${WORKSPACE_EVIDENCE_DIR}/${REPLAY_SLUG}-replay.txt" "$EVIDENCE_DIR/${REPLAY_SLUG}-replay.txt"
cp "${WORKSPACE_EVIDENCE_DIR}/${REPLAY_SLUG}-replay-report.csv" "$EVIDENCE_DIR/${REPLAY_SLUG}-replay-report.csv"
cp "$REPLAY_JSON" "$EVIDENCE_DIR/${REPLAY_SLUG}-replay-report.json"

REPLAY_VERIFIER_RATIO="$(jq -r '.suites[0].verifier_event_ratio_bps' "$REPLAY_JSON")"
if [[ "$REPLAY_VERIFIER_RATIO" -le 0 ]]; then
    echo "replay verifier event ratio did not detect bad-turn fixture events" >&2
    exit 1
fi
for reason in anti_template exposure_violation over_explain; do
    count="$(jq -r ".suites[0].verifier_reason_counts.${reason} // 0" "$REPLAY_JSON")"
    if [[ "$count" -le 0 ]]; then
        echo "replay verifier reason was not detected: ${reason}" >&2
        exit 1
    fi
done

printf '\nStrict 2026 release gate passed.\n'
