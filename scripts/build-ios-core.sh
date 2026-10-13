#!/usr/bin/env bash
# Build the Rust core for iOS and generate the UniFFI Swift bindings (plan 7.1, ADR-065).
# macOS with Xcode only. Produces, all git-ignored:
#   apps/ios/Generated/SquibCoreFFI.xcframework   static core + C header + module map
#   apps/ios/Sources/SquibCore/squib_mobile.swift  Swift bindings
# Usage: scripts/build-ios-core.sh [all|sim]
#   all (default): device (arm64) and simulator (arm64 and x86_64) slices
#   sim: simulator slices only, for CI and local simulator runs
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WHICH="${1:-all}"
if [ "$(uname -s)" != "Darwin" ] || ! command -v xcodebuild > /dev/null; then
  echo "build-ios-core.sh needs macOS with Xcode; on Linux use scripts/swift-linux-test.sh" >&2
  exit 1
fi
PKG="$ROOT/apps/ios"
GEN="$PKG/Generated"
WORK="$ROOT/target/ios"
# Matches the app's deployment target (project.yml) and Package.swift's platforms.
export IPHONEOS_DEPLOYMENT_TARGET=17.0

case "$WHICH" in
  all) TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios) ;;
  sim) TARGETS=(aarch64-apple-ios-sim x86_64-apple-ios) ;;
  *) echo "unknown slice set: $WHICH (all or sim)" >&2; exit 1 ;;
esac

cd "$ROOT"
rustup target add "${TARGETS[@]}"
for t in "${TARGETS[@]}"; do
  # The crate is a cdylib for Android; Apple apps link the core statically.
  cargo rustc --locked -p squib-mobile --lib --release --target "$t" --crate-type staticlib
done

# Bindings come from the host build's embedded metadata (UniFFI library mode), as for
# Kotlin; they do not depend on the target.
cargo build --locked --release -p squib-mobile
rm -rf "$WORK"
mkdir -p "$WORK/bindings" "$WORK/headers" "$WORK/sim"
cargo run --locked -q -p uniffi-bindgen -- generate --library "$ROOT/target/release/libsquib_mobile.dylib" \
  --language swift --out-dir "$WORK/bindings" --no-format
cp "$WORK/bindings/squib_mobileFFI.h" "$WORK/headers/"
cp "$WORK/bindings/squib_mobileFFI.modulemap" "$WORK/headers/module.modulemap"

lib() { echo "$ROOT/target/$1/release/libsquib_mobile.a"; }
# One simulator library for both simulator architectures.
lipo -create "$(lib aarch64-apple-ios-sim)" "$(lib x86_64-apple-ios)" -output "$WORK/sim/libsquib_mobile.a"
ARGS=(-library "$WORK/sim/libsquib_mobile.a" -headers "$WORK/headers")
if [ "$WHICH" = all ]; then ARGS+=(-library "$(lib aarch64-apple-ios)" -headers "$WORK/headers"); fi

rm -rf "$GEN/SquibCoreFFI.xcframework"
mkdir -p "$GEN" "$PKG/Sources/SquibCore"
xcodebuild -create-xcframework "${ARGS[@]}" -output "$GEN/SquibCoreFFI.xcframework"
cp "$WORK/bindings/squib_mobile.swift" "$PKG/Sources/SquibCore/squib_mobile.swift"
echo "core: $GEN/SquibCoreFFI.xcframework; bindings: $PKG/Sources/SquibCore/squib_mobile.swift"
