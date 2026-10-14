# Squib

[![CI](https://img.shields.io/github/actions/workflow/status/digital-grease/squib/ci.yml?branch=main&logo=github&label=CI)](https://github.com/digital-grease/squib/actions/workflows/ci.yml)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](https://github.com/digital-grease/squib/blob/main/LICENSE)
[![Android API](https://img.shields.io/badge/API-26%2B-brightgreen?logo=android)](https://developer.android.com/about/versions/oreo)
[![GitHub Issues](https://img.shields.io/github/issues/digital-grease/squib)](https://github.com/digital-grease/squib/issues)

<a href="https://www.buymeacoffee.com/digitalgrease" target="_blank"><img src="https://cdn.buymeacoffee.com/buttons/v2/default-red.png" alt="Buy Me A Coffee" style="height: 60px !important;width: 217px !important;" ></a>

Squib turns an Android phone into a practice timer for shooting sports. It is built to be useful with nothing but the phone you already own: no account, no internet connection, and no extra equipment.

**No equipment required. Better equipment supported.**

## What it does

- **Par timer.** Set a start delay (instant, fixed, or random), add one or more par times, and press Arm. The phone plays a start beep after the delay and a beep at each par time. This mode never uses the microphone. Instead of beeps you can choose vibration or a screen signal (GO in green, PAR in amber). Each is recorded as its own kind of start and labelled with what it measures: when the phone was asked to vibrate, or the screen frame that showed the signal. The motor and display delays are not measured. Screen cues are kept at least 0.35 s apart so the screen changes no more than about three times a second.
- **Shot timing (experimental).** The phone listens for its own start beep and for shots, then shows your first-shot time, the splits between shots, and the total. It can optionally stop by itself about a second after it accepts a set number of shots; that stop depends on detection, so check the result in review.
- **Review and correct.** After a run you can look at every detected shot, keep or reject uncertain ones, add a shot the phone missed, or nudge a time. Your corrections are saved as a new version; the original detections are never thrown away.
- **Equipment (optional).** Name what you shoot with (pistol, rifle, PCC, shotgun, rimfire, other) and pick it on the Timer. Each run remembers the version it used, editing keeps earlier runs unchanged, summaries can compare one setup, and no serial numbers are asked for.
- **Drills.** Save your own drills (start delay, par times, strings and rest, scoring) or start from simple examples. Editing a drill creates a new version, so earlier results stay comparable. For drills with several strings you can turn on automatic repeat: after each complete string, a countdown through the rest is shown with a Cancel button, then the next string arms with a new start delay. It stops after the last string, after a string that did not complete, or when you leave the Timer. Drills can be shared as small files, or as a QR code another phone scans with Squib: the code holds the drill itself (never a link), and scanning shows what it contains before anything is saved.
- **Scoring.** Generic practice scoring: time only, points and hit factor, or time plus penalties. A drill with several strings can have a set result (the total or average of its strings, optionally counting only the best few); unfinished or unscored strings never count, and a short set stays incomplete. These are practice tools, not official rule sets. A missing score stays "incomplete" instead of counting as zero.
- **Results from another timer.** Type in the times from a club or borrowed timer; they are kept at that timer's precision and marked as manual.
- **History and progress.** Every run is saved on the phone, including cancelled or interrupted ones, grouped into visits: a visit ends when you end it or after four quiet hours, and keeps its own notes, round total, the price per round you had then, and the conditions of its latest run. You can delete one visit without touching the rest. Summaries show medians and spread, always with how many runs were included and why others were left out; fewer than five comparable runs are labeled too few for a trend. Charts show final times and hit factors in order with their median, a distribution, and the included runs with time and score side by side; they describe your runs and make no trend predictions. Separate shooter profiles let several people share one phone.
- **Day plans.** Plan a practice or match day: drills to run (with how many strings), stage notes such as start position and round count, and times like the shooters meeting. The plan shows what is next and counts only runs you actually finished; skipping an item never invents a result. Each plan has a checklist copied from your own template, and a walkthrough countdown keeps running, shown in a banner, while you use the rest of the app.
- **Coaching.** The timer always shows who is shooting. A coach can switch shooters between runs or set a rotation, and after each run Squib offers the next shooter. History shows who fired each run.
- **Video clips (experimental).** Turn on video and the back camera records a short clip with each run (video only, no sound, up to 2 minutes). In Review the clip plays with markers for the start, par cues, and shots, and you can jump to each one. Squib says how it lined the video up with the timer and how precise that is (about one video frame at best); if it cannot line them up reliably, the video plays without markers instead of guessing.
- **Diagnostic recordings (research, opt-in).** Switch on a recording for one live run to listen back with shot markers, and optionally export it with your review to help improve shot detection. Off by default, never uploaded.
- **Rounds and photos.** Confirm how many rounds you fired (with an optional cost per round), and attach target photos (stored upright, with location and camera details removed). Tap a photo to mark hits, misses, and notes yourself (Squib does not detect hits); the marks are saved as versions tied to the run's review and score, kept in backups, and left out of shared results.
- **Ballistics calculator (experimental).** From Conditions, save a load (bullet weight, published G1 or G7 ballistic coefficient, your measured muzzle velocity, sight height and zero) and get a range card: elevation and wind corrections in mil or MOA, drop, velocity, time of flight and energy at the distances you choose, using the current conditions or air you enter, a crosswind or any other wind, and an uphill or downhill angle. It is a point-mass model checked against an independent solver and against published manufacturer tables for supersonic flight; distances where the bullet has slowed below Mach 1.2 are marked less reliable. Spin drift (from the barrel twist and bullet size) and the Earth's rotation can be added, each off unless you choose it and checked against the same solver. A saved card keeps every input, the load version and conditions it used, and the method. Confirm every distance on paper before relying on it.
- **Your data stays yours.** Save a full private backup, optionally encrypted with a passphrase, and restore it on any phone (if it clashes with changes made on this phone, you can import it as a copy instead of overwriting anything), export a spreadsheet (CSV), or share a single result without location details. You can delete one run or everything.
- **Setup help.** A sensitivity check measures how loud your surroundings are, and a cue test confirms the microphone can hear the start beep before you rely on it.
- **Accessible by design.** Large controls (Arm and Stop are the biggest things on screen), text labels and shapes as well as colours, high-contrast light and dark themes checked against WCAG AA, and layouts that keep working at twice the normal text size. With a screen reader, every control says what it does and to what (for example "Start Repeated par"), switches are read with their labels, single-choice options are read as radio buttons, and the Back gesture returns to the previous screen. Start and par cues can be sound, vibration, or on screen.
- **Range conditions.** Temperature, humidity, wind, and pressure for your range, from nearby weather stations (US National Weather Service stations, or airport reports anywhere in the world), the phone's barometer if it has one, or values you enter. Every value shows where it came from and how old it is, and each run keeps a copy of the conditions when it started. A lookup that hits a temporary server error or timeout is tried up to twice more, a little later each time and never sooner than the service asks, all within 20 seconds; without a network Squib uses what it saved earlier instead of retrying. In the US, weather lookup also shows active National Weather Service alerts for your place (real, current ones only, with how old they are). When it is hot, a heat index card shows the NWS heat index and its caution category, for shade, with the method named. With actual pressure where you are (from the phone's barometer or your entry, never an altimeter setting), temperature and humidity, Squib calculates air density, density altitude and pressure altitude, naming the method and the inputs it used; with any input missing it shows no number. Optionally, terrain elevation (US only) gives the ground height at your place from public-domain USGS data, labelled with its reference (NAVD88 in the lower 48 states) and never compared with readings on other references. With weather lookup on, conditions also update by themselves every 15 minutes while the Timer or Conditions screen is open, never during a run and never in the background.

## Current status

Squib is an early prototype and is not yet available in any app store.

- The par timer works.
- Range conditions work with US weather stations (National Weather Service) and, anywhere else, with nearby airport reports (METAR). Where no station reports nearby, enter values by hand.
- Shot timing works in testing on a computer and in the Android emulator, but it has **not yet been tested on a real phone at a real range**. Treat its times as experimental until that testing is done.
- Long sessions and screen response have been measured on a computer and in the emulator only: 30 minutes of continuous capture with no gaps, and 95% of on-screen cues drawn within 34 ms of when they were due, the slowest in 87 ms (the target is 95% under 150 ms). These still need measuring on real phones.
- An iOS version exists as an experiment: the par timer only, on the same core. It builds and passes its tests on the iOS simulator in the automated checks, and has never run on an iPhone; shot timing on iOS needs one to develop.
- Squib does not claim any particular accuracy. It labels uncertain results, interrupted runs, and setups it cannot vouch for instead of guessing.

## Privacy

- No account, no ads, and no tracking.
- Squib uses the internet only for two lookups, each off until you turn it on:
  - Weather lookup (including weather alerts) sends your place, rounded to about 1 km, to the US National Weather Service, or for places it does not cover (or if you choose airport reports everywhere), to the US NOAA Aviation Weather Center.
  - Terrain elevation (US) sends your place to about 10 m, more precisely than weather, to the US Geological Survey's Elevation Point Query Service. While it is on, Squib keeps that more precise place on the phone; turning it off forgets it.

  Nothing else is ever sent, and Squib never sends audio anywhere.
- By default Squib never saves audio. Sound is analyzed in memory and discarded; only shot times and a coarse loudness outline are kept. Video clips, when you turn them on, have no sound track.
- A diagnostic recording keeps one run's microphone audio only when you switch it on for that run (it turns itself off afterwards, up to 60 s). It stays on the phone until you delete it, is left out of backups unless you choose to include it, and is never in shared results or problem reports. Exporting it for research is a separate step: you see exactly what is included, confirm a research-only statement, and choose where the file goes. Squib does not upload it.
- The microphone is requested only when you choose shot timing, sensitivity setup, or the cue test. The camera is requested only when you arm a run with video turned on or scan a drill QR code; the par timer never asks for it, and the scanner uses it only while its screen is open, without saving frames. Vibration cues use Android's vibrate permission, which has no prompt, and follow the phone's own vibration and Do Not Disturb settings. Location is requested only when you tap "Use my location"; typing coordinates or picking a saved place works without it.
- By default, runs keep weather values but not your coordinates, station names, or elevation. You can opt in to keeping full detail.
- History stays on your phone and is not included in Android's automatic backups. To move it, save a private backup file and restore it on the new phone. A backup is encrypted only if you choose "Encrypt with a passphrase" when saving it; otherwise anyone with the file can read it. Either way, store it somewhere you trust.
- Encrypted backups use [age](https://age-encryption.org), a standard file encryption format, with your passphrase (scrypt). Squib cannot recover a forgotten passphrase, and neither can anyone else. Because the format is standard, `age -d squib-backup.zip.age > squib-backup.zip` with any age tool gives you the ordinary backup.
- Sharing a result leaves out your location, station names, notes, photos, videos, and identifiers. Plan and stage notes are never shared.
- "Report a problem" opens a pre-filled GitHub issue in your browser for you to review and submit. It includes app versions, device model, and timing diagnostics, never audio, location, or notes.
- "Share a compatibility report" (in Data and backup) helps build the list of phones and audio routes that time shots well. You choose where the phone was, where you were shooting, and how well it worked, and see the exact text first. It summarises your recent live runs by audio route (counts, clock quality, sensitivity setup) with the device model, versions, and the date, never audio, location, notes, run identifiers, or times. You can open it as a pre-filled GitHub issue to review and submit, or copy it.

## Planned

Support for one dedicated shot-timer accessory and phone or watch remote controls, both waiting on suitable test hardware. Shot timing on iOS is waiting on an iPhone to test with. Video needs testing on real phones before it can leave experimental status.

## License

Squib is free software, licensed under the [GNU Affero General Public License v3.0 or later](LICENSE). You can use, study, change, and share it. If you distribute a modified version, or run one as a network service, you must share your source code under the same license.

In the app, History › Data and backup › About shows the version, the full license, a link to this source code, the licenses of the libraries Squib is built on, and a way to support development.

---

## Technical details

### Architecture

A shared Rust core holds all timing logic, shot detection, the run state machine, and the SQLite journal. The Android app (Kotlin, Jetpack Compose) handles permissions, audio capture and playback, and the screens. Control calls cross into Rust through UniFFI; audio samples cross through a small JNI bridge into a bounded, preallocated queue processed on its own thread. The experimental iOS app (SwiftUI) uses the same core through UniFFI's Swift bindings, statically linked, and plays cues with AVAudioEngine; it has no audio capture yet.

Range conditions are resolved field by field in Rust: a manual value wins, then a fresh phone sensor, then the best-ranked fresh nearby station, otherwise Unavailable. Pressure kinds are separate fields, so a station's altimeter setting or sea-level pressure can never stand in for the actual pressure where you are (NWS observations do not include station pressure). Airport METAR reports are parsed from their raw text rather than the API's rounded convenience fields, keeping the units each report states; humidity from METAR is derived from temperature and dew point and labelled as derived, and a QFE remark (actual pressure at the airport) stays a remote-station value. Ages always come from observation time, not fetch time. The Android app performs the HTTP requests the core plans and validates; snapshots saved with runs are redacted unless precise retention is enabled.

Timing comes from audio sample positions, not from when the app happens to receive audio. When a run's start beep and shots are captured in the same recording, the times between them are exact sample counts. Raw audio exists only in short-lived memory buffers unless the user turns on a diagnostic recording for a run: then the DSP worker offers each processed block to a writer thread through preallocated buffers (it never waits; anything the writer cannot keep becomes silence listed as a gap), so the WAV is sample-for-sample what the detector saw. `squib-replay diag` checks an export reproduces the phone's detections exactly and scores detector variants against the user's own review.

Video clips (experimental) are recorded with Camera2 into a surface encoder with no audio track, so the timing microphone is never shared. The app keeps each clip's first-frame camera timestamp and one set of clock readings taken next to a camera frame; the core decides from those which phone clock the camera follows and labels the result measured, assumed, or unavailable. Markers are drawn from the run's latest review revision, never from the video. Because a camera session can change how the microphone is processed, live timing with video counts as a separate audio route for sensitivity setups.

### Layout

```text
crates/domain/            run config, states, quality events, candidates, revisions, results
crates/timing/            clock mapping, capture integrity, cue matching, detector, run state machine, replay
crates/environment/       range conditions: measurement candidates, NWS and METAR adapters, resolver, privacy redaction
crates/training/          drill recipes, generic practice scoring, analytics, manual strings, round counts
crates/storage/           SQLite repository, migrations, recovery, day plans and checklists
crates/archive/           private backup/restore, CSV export, redacted result sharing
crates/ballistics/        point-mass ballistics solver and its validation suite (experimental)
crates/mobile-api/        UniFFI engine (control plane), JNI PCM bridge (data plane), DSP and store actors
tools/replay/             squib-replay: fixtures, replay, corpus evaluation, benchmark, stress study, diagnostic exports
tools/uniffi-bindgen/     pinned binding generator
testing/ballistics/       generators for the ballistics cross-solver reference data (development only)
apps/android/             Kotlin + Jetpack Compose app
apps/ios/                 iOS, best effort: SwiftUI par timer (project.yml for XcodeGen) on SquibKit, the Swift bindings and par-timer model
fixtures/synthetic/       golden synthetic WAV + label fixtures with SHA-256 manifest
fixtures/nws/             captured api.weather.gov responses (public domain) for adapter tests
fixtures/metar/           captured aviationweather.gov METAR responses (public domain) for adapter tests
fixtures/epqs/            captured USGS elevation responses (public domain) for terrain tests
testing/                  dependency-free mutation fuzzer shared by the parser robustness tests
scripts/                  Android and iOS core builds, Linux Swift check, emulator UI driver, third-party notices generator
fastlane/                 F-Droid store listing (descriptions, changelogs)
fdroid/                   draft F-Droid build recipe and open packaging points (not submitted)
.github/                  CI workflow and issue forms (used by the in-app problem report)
```

### Requirements

Rust 1.95.0 with the `aarch64-linux-android` and `x86_64-linux-android` targets, `cargo-ndk`, the Android SDK (platform 37, build-tools) and NDK 28.2.13676358 under `$ANDROID_HOME` (default `~/Android/Sdk`), and JDK 21 (Gradle can provision it). Minimum Android version is 8.0 (API 26).

### Build and test

```sh
# Shared core
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
# Parser robustness: a short fuzz run is part of cargo test; this is a longer one
SQUIB_FUZZ_ITERS=20000 SQUIB_FUZZ_SEED=12345 cargo test --release --workspace robust

# Replay tool
cargo build --release -p squib-replay
./target/release/squib-replay gen fixtures/synthetic        # regenerate fixtures (deterministic)
./target/release/squib-replay corpus fixtures/synthetic     # evaluate; non-zero exit on failure
./target/release/squib-replay run fixtures/synthetic/basic_5_shots_48k.wav \
    --labels fixtures/synthetic/basic_5_shots_48k.labels.json --random-chunks 7
./target/release/squib-replay bench --seconds 120
./target/release/squib-replay stress --markdown             # synthetic detector stress study (research)
./target/release/squib-replay diag squib-diagnostic.zip     # replay and score an opt-in diagnostic export

# Android (builds the Rust core with cargo-ndk and generates Kotlin bindings first)
cd apps/android
./gradlew :app:assembleDebug
./gradlew :app:connectedDebugAndroidTest    # needs a device or emulator
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

`-PsquibSkipRust=true` reuses previously built native libraries. `-PsquibRustProfile=debug` builds the core unoptimized.

### iOS (experimental)

```sh
# macOS with Xcode 26 and XcodeGen 2.46: the core for the simulator, the Swift bindings, the project
./scripts/build-ios-core.sh sim
cd apps/ios && xcodegen generate
xcodebuild test -project Squib.xcodeproj -scheme Squib -destination 'platform=iOS Simulator,name=iPhone 17'

# Linux with Docker: the Swift bindings and the par-timer model against the Linux core
./scripts/swift-linux-test.sh
```

`build-ios-core.sh` without `sim` also builds the device slice. The Xcode project is generated from `apps/ios/project.yml` and not committed; there is no signing setup, so builds are for the simulator.

### Release build

```sh
cd apps/android
./gradlew :app:assembleRelease    # minified with R8; unsigned unless signing is configured
```

Signing material never enters the repository. To sign, either create `apps/android/keystore.properties` (git-ignored) with `storeFile`, `storePassword`, `keyAlias` and `keyPassword`, or set `SQUIB_KEYSTORE`, `SQUIB_KEYSTORE_PASSWORD`, `SQUIB_KEY_ALIAS` and `SQUIB_KEY_PASSWORD`. Without either, the build produces `app-release-unsigned.apk`, as CI does.

The About screen's third-party licence notices (`app/src/main/assets/third_party_notices.json`) are generated from the Rust and Android dependency graphs: after changing a dependency, run `python3 scripts/third_party_notices.py` (CI fails while they are out of date).

The version is set once, as `squibVersion` (`MAJOR.MINOR.PATCH`) in `apps/android/app/build.gradle.kts`; the version code is derived from it (`MAJOR*10000 + MINOR*100 + PATCH`), so a source build always gets the same code. The R8 mapping for each release build is in `app/build/outputs/mapping/release/mapping.txt`.

To run the instrumented suite against the minified build on an emulator (local testing only; the debug key makes it installable):

```sh
./gradlew -PsquibTestBuildType=release -PsquibSignReleaseWithDebugKey=true :app:connectedReleaseAndroidTest
```

### Evidence so far

Unit, integration, and replay tests run on the desktop against synthetic recordings with known event positions. UI flows, permission denial, process termination recovery, and the audio transfer path have been exercised on an Android emulator. Synthetic audio and emulators cannot qualify a real microphone, speaker, or range environment; device and field testing are still to come. On iOS, the Swift layer is tested against the real core on Linux, and the app's unit and UI tests pass on the iOS simulator in CI; nothing has run on an iPhone.
