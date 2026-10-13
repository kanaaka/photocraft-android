# PhotoCraft for Android (arm64-v8a)

An unofficial Android port of **[PhotoCraft](https://github.com/storytold/photocraft)** — the real engine
and egui interface, packaged as a NativeActivity APK for sideloading on arm64 devices.

| | |
|---|---|
| Upstream project | [`storytold/photocraft`](https://github.com/storytold/photocraft) — Apache-2.0 OR MIT |
| Pinned commit | `7a6058e` (upstream version `0.7.0`) |
| Android package | `ai.storyteller.photocraft` |
| Published APK | `photocraft-0.7.0-arm64-v8a-saf-singlewindow.apk` — 25.6 MB, arm64-v8a, signed (release **v0.1.7**) |
| APK sha256 | `5d0189b6e474182ef8896920f8bf984b57d3103c190a7c8f76688a702e0ad0b3` |
| Upstream files changed | 2 files — see [`patches/`](patches/) |
| New port crate | `apps/photocraft-android/` — the shell, the SAF picker bridge, the modifier fix |

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
JDK 21+, and `cargo-apk` (build the git version if you want `launch_mode`; a crates.io release may
predate it):

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

# then, because `cargo apk build` packages a native library and nothing else, and the Activity
# in the manifest has to be a compiled class (apps/photocraft-android/java, ~40 lines):
bash /path/to/photocraft-android/scripts/add-picker-activity.sh
```

That second command compiles the one Java class with `javac` + `d8`, drops the resulting
`classes.dex` into the APK with `aapt add`, and re-aligns and re-signs it — the same `zipalign` +
`apksigner` pair cargo-apk runs, because adding an entry moves what follows it.

`scripts/android-env.sh` is the cross-compile environment this port was built with (the NDK linker
and the C compiler the native dependencies need), `scripts/build-apk.sh` wraps the build, and
`scripts/verify-apk.sh` checks the artifact: `aapt2 dump badging`, `apksigner verify`, and `llvm-nm`
on the `.so` inside the APK.

## Install

```bash
adb install -r photocraft-0.7.0-arm64-v8a-fonts-keys.apk
```
or copy the APK to the device and open it (installing from unknown sources must be allowed).

**File ▸ Open and Save As use Android's own file picker**, so no storage permission is needed to reach
your files. The in-app browser is only the fallback for a device where the system picker cannot start;
if you ever see it, granting **Settings ▸ Apps ▸ PhotoCraft ▸ Special access ▸ All files access** widens
what it can reach (Android 10+ otherwise refuses a `read_dir` of shared storage by path).

**PhotoCraft is also offered in *Open with*** for images and Photoshop files, so a file picked in the
Files app or the gallery opens in it. A file opened that way has no path of its own — *Save* asks where
to write, through the same picker.

## Verification status

Everything below was checked on the machine this port was published from. A tick means the check was
*run*, not inferred; the wording says how far it goes.

| Claim | How it was checked | Result |
|---|---|---|
| `patches/…patch` applies to a clean checkout of the pin | fresh `git fetch` of `7a6058e6b4a5dbb725e75e97f7ee019d2bfb7f0f` + `git apply --check` | ✅ applies cleanly |
| The port crate compiles for Android | `cargo check -p photocraft-android --target aarch64-linux-android`, in a tree assembled from scratch (fresh fetch of the pin, patch applied, port crate copied in) | ✅ exit 0, one pre-existing dead-field warning |
| The desktop app still builds after the shared-crate patch | `cargo check -p photocraft` (host) | ✅ exit 0 |
| Modifier-key synthesis (Ctrl / Shift / Alt) | `cargo test -p photocraft-android` (host, pure logic) | ✅ pass, including the cross-frame Shift case that *is* the bug |
| The port crate's own tests | `cargo test -p photocraft-android` | ✅ 13/13 (5 modifier, 4 picker-name/MIME, 4 Java/native wiring) |
| APK is signed and is the right artifact | `apksigner verify --verbose`, `aapt2 dump badging`, sha256 | ✅ v2/v3 signatures true, `native-code: 'arm64-v8a'`, package `ai.storyteller.photocraft` |
| *Open with* is declared | apt2 dump xmltree --file AndroidManifest.xml | two ACTION_VIEW filters (image/* + four Photoshop types; and .*\.(psd\|psb) by extension) with DEFAULT+BROWSABLE over content and ile, beside the launcher's MAIN/LAUNCHER one |
| The single-window fix is in the manifest | `aapt2 dump xmltree --file AndroidManifest.xml` | ✅ `android:launchMode=2` (singleTask), `android:configChanges=0x40003ff4` |
| The SAF bridge is in the binary | `dexdump -d` on the `classes.dex` inside the APK, and `llvm-nm -D` on the `.so` | ✅ one class `ai.storyteller.photocraft.PickerActivity` extending `NativeActivity`; its `<clinit>` calls `System.loadLibrary("photocraft")` *first* (the loader trap that made v0.1.3 crash on launch); `onPickerResult`/`onPickerPause`/`onPickerResume` native, with the three matching `Java_…_PickerActivity_*` symbols exported; `aapt list` shows `classes.dex`, manifest `hasCode=true` |
| The Java/native wiring agrees | four host tests over `PickerActivity.java` and this crate's source | ✅ every declared native method has a symbol; the class loads the library itself; `Uri` is parsed rather than constructed; a staged save is written through a descriptor rather than an `OutputStream`. Each one was confirmed to fail when the thing it guards is put back |
| **The app starting, and the picker round trip** | — | ❌ **not run**: there is no device in this loop. The bridge compiles, its class, its `<clinit>` and its symbols are in the artifact, but whether a picked file opens and a saved one lands where the user chose is unproven until it is run |

**Before trusting the APK:** the source side is verified as listed above; runtime behaviour only as far
as [`docs/PORT-NOTES.md`](docs/PORT-NOTES.md) says per fix. There is no automated device test in this
repository.

## Known gaps

- **Runtime verification is hand-testing only.** No automated device test exists here; see
  *Verification status* above for exactly what was and was not checked.
- **A save's remembered path is a staging file.** A file saved through Android's picker is written to
  a private path first and copied to the location the user chose, so the document's *recent files* entry
  names that private path. The bytes are where the user put them; the name is not.
- **All files access is a manual grant.** The system picker does not need it; the in-app browser that
  backs it up does, and it says so on the locked root.
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
