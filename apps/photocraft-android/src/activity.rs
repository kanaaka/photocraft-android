//! The Java side of the Activity, reached over JNI.
//!
//! # Why there is Java at all
//!
//! `android-activity` exposes neither `startActivityForResult` nor a result callback, and the
//! first version of this port reached for a shortcut: export `ANativeActivity_onActivityResult`
//! and hope the VM called it. It never does. `android.app.NativeActivity` declares no
//! `onActivityResult` — read its source: the only natives it declares are
//! `loadNativeCode`, `onStartNative`, `onResumeNative`, … — so there is nothing for the VM to
//! resolve, and the `ANativeActivity_onActivityResult` that the NDK's C glue defines is dead
//! code. A native-only APK can start an activity for a result and is never told the answer,
//! which is every Storage Access Framework picker.
//!
//! So the port now ships one Java class, `java/…/PickerActivity.java`: a `NativeActivity`
//! subclass of about forty lines whose whole job is to call three `native` methods in
//! `libphotocraft.so`. The manifest points at it, `scripts/build-apk.sh` compiles it into the
//! APK's `classes.dex` (one class, a few hundred bytes of dex), and the entry points are the
//! `#[unsafe(no_mangle)]` functions below. Everything else in this crate is Rust and JNI.
//!
//! # How a picker request travels
//!
//! [`launch`] is called on the frame that asks for a dialog and returns at once. It builds the
//! `Intent` on the Java main thread - `Activity.startActivityForResult` must be called there,
//! and that call is the one that can fail, so its failure comes back as a fallback rather than
//! a hang. The result arrives on the Java main thread in
//! `Java_…_PickerActivity_onPickerResult`, which hands the URIs to a worker thread: the bytes
//! of a large PSD must not be read on the UI thread, or Android offers "Force close". The
//! worker leaves the answer in [`poll`] and the frame loop sends it through the same
//! `FileDialogReply` the in-app browser uses.
//!
//! **The answer can never be lost.** `onPickerPause` and `onPickerResume` say when the picker
//! was in front of us. Android delivers `onActivityResult` *before* `onResume`, so a resume
//! with no result in hand means the answer was lost, and [`resume`] cancels the request rather
//! than leaving the app waiting for a reply that will never come. That is also what a stuck
//! dialog costs otherwise: `PhotocraftApp` suppresses its unsaved-changes prompt while
//! `file_dialog_open()`, so every command that needs it — including the tab's close button —
//! silently does nothing.
//!
//! # What a `content://` URI means for the rest of the app
//!
//! PhotoCraft works in filesystem paths: the write services take a `path`, and a document
//! remembers the path it was saved to. A `content://` URI is neither.
//!
//! - **Opening** is answered with [`FileDialogAnswer::Contents`] / `ContentsMany` - the bytes
//!   and the display name - which is what the web build answers with and what `open_bytes`
//!   already handles. No temporary file is involved.
//! - **Saving** cannot be answered with a URI. The chosen URI is recorded against a private
//!   staging file that the app writes to as if it were the destination, and [`save_target`]
//!   tells the write services where the bytes have to go afterwards.

use photocraft_ui_egui::{FileDialogAnswer, FileDialogRequest};

/// The Java `Activity` handle `android-activity` hands to `android_main`.
///
/// Only Android has one. Everywhere else this is `()` and every function below is a no-op,
/// which is what keeps `cargo test` on the host building at all.
#[cfg(target_os = "android")]
pub type AndroidActivity = winit::platform::android::activity::AndroidApp;

#[cfg(not(target_os = "android"))]
pub type AndroidActivity = ();

/// What the system picker is being asked for.
#[derive(Debug)]
pub enum Outcome {
    /// Still open, or busy reading the chosen files. What the user is told about it comes
    /// from [`overlay`], which reads the same state.
    Waiting,
    /// The user answered. `None` is a cancel, which is not an error.
    Answer(Option<FileDialogAnswer>),
    /// The picker could not be used. The caller falls back to the in-app browser.
    Failed(String),
}

/// Start the system file picker. Returns at once; the answer comes through [`poll`].
///
/// `Err` means the request could not even be started, so the caller should show its own
/// browser instead.
pub fn launch(request: &FileDialogRequest) -> Result<(), String> {
    imp::launch(request)
}

/// The answer so far, if anything is going on. See [`Outcome`].
pub fn poll() -> Option<Outcome> {
    imp::poll()
}

/// Whether the system picker can be used at all (always `false` off Android).
pub fn available() -> bool {
    imp::available()
}

/// Remember the Java `Activity` to work through. Called once, from `android_main`.
pub fn install(app: AndroidActivity) {
    imp::install(app)
}

/// Close this `Activity`, on the Java main thread.
///
/// Used to dismiss a duplicate instance: the running window is the one worth keeping.
pub fn finish() {
    imp::finish()
}

/// Copy a staged save to the `content://` URI it was chosen for, then forget it.
///
/// Call this from whatever thread wrote the file - a worker, where there is one - since a
/// large document must not be copied on the event loop.
pub fn copy_staged(path: &str) -> Result<(), String> {
    imp::copy_staged(path)
}

/// The URI `path` was chosen for, if it is a staged save.
///
/// Any write below a staged directory counts, so this survives the app rewriting the
/// extension after the dialog answered (Save As . PSD picks `.psd` whatever was typed).
pub fn save_target(path: &str) -> Option<String> {
    imp::save_target(path)
}

/// Files the user opened PhotoCraft with from a file manager or the gallery, whose bytes have
/// been read and are waiting for the frame loop to open them.
///
/// Taking them is what hands them to the app: [`photocraft_ui_egui::PhotocraftApp::open_bytes`]
/// wants bytes, not a path, because a `content://` URI is not one — the same answer the system
/// file picker gives, so a file opened from the gallery behaves like one picked from inside
/// the app (no path of its own, so *Save* asks where).
pub fn take_opened() -> Vec<(String, Vec<u8>)> {
    imp::take_opened()
}

/// Files opened with the app, for [`take_opened`], and anything that went wrong doing it.
pub fn overlay(ctx: &egui::Context) {
    imp::overlay(ctx)
}

