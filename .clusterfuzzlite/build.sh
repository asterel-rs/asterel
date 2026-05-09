#!/bin/bash -eu
cd $SRC/asterel
cargo fuzz build -O --debug-assertions
RUST_TARGET=$(rustc -vV | sed -n 's/host: //p')
FUZZ_TARGET_OUTPUT_DIR=fuzz/target/${RUST_TARGET}/release
for f in fuzz/fuzz_targets/*.rs; do
    FUZZ_TARGET_NAME=$(basename "${f%.*}")
    if [ -f "$FUZZ_TARGET_OUTPUT_DIR/$FUZZ_TARGET_NAME" ]; then
        cp "$FUZZ_TARGET_OUTPUT_DIR/$FUZZ_TARGET_NAME" "$OUT/"
    fi
done
