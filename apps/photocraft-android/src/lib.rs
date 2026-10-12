//! PhotoCraft on Android (arm64-v8a): the real engine and egui shell, running
//! on winit's NativeActivity backend.
//!
//! # What this crate is
//!
//! A shell around the *unmodified* PhotoCraft crates — `photocraft-engine` (the
//! document model and command registry), `photocraft-io` (PSD and flat codecs)
//! and `photocraft-ui-egui` (the egui UI). Nothing in those crates is patched.
//! This crate supplies only what `apps/photocraft` would normally supply: the
//! [`Services`] implementation, the window options, and the entry point.
//!
//! # Why a separate crate, and not a feature on `apps/photocraft`
//!
//! That crate cannot build for Android as written: it depends on `rfd` (native
//! file dialogs), `arboard` (OS clipboard) and `open`, all of which are
//! desktop-only. `rfd` fails with `the trait bound FileDialog:
//! FilePickerDialogImpl is not satisfied` and `arboard` with `cannot find Set in
//! module platform` — they have no Android backends at all.
//!
//! Those uses are confined to that crate's `services.rs`, and
//! `photocraft_ui_egui::Services` is a plain struct of `Option` closures that
//! derives `Default`. So the fix is not to patch the app but to build a
//! platform that supplies what it can and omits the rest — which is exactly what
//! `Services` was shaped for.
//!
//! # What Android cannot do yet
//!
//! These are honest gaps, not oversights:
//!
//! - **No file dialogs.** `file_dialog` stays `None`, so Open… and Save As… do
//!   nothing. Android needs the Storage Access Framework, i.e. a JNI bridge to
//!   `Intent.ACTION_OPEN_DOCUMENT` plus `onActivityResult` plumbing. Any path
//!   already reachable in the app's own data directory still works through
//!   `write`/`export`/`import`.
//! - **No clipboard image transfer.** `arboard` has no Android backend.
//! - **No stylus pressure, tilt or rotation.** `crates/tablet` implements macOS
//!   AppKit and Linux X11 XInput2 only, so `#[cfg]`s it to a no-op here and a
//!   pen degrades to ordinary touch. Android does expose those values on
//!   `MotionEvent`, so a backend is writable — but it does not exist yet.
//! - **No crash recovery.** `recover`/`autosave` are unwired for now.
//!
//! Editing itself is unaffected: document model, history, the 500+ command
//! registry, the CPU compositor and the wgpu GPU canvas are all
//! platform-neutral.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

use photocraft_doc::Document;
use photocraft_engine::Session;
use photocraft_ui_egui::{ExportSettings, PhotocraftApp, Services};

mod file_picker;
mod modifiers;

/// The app's private data directory, captured in `android_main` before any
/// service closure runs.
///
/// `internal_data_path()` needs no runtime permission — it is the app's own
/// sandbox — which is why preferences and exports land here rather than in
/// shared storage.
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

fn prefs_file() -> Option<PathBuf> {
    DATA_DIR.get().map(|d| d.join("preferences.json"))
}

/// Mirrors the desktop shell's `export_options`
/// (`apps/photocraft/src/services.rs`), so Export As behaves identically.
fn export_options(settings: &ExportSettings) -> photocraft_io::ExportOptions {
    let mut opts = photocraft_io::ExportOptions::default();
    if let Some(q) = settings.jpeg_quality {
        opts.encode.jpeg_quality = q;
    }
    opts.encode.webp_lossless = settings.webp_lossless;
    if let Some(q) = settings.webp_quality {
        opts.encode.webp_quality = q;
    }
    opts.tiff_layers = settings.tiff_layers;
    opts.xmp = if settings.xmp_all {
        photocraft_io::XmpEmbed::All
    } else {
        photocraft_io::XmpEmbed::None
    };
    opts.tga_bits = settings.tga_bits;
    opts
}

