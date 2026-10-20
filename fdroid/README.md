# F-Droid packaging (draft)

Owner ruling D9: Squib is distributed through GitHub Releases and F-Droid, not Google Play. The owner has approved F-Droid submission (D18); nothing has been submitted yet.

- `net.digitalgrease.squib.yml` is a draft of the recipe F-Droid keeps in its own [fdroiddata](https://gitlab.com/fdroid/fdroiddata) repository. It carries the `Donate:` link (owner ruling D11).
- The store listing (title, descriptions, changelogs) is read from `fastlane/metadata/android/en-US/` in this repository. The changelog file name is the version code (`100.txt` for 0.1.0).
- The release build already suits F-Droid: the version code is derived from the version name, signing comes only from outside the repository (F-Droid signs with its own key), and Google's dependency-metadata block is off (ADR-058).

Open points before submitting:

1. **JNA's prebuilt native library.** Settled: the UniFFI bindings use the JNA AAR from Maven Central (5.19.1), which ships `libjnidispatch.so` prebuilt. Element X (`io.element.android.x` in fdroiddata) uses the same artifact with no srclib, scanner exception or `scandelete`, and the local test build below passed the scanner. Our own `libsquib_mobile.so` is built from source and is not committed.
2. **Rust toolchain.** Settled. The recipe installs rustup through the `rustup@1.28.2` srclib (the newest version in common use in fdroiddata), runs `rustup toolchain install` at the repository root so `rust-toolchain.toml` picks the toolchain, targets and components, and installs cargo-ndk 4.1.2. The Rust core is built in the recipe's `build` step and Gradle skips its own Rust task (`squibSkipRust=true`), because Gradle runs in a separate shell without cargo on its path. F-Droid installs NDK r28c (28.2.13676358), matching `ndkVersion`.

   Verified 2026-10-19 with fdroiddata's CI procedure run locally (image `fdroidserver:buildserver-trixie`, fdroidserver at the commit fdroiddata pins): `fdroid lint` passes, `fdroid rewritemeta` leaves the recipe unchanged apart from removing the header comment, and `fdroid build --test --refresh-scanner --on-server` built version code 100 from commit b15f0f7 with no scanner findings. The APK holds `libsquib_mobile.so` and `libjnidispatch.so` for arm64-v8a and x86_64, versionCode 100, versionName 0.1.0.
3. **Reproducible builds.** Deferred (owner ruling D21): F-Droid signs Squib with its own key, so the F-Droid build and the GitHub Release APK (signed with the Squib key) cannot be installed over each other. If adopted later, the Rust build may need `--remap-path-prefix` so paths do not differ between builders, and the recipe would gain `binary:` and `AllowedAPKSigningKeys:`.
4. **Screenshots and icon.** None in `fastlane/metadata/android/en-US/images/` yet (branding is low priority, owner ruling D15).
5. **First version.** Settled: the first public version is `0.1.0` (code 100), tag `v0.1.0`.
