#!/usr/bin/env bash
# Build and test the Swift layer of the iOS app (the UniFFI bindings and SquibKit)
# against the Linux build of the Rust core, in the pinned official Swift container
# (plan 7.1, ADR-065). This checks the generated Swift and the par-timer model with the
# real core. It is not iOS evidence: no UIKit, SwiftUI, AVFoundation or simulator.
# Usage: scripts/swift-linux-test.sh [swift test arguments...]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# swift:6.3.3-noble (the Swift version nearest the pinned Xcode), by digest.
IMAGE="swift@sha256:cd45c27b3abc42310c33cfaf3008b18156cbdc9187834c9617e8f43a26d2675c"
PKG="$ROOT/apps/ios"
GEN="$PKG/Generated/linux"

cd "$ROOT"
cargo build --locked --release -p squib-mobile
LIB="$ROOT/target/release/libsquib_mobile.so"

OUT="$(mktemp -d)"
trap 'rm -rf "$OUT"' EXIT
cargo run --locked -q -p uniffi-bindgen -- generate --library "$LIB" --language swift --out-dir "$OUT" --no-format
mkdir -p "$GEN" "$PKG/Sources/SquibCore"
cp "$OUT/squib_mobile.swift" "$PKG/Sources/SquibCore/squib_mobile.swift"
cp "$OUT/squib_mobileFFI.h" "$GEN/"
# The generated module map names Darwin modules; on Linux the header needs only the C
# standard headers, and the module links the core's shared library.
cat > "$GEN/module.modulemap" <<'MAP'
module squib_mobileFFI {
    header "squib_mobileFFI.h"
    link "squib_mobile"
    export *
}
MAP

docker run --rm \
  --user "$(id -u):$(id -g)" -e HOME=/tmp \
  -v "$PKG:/pkg" -v "$ROOT/target/release:/core:ro" \
  -e LD_LIBRARY_PATH=/core -w /pkg \
  "$IMAGE" \
  swift test --scratch-path /pkg/.build-linux -Xlinker -L/core "$@"
