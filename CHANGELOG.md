# Changelog

## v0.1.7 — PhotoCraft appears in *Open with*

| | |
|---|---|
| APK | `photocraft-0.7.0-arm64-v8a-saf-singlewindow.apk` (25.6 MB) — supersedes the v0.1.6 asset |
| sha256 | `5d0189b6e474182ef8896920f8bf984b57d3103c190a7c8f76688a702e0ad0b3` |
| Built from | upstream `storytold/photocraft` @ `7a6058e6b4a5dbb725e75e97f7ee019d2bfb7f0f` + this repository at the commit that released it |
| Package | `ai.storyteller.photocraft` — versionName `0.7.0`, versionCode `16779008`, minSdk 26, targetSdk 35 |
| ABI | `arm64-v8a` only |
| Signature | a local test key, **not** the v0.1.1 one |

### Added
- **PhotoCraft is offered in *Open with*** for every image (`image/*`, so jpeg, png, webp, gif,
  tiff, heic, raw) and for Photoshop files, from the Files app, the gallery and anything else that
  sends an `ACTION_VIEW` intent.
  - Two intent filters, because the two ways a file manager can name a file do not agree: one by
    MIME type (`image/*` plus the four spellings providers use for a PSD), and one by extension
    (`.*\.(psd|psb)`) for a `.psd` a provider reports as `application/octet-stream` — which is what
    it does for an extension it has never heard of. A filter with *both* would need both to match.
  - The file arrives as a `content://` URI, which is read on a worker and opened as *contents*:
    the same path the in-app system picker uses, so a file opened from the gallery behaves like
    one picked from inside the app — including *Save* asking where to save rather than pretending a
    URI is a path.
  - Opening a second file while the app is already running reuses the running window
    (`singleTask` delivers it to `onNewIntent`), so a second window is not created — the fix for
    the overlapping windows applies here too.
  - Needs **no permission**: it is a URI the sender already granted.

### Notes
The bridge gained one more native callback (`onOpenWith`), covered by the existing host test that
every `native` method in `PickerActivity.java` has a matching symbol — now 13/13.

## v0.1.6 — saving: a staged save is written through a file descriptor

| | |
|---|---|
| APK | `photocraft-0.7.0-arm64-v8a-saf-singlewindow.apk` (25.6 MB) — supersedes the v0.1.5 asset |
| sha256 | `32760c4ccb493a46c15f904db9b6262ad94db67b284016026a0f06ec28b3295a` |
| Built from | upstream `storytold/photocraft` @ `7a6058e6b4a5dbb725e75e97f7ee019d2bfb7f0f` + this repository at the commit that released it |
| Package | `ai.storyteller.photocraft` — versionName `0.7.0`, versionCode `16779008`, minSdk 26, targetSdk 35 |
| ABI | `arm64-v8a` only |
| Signature | a local test key, **not** the v0.1.1 one |

### Fixed
- **Nothing could be saved**, PSD or exported image: `Method not found: write ([B)` followed by
  *Could not write content://…*. The chosen location was reached correctly and the staging file was
  written correctly; pushing the bytes to it was the failure. `ContentResolver.openOutputStream`
  returns a `ParcelFileDescriptor.AutoCloseOutputStream`, which overrides only `write(int)`, so
  `write(byte[])` on it is `OutputStream`'s base-class loop that writes **one byte per JNI call** —
  and the signature asked for was `([B)I` where `OutputStream.write(byte[])` returns **void**, which
  matches nothing. (jni's error prints only the argument part of a signature, so the return type —
  the whole difference — is not visible in the message.)
  A save now goes through `ContentResolver.openFileDescriptor(uri, "wt")` and
  `ParcelFileDescriptor.detachFd()`: an ordinary buffered `std::io::copy` onto a descriptor this
  process owns, with no Java array the size of the document and no per-byte JNI calls.
- **A fourth host test guards it** (`cargo test -p photocraft-android`, 13/13): a staged save must
  not go through `openOutputStream`. Confirmed to fail when it does.

Read the other way round is worth saying too: opening worked and still does. This is the fourth
framework shape assumed rather than checked; `docs/PORT-NOTES.md` now carries the AOSP source for
each one.

## v0.1.5 — the picked file could not be turned back into a `Uri`

| | |
|---|---|
| APK | `photocraft-0.7.0-arm64-v8a-saf-singlewindow.apk` (25.6 MB) — supersedes the v0.1.4 asset |
| sha256 | `82f3d941e79f614b3bc0135205937afbdab2689f9e34978bca494d8b3fc39034` |
| Built from | upstream `storytold/photocraft` @ `7a6058e6b4a5dbb725e75e97f7ee019d2bfb7f0f` + this repository at the commit that released it |
| Package | `ai.storyteller.photocraft` — versionName `0.7.0`, versionCode `16779008`, minSdk 26, targetSdk 35 |
| ABI | `arm64-v8a` only |
| Signature | a local test key, **not** the v0.1.1 one |

