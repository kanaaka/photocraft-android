# Port notes — PhotoCraft on Android

Upstream: [`storytold/photocraft`](https://github.com/storytold/photocraft) @
`7a6058e6b4a5dbb725e75e97f7ee019d2bfb7f0f` (v0.7.0)
Published APK: `photocraft-0.7.0-arm64-v8a-fonts-keys.apk`

This is the edit log for the port: what was wrong, why, what changed, and how far the change was
checked. Each fix below was found by running the app on an Android tablet and observing the failure,
not by reading the code and guessing.

## 1. The shell crate — `apps/photocraft-android/`

PhotoCraft's UI crate defines a `Services` struct of `Option<Box<dyn Fn…>>` fields: file dialogs,
preferences load/save, quitting, fonts. The desktop binary and the browser shell fill that struct in
their own way; Android needs a third one, and once it exists the whole engine and egui interface run
unmodified.

Three details are load-bearing:

- **`[lib] name = "photocraft"` with `crate-type = ["cdylib"]`.** cargo-apk turns that into
  `lib/arm64-v8a/libphotocraft.so` and points the injected `android.app.lib_name` meta-data at the
  artifact name (`-` → `_`). A mismatch produces an APK that installs and dies on launch.
- **`options.android_app = Some(app)`** in `NativeOptions`. `eframe::run_native` errors without it:
  the Android event loop is built from that handle.
- **The `unstable` window options and a file logger into the app's own data dir.** logcat needs adb
  and a sideloaded build has none, so the log is a file beside the preferences.

`src/lib.rs` (315 lines) is the shell and the entry point; `src/file_picker.rs` (381) is the picker;
`src/modifiers.rs` (210) is the keyboard fix; `res/` is the theme and the icon.

## 2. The screen reader feature was a build error — `crates/ui-egui/Cargo.toml`

`eframe` refuses `accesskit` together with `android-native-activity`
(`compile_error!("`accesskit` feature is only available with `android-game-activity`")`) — on Android
egui's screen-reader integration goes through the GameActivity backend, while this port packages
NativeActivity. Cargo applies features only from target tables whose `cfg` matches, so the crate now
declares `eframe` twice:

```toml
[dependencies]
eframe = { workspace = true, features = ["default_fonts", "wgpu"] }
[target.'cfg(not(target_os = "android"))'.dependencies]
eframe = { workspace = true, features = ["accesskit"] }
```

Desktop keeps screen-reader support, Android drops it. **Checked:** `cargo check -p photocraft`
(host) and `cargo check --target aarch64-linux-android` both exit 0.

## 3. The menu bar sat under the notification bar — `res/values/styles.xml`

The APK targets `targetSdkVersion 35`, and Android 15 enforces edge-to-edge for every app targeting
35+: the system stops insetting the window and draws the app underneath a transparent status bar.
winit 0.30.13 has no Android safe-area API, so egui lays its menu bar out at `y = 0` — behind the
notification bar, where it is drawn but never receives touches.

`res/values/styles.xml` ships **two** one-attribute themes so the tablet can choose:

- **Variant A** — `windowOptOutEdgeToEdgeEnforcement`: the window is inset below the status bar
  again, so the app's own top edge is tappable. Android 15 only; Android 16 ignores the attribute.
- **Variant B** — hide the status bar entirely and run immersive, so there is no bar to collide with.

The port ships variant B (`theme = "@style/PhotocraftFullscreen"`); variant A is kept in the file,
commented, because which one is better depends on the device.

## 4. File ▸ Exit did nothing

Desktop PhotoCraft answers Exit with `egui::ViewportCommand::Close` and lets the window closing take
the process with it. **On Android that ends nothing:** `egui-winit` turns the command into a plain
`ViewportEvent::Close` and never touches winit's window API, which is stubbed on Android anyway. The
app then sits on a stale close request, dies on the next input event, and cannot be reopened until it
is cleared from Recents — because the native thread returned while the Activity lived on.

`Services::quit` is filled with a real process exit, and the logger is flushed first (a process exit
runs no `on_exit`, so an unflushed file logger loses its last lines):

```rust
quit: Some(Box::new(|| {
    log::info!("File > Exit: ending the process");
    log::logger().flush();
    std::process::exit(0);
})),
```

## 5. Ctrl, Shift and Alt did nothing — `src/modifiers.rs`

**Root cause, from winit's own source:** its Android backend emits only six window events and never
`ModifiersChanged`. `egui-winit` updates modifiers from that event and nowhere else, and egui carries
modifiers as an *event*, so on Android `ctx.input(|i| i.modifiers)` returns the all-false default for
the entire run. Ctrl+A, Ctrl+S and Ctrl+Z are dead, and Shift-click selects nothing.

The key events *do* arrive, so `modifiers.rs` tracks them in the shell's `raw_input_hook` and
re-injects the result the way egui expects: folded into the held state, stamped onto every `Key` and
`PointerButton` event, plus one `ModifiersChanged` per frame. State is carried **between** frames,
because the press and the click usually land in different ones.

**Checked:** the logic is pure, so five unit tests run on the host. In a tree assembled from scratch
(fresh `git fetch` of the pin, patch applied, port crate copied in), `cargo test -p photocraft-android`
passes **5/5** — including `shift_held_in_one_frame_reaches_a_click_in_the_next`, the cross-frame case
that *is* the bug.

## 6. No system fonts, and no way to install one — `crates/text/src/fonts.rs`

Two separate problems, both silent:

- **The app had no system fonts at all.** `system_font_dirs()` branches macOS / Windows / else-Linux,
  and Android falls into the Linux branch: it scans `/usr/share/fonts`, `~/.fonts` and
  `$XDG_DATA_HOME/fonts`, none of which exist on a tablet. Missing directories are skipped without a
  word, so every document set in another typeface fell back to a bundled family and the app reported
  nothing. The patch adds the platform's own directories (`/system/fonts`, `/product/fonts`,
  `/system_ext/fonts`, `/vendor/fonts`).