#[cfg(target_os = "android")]
mod imp {
    use super::{
        AndroidActivity as AndroidApp, FileDialogAnswer, FileDialogRequest, Outcome, file_name, lowercase_extension, mime_for,
        mime_types, sanitise,
    };
    use jni::objects::{JObject, JObjectArray, JString, JValue};
    use jni::signature::RuntimeMethodSignature;
    use jni::strings::JNIString;
    use jni::vm::JavaVM;
    use jni::Env;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    /// `startActivityForResult` request codes. Anything else is not ours.
    const CODE_OPEN: jni::sys::jint = 0x5011;
    const CODE_SAVE: jni::sys::jint = 0x5012;

    // `Intent.FLAG_GRANT_*`: stable API 1 constants.
    const GRANT_READ: i32 = 0x0000_0001;
    const GRANT_WRITE: i32 = 0x0000_0002;
    const GRANT_PERSISTABLE: i32 = 0x0000_0040;

    /// `android.app.Activity.RESULT_OK`. Anything else (notably `RESULT_CANCELED`) is a cancel.
    const RESULT_OK: jni::sys::jint = -1;

    /// How long a finished transfer, or a failure, stays on screen.
    const NOTE_AGE: Duration = Duration::from_secs(6);

    /// A pause longer than this before a resume means the Activity really was in the
    /// background, not just briefly interrupted.
    const AWAY_FOR: Duration = Duration::from_millis(1200);

    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        Open,
        Save,
    }

    /// What the system picker is doing right now.
    struct Pending {
        /// What the user last saw, for the overlay.
        note: String,
        /// Set the moment Android reports a result, whatever that result turns out to be: a
        /// resume in the same moment must not mistake the worker still reading the bytes for a
        /// lost answer.
        reported: bool,
        /// When the Activity was last paused while this was pending.
        paused_at: Option<Instant>,
        /// The answer, once there is one.
        answer: Option<Result<Option<FileDialogAnswer>, String>>,
    }

    impl Pending {
        fn new(note: String) -> Self {
            Self { note, reported: false, paused_at: None, answer: None }
        }
    }

    /// A file on its way in or out, for the overlay.
    struct Transfer {
        note: String,
        started: Instant,
        error: Option<String>,
    }

    static APP: OnceLock<AndroidApp> = OnceLock::new();
    static PENDING: Mutex<Option<Pending>> = Mutex::new(None);
    static TRANSFERS: Mutex<Vec<Transfer>> = Mutex::new(Vec::new());
    /// URIs the user opened the app with, waiting for the frame loop to start reading them.
    /// Written from `onOpenWith`, which the JVM calls on the Java main thread — often before
    /// `android_main` has even reached `install`, which is why it is a queue of its own and not
    /// anything that needs the `AndroidApp` to already exist.
    static OPEN_REQUESTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    /// Files that have been read and are waiting for the frame loop to open them.
    static OPENED: Mutex<Vec<(String, Vec<u8>)>> = Mutex::new(Vec::new());
    /// Staging directory -> the `content://` URI the user chose for it. `None` until `install`.
    static SAVED: Mutex<Option<HashMap<PathBuf, String>>> = Mutex::new(None);
    /// The staging root, inside the app's own data directory.
    static STAGE: OnceLock<PathBuf> = OnceLock::new();

    pub fn install(app: AndroidApp) {
        let stage = app.internal_data_path().map(|d| d.join("SAF"));
        let _ = APP.set(app);
        // The staging root needs no permission: it is the app's own directory.
        if let Some(dir) = stage {
            let _ = std::fs::create_dir_all(&dir);
            clear_stale(&dir);
            let _ = STAGE.set(dir);
            *SAVED.lock().unwrap() = Some(HashMap::new());
        }
    }

    pub fn available() -> bool {
        APP.get().is_some() && STAGE.get().is_some()
    }

    pub fn launch(request: &FileDialogRequest) -> Result<(), String> {
        let app = APP.get().ok_or("no Android Activity")?.clone();
        let (kind, title, extensions, multiple) = match request {
            FileDialogRequest::Open { multiple, extensions, .. } => (Kind::Open, String::new(), extensions.clone(), *multiple),
            FileDialogRequest::Save { suggested } => (Kind::Save, file_name(suggested), None, false),
        };
        {
            let mut slot = PENDING.lock().unwrap();
            if slot.is_some() {
                return Err("a file picker is already open".into());
            }
            *slot = Some(Pending::new("Waiting for Android's file picker...".to_string()));
        }
        let owned = app.clone();
        app.run_on_java_main_thread(Box::new(move || {
            if let Err(e) = start(&owned, kind, &title, extensions.as_deref(), multiple) {
                log::error!("the system file picker would not open: {e}");
                deliver(Err(e));
            }
        }));
        Ok(())
    }

    /// Build the intent and start the picker, on the Java main thread.
    fn start(app: &AndroidApp, kind: Kind, title: &str, extensions: Option<&[String]>, multiple: bool) -> Result<(), String> {
        let save = kind == Kind::Save;
        with_env(app, |env| {
            let activity = activity(env, app)?;
            let action = if save { "android.intent.action.CREATE_DOCUMENT" } else { "android.intent.action.OPEN_DOCUMENT" };
            let action = JString::from_str(env, action)?;
            let intent = new_object(env, "android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)])?;
            // Both actions want OPENABLE: without it the picker may hand back a directory.
            let openable = JString::from_str(env, "android.intent.category.OPENABLE")?;
            intent_call(
                env,
                &intent,
                "addCategory",
                "(Ljava/lang/String;)Landroid/content/Intent;",
                &[JValue::Object(&openable)],
            )?;
            if save {
                let mime = JString::from_str(env, mime_for(title))?;
                intent_call(
                    env,
                    &intent,
                    "setType",
                    "(Ljava/lang/String;)Landroid/content/Intent;",
                    &[JValue::Object(&mime)],
                )?;
                let key = JString::from_str(env, "android.intent.extra.TITLE")?;
                let title = JString::from_str(env, title)?;
                intent_call(
                    env,
                    &intent,
                    "putExtra",
                    "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
                    &[JValue::Object(&key), JValue::Object(&title)],
                )?;
            } else {
                // "*/*", deliberately. A filter built from the app's own list would hide
                // exactly the files this picker is here to reach: an extension with no MIME
                // mapping, a camera RAW under a vendor type, a cloud provider's own.
                // DocumentsUI offers its own filter; ours could only take files away. A
                // request that *does* carry an extension list (brushes, scripts, presets)
                // gets one.
                let all = JString::from_str(env, "*/*")?;
                intent_call(
                    env,
                    &intent,
                    "setType",
                    "(Ljava/lang/String;)Landroid/content/Intent;",
                    &[JValue::Object(&all)],
                )?;
                if let Some(exts) = extensions.filter(|e| !e.is_empty()) {
                    let key = JString::from_str(env, "android.intent.extra.MIME_TYPES")?;
                    let mimes = string_array(env, &mime_types(exts))?;
                    intent_call(
                        env,
                        &intent,
                        "putExtra",
                        "(Ljava/lang/String;[Ljava/lang/String;)Landroid/content/Intent;",
                        &[JValue::Object(&key), JValue::Object(&mimes)],
                    )?;
                }
                if multiple {
                    let key = JString::from_str(env, "android.intent.extra.ALLOW_MULTIPLE")?;
                    intent_call(
                        env,
                        &intent,
                        "putExtra",
                        "(Ljava/lang/String;Z)Landroid/content/Intent;",
                        &[JValue::Object(&key), JValue::Bool(true)],
                    )?;
                }
            }
            let mut flags = GRANT_READ | GRANT_PERSISTABLE;
            if save {
                flags |= GRANT_WRITE;
            }
            intent_call(env, &intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(flags)])?;
            let code = if save { CODE_SAVE } else { CODE_OPEN };
            call_void(
                env,
                &activity,
                "startActivityForResult",
                "(Landroid/content/Intent;I)V",
                &[JValue::Object(&intent), JValue::Int(code)],
            )?;
            Ok(())
        })
    }

    /// Close this Activity, on the Java main thread.
    pub fn finish() {
        let Some(app) = APP.get().cloned() else { return };
        let owned = app.clone();
        app.run_on_java_main_thread(Box::new(move || {
            let _ = with_env(&owned, |env| {
                let activity = activity(env, &owned)?;
                call_void(env, &activity, "finish", "()V", &[])
            });
        }));
    }

    pub fn poll() -> Option<Outcome> {
        let mut slot = PENDING.lock().unwrap();
        let answer = slot.as_mut()?.answer.take();
        match answer {
            // Only a request that never got off the ground falls back; a file that could not
            // be read is reported to the user instead.
            Some(answer) => {
                *slot = None;
                Some(match answer {
                    Ok(answer) => Outcome::Answer(answer),
                    Err(e) => Outcome::Failed(e),
                })
            }
            None => Some(Outcome::Waiting),
        }
    }

    /// `PickerActivity.onPickerResult(int, int, Intent)` - the result of
    /// `startActivityForResult`, on the Java main thread.
    ///
    /// The Java class is the only reason this can work at all; see the module docs.
    ///
    /// # Safety
    ///
    /// Called by the JVM: `env` its JNI environment, `class` the `PickerActivity` class, and
    /// `data` the `Intent` the picker answered with, or null.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_ai_storyteller_photocraft_PickerActivity_onPickerResult(
        _env: *mut jni::sys::JNIEnv,
        _class: *mut jni::sys::jclass,
        request_code: jni::sys::jint,
        result_code: jni::sys::jint,
        data: jni::sys::jobject,
    ) {
        // A panic must not unwind into the VM.
        let _ = std::panic::catch_unwind(|| {
            let Some(app) = APP.get().cloned() else { return };
            let kind = match request_code {
                CODE_OPEN => Kind::Open,
                CODE_SAVE => Kind::Save,
                // Another activity's result, not ours to answer.
                _ => return,
            };
            // Marked before anything else: the resume that follows must not read a worker
            // still copying the bytes as an answer that never arrived.
            if let Some(p) = PENDING.lock().unwrap().as_mut() {
                p.reported = true;
            }
            // The user backed out. Not an error: the app reads a cancel as a cancel.
            if result_code != RESULT_OK || data.is_null() {
                deliver(Ok(None));
                return;
            }
            match chosen_uris(&app, data, kind) {
                Err(e) => deliver(Err(e)),
                // A picker that answered without a file answered "no".
                Ok(uris) if uris.is_empty() => deliver(Ok(None)),
                Ok(uris) => match kind {
                    Kind::Save => save_answer(&app, &uris[0]),
                    Kind::Open => open_answer(&app, uris),
                },
            }
        });
    }

    /// `PickerActivity.onPickerPause()` - this Activity is no longer in front, so a picker is
    /// probably up.
    ///
    /// # Safety
    ///
    /// Called by the JVM with the JNI environment and the `PickerActivity` class.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_ai_storyteller_photocraft_PickerActivity_onPickerPause(
        _env: *mut jni::sys::JNIEnv,
        _class: *mut jni::sys::jclass,
    ) {
        let _ = std::panic::catch_unwind(|| {
            if let Some(p) = PENDING.lock().unwrap().as_mut() {
                p.paused_at.get_or_insert_with(Instant::now);
            }
        });
    }

    /// `PickerActivity.onPickerResume()` - this Activity is in front again.
    ///
    /// Android delivers `onActivityResult` before `onResume`, so coming back with no result in
    /// hand means the answer was lost. Cancelling here is what stops a lost answer from
    /// leaving the app stuck: `PhotocraftApp` treats an open file dialog as "a modal is up"
    /// and suppresses its unsaved-changes prompt, so while it waits nothing that needs that
    /// prompt works - not the tab's close button, not File . Close, not anything else.
    ///
    /// # Safety
    ///
    /// Called by the JVM with the JNI environment and the `PickerActivity` class.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_ai_storyteller_photocraft_PickerActivity_onPickerResume(
        _env: *mut jni::sys::JNIEnv,
        _class: *mut jni::sys::jclass,
    ) {
        let _ = std::panic::catch_unwind(|| {
            let lost = {
                let mut slot = PENDING.lock().unwrap();
                let Some(p) = slot.as_mut() else { return };
                let lost = !p.reported && p.paused_at.is_some_and(|t| t.elapsed() > AWAY_FOR);
                p.paused_at = None;
                if lost && p.answer.is_none() {
                    p.answer = Some(Ok(None));
                }
                lost
            };
            if lost {
                log::error!("the file picker went away without an answer; treating it as a cancel");
                note("Android did not report the file you picked".to_string(), None);
            }
        });
    }

    /// Read the chosen files on a worker: the bytes of a large document must not be read on
    /// the Java main thread, or Android offers "Force close".
    fn open_answer(app: &AndroidApp, uris: Vec<String>) {
        note(format!("Reading {} file(s)...", uris.len()), None);
        let owned = app.clone();
        std::thread::spawn(move || {
            let read = with_env(&owned, |env| {
                let activity = activity(env, &owned)?;
                let resolver = call(env, &activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
                let mut files = Vec::with_capacity(uris.len());
                for chosen in &uris {
                    files.push(read_document(env, &resolver, chosen)?);
                }
                Ok(files)
            });
            match read {
                Ok(mut files) => {
                    clear_notes();
                    // `open_bytes` takes one file's name and bytes; a multiple pick takes a
                    // list, which `ContentsMany` is.
                    let answer = if files.len() == 1 {
                        let (name, bytes) = files.swap_remove(0);
                        FileDialogAnswer::Contents(name, bytes)
                    } else {
                        FileDialogAnswer::ContentsMany(files)
                    };
                    deliver(Ok(Some(answer)));
                }
                Err(e) => {
                    log::error!("could not read the chosen files: {e}");
                    note("Could not open the chosen file(s)".to_string(), Some(e.to_string()));
                    deliver(Err(format!("Android could not open the chosen file(s): {e}")));
                }
            }
        });
    }

    /// `PickerActivity.onOpenWith(String)` — a file the user opened the app with.
    ///
    /// # Safety
    ///
    /// Called by the JVM with the JNI environment, the `PickerActivity` class and the URI string.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_ai_storyteller_photocraft_PickerActivity_onOpenWith(
        _env: *mut jni::sys::JNIEnv,
        _class: *mut jni::sys::jclass,
        uri: jni::sys::jstring,
    ) {
        let _ = std::panic::catch_unwind(|| {
            if uri.is_null() {
                return;
            }
            // This runs on the Java main thread, and the singleton is already set:
            // `NativeActivity.onCreate` called `loadNativeCode` - this library - before it got
            // here. So the URI can be read now, even though `android_main` (which is what sets
            // `APP`) is only just being spawned on its own thread.
            let Ok(vm) = JavaVM::singleton() else { return };
            let text = vm.attach_current_thread(|env| {
                // SAFETY: the JVM's own local reference for this call; `cast_local` takes it over,
                // so it is released once and only once.
                let value = unsafe { JObject::from_raw(env, uri as jni::sys::jobject) };
                let value: JString = env.cast_local::<JString>(value)?;
                value.try_to_string(env)
            });
            if let Ok(text) = text {
                log::info!("opened with {text}");
                start_open(&text);
            }
        });
    }

    /// Begin reading a file the app was opened with.
    ///
    /// Called on the Java main thread (from `onCreate` or `onNewIntent`), which is no place to
    /// read a 70 MB PSD: the frame loop takes the queue instead, and reads on a worker.
    fn start_open(uri: &str) {
        let mut queue = OPEN_REQUESTS.lock().unwrap();
        // The launch intent is offered again on every resume of a paused-then-restored Activity, and
        // the same file twice would open two copies of it.
        if !queue.iter().any(|queued| queued == uri) {
            queue.push(uri.to_string());
        }
    }

    /// Read the queued files and remember them for the frame loop, on a worker.
    ///
    /// Called from the frame loop, which means the `AndroidApp` exists by now.
    fn drain_open_requests() {
        let Some(app) = APP.get().cloned() else { return };
        for uri in OPEN_REQUESTS.lock().unwrap().drain(..) {
            note(format!("Opening {uri}..."), None);
            let owned = app.clone();
            std::thread::spawn(move || {
                let read = with_env(&owned, |env| {
                    let activity = activity(env, &owned)?;
                    let resolver = call(env, &activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
                    read_document(env, &resolver, &uri)
                });
                match read {
                    Ok(file) => {
                        clear_notes();
                        OPENED.lock().unwrap().push(file);
                    }
                    Err(e) => {
                        log::error!("could not open {uri}: {e}");
                        note(format!("Could not open {}", file_name(&uri)), Some(e.clone()));
                    }
                }
            });
        }
    }

    pub fn take_opened() -> Vec<(String, Vec<u8>)> {
        drain_open_requests();
        std::mem::take(&mut *OPENED.lock().unwrap())
    }

    /// One document: its provider's name for it, and its bytes.
    fn read_document(env: &mut Env, resolver: &JObject, uri: &str) -> jni::errors::Result<(String, Vec<u8>)> {
        let parsed = uri_of(env, uri)?;
        let name = display_name(env, resolver, &parsed).unwrap_or_else(|| file_name(uri));
        let stream = call(env, resolver, "openInputStream", "(Landroid/net/Uri;)Ljava/io/InputStream;", &[JValue::Object(&parsed)])?.l()?;
        if stream.is_null() {
            return Err(jni::errors::Error::NullPtr("the provider returned no stream"));
        }
        let bytes = read_stream(env, &stream)?;
        let _ = call_void(env, &stream, "close", "()V", &[]);
        Ok((name, bytes))
    }

    /// A chosen place to save: a staging file inside the app, mapped to the URI.
    fn save_answer(app: &AndroidApp, uri: &str) {
        // The URI of a created document is `…/document/primary%3ADownload%2Fname.psd`: the last
        // segment is percent-encoded and carries the folder too, so ask the provider for the name
        // the user actually typed before falling back to it.
let name = with_env(app, |env| {
            let parsed = uri_of(env, uri)?;
            let activity = activity(env, app)?;
            let resolver = call(env, &activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
            Ok(display_name(env, &resolver, &parsed))
        });
        let name = name.ok().flatten().filter(|n| !n.is_empty()).unwrap_or_else(|| file_name(uri));
        match stage_save_target(uri, &name) {
            Ok(path) => deliver(Ok(Some(FileDialogAnswer::SaveTo(path)))),
            Err(e) => deliver(Err(e)),
        }
    }

    /// A staging file for a save, mapped to `uri`.
    ///
    /// A private file rather than the URI itself because the app writes atomically - a
    /// temporary file beside the target and a rename - which a `content://` URI cannot do.
    fn stage_save_target(uri: &str, name: &str) -> Result<String, String> {
        let root = STAGE.get().ok_or("no staging directory")?;
        clear_stale(root);
        let dir = root.join(format!("save-{}", unique_suffix()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        // The app lowercases the extension of whatever path it is handed (Save As . PSD),
        // which would change the path the writer gets and miss the mapping; name the staging
        // file as it will be written.
        let path = dir.join(sanitise(&lowercase_extension(name)));
        let mut saved = SAVED.lock().unwrap();
        let map = saved.as_mut().ok_or("no staging directory")?;
        map.insert(dir, uri.to_string());
        Ok(path.to_string_lossy().into_owned())
    }

    /// The nearest staged directory above `path`, and the URI it was chosen for.
    pub fn save_target(path: &str) -> Option<String> {
        let saved = SAVED.lock().unwrap();
        let map = saved.as_ref()?;
        let mut dir = Path::new(path).parent();
        while let Some(d) = dir {
            if let Some(uri) = map.get(d) {
                return Some(uri.clone());
            }
            dir = d.parent();
        }
        None
    }

    /// Copy a staged save to the location the user chose.
    ///
    /// **Through a file descriptor, not through an `OutputStream`.** The stream
    /// `ContentResolver.openOutputStream` hands back is a `ParcelFileDescriptor.AutoCloseOutputStream`,
    /// which overrides only `write(int)`: `write(byte[])` on it is `OutputStream`'s base-class
    /// loop that writes *one byte per JNI call*, which is 70 million calls for one of these
    /// documents. Asking for it also has to get the return type right — `OutputStream.write(byte[])`
    /// returns **void**, and a signature of `([B)I` does not match it, which is the
    /// `Method not found: write ([B)` this first did (jni's message prints only the argument part,
    /// so the return type is not visible in it). A descriptor sidesteps both, and gives buffered
    /// writes and real error reporting on close.
    pub fn copy_staged(path: &str) -> Result<(), String> {
        let Some(uri) = save_target(path) else { return Ok(()) };
        let app = APP.get().ok_or("no Android Activity")?.clone();
        note(format!("Writing {uri}..."), None);
        let written: Result<(), String> = (|| {
            let mut destination = writable_file(&app, &uri)?;
            let mut source = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
            std::io::copy(&mut source, &mut destination).map_err(|e| format!("{uri}: {e}"))?;
            // `write(2)` on the descriptor is where a full disk or a revoked grant surfaces, and
            // `std::io::copy` hands those back. Syncing is the provider's business and some
            // descriptors cannot do it at all, so it is asked for and not believed.
            if let Err(e) = destination.sync_data() {
                log::warn!("could not sync {uri} after writing it: {e}");
            }
            Ok(())
        })();
        match written {
            Ok(()) => {
                forget(&uri);
                note(format!("Saved to {uri}"), None);
                Ok(())
            }
            Err(e) => {
                log::error!("could not write {uri}: {e}");
                note(format!("Could not write {uri}"), Some(e.clone()));
                Err(e)
            }
        }
    }

    /// The chosen destination, as a file this process owns.
    ///
    /// `detachFd()` hands the raw descriptor over and leaves the Java object with nothing to close,
    /// so the `File` here is the only owner and closes it on drop.
    fn writable_file(app: &AndroidApp, uri: &str) -> Result<std::fs::File, String> {
        let fd = with_env(app, |env| {
            let parsed = uri_of(env, uri)?;
            let activity = activity(env, app)?;
            let resolver = call(env, &activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
            // "wt" truncates: re-saving a shorter document over a longer one must not leave the
            // tail of the previous file behind.
            let mode = JString::from_str(env, "wt")?;
            let descriptor = call(
                env,
                &resolver,
                "openFileDescriptor",
                "(Landroid/net/Uri;Ljava/lang/String;)Landroid/os/ParcelFileDescriptor;",
                &[JValue::Object(&parsed), JValue::Object(&mode)],
            )?
            .l()?;
            if descriptor.is_null() {
                return Err(jni::errors::Error::NullPtr("the provider returned no file descriptor"));
            }
            let raw = call(env, &descriptor, "detachFd", "()I", &[])?.i()?;
            let _ = call_void(env, &descriptor, "close", "()V", &[]);
            if raw < 0 {
                return Err(jni::errors::Error::NullPtr("the provider would not hand over its descriptor"));
            }
            Ok(raw)
        })?;
        use std::os::fd::FromRawFd;
        // SAFETY: `detachFd` transferred ownership of `fd` to this process, and the `ParcelFileDescriptor`
        // was closed with nothing left to release, so this `File` is the only owner.
        Ok(unsafe { std::fs::File::from_raw_fd(fd) })
    }

    /// The overlay: what the system picker is waiting for, what is being read or written,
    /// and anything that went wrong.
    pub fn overlay(ctx: &egui::Context) {
        let waiting = PENDING.lock().unwrap().as_ref().map(|p| p.note.clone());
        let now = Instant::now();
        let mut transfers = TRANSFERS.lock().unwrap();
        transfers.retain(|t| now.duration_since(t.started) < NOTE_AGE);
        if transfers.is_empty() && waiting.is_none() {
            return;
        }
        // The expiry is time-based, so nothing else would wake the loop to take it away.
        ctx.request_repaint();
        egui::Window::new("Android")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(0.0, 8.0))
            .show(ctx, |ui| {
                if let Some(note) = &waiting {
                    ui.label(egui::RichText::new(note).weak());
                    ui.separator();
                }
                for t in transfers.iter() {
                    let text = match &t.error {
                        Some(e) => egui::RichText::new(format!("{e}\n\n{}", t.note)).color(egui::Color32::from_rgb(230, 120, 120)),
                        None => egui::RichText::new(&t.note).weak(),
                    };
                    ui.label(text);
                }
            });
    }

    // --- state -------------------------------------------------------------------------

    fn note(text: String, error: Option<String>) {
        let mut transfers = TRANSFERS.lock().unwrap();
        transfers.retain(|t| t.note != text);
        transfers.insert(0, Transfer {
            note: text,
            started: Instant::now(),
            error,
        });
    }

    fn clear_notes() {
        TRANSFERS.lock().unwrap().clear();
    }

    /// The answer for the request in flight. Only the first one counts: a late cancel must
    /// not overwrite a result that already arrived.
    fn deliver(a: Result<Option<FileDialogAnswer>, String>) {
        let mut slot = PENDING.lock().unwrap();
        let Some(p) = slot.as_mut() else {
            // Nothing was waiting. The usual cause is a resume that gave up on a picker which
            // then answered anyway (see `Java_…_onPickerResume`).
            log::error!("a file picker answered, but nothing was waiting for it");
            return;
        };
        if p.answer.is_none() {
            p.answer = Some(a);
        }
    }

    /// Drop a staging directory once its bytes have reached the URI.
    fn forget(uri: &str) {
        let mut saved = SAVED.lock().unwrap();
        let Some(map) = saved.as_mut() else { return };
        let dirs: Vec<PathBuf> = map.iter().filter(|(_, u)| u.as_str() == uri).map(|(d, _)| d.clone()).collect();
        for dir in &dirs {
            map.remove(dir);
            // A staging directory belongs to one save, so it can go with it.
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// Anything left in the staging root is from a previous run: its URI grant died with
    /// that process.
    fn clear_stale(root: &Path) {
        let Ok(entries) = std::fs::read_dir(root) else { return };
        for e in entries.flatten() {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    }

    // --- JNI plumbing ------------------------------------------------------------------

    fn with_env<T>(app: &AndroidApp, f: impl FnOnce(&mut Env) -> jni::errors::Result<T>) -> Result<T, String> {
        // SAFETY: `vm_as_ptr` is this process's `JavaVM`, which outlives every attach.
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as _) };
        vm.attach_current_thread(f).map_err(|e| e.to_string())
    }

    /// The `Activity`, as a local reference of this frame.
    ///
    /// `activity_as_ptr` hands out android-activity's own reference, so it is only *borrowed*
    /// (`as_cast_raw`) and then turned into a new local one: android-activity keeps using its
    /// reference for the life of the `AndroidApp`, so it must not be deleted here.
    fn activity<'local>(env: &mut Env<'local>, app: &AndroidApp) -> jni::errors::Result<JObject<'local>> {
        let raw = app.activity_as_ptr() as jni::sys::jobject;
        // SAFETY: android-activity's reference, valid while `app` is.
        let borrowed = unsafe { env.as_cast_raw::<JObject>(&raw) }?;
        // A *new local* reference: this one is released with the frame, and android-activity's
        // own reference is left alone.
        env.new_local_ref(borrowed)
    }

    /// `Env::call_method` with the plain `&str` name and signature this file is written in.
    ///
    /// `jni` 0.22 wants a `JNIStr` and a parsed signature; every signature here is a literal,
    /// and [`tests::every_jni_signature_is_well_formed`] parses all of them, so an unparsable
    /// one would be a bug in this file rather than something a device could produce.
    fn call<'local>(
        env: &mut Env<'local>,
        obj: &JObject<'_>,
        name: &str,
        sig: &str,
        args: &[JValue<'_>],
    ) -> jni::errors::Result<jni::objects::JValueOwned<'local>> {
        let name = JNIString::new(name);
        let sig = RuntimeMethodSignature::from_str(sig).expect("a literal JNI method signature");
        env.call_method(obj, name, sig.method_signature(), args)
    }

    /// `Env::new_object`, in the same shape as [`call`].
    fn new_object<'local>(
        env: &mut Env<'local>,
        class: &str,
        sig: &str,
        args: &[JValue<'_>],
    ) -> jni::errors::Result<JObject<'local>> {
        let class = JNIString::new(class);
        let sig = RuntimeMethodSignature::from_str(sig).expect("a literal JNI method signature");
        env.new_object(class, sig.method_signature(), args)
    }

    /// [`call`] for a method that returns nothing.
    fn call_void(env: &mut Env<'_>, obj: &JObject<'_>, name: &str, sig: &str, args: &[JValue<'_>]) -> jni::errors::Result<()> {
        call(env, obj, name, sig, args)?;
        Ok(())
    }

    /// [`call`] for a static method.
    fn call_static<'local>(
        env: &mut Env<'local>,
        class: &str,
        name: &str,
        sig: &str,
        args: &[JValue<'_>],
    ) -> jni::errors::Result<jni::objects::JValueOwned<'local>> {
        let cls = env.find_class(JNIString::new(class))?;
        let name = JNIString::new(name);
        let sig = RuntimeMethodSignature::from_str(sig).expect("a literal JNI method signature");
        env.call_static_method(&cls, name, sig.method_signature(), args)
    }

    /// `android.net.Uri.parse(String)` — a `Uri` for a URI string.
    ///
    /// **Not** `new android.net.Uri(text)`: that constructor is *private*, and asking for it
    /// fails at runtime with `NoSuchMethodError: Method not found: <init> (Ljava/lang/String;)V`
    /// — which is what the first version of this did, on the device, after the picker had already
    /// returned a file. `Uri.parse` is the public way in, and the only one.
    fn uri_of<'local>(env: &mut Env<'local>, text: &str) -> jni::errors::Result<JObject<'local>> {
        let arg = JString::from_str(env, text)?;
        let uri = call_static(env, "android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&arg)])?.l()?;
        if uri.is_null() {
            return Err(jni::errors::Error::NullPtr("Uri.parse returned null"));
        }
        Ok(uri)
    }

    /// The URIs the picker answered with.
    ///
    /// `ACTION_OPEN_DOCUMENT` puts its one URI in `getData()`; a multiple pick puts them in
    /// the clip data. The read grant is made persistent where the provider allows it, or the
    /// next save of the same document would have nothing to write to.
    fn chosen_uris(app: &AndroidApp, data: jni::sys::jobject, kind: Kind) -> Result<Vec<String>, String> {
        with_env(app, |env| {
            // SAFETY: the JVM's own reference, valid for the length of this call. A local
            // reference to it is made so it can be released with this frame like any other.
            let borrowed = unsafe { env.as_cast_raw::<JObject>(&data) }?;
            let data = env.new_local_ref(borrowed)?;
            let mut uris: Vec<String> = Vec::new();
            let single = call(env, &data, "getData", "()Landroid/net/Uri;", &[])?.l()?;
            if !single.is_null() {
                if let Some(text) = to_string(env, &single) {
                    uris.push(text);
                }
            }
            let clip = call(env, &data, "getClipData", "()Landroid/content/ClipData;", &[])?.l()?;
            if !clip.is_null() {
                let count = call(env, &clip, "getItemCount", "()I", &[])?.i()?;
                for i in 0..count {
                    let item = call(env, &clip, "getItemAt", "(I)Landroid/content/ClipData$Item;", &[JValue::Int(i)])?.l()?;
                    let uri = call(env, &item, "getUri", "()Landroid/net/Uri;", &[])?.l()?;
                    if !uri.is_null() && let Some(text) = to_string(env, &uri) {
                        uris.push(text);
                    }
                }
            }
            let mut flags = GRANT_READ;
            if kind == Kind::Save {
                flags |= GRANT_WRITE;
            }
            let activity = activity(env, app)?;
            let resolver = call(env, &activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
            for text in &uris {
                let uri = uri_of(env, text)?;
                // Best effort: not every provider offers a persistable grant, and this throws
                // when it does not.
                let _ = call(
                    env,
                    &resolver,
                    "takePersistableUriPermission",
                    "(Landroid/net/Uri;I)V",
                    &[JValue::Object(&uri), JValue::Int(flags)],
                );
            }
            Ok(uris)
        })
    }

    /// `InputStream.read(byte[])`, which returns -1 at the end of the stream.
    fn read_stream(env: &mut Env, stream: &JObject) -> jni::errors::Result<Vec<u8>> {
        const CHUNK: usize = 64 * 1024;
        let mut out = Vec::new();
        let buf = vec![0u8; CHUNK];
        loop {
            let array = env.byte_array_from_slice(&buf)?;
            let read = call(env, stream, "read", "([B)I", &[JValue::Object(&array)])?.i()?;
            if read < 0 {
                return Ok(out);
            }
            if read as usize > CHUNK {
                return Err(jni::errors::Error::NullPtr("the provider returned an impossible read length"));
            }
            let got = env.convert_byte_array(&array)?;
            out.extend_from_slice(&got[..read as usize]);
        }
    }

    /// The `_display_name` a provider offers, or nothing.
    fn display_name<'local>(env: &mut Env<'local>, resolver: &JObject<'_>, uri: &JObject<'_>) -> Option<String> {
        const COLUMN: &str = "_display_name";
        let projection = string_array(env, &[COLUMN]).ok()?;
        let none = JObject::null();
        let cursor = call(
            env,
            resolver,
            "query",
            "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;",
            &[JValue::Object(uri), JValue::Object(&projection), JValue::Object(&none), JValue::Object(&none), JValue::Object(&none)],
        )
        .ok()?
        .l()
        .ok()?;
        if cursor.is_null() {
            return None;
        }
        let name = (|| {
            if !call(env, &cursor, "moveToFirst", "()Z", &[]).ok()?.z().ok()? {
                return None;
            }
            let column = JString::from_str(env, COLUMN).ok()?;
            let index = call(env, &cursor, "getColumnIndex", "(Ljava/lang/String;)I", &[JValue::Object(&column)]).ok()?.i().ok()?;
            if index < 0 {
                return None;
            }
            let value = call(env, &cursor, "getString", "(I)Ljava/lang/String;", &[JValue::Int(index)]).ok()?.l().ok()?;
            to_string(env, &value)
        })();
        let _ = call_void(env, &cursor, "close", "()V", &[]);
        name
    }

    /// `Object.toString()`, for a `Uri` or for a value out of a `Cursor`.
    fn to_string(env: &mut Env, obj: &JObject) -> Option<String> {
        if obj.is_null() {
            return None;
        }
        let value = call(env, obj, "toString", "()Ljava/lang/String;", &[]).ok()?.l().ok()?;
        let text: JString = env.cast_local::<JString>(value).ok()?;
        text.try_to_string(env).ok()
    }

    fn string_array<'local>(env: &mut Env<'local>, values: &[&str]) -> jni::errors::Result<JObjectArray<'local, JString<'local>>> {
        let element = JString::from_str(env, values.first().copied().unwrap_or(""))?;
        let array = JObjectArray::<JString>::new(env, values.len(), &element)?;
        for (i, v) in values.iter().enumerate() {
            let item = JString::from_str(env, v)?;
            array.set_element(env, i, &item)?;
        }
        Ok(array)
    }

    /// `addCategory`, `setType`, `putExtra`, `addFlags`: each returns the intent, unused.
    fn intent_call(env: &mut Env, intent: &JObject, name: &str, sig: &str, args: &[JValue]) -> jni::errors::Result<()> {
        call_void(env, intent, name, sig, args)?;
        Ok(())
    }
}

