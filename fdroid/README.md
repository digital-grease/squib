# F-Droid packaging

Owner ruling D9: Squib is distributed through GitHub Releases and F-Droid, not Google Play. The owner has approved F-Droid submission (D18).

- `net.digitalgrease.squib.yml` is the recipe F-Droid keeps in its own [fdroiddata](https://gitlab.com/fdroid/fdroiddata) repository. The copy submitted there drops the header comment and gives each build's `commit` as the full hash of the release tag, never the tag name.
- The store listing (title, descriptions, changelogs, icon and screenshots) is read from `fastlane/metadata/android/en-US/` in this repository at the built commit. The summary must stay under 80 characters.
- F-Droid builds one APK per ABI (`-PsquibAbi=arm64-v8a` and `-PsquibAbi=x86_64`). Each gets version code `code * 10 + 2` or `code * 10 + 3`, where `code` is `squibVersionCode`; the GitHub Release keeps one APK for both ABIs at `code`. Changelogs are named by version code, so each release needs three copies (for 0.1.1: `101.txt`, `1012.txt`, `1013.txt`); the release workflow checks they exist.
- `fdroid checkupdates` reads `squibVersionCode` and `squibVersion` from `apps/android/app/build.gradle.kts` (`UpdateCheckData`), so both stay plain literals there, and `VercodeOperation` turns the code into the per-ABI codes.
- Signing comes only from outside the repository. F-Droid signs with its own key, and Google's dependency-metadata block is off (ADR-058).

Settled points:

1. **JNA's prebuilt native library.** The UniFFI bindings use the JNA AAR from Maven Central (5.19.1), which ships `libjnidispatch.so` prebuilt. Element X (`io.element.android.x` in fdroiddata) uses the same artifact with no srclib, scanner exception or `scandelete`, and the local test builds passed the scanner. Our own `libsquib_mobile.so` is built from source and is not committed.
2. **Rust toolchain.** The recipe installs rustup through the `rustup@1.28.2` srclib (the newest version in common use in fdroiddata), runs `rustup toolchain install` at the repository root so `rust-toolchain.toml` picks the toolchain, targets and components, and installs cargo-ndk 4.1.2. The Rust core is built for the entry's ABI in the recipe's `build` step and Gradle skips its own Rust task (`squibSkipRust=true`), because Gradle runs in a separate shell without cargo on its path. F-Droid installs NDK r28c (28.2.13676358), matching `ndkVersion`.
3. **Reproducible builds.** Deferred (owner ruling D21), as for fauxx and Signet: F-Droid signs Squib with its own key, so the F-Droid build and the GitHub Release APK cannot be installed over each other, and switching an existing F-Droid install to the Squib key later would need users to reinstall. If adopted, the Rust build may need `--remap-path-prefix` so paths do not differ between builders, and the recipe would gain `binary:` and `AllowedAPKSigningKeys:`.
4. **Network services.** The recipe declares `NonFreeNet` for the optional weather (aviationweather.gov, api.weather.gov) and terrain (epqs.nationalmap.gov) lookups, both off by default, following fdroiddata practice for apps that use NOAA, NWS or USGS services.
5. **First version.** 0.1.1 (owner, 2026-10-19). The v0.1.0 tag predates the literal version code that `checkupdates` needs and was not published.

Checked locally with fdroiddata's own CI procedure (image `fdroidserver:buildserver-trixie`, fdroidserver at the commit fdroiddata pins): `fdroid lint`, `fdroid rewritemeta` (no changes), the metadata JSON schema, `tools/check-fastlane.py` (no warnings) and `fdroid build --test --refresh-scanner --on-server`.