### Fixed
- **Picking a file gave up at the last step:** `Method not found: <init> (Ljava/lang/String;)V`.
  `android.net.Uri`'s one-argument constructor is **private**; the public way in is the static
  `Uri.parse(String)`. The picker opened and returned a file, and turning its `content://` URI back
  into a `Uri` object to read it threw `NoSuchMethodError` — so the app reported
  *"Android's file picker did not work"* and dropped into the in-app browser, every time. All four
  places that built a `Uri` (take the persistable grant, read the bytes, write the bytes, ask the
  provider for the file's name) went through one helper now, and it uses `Uri.parse`.
- **A third host test guards it** (`cargo test -p photocraft-android`, 12/12): this crate must not
  construct a `android.net.Uri`. Confirmed to fail when it does.

This was the third fix in a row that compiled, linked, signed and installed perfectly and failed only
on the device — a framework API that does not exist in the form it was assumed to take, a native
method the VM could not resolve, a class loader that was never told. Each is now either guarded by a
host test over the source or written down with the framework source that says why, in
[`docs/PORT-NOTES.md`](docs/PORT-NOTES.md).

## v0.1.4 — the Java bridge could not find its native methods

| | |
|---|---|
| APK | `photocraft-0.7.0-arm64-v8a-saf-singlewindow.apk` (25.6 MB) — supersedes the v0.1.3 asset |
| sha256 | `f668d471d30921d290b05f7e02ba27b06fb09130808893a986fad51b98572484` |
| Built from | upstream `storytold/photocraft` @ `7a6058e6b4a5dbb725e75e97f7ee019d2bfb7f0f` + this repository at the commit that released it |
| Package | `ai.storyteller.photocraft` — versionName `0.7.0`, versionCode `16779008`, minSdk 26, targetSdk 35 |
| ABI | `arm64-v8a` only |
| Signature | a local test key, **not** the v0.1.1 one |

### Fixed
- **The app crashed on launch:** `java.lang.UnsatisfiedLinkError: No implementation found for void
  ai.storyteller.photocraft.PickerActivity.onPickerResume()`. `NativeActivity.onCreate` loads the
  library with `System.load(path)`, and `System.load` registers a native library against the class
  loader of the class that called it — the *boot* loader, since `NativeActivity` is a framework
  class. The VM resolves a native method against the loader of the class that declares it, and
  `PickerActivity` comes from the app's own `classes.dex`, so it looked in the app's loader, found
  the library missing there and threw. The three symbols were exported all along (`llvm-nm -D`
  shows them); nothing was wrong with them, only with who had told the VM about the library.
  `PickerActivity` now calls `System.loadLibrary("photocraft")` in a static initialiser, which
  registers the same library with the app's loader as well; the block runs before the system
  constructs the Activity.
- **Two host tests now guard that wiring** (`cargo test -p photocraft-android`, 11/11): one checks
  that every `native` method declared in `PickerActivity.java` has a matching
  `Java_ai_storyteller_photocraft_PickerActivity_*` symbol in this crate, the other that the class
  loads the library itself. Both are checks nothing else can make: JNI entry points are resolved by
  name, by the VM, on a device, and the wrong wiring compiles and links perfectly. The second test
  was confirmed to fail when the static block is removed.

### Notes
The rest of v0.1.3 stands: the system picker, the lost-result watchdog, and the single-window fix.
Still signed with a local test key that is **not** the v0.1.1 one, so the v0.1.1 APK has to be
uninstalled before this one installs.

### Verified on the artifact, not on a device
`dexdump -d` on the shipped `classes.dex`: `PickerActivity.<clinit>` contains
`const-string "photocraft"` + `System.loadLibrary`, ahead of `onPickerResult` / `onPickerPause` /
`onPickerResume` as `PRIVATE STATIC NATIVE`. `llvm-nm -D` on the `.so`: the three matching symbols.
Manifest: `android:hasCode=true`, activity `ai.storyteller.photocraft.PickerActivity`,
`launchMode=2`. `apksigner verify --verbose`: v2 and v3 true. The app starting at all, and the
picker round trip, are still **not** verified here — there is no device in this loop.

## v0.1.3 — the picker actually answers, and the app cannot get stuck

| | |
|---|---|
| APK | `photocraft-0.7.0-arm64-v8a-saf-singlewindow.apk` (25.6 MB) — supersedes the v0.1.2 asset |
| sha256 | `b59134443906dae71b0d95908b650138256c26fad101a5c2ced7f69ad2b779c4` |
| Built from | upstream `storytold/photocraft` @ `7a6058e6b4a5dbb725e75e97f7ee019d2bfb7f0f` + this repository at the commit that released it |
| Package | `ai.storyteller.photocraft` — versionName `0.7.0`, versionCode `16779008`, minSdk 26, targetSdk 35 |
| ABI | `arm64-v8a` only |
| Signature | a local test key, **not** the v0.1.1 one |

### Fixed
- **The system picker's answer was never delivered.** v0.1.2 exported `ANativeActivity_onActivityResult`,
  on the reasoning that `NativeActivity.onActivityResult` is a native method the VM would resolve
  here. It is not: AOSP's `NativeActivity.java` declares no `onActivityResult` at all, so nothing was
  ever resolved and the NDK glue's identically named symbol is dead code. Picking a file closed the
  picker and left the app waiting for ever. The APK now carries one small Java class,
  `ai.storyteller.photocraft.PickerActivity` (a `NativeActivity` subclass, 1 056 bytes of dex), which
  calls three native methods in `libphotocraft.so`.
- **The tab's close button (and File ▸ Close, and File ▸ Exit) did nothing.** A consequence of the
  above: while a file dialog is open, PhotoCraft suppresses its unsaved-changes prompt and drops any
  command that needs it. With the dialog stuck open, closing a modified document was silently
  ignored.
- **A lost picker result can no longer strand the app.** The same Java class reports pause and resume;
  Android delivers `onActivityResult` before `onResume`, so a resume with no result means the answer
  was lost, and the request is cancelled rather than waited on for ever.

### Changed
- The build is two commands now: `cargo apk build --release -p photocraft-android`, then
  `scripts/add-picker-activity.sh`, which compiles the Java class with `javac` + `d8`, adds
  `classes.dex` to the APK with `aapt add` and re-aligns and re-signs it. `cargo apk build` packages
  a native library and nothing else.

### Notes
Still signed with a local test key that is **not** the v0.1.1 one, so the v0.1.1 APK has to be
uninstalled before this one installs. The single-window fix is unchanged and still in the manifest.

### Verified on the artifact, not on a device
`aapt2 dump badging`: package `ai.storyteller.photocraft`, `native-code: 'arm64-v8a'`.
`aapt2 dump xmltree`: `android:launchMode=2`, `android:hasCode=true`, activity
`ai.storyteller.photocraft.PickerActivity`. `aapt list`: `classes.dex` present.
`apksigner verify --verbose`: v2 and v3 true. `dexdump -f` on the shipped `classes.dex`:
`Lai/storyteller/photocraft/PickerActivity;` extending `Landroid/app/NativeActivity;` with
`onPickerResult` / `onPickerPause` / `onPickerResume` as `PRIVATE STATIC NATIVE`.
`llvm-nm -D` on the `.so` inside the APK: the three matching `Java_ai_storyteller_photocraft_PickerActivity_*`
symbols. The round trip itself is **not** verified here — there is no device in this loop; see
*Verification status* in the README.

## v0.1.2 — the system file picker, and one window

| | |
|---|---|
| APK | `photocraft-0.7.0-arm64-v8a-saf-singlewindow.apk` (25.6 MB) — supersedes the v0.1.1 asset |
| sha256 | `5f98c580606e947e5d35358c1538a0745103811c4747c3c0785bf58a8924e166` |
| Built from | upstream `storytold/photocraft` @ `7a6058e6b4a5dbb725e75e97f7ee019d2bfb7f0f` + this repository at the commit that released it |
| Package | `ai.storyteller.photocraft` — versionName `0.7.0`, versionCode `16779008`, minSdk 26, targetSdk 35 |
| ABI | `arm64-v8a` only |
| Signature | a local test key, **not** the v0.1.1 one |

### Changed
- **File ▸ Open and Save As now open Android's own file picker** (the Storage Access Framework:
  `ACTION_OPEN_DOCUMENT` / `ACTION_CREATE_DOCUMENT`), not the browser this port drew itself. That
  browser could only ever show what `read_dir` returns — no cloud provider, no the Downloads
  collection, no card reader, and on Android 10+ nothing outside the app's own directory unless *All
  files access* was granted by hand — which is what "it misses files" was.
  - Reached over JNI: `android-activity` exposes neither `startActivityForResult` nor a result
    callback, but Java's `NativeActivity.onActivityResult` is a `native` method the VM resolves in
    this library, so the bridge is one exported symbol. Still no Java in the project.
  - A save is written to a private staging file and then streamed into the location the user chose,
    because the app writes atomically to a *path* and a `content://` URI is not one.
  - The in-app browser is kept as the fallback, for a device whose DocumentsUI cannot start.