/// The platform-neutral half of the desktop `services::native()`.
///
/// Everything omitted here is omitted deliberately — see the crate docs. The
/// codecs are wired identically, so opening and exporting real files works.
fn android_services() -> Services {
    Services {
        import: Some(Box::new(
            |name: &str, bytes: &[u8], max_svg_group_depth: usize| {
                photocraft_io::import_with_svg_group_depth(name, bytes, max_svg_group_depth)
                    .map(|r| (r.document, r.warnings))
                    .map_err(|e| e.to_string())
            },
        )),
        export: Some(Box::new(
            |doc: &Document, path: &str, settings: &ExportSettings| {
                let opts = export_options(settings);
                photocraft_io::export(doc, path, &opts)
                    .map(|r| (r.bytes, r.warnings))
                    .map_err(|e| e.to_string())
            },
        )),
        write: Some(Box::new(|path: &str, bytes: &[u8]| {
            photocraft_format::atomic_write(Path::new(path), bytes).map_err(|e| e.to_string())
        })),
        encode_png: Some(Box::new(|w, h, rgba| {
            let img = photocraft_codecs::Image::from_u8(
                w,
                h,
                photocraft_codecs::ChannelLayout::Rgba,
                rgba.to_vec(),
            )
            .map_err(|e| e.to_string())?;
            photocraft_codecs::encode(
                &img,
                photocraft_codecs::Format::Png,
                &photocraft_codecs::EncodeOptions::default(),
            )
            .map_err(|e| e.to_string())
        })),
        load_prefs: Some(Box::new(|| std::fs::read_to_string(prefs_file()?).ok())),
        save_prefs: Some(Box::new(|text: &str| {
            let path = prefs_file().ok_or("no data directory")?;
            photocraft_format::atomic_write(&path, text.as_bytes()).map_err(|e| e.to_string())
        })),
        // File ▸ Exit (and the OS's close request; also `app.quit`) all end up in
        // `discard_ui::guard_window_close`, which — once nothing is unsaved — cancels the
        // close and calls this. Leaving it `None`, as the first version of this port did,
        // sends `ViewportCommand::Close` and ends nothing: egui-winit turns that into a
        // plain `ViewportEvent::Close` and never touches winit's window API, which on
        // Android is stubbed anyway. The app then hangs on a stale close request, dies on
        // the next input, and cannot be reopened until it is cleared from Recents —
        // because the native thread returns while the Activity lives on.
        //
        // Ending the process is what the desktop shell does on macOS, and what the
        // VectorCraft port does. `on_exit` cannot run after a process exit, and
        // PhotocraftApp has none; preferences are persisted by the `save_prefs` service as
        // they change (`prefs_ui`), so nothing is lost by not going through `save`.
        quit: Some(Box::new(|| {
            log::info!("File > Exit: ending the process");
            log::logger().flush();
            std::process::exit(0);
        })),
        ..Default::default()
    }
}

/// A minimal file logger, so a sideloaded APK leaves a readable trace.
///
/// `logcat` is only reachable over adb; a file in the app's own data directory
/// can be pulled with any file manager, which is what a sideloaded build needs.
struct FileLogger {
    sink: Mutex<Option<std::fs::File>>,
}

impl log::Log for FileLogger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        if let Ok(mut guard) = self.sink.lock() {
            if let Some(f) = guard.as_mut() {
                use std::io::Write;
                let _ = writeln!(f, "[{}] {} {}", record.level(), record.target(), record.args());
            }
        }
    }
    fn flush(&self) {}
}

static LOGGER: OnceLock<FileLogger> = OnceLock::new();

fn init_logging(dir: &Path) {
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("photocraft.log"))
    else {
        return;
    };
    let logger = LOGGER.get_or_init(|| FileLogger {
        sink: Mutex::new(None),
    });
    if let Ok(mut guard) = logger.sink.lock() {
        *guard = Some(file);
    }
    if log::set_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// The Android entry point.
