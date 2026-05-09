#!/usr/bin/env bash
# Convenience wrapper for running fuzz targets with their dictionaries.
#
# Usage:
#   ./fuzz/run.sh <target> [extra cargo-fuzz args...]
#
# Examples:
#   ./fuzz/run.sh fuzz_command_policy
#   ./fuzz/run.sh fuzz_scrub -- -max_total_time=300
#   ./fuzz/run.sh all                     # Run all targets (60s each)
set -euo pipefail

FUZZ_DIR="$(cd "$(dirname "$0")" && pwd)"
DICT_DIR="$FUZZ_DIR/dictionaries"

# Map target names to their dictionary files.
declare -A DICTS=(
    [fuzz_command_policy]="command_policy.dict"
    [fuzz_path_policy]="path_policy.dict"
    [fuzz_url_validation]="url_validation.dict"
    [fuzz_scrub]="scrub.dict"
    [fuzz_normalize]="normalize.dict"
)

ALL_TARGETS=(
    fuzz_command_policy
    fuzz_path_policy
    fuzz_url_validation
    fuzz_scrub
    fuzz_normalize
)

run_target() {
    local target="$1"
    shift
    local dict_file="${DICTS[$target]:-}"
    local dict_args=()

    if [[ -n "$dict_file" && -f "$DICT_DIR/$dict_file" ]]; then
        dict_args=(-- "-dict=$DICT_DIR/$dict_file")
    fi

    echo "=== Fuzzing $target ==="
    cargo fuzz run "$target" "${dict_args[@]}" "$@"
}

if [[ $# -lt 1 ]]; then
    echo "Usage: $0 <target|all> [extra args...]"
    echo ""
    echo "Available targets:"
    for t in "${ALL_TARGETS[@]}"; do
        echo "  $t"
    done
    exit 1
fi

TARGET="$1"
shift

if [[ "$TARGET" == "all" ]]; then
    DEFAULT_TIME=${FUZZ_DURATION:-60}
    for t in "${ALL_TARGETS[@]}"; do
        run_target "$t" -- "-max_total_time=$DEFAULT_TIME" "$@" || true
    done
else
    run_target "$TARGET" "$@"
fi