- **No second window.** The manifest now asks for `singleTask` and for the app to handle every
  configuration change itself, and `android_main` refuses to start a second event loop in one process:
  a duplicate Activity closes itself instead of sitting on top of the running one holding the input
  focus, which is what left the app unusable.

### Notes
The picker needs **no storage permission**; the declarations in `Cargo.toml` are now there for the
fallback browser only. Signed with a local test key that is **not** the v0.1.1 one, so the previous
APK has to be uninstalled before this one installs.

### Verified on the artifact, not on a device
`aapt2 dump badging`: package `ai.storyteller.photocraft`, `native-code: 'arm64-v8a'`.
`aapt2 dump xmltree`: `android:launchMode=2` (singleTask) and `android:configChanges=0x40003ff4`, which
is the manifest half of the one-window fix. `apksigner verify --verbose`: v2 and v3 true.
`llvm-nm -D` on the `.so` inside the APK: `ANativeActivity_onActivityResult` (the JNI callback this
picker needs), `ANativeActivity_onCreate` and `android_main` all exported. The picker round trip
itself is **not** verified here — there is no device in this loop; see *Verification status* in the
README.

## v0.1.1 — 2026-10-13 (scrolling, modifiers, fonts, picker wording)

| | |
|---|---|
| APK | `photocraft-0.7.0-arm64-v8a-20261013.apk` (25.2 MB) — supersedes the v0.1.0 asset, same signing key |
| sha256 | `4cfcfc2a87399f23c3af67c4eb16907e58550f648dc09c9a716c15377a926e27` |
| Built from | upstream `storytold/photocraft` @ `7a6058e6b4a5dbb725e75e97f7ee019d2bfb7f0f` + this repository at the commit that released it |
| Package | `ai.storyteller.photocraft` — versionName `0.7.0`, versionCode `16779008`, minSdk 26, targetSdk 35 |
| ABI | `arm64-v8a` only |

