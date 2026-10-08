# Squib

Squib turns an Android phone into a practice timer for shooting sports. It is built to be useful with nothing but the phone you already own: no account, no internet connection, and no extra equipment.

**No equipment required. Better equipment supported.**

## What it does

- **Par timer.** Set a start delay (instant, fixed, or random), add one or more par times, and press Arm. The phone plays a start beep after the delay and a beep at each par time. This mode never uses the microphone.
- **Shot timing (experimental).** The phone listens for its own start beep and for shots, then shows your first-shot time, the splits between shots, and the total.
- **Review and correct.** After a run you can look at every detected shot, keep or reject uncertain ones, add a shot the phone missed, or nudge a time. Your corrections are saved as a new version; the original detections are never thrown away.
- **History.** Every run is saved on the phone with its result, including runs that were cancelled or interrupted.
- **Setup help.** A sensitivity check measures how loud your surroundings are, and a cue test confirms the microphone can hear the start beep before you rely on it.
- **Range conditions.** Temperature, humidity, wind, and pressure for your range, from nearby US weather stations, the phone's barometer if it has one, or values you enter. Every value shows where it came from and how old it is, and each run keeps a copy of the conditions when it started.

## Current status

Squib is an early prototype and is not yet available in any app store.

- The par timer works.
- Range conditions work with US weather stations (National Weather Service). Outside the US, enter values by hand.
- Shot timing works in testing on a computer and in the Android emulator, but it has **not yet been tested on a real phone at a real range**. Treat its times as experimental until that testing is done.
- Squib does not claim any particular accuracy. It labels uncertain results, interrupted runs, and setups it cannot vouch for instead of guessing.

## Privacy

- No account, no ads, and no tracking.
- The only internet use is weather lookup, which is off until you turn it on. It sends your place, rounded to about 1 km, to the US National Weather Service. Nothing else is ever sent, and audio never leaves the phone.
- Squib never records or saves audio. Sound is analyzed in memory and discarded; only shot times and a coarse loudness outline are kept.
- The microphone is requested only when you choose shot timing, sensitivity setup, or the cue test. The par timer never asks for it. Location is requested only when you tap "Use my location"; typing coordinates or picking a saved place works without it.
- By default, runs keep weather values but not your coordinates, station names, or elevation. You can opt in to keeping full detail.
- History stays on your phone. It is not included in Android backups yet, so it does not move to a new phone. Export is planned.

## Planned

Drill recipes, practice scoring, and export/backup of your history.

## License

Squib is free software, licensed under the [GNU Affero General Public License v3.0 or later](LICENSE). You can use, study, change, and share it. If you distribute a modified version, or run one as a network service, you must share your source code under the same license.

---

## Technical details

### Architecture

A shared Rust core holds all timing logic, shot detection, the run state machine, and the SQLite journal. The Android app (Kotlin, Jetpack Compose) handles permissions, audio capture and playback, and the screens. Control calls cross into Rust through UniFFI; audio samples cross through a small JNI bridge into a bounded, preallocated queue processed on its own thread.

Range conditions are resolved field by field in Rust: a manual value wins, then a fresh phone sensor, then the best-ranked fresh nearby station, otherwise Unavailable. Pressure kinds are separate fields, so a station's altimeter setting or sea-level pressure can never stand in for the actual pressure where you are (NWS observations do not include station pressure). Ages always come from observation time, not fetch time. The Android app performs the HTTP requests the core plans and validates; snapshots saved with runs are redacted unless precise retention is enabled.

Timing comes from audio sample positions, not from when the app happens to receive audio. When a run's start beep and shots are captured in the same recording, the times between them are exact sample counts. Raw audio exists only in short-lived memory buffers.

### Layout

```text
crates/domain/            run config, states, quality events, candidates, revisions, results
crates/timing/            clock mapping, capture integrity, cue matching, detector, run state machine, replay
crates/environment/       range conditions: measurement candidates, NWS adapter, resolver, privacy redaction
crates/storage/           SQLite repository, migrations, recovery
crates/mobile-api/        UniFFI engine (control plane), JNI PCM bridge (data plane), DSP and store actors
tools/replay/             squib-replay: fixtures, replay, corpus evaluation, benchmark
tools/uniffi-bindgen/     pinned binding generator
apps/android/             Kotlin + Jetpack Compose app
fixtures/synthetic/       golden synthetic WAV + label fixtures with SHA-256 manifest
fixtures/nws/             captured api.weather.gov responses (public domain) for adapter tests
scripts/                  Android core build, emulator UI driver
```

### Requirements

Rust 1.95.0 with the `aarch64-linux-android` and `x86_64-linux-android` targets, `cargo-ndk`, the Android SDK (platform 37, build-tools) and NDK 28.2.13676358 under `$ANDROID_HOME` (default `~/Android/Sdk`), and JDK 21 (Gradle can provision it). Minimum Android version is 8.0 (API 26).

### Build and test

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

### Evidence so far

Unit, integration, and replay tests run on the desktop against synthetic recordings with known event positions. UI flows, permission denial, process termination recovery, and the audio transfer path have been exercised on an Android emulator. Synthetic audio and emulators cannot qualify a real microphone, speaker, or range environment; device and field testing are still to come.
