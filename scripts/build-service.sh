#!/usr/bin/env bash
# Build a PVM service crate into a JAM (GP 0.8.0) blob.
# usage: build-service.sh <crate-dir> <output.jam> [extra jamx-pack args...]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLS="${JAMX_TOOLS:-$ROOT/../tools}"
POLKATOOL="${POLKATOOL:-$TOOLS/cargo/bin/polkatool}"
CRATE_DIR="$(cd "$1" && pwd)"; OUT="$2"; shift 2
TARGET_JSON="$("$POLKATOOL" get-target-json-path --bitness 64)"
PACK="$ROOT/crates/jamx-pack/target/release/jamx-pack"
if [ ! -x "$PACK" ]; then (cd "$ROOT/crates/jamx-pack" && cargo build --release -q); fi
NAME="$(sed -n 's/^name *= *"\(.*\)"/\1/p' "$CRATE_DIR/Cargo.toml" | head -1)"
VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$CRATE_DIR/Cargo.toml" | head -1)"
(cd "$CRATE_DIR" && RUSTC_BOOTSTRAP=1 cargo build --release -q ${JAMX_FEATURES:+--features "$JAMX_FEATURES"} \
    -Z build-std=core,alloc -Z json-target-spec --target "$TARGET_JSON")
ELF="$CRATE_DIR/target/riscv64emac-unknown-none-polkavm/release/$NAME"
"$PACK" "$ELF" "$OUT" --name "$NAME" --version "$VERSION" "$@"
