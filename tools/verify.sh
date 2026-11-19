#!/bin/bash
# Check the Rust core against Pillow and the original Python project. No crates needed
# (plain rustc). Usage: tools/verify.sh /path/to/original/python/repo
set -euo pipefail
ORIG="${1:?usage: tools/verify.sh /path/to/python/ddn_dither/repo}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$(mktemp -d)"
# build.rs (built-in frame registry from assets/frame_*.bin) runs the same way cargo runs it.
export CARGO_MANIFEST_DIR="$HERE"
export OUT_DIR="$OUT/gen"
mkdir -p "$OUT_DIR"
rustc --edition 2021 "$HERE/build.rs" -o "$OUT/build_gen"
"$OUT/build_gen"
rustc --edition 2021 --test "$HERE/src/lib.rs" -o "$OUT/unit_tests"
"$OUT/unit_tests" -q
rustc --edition 2021 -O --crate-type lib --crate-name ddn_dither "$HERE/src/lib.rs" -o "$OUT/libddn_dither.rlib"
rustc --edition 2021 -O "$HERE/tools/harness.rs" --extern ddn_dither="$OUT/libddn_dither.rlib" -o "$OUT/harness"
python3 -I "$HERE/tools/verify_against_pillow.py" "$OUT/harness" "$ORIG"
