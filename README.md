# Squib

Offline-first shot/par timer and training journal. **No equipment required. Better equipment supported.**

Status: M0/M1 prototype. Par-only timing is usable; live acoustic shot timing is **Experimental** and has not been tested on a physical device or field-qualified. See `docs/implementation/m0-m1-report.md`.

## Layout

```text
docs/squib/               handoff specification (01-12) and decisions/ (ADR-014..022)
docs/implementation/      milestone reports
crates/domain/            run config, states, quality events, candidates, revisions, results
crates/timing/            clock mapping, capture integrity, cue matching, detector, run state machine, replay
crates/storage/           SQLite repository, migrations, recovery
crates/mobile-api/        UniFFI engine (control plane), JNI PCM bridge (data plane), DSP and store actors
tools/replay/             squib-replay: fixtures, replay, corpus evaluation, benchmark
tools/uniffi-bindgen/     pinned binding generator
apps/android/             Kotlin + Jetpack Compose app
fixtures/synthetic/       golden synthetic WAV + label fixtures with SHA-256 manifest
scripts/                  Android core build, emulator UI driver
```

## Requirements

Rust 1.95.0 with `aarch64-linux-android` and `x86_64-linux-android` targets, `cargo-ndk`, Android SDK (platform 37, build-tools) and NDK 28.2.13676358 under `$ANDROID_HOME` (default `~/Android/Sdk`), JDK 21 (Gradle can provision it).

## Build and test

```sh
# Shared core
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# Replay tool
cargo build --release -p squib-replay
./target/release/squib-replay gen fixtures/synthetic        # regenerate fixtures (deterministic)
./target/release/squib-replay corpus fixtures/synthetic     # evaluate; non-zero exit on failure
./target/release/squib-replay run fixtures/synthetic/basic_5_shots_48k.wav \
    --labels fixtures/synthetic/basic_5_shots_48k.labels.json --random-chunks 7
./target/release/squib-replay bench --seconds 120

# Android (builds the Rust core with cargo-ndk and generates Kotlin bindings first)
cd apps/android
./gradlew :app:assembleDebug
./gradlew :app:connectedDebugAndroidTest    # needs a device or emulator
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

`-PsquibSkipRust=true` reuses previously built native libraries. `-PsquibRustProfile=debug` builds the core unoptimized.

## Privacy defaults

No account, no network permission, no telemetry. Raw microphone audio is processed in bounded memory and discarded; only a coarse 10 ms energy envelope, detections, and quality events are stored. The journal is excluded from OS backup until portable export ships (ADR-016).

## License

Not selected yet (owner decision).