- **There was no way to add a font.** The patch also adds `set_extra_font_dirs`, a one-shot hook the
  shell registers *before* the window opens (the scan is one-shot too: register late and it is ignored
  forever). The shell points it at the app's own **USB-visible** folder:

  ```
  Android/media/ai.storyteller.photocraft/Fonts
  ```

  Connect the tablet over USB, drop a `.ttf`/`.otf`/`.ttc` in, restart PhotoCraft, and the family
  appears in the type tool. It is deliberately **not** `Android/data`: Android 11+ hides that from
  MTP, so a font copied there over a cable silently never arrives. Both it and a
  *"App media (USB)"* root are offered in the picker.

**Checked:** all seven new strings (`/system/fonts` … `/product/fonts`) are present in the `.so`
shipped inside the released APK, so the shared-crate patch really is in the Android binary; the
desktop check stays green.

## 7. File ▸ Open did nothing — `src/file_picker.rs`

`rfd` (the desktop's dialog library) has no Android backend at all: its `backend.rs` compiles no
module for a target that is neither Linux/BSD, macOS, Windows nor wasm32, and the build fails with
`the trait bound FileDialog: FilePickerDialogImpl is not satisfied`.

So the port draws its own browser with egui, on the same seam, between frames. That browser is now
the **fallback** (see *2026-10-13 — Open and Save As use Android's own file picker* below); the
details it has:

- Sized from `ctx.content_rect()` (a portrait tablet is narrower than egui's desktop default), with
  enlarged button padding and item spacing for a finger.
- `ctx.request_repaint()` while it is open — the app only asks for a frame once a second while it
  waits, so a tap would otherwise lag.
- Roots are probed once per visit and cached, not once per frame; the listing is filtered in memory.
- A root that cannot be read stays **tappable and explains itself** (which permission, which Settings
  path) instead of sitting greyed out with a padlock — the likeliest real cause of "my files aren't
  listed" is that Android 10+ refuses the `read_dir` without *All files access*.
- The filter is the union of every extension the app asked for, with **"Show all files"** and a count
  of what the filter is hiding, so a filtered-out file is never indistinguishable from a missing one.
- Saving returns `dir.join(name)`: the app's atomic write needs the parent directory to exist.

## 8. Storage permissions — `Cargo.toml`

Declared in `[package.metadata.android]`, because without them shared storage shows as unreadable no
matter what the picker does: `MANAGE_EXTERNAL_STORAGE` (granted by hand in Settings ▸ Special access;
Google Play restricts it to file-manager-class apps, which does not apply to a sideloaded build),
`READ_MEDIA_IMAGES` (Android 13+), and `READ_EXTERNAL_STORAGE` / `WRITE_EXTERNAL_STORAGE` capped with
`max_sdk_version` for older releases.

## 9. HEIC opening was missing by default

The app crate declares `heif = ["photocraft-codecs/heif"]` with a comment that official builds pass
`--features heif`. A port that omits it ships an app that cannot open a phone's default photo format,
so the port crate makes it a `default` feature instead of patching upstream.

## 10. Icon and label

cargo-apk ships neither unless asked. `res/mipmap-*/ic_launcher.png` (48/72/96/144/192) plus
`res/mipmap-anydpi-v26/ic_launcher.xml` for the adaptive icon, whose foreground is sized to the
visible 66dp safe zone rather than the 108dp canvas, and `application.label = "PhotoCraft"` so the
launcher shows a name.

## Upstream files this port changes

| File | Change | Why |
|---|---|---|
| `crates/ui-egui/Cargo.toml` | `eframe` declared twice, `accesskit` gated to non-Android | the feature combination is a compile error on the NativeActivity backend (§2) |
| `crates/text/src/fonts.rs` | Android font directories + `set_extra_font_dirs` | the Linux branch finds nothing on a tablet, and the app had no way to add a font (§6) |

Everything else lives in `apps/photocraft-android/` and touches no upstream file.

## 2026-10-13 — A save no longer blocks the event loop (the "Force close" ANR)

**The complaint:** saving or exporting a large document made Android offer *Force close* while the
work carried on and finished anyway — the watchdog's verdict on a UI thread that has stopped
draining input for ~5 seconds.

**Why:** PhotoCraft's long operations have a background-job path (`jobs_ui` with progress and Cancel),
and the desktop turns it on while the default is **off** ("tests and scripts rely on" the inline
path). The port left it off, so every long command and every document write ran inline on the event
loop, and `write_document` re-ran `export` + `write` there instead of on a worker.

**The change:** `app.background_jobs = true`, plus the worker-thread `save_file` service it requires
(`Services::save_file`). Both halves are needed: with the flag on but no `save_file`, `write_document`
still falls back to `export` + `write` on the loop.

**Checked:** the release build for this change is the one named `…-noanr.apk` in the porter's local
history; the shell compiles for `aarch64-linux-android` and the desktop check stays green.

## 2026-10-13 — The picker no longer tells a *granted* user to grant access

The empty-folder message in shared storage used to end with "Grant *All files access*…", and it was
shown to everyone, including users who had already granted it. The trigger had been
`/storage/emulated/0/Android/data` failing to list — **measured on the emulator: that folder stays
unlistable to the app on API 35 even with the grant**, so the tell was wrong and the advice was
insulting to the one user who had done what it asked.

Now the note is conditional on the folder being shared storage, is worded as a possibility ("If your
files are here, the app may lack shared-storage access…"), and leaves the decision to the user,
because the grant cannot be probed from inside the app.

The USB tip became folder-dependent for the same reason — Android 11+ hides `Android/data` from MTP,
so a file copied there over a cable never arrives and the folder merely looks empty (which reads as a
picker bug), while `Android/media` *is* reachable over a cable. Saying the wrong one of those two is
worse than saying nothing.

## 2026-10-13 — Open and Save As use Android's own file picker — `src/activity.rs`

**The complaint:** the in-app browser misses files. That is not a bug in it, it is what a `read_dir`
can see: no cloud provider, no the Downloads collection as Android sees it, no card reader, no other
mounted volume, and on Android 10+ nothing outside the app's own directory unless *All files access*
was granted by hand — and even with it, `/storage/emulated/0/Android/data` stays unlistable to the app
(measured on the emulator, above).

**Why it was not done before:** the Storage Access Framework answers through `onActivityResult`, which
is a Java callback, and `android-activity` 0.6 exposes neither `startActivityForResult` nor a result
callback. This port's earlier note called that a wall. It is not — but the way through is not the one
first assumed, and getting it wrong is worth writing down.

**The wrong way, and what it looked like.** The first attempt exported
`ANativeActivity_onActivityResult` from the `.so`, reasoning that Java's
`android.app.NativeActivity.onActivityResult` is a `native` method the VM resolves in the app's own
libraries — the way it resolves `ANativeActivity_onCreate`, which `android-activity` exports. The
picker opened, the user picked a file, DocumentsUI closed, and the app sat there saying *Waiting for
Android's file picker…* for ever. The reason is in AOSP's `NativeActivity.java`: **there is no
`onActivityResult`**. The only natives that class declares are `loadNativeCode`, `onStartNative`,
`onResumeNative`, `onSaveInstanceStateNative`, `onPauseNative`, `onStopNative`,
`onConfigurationChangedNative`, `onLowMemoryNative`, `onWindowFocusChangedNative`, the surface
callbacks, the input-queue callbacks, `onContentRectChangedNative`, `onTrimMemoryNative` and
`onWindowInsetsChangedNative`. Nothing for a result, so nothing is ever resolved, and the
`ANativeActivity_onActivityResult` that the NDK's C glue defines is dead code. A native-only APK can
start an activity for a result and is never told the answer.

**The way that works.** `java/ai/storyteller/photocraft/PickerActivity.java` — a `NativeActivity`
subclass of about fifty lines whose whole job is to call three `native` methods in
`libphotocraft.so`: `onPickerResult`, `onPickerPause`, `onPickerResume`. The manifest names it
instead of `android.app.NativeActivity` and sets `android:hasCode="true"`, because without a
class loader the class does not exist at runtime. The APK therefore carries a 1 240-byte
`classes.dex` with exactly one class, compiled by `scripts/add-picker-activity.sh` — `cargo apk build`
packages a native library and nothing else, so that script runs after it: `javac` + `d8`, `aapt add`
for the dex, then the same `zipalign` + `apksigner` pair cargo-apk runs, because adding an entry
moves everything after it and invalidates the signature.

That single class also pays for itself twice over, see the next entry.

**And the class loader has to be told, which cost another round.** The first version of this class
crashed the app on launch with `java.lang.UnsatisfiedLinkError: No implementation found for void
ai.storyteller.photocraft.PickerActivity.onPickerResume()`. The symbols were exported — `llvm-nm -D`
showed all three — so the VM was looking somewhere else, and it is somewhere specific:
`NativeActivity.onCreate` loads the library with `System.load(path)`, and `System.load` registers a
native library against the class loader of the class that called it, which for a framework class is
the **boot** loader. The VM resolves a native method against the loader of the class that *declares*
it; `PickerActivity` comes from the app's own `classes.dex`, so it looked in the app's loader, found
the library missing there and gave up. The fix is one line — `System.loadLibrary("photocraft")` in a
static initialiser, which registers the same library with the app's loader too (the file is
`dlopen`ed once either way) and runs before the system constructs the Activity.

Two host tests now guard exactly this, because nothing else can: `cargo test -p photocraft-android`
parses `PickerActivity.java` and checks that every `native` method it declares has a matching
`Java_ai_storyteller_photocraft_PickerActivity_*` symbol in this crate, and that the class loads the
library itself. The wrong wiring compiles, links, signs and installs perfectly — the only thing that
ever notices is the device.

## 2026-10-13 — The picked file could not be turned back into a `Uri`

**The complaint:** the picker opened, the image was chosen, DocumentsUI closed — and the app said
*Android's file picker did not work (Method not found: `<init>` (Ljava/lang/String;)V)* and fell back
to the in-app browser.

**Why:** `android.net.Uri` has **no public one-argument constructor**. Its constructors are
`Uri(ContentResolver, String)`, `Uri(String scheme, String ssp, String fragment)` and a private
`Uri(String uriString)`; a URI string goes in through the static `Uri.parse(String)`. Asking for the
one that is not there throws `NoSuchMethodError: Method not found: <init> (Ljava/lang/String;)V` at
runtime. This crate built its `Uri` objects with `new_object(env, "android/net/Uri", "(Ljava/lang/String;)V", …)`
in four places — take the persistable grant, read the chosen bytes, write the staged bytes, ask the
provider for the file's name — so the result arrived and then died on the first conversion.

**The fix:** one helper, [`uri_of`], is now the only way this crate makes a `Uri`, and it calls
`Uri.parse`. A third host test keeps it that way: the source must not contain
`new_object(env, "android/net/Uri"`. It was confirmed to fail when that call is put back.

**The pattern worth naming:** four of this port's fixes were framework APIs that do not exist in the
form they were assumed to take — a native method the VM cannot resolve, a native method no class
loader claims, a private constructor, a method whose return type in the signature was wrong. All
four compile, link, sign and install perfectly and are invisible until the device runs them. What
there is instead of a device test: the framework source quoted in each section, four host tests over
the Java/native wiring, and `photocraft.log`, which now records which of them happened.

[`uri_of`]: ../apps/photocraft-android/src/activity.rs

## 2026-10-14 — PhotoCraft is offered in *Open with*

**The ask:** pick a file in the Files app or the gallery, choose PhotoCraft, and have it open —
rather than only being able to reach files through its own picker.

**The manifest.** An `<intent-filter>` with `ACTION_VIEW`, the `DEFAULT` category (without which an
Activity is in no *Open with* list at all) and `BROWSABLE`, over the `content` and `file` schemes.
Two filters rather than one, because the two ways a file manager can name a file do not agree:

- by **MIME type** — `image/*` covers jpeg, png, webp, gif, tiff, heic and the camera raws, plus the
  four spellings providers use for a PSD (`image/vnd.adobe.photoshop`, `application/x-photoshop`,
  `application/vnd.adobe.photoshop`, `image/x-psd`);
- by **extension** — `.*\.(psd|psb)`, for a PSD a provider reports as `application/octet-stream`,
  which is what it does for an extension it has never heard of. Without this, exactly the file the
  user cares about would be the one that does not list the app.

They cannot be one filter: a filter that names both a MIME type and a path pattern needs *both* to
match, so `image/*` plus a path pattern would only ever accept images *and* .psd files. cargo-apk
adds its own `MAIN`/`LAUNCHER` filter beside them, because neither of these is `MAIN`.

**Receiving it.** `NativeActivity` has no `onNewIntent` bridge either, and the launch intent is
`getIntent()` — so `PickerActivity` forwards both: `onCreate` offers what it was launched with, and
`onNewIntent` offers what a second file brought. With `launchMode="singleTask"` a second file arrives
at the *running* Activity rather than starting a second one, which is the same reason the overlapping
windows are gone.

**Opening it.** A `content://` URI is not a path, so the bytes go through the same read the in-app
system picker uses — on a worker, because a large PSD on the UI thread is what earns Android's
"Force close" — and are then handed to `PhotocraftApp::open_bytes`. That is the same answer the picker
gives, so a file opened from the gallery behaves exactly like one picked from inside the app: no path
of its own, and *Save* asks where to write. No permission is needed; the URI grant comes from whoever
sent the intent.

**Checked:** the manifest in the built APK, `dexdump` on the shipped `classes.dex` (`onOpenWith`,
`onNewIntent`, `onCreate` alongside the picker callbacks) and `llvm-nm -D` on the `.so` (the
`Java_…_PickerActivity_onOpenWith` symbol, among the four). **Not checked:** that a file manager on
the tablet actually offers the app — there is no device in this loop.

## 2026-10-14 — A staged save is written through a file descriptor

**The complaint:** opening works, saving does not. `Method not found: write ([B)`, then
*Could not write content://com.android.providers.downloads.documents/document/1712* — for a PSD and
for an exported image alike, and the picker itself is now working correctly, which is what made this
visible.

**Why:** the bytes got all the way to the chosen document and then failed on the way in.
`ContentResolver.openOutputStream(uri, "wt")` returns a
`ParcelFileDescriptor.AutoCloseOutputStream`, which overrides **only** `write(int)`. So
`write(byte[])` on it is `OutputStream`'s base-class implementation —

```java
    public void write(byte b[]) throws IOException {
        write(b, 0, b.length);
    }
    public void write(byte b[], int off, int len) throws IOException {
        for (int i = 0 ; i < len ; i++) {
            write(b[off + i]);          // one JNI call per byte
        }
    }
```

— a JNI call *per byte*, seventy million of them for one of these documents. And the signature this
crate asked for was `([B)I`, while `OutputStream.write(byte[])` returns **void**: JNI method lookup
includes the return type, so `([B)I` matches nothing and `GetMethodID` returns null. The tablet's
`Method not found: write ([B)` shows only the argument part of a signature — jni's message prints
`{name} {sig}` and the sig it holds renders without the return type — so the one character that
mattered was invisible in the error.

**The fix:** stop going through the stream. `ContentResolver.openFileDescriptor(uri, "wt")` →
`ParcelFileDescriptor.detachFd()` (which hands the raw descriptor over and leaves the Java object
with nothing to close) → an ordinary `std::io::copy` onto a `std::fs::File` this process owns. That
is a buffered write with no Java array the size of the document, no per-byte JNI, and real error
reporting: `write(2)` is where a full disk or a revoked grant comes back. Syncing is asked for and
not believed — some descriptors cannot `fsync`, and a save that reports failure after the provider
has the bytes is worse than one that trusts them.

A fourth host test keeps it that way: this crate must not ask a `ContentResolver` for an
`openOutputStream`. It was confirmed to fail when that call is put back.

**Read the other way round is worth saying:** opening is unchanged and works. `openInputStream` plus
`InputStream.read(byte[])`, which *is* declared on `InputStream` and does return `int`, is the right
shape for that direction, and it is what the tablet is now using to open a HEIF.

**How a request travels:** `file_picker::request` hands the request to `activity::launch`, which
builds the `Intent` on the Java main thread (that is where `startActivityForResult` has to be called,
and it is the call that can fail, so its failure is reported back as a fallback rather than a hang).
The result comes in on the Java main thread and the URIs go to a **worker**, because reading the bytes
of a large PSD on the UI thread is what earns Android's "Force close" dialog. The worker leaves the
answer in `activity::poll`, and `file_picker::draw` sends it through the same `FileDialogReply` the
old browser used — the app cannot tell which picker ran, and neither does `Services`.

**Open** is answered with `FileDialogAnswer::Contents` / `ContentsMany` — the bytes and the provider's
`_display_name`. That variant already existed for the web build and `open_bytes` already handles it, so
no temporary file is involved.

**Save** cannot be answered with a URI: PhotoCraft writes atomically (a temporary file beside the target
and a rename) to a *path*, and `content://` is not one. So the chosen URI is recorded against a private
staging file inside the app's own data directory, the app saves there as if it were the destination, and
the bytes are then streamed into the URI — on the worker for a document save (`save_file`), off the event
loop for anything else (`write`). The mapping is kept per staging *directory*, so it survives the app
rewriting the extension after the dialog answered (Save As ▸ PSD picks `.psd` whatever was typed), and
the staging directory is deleted once the bytes have arrived.

**Filter:** `ACTION_OPEN_DOCUMENT` is given `*/*` and, for File ▸ Open, no `EXTRA_MIME_TYPES` at all. A
filter built from the app's extension list hides exactly the files this picker is here to reach — an
extension with no MIME mapping, a camera RAW under a vendor type, a cloud provider's own. DocumentsUI
offers its own filter; ours could only take files away. A request that *does* carry an extension list
(a brush, a script, a preset) gets one, always including `image/*` and `application/octet-stream` so a
file whose provider reports nothing is still offered.

**Permissions:** the system picker needs none, which is most of the point. The declarations in
`Cargo.toml` are now there for the fallback browser only.

**If the system picker cannot start** (a device with no DocumentsUI to answer the intent), `activity::launch`
reports it, the note says so in plain words, and the in-app browser takes over. Both draw through the same
state and answer through the same reply.

**Checked:** the name, staging-name and MIME-type logic is platform-neutral, so it is covered by four
host unit tests (`cargo test -p photocraft-android`, 9/9 in total). The JNI compiles for
`aarch64-linux-android`, and on the artifact itself `dexdump` shows one class,
`Lai/storyteller/photocraft/PickerActivity;` extending `Landroid/app/NativeActivity;` with the three
`PRIVATE STATIC NATIVE` methods, while `llvm-nm -D` on the `.so` inside the APK shows the matching
`Java_ai_storyteller_photocraft_PickerActivity_*` symbols. **What is not checked here is the round
trip on a device** — the intent, the result callback and the copy are all unexercised until the APK
is run on the tablet.

## 2026-10-13 — The app can no longer get stuck waiting for the picker

**The complaint, after the picker was first wired up:** the system picker opened, the user picked a
file, it closed, and the app stayed on *Waiting for Android's file picker…* with nothing left to do.
And the tab's close button did nothing.

Both are one bug. `PhotocraftApp` treats an open file dialog as a modal: `discard_ui::show` returns
early while `app.file_dialog_open()`, so the unsaved-changes prompt is never put up — and
`discard_ui::intercept` has already swallowed the command. Every command that needs that prompt is
therefore silently dropped while the dialog is open, which is *File ▸ Close*, the tab's ✕ on a
document with unsaved changes, and File ▸ Exit. Nothing is broken in that code; it is waiting for a
dialog that never answers.

**The fix, in two parts.**

- **The result arrives.** That is the `PickerActivity` above — the first version's symbol was never
  called, so no result ever arrived.
- **A lost result can no longer strand the app.** `PickerActivity` also reports pause and resume.
  Android delivers `onActivityResult` *before* `onResume`, so a resume with no result in hand means
  the answer was lost: `Java_…_onPickerResume` cancels the request and says so in the overlay and in
  `photocraft.log`. The pause has to have lasted more than `AWAY_FOR` (1.2 s) to count, so a
  momentary interruption while the picker is still up does not cancel a live request — and the one
  false positive left (the screen going off and on while the picker is open) costs a retry of File ▸
  Open rather than a stuck session. The `reported` flag is set the moment a result arrives, before
  the worker starts reading bytes, so a resume arriving a millisecond later cannot mistake "still
  copying" for "no answer".

**Checked:** it compiles for `aarch64-linux-android`, the two entry points are in the shipped `.so`,
and the state machine is the only new logic — the timings it compares are `Instant::elapsed` calls,
not something a test can pin down without a device.

## 2026-10-13 — One window: a second Activity no longer spawns a second one

**The complaint:** opening the app sometimes puts a second window on the device, behind the first one,
which cannot be used or closed, and which holds the input focus — so the window that *is* visible stops
responding to touches either.

**Why, from the code and winit's own source:** winit 0.30.13's Android backend has no window creation at
all — `ActiveEventLoop` on that target implements no `create_window`, so nothing in egui (or anything
else) can ask for a second OS window. egui also embeds extra viewports on Android already: `eframe`'s
`create_egui_context` calls `set_embed_viewports(!IS_DESKTOP)` and `IS_DESKTOP` is false there, so
Window ▸ Arrange ▸ New Window and the Camera Raw dialog draw inside the one window rather than opening
their own. So the second window is **not** an egui viewport. It is a second **Activity**: a second
`ANativeActivity` means `android-activity` runs `android_main` again, on a second thread, with a second
winit event loop and a second window in the same process.

The manifest's launch mode was cargo-apk's default, `standard`, which is exactly what produces that:
tapping the launcher icon (or being launched by anything else) while PhotoCraft is already running
creates a *new* Activity instance in the running task. Two windows, two event loops, one process — and
whichever one the system gave the input focus to is the one taps go to, whichever one is drawn on top
is the one that looks alive.

**The fix, in two parts:**

- `launch_mode = "singleTask"` in `[package.metadata.android.application]`. A second launch is now
  delivered to the running Activity as `onNewIntent` instead of starting a new one.
- A process-wide claim in `android_main`. If `android_main` runs while another window is already up —
  whatever the cause, launch mode included — the newcomer logs it, calls `Activity.finish()` through the
  same JNI bridge, and returns without starting a second event loop. The running window is the one worth
  keeping, and the duplicate closes itself instead of sitting there holding the focus.
- `cc.egui_ctx.set_embed_viewports(true)` is now stated explicitly in the creation closure. It is what
  `eframe` already does on Android; it is written down here so the invariant lives in this file.

**Checked:** it compiles for `aarch64-linux-android` and the launch mode is verifiable in the built
APK (`aapt2 dump badging` prints `launchMode='singleTask'`). **What is not checked here is the
duplicate itself**: reproducing it needs the tablet, and the fix is verified by reading the manifest and
by the claim in the log (`a second Activity arrived while one was already running`) if it is ever hit
again.

## What is deliberately *not* in the patch

`Cargo.lock` is excluded on purpose: it moves only because the port crate is a new workspace member,
and `cargo` regenerates it on the first build. The patch contains only changes that matter to a reader
— every tracked file the port touches, plus any new file that has to live inside an upstream directory.

## Not verified here

The APK attached to **v0.1.7** was built from exactly this tree (upstream pin + the patch + the port
crate) with `cargo apk build --release -p photocraft-android` and then `scripts/add-picker-activity.sh`,
NDK r27, and signed with a local test key. What was checked on the artifact: `aapt2 dump badging`
(package, `arm64-v8a`), `aapt2 dump xmltree` (`android:launchMode=2`, `android:hasCode=true`, the
Activity named `ai.storyteller.photocraft.PickerActivity`), `aapt list` (`classes.dex` present),
`apksigner verify --verbose` (v2 + v3), `dexdump -d` (the class, its `<clinit>` and its three
native methods) and `llvm-nm -D` on the `.so` inside the APK (the three matching `Java_…` symbols).
What was checked on the source: `cargo check -p photocraft-android --target aarch64-linux-android`,
`cargo check -p photocraft` on the host, and `cargo test -p photocraft-android` (12/12).

What that does **not** buy is a runtime claim. There is no device or emulator in this loop, so the
fixes above are verified by compiling, by their classes, symbols and manifest attributes being where
they should be, and by the reasoning in each section — not by watching them work. Three of them
(the picker answering at all, the app starting with a Java Activity, the duplicate Activity never
appearing) were each wrong once in exactly that way: compiling, signing and installing perfectly, and
failing on the device. The picker round trip in particular (does `onActivityResult` fire, do the bytes
land where the user chose) is unproven until the APK is installed on the tablet, and the two host tests
that guard the Java/native wiring are the closest thing to a check that exists without one. There is
no automated device test in this repository.
