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
`the trait bound FileDialog: FilePickerDialogImpl is not satisfied`. Android's own picker is not a
substitute: the Storage Access Framework needs `startActivityForResult`, which `android-activity`
0.6 does not expose.

So the picker is drawn with egui, on the same seam, between frames. The details that matter on a
tablet:

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
## What is deliberately *not* in the patch

`Cargo.lock` is excluded on purpose: it moves only because the port crate is a new workspace member,
and `cargo` regenerates it on the first build. The patch contains only changes that matter to a reader
— every tracked file the port touches, plus any new file that has to live inside an upstream directory.

## Not verified here

The APK attached to **v0.1.1** was built from exactly this tree on 2026-10-13 (upstream pin + the patch + the port crate), and the checks in the README's verification table were run against it. What that does *not* buy is a runtime claim: it was not launched on a device or emulator as part of publishing, so every behavioural statement below rests on the hand-testing the porter did on an arm64 tablet. The fixes above
were found and iterated on the porter's arm64 tablet, and the binary shipped in Releases is the one
that build produced; there is no automated device test in this repository.
