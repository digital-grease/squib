# F-Droid packaging (draft)

Owner ruling D9: Squib is distributed through GitHub Releases and F-Droid, not Google Play. The owner has approved F-Droid submission (D18); nothing has been submitted yet.

- `net.digitalgrease.squib.yml` is a draft of the recipe F-Droid keeps in its own [fdroiddata](https://gitlab.com/fdroid/fdroiddata) repository. It carries the `Donate:` link (owner ruling D11).
- The store listing (title, descriptions, changelogs) is read from `fastlane/metadata/android/en-US/` in this repository. The changelog file name is the version code (`100.txt` for 0.1.0).
- The release build already suits F-Droid: the version code is derived from the version name, signing comes only from outside the repository (F-Droid signs with its own key), and Google's dependency-metadata block is off (ADR-058).

Open points before submitting:

1. **JNA's prebuilt native library.** Likely settled: the UniFFI bindings use the JNA AAR from Maven Central (5.19.1), which ships `libjnidispatch.so` prebuilt. Element X (`io.element.android.x` in fdroiddata, checked 2026-10-17) uses the same artifact, version and AAR form with no srclib, scanner exception or `scandelete` for it, so F-Droid accepts it. Our own `libsquib_mobile.so` is built from source by the `buildRustCore` Gradle task and is not committed. Confirm with `fdroid scanner` during the test build.
2. **Rust toolchain.** The recipe installs Rust 1.95.0 (pinned in `rust-toolchain.toml`) and cargo-ndk 4.1.2 through the rustup srclib. Check the srclib version name and syntax with `fdroid lint`, and that the build server's NDK matches `ndkVersion` (28.2.13676358, r28c).
3. **Reproducible builds.** Not yet checked against F-Droid's builder. If wanted, the Rust build may need `--remap-path-prefix` so paths do not differ between builders.
4. **Screenshots and icon.** None in `fastlane/metadata/android/en-US/images/` yet (branding is low priority, owner ruling D15).
5. **First version.** Settled: the first public version is `0.1.0` (code 100), tag `v0.1.0`.