///
/// `android-activity` spawns a dedicated thread for this and calls it with the
/// activity handle; `#[unsafe(no_mangle)]` with the Rust ABI is the required
/// shape (edition 2024 requires the `unsafe(..)` wrapper around `no_mangle`).
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    let data_dir = app
        .internal_data_path()
        .unwrap_or_else(|| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&data_dir);
    let _ = DATA_DIR.set(data_dir.clone());
    // The app's external files directory: always readable and writable with no
    // permission, so the picker can fall back to it for both open and save.
    if let Some(ext) = app.external_data_path() {
        let _ = std::fs::create_dir_all(&ext);
        let _ = file_picker::EXT_DIR.set(ext);
    }
    init_logging(&data_dir);
    log::info!(
        "PhotoCraft {} starting on Android; data dir {}",
        photocraft_engine::build_info::long_version(),
        data_dir.display()
    );

    // Before the window opens: the font scan is one-shot and the first one to run wins, so the
    // app's own Fonts folder has to be registered here or it is never seen (`file_picker`).
    file_picker::register_fonts();

    let mut options = eframe::NativeOptions::default();
    // Required on Android: eframe hands this to
    // `EventLoopBuilderExtAndroid::with_android_app` and errors without it.
    options.android_app = Some(app);

    let result = eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(|cc| {
            let shared = file_picker::channel();
            let mut services = android_services();
            // Without this, `ask_file` returns CANCELLED and Open/Save do nothing.
            let for_dialog = shared.clone();
            services.file_dialog = Some(Box::new(move |request, _parent, reply| {
                file_picker::request(&for_dialog, request, reply);
            }));
            let mut app = PhotocraftApp::new(Session::new(), services);
            // Put the canvas on the GPU when the device gave us a wgpu device;
            // otherwise the engine's CPU compositor carries it.
            if let Some(rs) = cc.wgpu_render_state.clone() {
                log::info!("wgpu adapter: {:?}", rs.adapter.get_info());
                app.set_wgpu(rs);
            } else {
                log::warn!("no wgpu render state; the canvas will composite on the CPU");
            }
            Ok(Box::new(Shell { inner: app, modifiers: modifiers::Tracked::default(), shared }))
        }),
    );

    match result {
        Ok(()) => log::info!("PhotoCraft exited normally"),
        Err(e) => log::error!("PhotoCraft failed to run: {e}"),
    }
}

/// Wraps the real app so the file picker can draw on top of it.
///
/// The same shape as the web build's `WebShell` (`apps/photocraft-web/src/web.rs`):
/// `PhotocraftApp` stays untouched, and the platform-specific layer hangs off the
/// outside. Every trait method is forwarded, so the app sees no difference.
struct Shell {
    inner: PhotocraftApp,
    /// Modifier keys folded back into the frame's events; Android reports none (see `modifiers`).
    modifiers: modifiers::Tracked,
    shared: Rc<std::cell::RefCell<file_picker::Shared>>,
}

impl eframe::App for Shell {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        eframe::App::logic(&mut self.inner, ctx, frame);
    }

    /// Forwarded because `PhotocraftApp` implements it and it is not just cosmetic:
    /// it runs `shortcuts::clipboard_keys` and appends `take_synthetic_step()` to the
    /// frame's events. Dropping it silently loses both — the same gap the VectorCraft
    /// port had to fix.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Before the app's own hook, so it sees the frame a desktop backend would have
        // produced. Android's winit never sends `ModifiersChanged`, so without this every
        // Ctrl shortcut, Shift+click and Alt keytip is dead (see `modifiers`).
        self.modifiers.apply(raw);
        eframe::App::raw_input_hook(&mut self.inner, ctx, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        eframe::App::ui(&mut self.inner, ui, frame);
        // After the app, so the picker floats above the editor.
        file_picker::draw(ui.ctx(), &self.shared);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::App::save(&mut self.inner, storage);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        eframe::App::clear_color(&self.inner, visuals)
    }

    fn persist_egui_memory(&self) -> bool {
        eframe::App::persist_egui_memory(&self.inner)
    }

    fn auto_save_interval(&self) -> std::time::Duration {
        eframe::App::auto_save_interval(&self.inner)
    }
}