/// The file name part of a path, a URI or a title.
pub fn file_name(s: &str) -> String {
    let name = s.rsplit(['/', '\\']).next().unwrap_or(s);
    // A `content://` URI has no useful last segment; whatever ends it is the best guess.
    let name = name.split(['?', '#']).next().unwrap_or(name);
    if name.is_empty() { "file".to_string() } else { name.to_string() }
}

/// A name that is safe to create inside the app's own directory.
pub fn sanitise(name: &str) -> String {
    let name: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    let name = name.trim().trim_start_matches('.').to_string();
    if name.is_empty() { "untitled".to_string() } else { name }
}

/// The extension lowercased, because the app lowercases it anyway once it is handed a
/// path (`PhotocraftApp::lowercased_extension`) and a staging file has to keep the name it
/// will be written under.
pub fn lowercase_extension(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => format!("{stem}.{}", ext.to_ascii_lowercase()),
        _ => name.to_string(),
    }
}

/// The MIME type Android is told to save under.
pub fn mime_for(name: &str) -> &'static str {
    mime_for_ext(name.rsplit_once('.').map(|(_, e)| e).unwrap_or(name))
}

/// [`mime_for`] for an extension, with or without its dot. What `FileDialogRequest` carries
/// is a bare extension, and what a save title carries is a whole name.
pub fn mime_for_ext(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "psd" | "psb" => "image/vnd.adobe.photoshop",
        "tif" | "tiff" => "image/tiff",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "heic" | "heif" => "image/heic",
        "avif" => "image/avif",
        "txt" | "md" | "log" => "text/plain",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

/// MIME types for an extension filter, deduplicated, with a catch-all so that a file whose
/// provider reports nothing is still offered.
pub fn mime_types(exts: &[String]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = vec!["image/*", "application/octet-stream"];
    for e in exts {
        let m = mime_for_ext(e);
        if !out.contains(&m) {
            out.push(m);
        }
    }
    out
}

#[cfg(not(target_os = "android"))]
mod imp {
    use super::{AndroidActivity, FileDialogRequest, Outcome};

    pub fn install(_app: AndroidActivity) {}

    pub fn available() -> bool {
        false
    }

    pub fn launch(_request: &FileDialogRequest) -> Result<(), String> {
        Err("not Android".into())
    }

    pub fn poll() -> Option<Outcome> {
        None
    }

    pub fn overlay(_ctx: &egui::Context) {}

    pub fn take_opened() -> Vec<(String, Vec<u8>)> {
        Vec::new()
    }

    pub fn finish() {}

    pub fn copy_staged(_path: &str) -> Result<(), String> {
        Ok(())
    }

    pub fn save_target(_path: &str) -> Option<String> {
        None
    }
}

/// The name handling around a picker result: what Android is asked to save a file as, and
/// what a staged file is called so the writer's path still finds its URI afterwards.
///
/// The two `java_bridge_*` tests are the ones that would have caught the wiring mistake this
/// crate shipped with twice: the JNI entry points are resolved by name, by the VM, on a
/// device, and nothing about them fails to compile if they are wrong.
#[cfg(test)]
mod tests {
    use super::*;

    const JAVA: &str = include_str!("../java/ai/storyteller/photocraft/PickerActivity.java");
    const RUST: &str = include_str!("activity.rs");

    #[test]
    fn java_bridge_every_native_method_has_a_symbol() {
        let mut checked = 0;
        for line in JAVA.lines() {
            let Some(rest) = line.trim().strip_prefix("private static native void ") else { continue };
            let name = rest.split('(').next().unwrap_or_default();
            assert!(
                !name.is_empty() && RUST.contains(&format!("Java_ai_storyteller_photocraft_PickerActivity_{name}")),
                "PickerActivity.{name}() is declared native but this crate exports no matching symbol"
            );
            checked += 1;
        }
        assert!(checked >= 3, "no native methods found in PickerActivity.java - did the file move?");
    }

    #[test]
    fn java_bridge_the_class_loads_the_library_from_itself() {
        // NativeActivity.onCreate loads the library with System.load, which registers it
        // against the *boot* class loader; the VM resolves a native method against the loader of
        // the class that declares it, and this class is not on the boot classpath. Without this
        // block every native method throws UnsatisfiedLinkError on the device, with nothing to
        // fail at compile time.
        assert!(
            JAVA.contains("System.loadLibrary(\"photocraft\")"),
            "PickerActivity must load libphotocraft itself, or the VM cannot resolve its native methods"
        );
    }

    #[test]
    fn a_staged_save_is_written_through_a_file_descriptor() {
        // `ContentResolver.openOutputStream` returns a `ParcelFileDescriptor.AutoCloseOutputStream`,
        // which overrides only `write(int)`; `write(byte[])` on it is `OutputStream`'s base-class
        // loop that writes one byte per JNI call — 70 million of them for one of these documents.
        // (It also returns void, so the `([B)I` signature asked for here matched nothing at all and
        // the tablet said `Method not found: write ([B)`.) `writable_file` is how a save is written.
        // A real call passes the method name as a string literal, so that is what is looked for:
        // the name also appears in the comments explaining why the stream is not used.
        assert!(
            !RUST.contains("\"openOutputStream\""),
            "a staged save must go through openFileDescriptor, not an OutputStream's write(byte[])"
        );
        assert!(RUST.contains("\"openFileDescriptor\""), "how is a staged save written now?");
    }

    #[test]
    fn android_uris_are_parsed_not_constructed() {
        // `android.net.Uri(String)` is a *private* constructor. Using it compiles, links, signs
        // and installs, and then fails on the device with `NoSuchMethodError: Method not found:
        // <init> (Ljava/lang/String;)V` - after the picker has already handed over a file.
        // `Uri.parse` is the public way in, and `uri_of` is the only way this crate builds one.
        assert!(
            !RUST.contains("new_object(env, \"android/net/Uri\""),
            "android.net.Uri must be built with Uri.parse (uri_of), not with its private constructor"
        );
        assert!(RUST.contains("\"parse\""), "Uri.parse is how a Uri is built here; is uri_of still called?");
    }

    #[test]
    fn a_save_title_is_the_file_name_not_the_path() {
        assert_eq!(file_name("/storage/emulated/0/Documents/Water.PSD"), "Water.PSD");
        assert_eq!(file_name("untitled.psd"), "untitled.psd");
        // A provider URI has no name; the id at the end is all there is.
        assert_eq!(file_name("content://com.android.providers.media.documents/document/image%3A42"), "image%3A42");
    }

    #[test]
    fn a_staged_name_cannot_escape_its_own_directory() {
        assert_eq!(sanitise("../../evil.psd"), "_.._evil.psd");
        assert_eq!(sanitise("a/b\\c.psd"), "a_b_c.psd");
        assert_eq!(sanitise(""), "untitled");
    }

    #[test]
    fn the_staged_name_is_the_name_the_writer_will_use() {
        // `PhotocraftApp::lowercased_extension` rewrites the extension of whatever path it is
        // given; a staging file that kept the upper case would miss its URI afterwards.
        assert_eq!(lowercase_extension("Water.PSD"), "Water.psd");
        assert_eq!(lowercase_extension("a.b.TXT"), "a.b.txt");
        assert_eq!(lowercase_extension("noextension"), "noextension");
        // A leading dot is a stem, not an extension.
        assert_eq!(lowercase_extension(".hidden"), ".hidden");
    }

    #[test]
    fn a_filter_never_hides_a_file_the_picker_is_here_to_find() {
        // Whatever the extension list says, the catch-all and `image/*` stay in: a provider
        // that reports `application/octet-stream` for a RAW file must still be offered.
        let mimes = mime_types(&["psd".to_string(), "abr".to_string()]);
        assert!(mimes.contains(&"image/*"));
        assert!(mimes.contains(&"application/octet-stream"));
        assert!(mimes.contains(&"image/vnd.adobe.photoshop"));
        // ...and an extension nothing maps to still resolves to something showable.
        assert_eq!(mime_for("scratch.zzz"), "application/octet-stream");
    }
}