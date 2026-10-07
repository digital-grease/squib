#!/usr/bin/env bash
# Cross-compile the Rust core for Android and generate UniFFI Kotlin bindings.
# Usage: scripts/build-android-core.sh [debug|release] [abi ...]
# Default ABIs: arm64-v8a (devices) and x86_64 (emulator).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="${1:-release}"
shift || true
ABIS=("$@")
if [ ${#ABIS[@]} -eq 0 ]; then ABIS=(arm64-v8a x86_64); fi
APP="$ROOT/apps/android/app"
JNI_OUT="$APP/src/main/jniLibs"
GEN_OUT="$APP/src/main/generated/uniffi"
: "${ANDROID_NDK_HOME:=${ANDROID_HOME:-$HOME/Android/Sdk}/ndk/28.2.13676358}"
export ANDROID_NDK_HOME

cd "$ROOT"
TARGET_ARGS=()
for abi in "${ABIS[@]}"; do TARGET_ARGS+=(-t "$abi"); done
PROFILE_ARGS=()
[ "$PROFILE" = "release" ] && PROFILE_ARGS=(--release)
# API 26 matches minSdk.
cargo ndk --platform 26 "${TARGET_ARGS[@]}" -o "$JNI_OUT" build -p squib-mobile "${PROFILE_ARGS[@]}"

# Bindings come from the library's embedded metadata (UniFFI library mode).
case "${ABIS[0]}" in
  arm64-v8a) TRIPLE=aarch64-linux-android ;;
  x86_64) TRIPLE=x86_64-linux-android ;;
  *) echo "unsupported ABI ${ABIS[0]}" >&2; exit 1 ;;
esac
LIB="$ROOT/target/$TRIPLE/$PROFILE/libsquib_mobile.so"
rm -rf "$GEN_OUT"
mkdir -p "$GEN_OUT"
cargo run -q -p uniffi-bindgen -- generate --library "$LIB" --language kotlin \
  --out-dir "$GEN_OUT" --no-format
echo "core libraries: $JNI_OUT; bindings: $GEN_OUT"