### Changed
- **A save no longer blocks the event loop.** Long commands and the document write now run through the
  app's background-job path (`background_jobs = true` plus the worker-thread `save_file` service that
  path needs), instead of inline on the event loop — the ~5-second stall that had Android offering
  *Force close* mid-save.
- **Picker wording corrected.** An empty folder in shared storage no longer tells a user who *has*
  granted *All files access* to grant it; the note is conditional and leaves the decision to the user.
  The USB tip now depends on the folder (`Android/data` cannot be reached over a cable; `Android/media`
  can).

### Notes
The build was made from exactly the source in this repository (upstream pin + the patch + the port
crate) and signed with the same local test key as v0.1.0, so it installs over it.


## v0.1.0 — first published port (PhotoCraft 0.7.0)

| | |
|---|---|
| APK | `photocraft-0.7.0-arm64-v8a-fonts-keys.apk` (25.2 MB) |
| sha256 | `314cf5dbc062c3bcef8233f55d89a2b3a627c881760c0d4eb6a12033de373cc9` |
| Built from | upstream `storytold/photocraft` @ `7a6058e6b4a5dbb725e75e97f7ee019d2bfb7f0f` + `patches/0001-photocraft-android-port.patch` + `apps/photocraft-android/` |
| Package | `ai.storyteller.photocraft` — versionName `0.7.0`, minSdk 26, targetSdk 35 |
| ABI | `arm64-v8a` only |
| Signature | throwaway local test key, not an official key |

### In this release
- Android shell crate: NativeActivity entry point, preferences under the app's own data directory,
  file logger, single-window lifecycle.
- In-app file picker drawn with egui, with readable-root probing, "Show all files", hidden-file
  count, and a locked root that names the permission it needs.
- Edge-to-edge fix for `targetSdkVersion 35` (`res/values/styles.xml`, variant B immersive).
- **File ▸ Exit** now really ends the process (it previously appeared to hang on a stale close).
- **Ctrl / Shift / Alt** work: the platform reports no modifiers, so they are tracked from key events
  and re-injected (5 host unit tests, all passing).
- **System fonts** (`/system/fonts` and friends) plus a USB-visible *Fonts* folder a user can drop a
  `.ttf` into.
- HEIC/HEIF opening enabled by default (as the official builds compile it).
- App icon (adaptive) and launcher label; `MANAGE_EXTERNAL_STORAGE` + media permissions declared.


### Known issues
See *Known gaps* in the [README](README.md) and [`docs/PORT-NOTES.md`](docs/PORT-NOTES.md).
