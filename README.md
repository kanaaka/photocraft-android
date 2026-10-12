# PhotoCraft for Android (arm64-v8a)

An unofficial Android port of **[PhotoCraft](https://github.com/storytold/photocraft)** — the real engine
and egui interface, packaged as a NativeActivity APK for sideloading on arm64 devices.

| | |
|---|---|
| Upstream project | [`storytold/photocraft`](https://github.com/storytold/photocraft) — Apache-2.0 OR MIT |
| Pinned commit | `7a6058e` (upstream version `0.7.0`) |
| Android package | `ai.storyteller.photocraft` |
| Published APK | `photocraft-0.7.0-arm64-v8a-fonts-keys.apk` — 25.2 MB, arm64-v8a, signed |
| APK sha256 | `314cf5dbc062c3bcef8233f55d89a2b3a627c881760c0d4eb6a12033de373cc9` |
| Upstream files changed | 2 files — see [`patches/`](patches/) |
| New port crate | `apps/photocraft-android/` — 906 lines of Rust plus resources |

## What this repository is

This is **not a fork** of PhotoCraft. It holds only the Android layer, so it stays small, readable,
and obviously separable from upstream:

```
apps/photocraft-android/     the new shell crate — copy it into your checkout of upstream
patches/                the diff against upstream — apply it after checking out the pin
docs/PORT-NOTES.md      what was changed and why, fix by fix, with how each one was checked
scripts/                cross-compile environment, build and verify scripts (from shared-android-port)
CHANGELOG.md            what is in the published APK
```

The Android layer is not a reimplementation. It is a shell crate that fills the same `Services`
struct the desktop and web shells fill, so the real engine and the real egui UI run unchanged.

## Build it

Prerequisites: Rust (rustup) with the `aarch64-linux-android` target, Android SDK + NDK r27, a
JDK 21, and `cargo install cargo-apk`.

```bash
git clone https://github.com/storytold/photocraft.git
cd photocraft
git fetch --depth 1 origin 7a6058e6b4a5dbb725e75e97f7ee019d2bfb7f0f
git checkout FETCH_HEAD                        # the exact commit this port targets
git apply -3 /path/to/0001-photocraft-android-port.patch
cp -r /path/to/photocraft-android apps/             # the shell crate from this repository

# any keystore of your own signs an installable APK; a throwaway test key is fine for sideloading
keytool -genkeypair -keystore ks.jks -alias photocraft -keyalg RSA -keysize 2048 -validity 10000 \
        -storepass android -keypass android -dname "CN=Local Test, O=Local, C=US"
export CARGO_APK_RELEASE_KEYSTORE="$PWD/ks.jks" CARGO_APK_RELEASE_KEYSTORE_PASSWORD=android
cargo apk build --release -p photocraft-android
```

`scripts/android-env.sh` is the cross-compile environment this port was built with (the NDK linker
and the C compiler the native dependencies need), `scripts/build-apk.sh` wraps the build, and
`scripts/verify-apk.sh` checks the artifact: `aapt2 dump badging`, `apksigner verify`, and `llvm-nm`
on the `.so` inside the APK.

## Install

```bash
adb install -r photocraft-0.7.0-arm64-v8a-fonts-keys.apk
```
or copy the APK to the device and open it (installing from unknown sources must be allowed).

On first run, grant **Settings ▸ Apps ▸ PhotoCraft ▸ Special access ▸ All files access** if you want
the in-app picker to reach files outside the app's own folders. Without it, Android 10+ refuses a
`read_dir` of shared storage by path, and the picker can only offer the app's own directory.

## Verification status

Everything below was checked on the machine this port was published from, in October 2026.
A tick means the check was *run*, not inferred; the wording says how far it goes.

| Claim | How it was checked | Result |
|---|---|---|
| `patches/…patch` applies to a clean checkout of the pin | fresh `git init` + `git fetch --depth 1 origin 7a6058e6b4a5dbb725e75e97f7ee019d2bfb7f0f` + `git apply --check` | ✅ applies cleanly |
| The assembled tree equals the tree the APK was built from | file-by-file SHA-256 (line endings normalised) over every patched file and every file of the port crate | ✅ 19 files identical, none differing |
| The port crate compiles for Android | `cargo check -p photocraft-android --target aarch64-linux-android`, run in a tree assembled from scratch (fresh `git fetch` of the pin, patch applied, port crate copied in) | ✅ exit 0 (host `cargo check -p photocraft` stays green after the shared-crate patch) |
| Modifier-key synthesis (Ctrl / Shift / Alt) | `cargo test -p photocraft-android` (host, pure logic) | ✅ 5/5 pass, including the cross-frame Shift case that *is* the bug |
| APK is signed and is the right artifact | `apksigner verify --verbose`, `aapt2 dump badging`, sha256 | ✅ v2/v3 signatures true, `native-code: 'arm64-v8a'`, package `ai.storyteller.photocraft` |
| Behaviour on real hardware | hand-testing on the porter's arm64 tablet | ⚠️ the fixes below were found and driven by that testing; each published binary is not individually re-verified here |

**Before trusting the APK:** the source side is verified as listed above; runtime behaviour only as far
as [`docs/PORT-NOTES.md`](docs/PORT-NOTES.md) says per fix, plus the tablet testing that produced those
fixes. There is no automated device test in this repository.

## Known gaps

- **Runtime verification is hand-testing only.** No automated device test exists here; see
  *Verification status* above for exactly what was and was not checked.
- **No second window.** PhotoCraft has no *New Window* command on Android; one document at a time per
  Activity.
- **All files access is a manual grant.** Until it is given, the picker offers only the app's own
  directory — by design, and it says so on the locked root.
- **Android 16 ignores the status-bar opt-out.** Variant A of the theme is Android 15 only; the
  proper fix is a JNI `setOnApplyWindowInsetsListener` plus an egui top spacer, which is not
  implemented here.
- **A system font is only as good as `/system/fonts`.** If a device keeps a family out of there, the
  app cannot see it; the *Fonts* folder under `Android/media/…` is the way round that.


## The rest of this set

- [vectorcraft-android](https://github.com/kanaaka/vectorcraft-android) — VectorCraft 0.9.0
- [wordcraft-android](https://github.com/kanaaka/wordcraft-android) — WordCraft 0.5.0
- [pdfcraft-android](https://github.com/kanaaka/pdfcraft-android) — PdfCraft 0.6.0
- [gridcraft-android](https://github.com/kanaaka/gridcraft-android) — GridCraft 0.5.0
- [deckcraft-android](https://github.com/kanaaka/deckcraft-android) — DeckCraft 0.5.0
- [shared-android-port](https://github.com/kanaaka/shared-android-port) — build/verify scripts + the cross-compile environment

## Credits and licence

PhotoCraft is © its authors at [`storytold/photocraft`](https://github.com/storytold/photocraft) and is
dual-licensed Apache-2.0 / MIT; both licence texts are kept here. The Android layer in this
repository is an unofficial modification by [@kanaaka](https://github.com/kanaaka), carrying the
same licence. Not affiliated with or endorsed by the upstream project. See
[`ATTRIBUTION.md`](ATTRIBUTION.md).
