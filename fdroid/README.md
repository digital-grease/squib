# F-Droid packaging (draft)

Owner ruling D9: Squib is distributed through GitHub Releases and F-Droid, not Google Play. Nothing here has been submitted; publishing needs separate authorization.

- `net.digitalgrease.squib.yml` is a draft of the recipe F-Droid keeps in its own [fdroiddata](https://gitlab.com/fdroid/fdroiddata) repository. It carries the `Donate:` link (owner ruling D11).
- The store listing (title, descriptions, changelogs) is read from `fastlane/metadata/android/en-US/` in this repository. The changelog file name is the version code (`100.txt` for 0.1.0).
- The release build already suits F-Droid: the version code is derived from the version name, signing comes only from outside the repository (F-Droid signs with its own key), and Google's dependency-metadata block is off (ADR-058).

Open points before submitting:

1. **JNA's prebuilt native library.** The UniFFI bindings use JNA, whose Maven artifact ships `libjnidispatch.so` prebuilt. F-Droid's scanner may require building JNA from source (a srclib) or a scanner exception; check current fdroiddata practice for JNA-based apps.
2. **Rust toolchain.** The recipe installs Rust 1.95.0 (pinned in `rust-toolchain.toml`) and cargo-ndk 4.1.2 through the rustup srclib. Check the srclib version name and syntax with `fdroid lint`, and that the build server's NDK matches `ndkVersion` (28.2.13676358, r28c).
3. **Reproducible builds.** Not yet checked against F-Droid's builder. If wanted, the Rust build may need `--remap-path-prefix` so paths do not differ between builders.
4. **Screenshots and icon.** None in `fastlane/metadata/android/en-US/images/` yet (branding is low priority, owner ruling D15).
5. **First version.** `0.1.0` (code 100) and the `v0.1.0` tag are placeholders until the owner chooses the first public version.
