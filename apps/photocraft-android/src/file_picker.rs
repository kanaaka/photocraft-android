//! An in-app file picker for Android.
//!
//! # Why this exists
//!
//! `photocraft-ui-egui::PhotocraftApp::ask_file` refuses to do anything when
//! `Services::file_dialog` is `None`:
//!
//! ```ignore
//! if self.services.file_dialog.is_none() { return Err(CANCELLED.into()); }
//! ```
//!
//! so File ▸ Open and Save As are *silently treated as cancelled* — the app looks
//! like it has no file access at all. The desktop build fills that slot with
//! `rfd`; Android has no equivalent available to a NativeActivity with no Java,
//! so this module draws the picker with egui instead.
//!
//! # How it plugs in
//!
//! `FileDialogFn` is `Box<dyn FnMut(FileDialogRequest, Option<&eframe::Frame>,
//! FileDialogReply)>`. `PhotocraftApp::logic` calls it from `poll_file_dialog`
//! once per frame while a dialog is queued, then polls the reply's channel; an
//! empty channel simply means "keep waiting". So the contract is: stash the
//! request and the reply, return at once, and send the answer from a later frame
//! through [`FileDialogReply::send`] — which is what [`Shared`] is for.
//!
//! # Permissions
//!
//! Android 10+ forbids reading `/storage/emulated/0/Download` by path. Two routes
//! exist and this picker works with either, because it probes each root and only
//! offers the ones it can actually read:
//!
//! - **App folder** — `<external_data_path>/`, the app's own directory. Always
//!   readable and writable, no permission at all. Files can be pushed here over
//!   USB.
//! - **Shared storage** — Download / Pictures / DCIM / Documents. Needs
//!   "All files access" (`MANAGE_EXTERNAL_STORAGE`, granted by hand in
//!   Settings ▸ Apps ▸ Special access) or `READ_MEDIA_IMAGES` for reading media.
//!
//! The proper alternative — the Storage Access Framework, i.e. the real Android
//! picker with no permission — needs Java to receive `onActivityResult`, and
//! `android-activity` does not expose `startActivityForResult`. That is future
//! work, not a gap here.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use photocraft_ui_egui::{FileDialogAnswer, FileDialogReply, FileDialogRequest};

/// The app's external files directory, captured in `android_main`.
pub static EXT_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

const SHARED_ROOT: &str = "/storage/emulated/0";

/// A queued dialog: what was asked, what is chosen so far, and how to answer.
#[derive(Default)]
pub struct Shared {
    pending: Option<Pending>,
    /// Directory being browsed.
    dir: PathBuf,
    /// File name for a Save dialog.
    name: String,
    /// A message to show inside the picker (usually a permission problem).
    note: Option<String>,
    /// Chosen files, for a multi-select Open.
    picked: Vec<PathBuf>,
    /// Whether the starting directory was already applied.
    started: bool,
}

struct Pending {
    request: FileDialogRequest,
    multiple: bool,
    extensions: Option<Vec<String>>,
    suggested: Option<String>,
    reply: Option<FileDialogReply>,
}

impl Default for Pending {
    fn default() -> Self {
        Self { request: FileDialogRequest::Open { multiple: false, initial_dir: None, extensions: None }, multiple: false, extensions: None, suggested: None, reply: None }
    }
}

/// Build the `FileDialogFn` and the state the shell draws from.
pub fn channel() -> Rc<RefCell<Shared>> {
    Rc::new(RefCell::new(Shared::default()))
}

/// The `FileDialogFn` body: queue the request and return immediately.
pub fn request(shared: &Rc<RefCell<Shared>>, request: FileDialogRequest, reply: FileDialogReply) {
    let mut s = shared.borrow_mut();
    let (multiple, extensions, suggested) = match &request {
        FileDialogRequest::Open { multiple, extensions, .. } => (*multiple, extensions.clone(), None),
        FileDialogRequest::Save { suggested } => (false, None, Some(suggested.clone())),
    };
    s.pending = Some(Pending { request, multiple, extensions, suggested, reply: Some(reply) });
    s.picked.clear();
    s.started = false;
    s.note = None;
}

/// `<external storage>/Android/media/<package>`: the app's own media folder.
///
/// Derived from `external_data_path()` (`…/Android/data/<package>/files`) so the package name
/// isn't written down twice.
///
/// This is the one folder that is both the app's own storage (**no permission**) and left
/// **visible over USB/MTP** by Android 11+ — `Android/data` is hidden there, so a font a user
/// copies into the app folder with a cable would silently never arrive.
pub fn media_dir() -> Option<PathBuf> {
    media_dir_from(EXT_DIR.get()?)
}

/// The app's own Fonts folder: `<App media (USB)>/Fonts`.
///
/// **This is how a font gets installed here.** PhotoCraft has no font-add step of its own, so a
/// user drops a `.ttf`/`.otf`/`.ttc` into this folder over USB and the next font scan picks it
/// up. It is registered with the text crate before the window opens (see [`register_fonts`]),
/// which is what makes the file's families appear in the type tool.
pub fn fonts_dir() -> Option<PathBuf> {
    Some(media_dir()?.join("Fonts"))
}

/// The derivation, on its own so it can be pinned by a test:
/// `…/Android/data/<package>/files` → `…/Android/media/<package>`.
fn media_dir_from(external_files: &Path) -> Option<PathBuf> {
    let package_dir = external_files.parent()?;
    let android = package_dir.parent()?.parent()?;
    Some(android.join("media").join(package_dir.file_name()?))
}

/// Register the font sources Android would otherwise leave the app without: the system's own
/// fonts (the text crate's `system_font_dirs` now lists Android's folders) and this app's Fonts
/// folder. One-shot — only the first call counts — and it must run before the first font scan,
/// so it is called before the window opens.
pub fn register_fonts() {
    if !cfg!(target_os = "android") {
        return;
    }
    if let Some(fonts) = fonts_dir() {
        let _ = std::fs::create_dir_all(&fonts);
        log::info!("fonts: the app's Fonts folder is {}", fonts.display());
        photocraft_text::fonts::set_extra_font_dirs(vec![fonts]);
    } else {
        log::warn!("fonts: no Fonts folder to register; installing a font will not work");
    }
}

/// Roots to offer, as (label, path).
fn roots() -> Vec<(String, PathBuf)> {
    let mut v = Vec::new();
    if let Some(d) = EXT_DIR.get() {
        v.push(("App folder".to_string(), d.clone()));
    }
    if let Some(m) = media_dir() {
        v.push(("App media (USB)".to_string(), m));
    }
    if let Some(d) = fonts_dir() {
        v.push(("Fonts (install here)".to_string(), d));
    }
    if let Some(d) = EXT_DIR.get().and_then(|d| d.parent()) {
        // Sibling of the app's own files, still owned by the app.
        v.push(("App storage".to_string(), d.to_path_buf()));
    }
    for (label, sub) in [
        ("Download", "Download"),
        ("Pictures", "Pictures"),
        ("DCIM", "DCIM"),
        ("Documents", "Documents"),
    ] {
        v.push((label.to_string(), Path::new(SHARED_ROOT).join(sub)));
    }
    v.push(("Internal storage".to_string(), PathBuf::from(SHARED_ROOT)));
    v
}

/// The best directory to open first: the app folder is always writable, so it is
/// the only safe default — desktop paths arriving in `initial_dir` never exist here.
fn default_dir() -> PathBuf {
    EXT_DIR.get().cloned().unwrap_or_else(|| PathBuf::from(SHARED_ROOT))
}

fn readable(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok()
}

/// Directory entries, folders first, filtered by `extensions` when given.
fn entries(dir: &Path, extensions: Option<&Vec<String>>) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let (mut dirs, mut files) = (Vec::new(), Vec::new());
    for entry in rd.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            dirs.push(path);
        } else if let Some(exts) = extensions {
            let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
            if ext.as_ref().is_some_and(|e| exts.iter().any(|w| w.eq_ignore_ascii_case(e))) {
                files.push(path);
            }
        } else {
            files.push(path);
        }
    }
    dirs.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()));
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()));
    Ok((dirs, files))
}

fn file_label(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn is_save(s: &Shared) -> bool {
    matches!(s.pending.as_ref().map(|p| &p.request), Some(FileDialogRequest::Save { .. }))
}

/// Draw the picker if a dialog is queued. Called from the shell after the app's
/// own `ui`, so the window floats on top of the editor.
pub fn draw(ctx: &egui::Context, shared: &Rc<RefCell<Shared>>) {
    let mut s = shared.borrow_mut();
    if s.pending.is_none() {
        return;
    }
    // First frame: pick a starting directory.
    if !s.started {
        s.started = true;
        let initial = match s.pending.as_ref().map(|p| &p.request) {
            Some(FileDialogRequest::Open { initial_dir: Some(d), .. }) if readable(Path::new(d)) => Some(PathBuf::from(d)),
            Some(FileDialogRequest::Save { suggested }) => {
                let p = PathBuf::from(suggested);
                if let Some(name) = p.file_name() {
                    s.name = name.to_string_lossy().to_string();
                }
                p.parent().filter(|d| readable(d) && !d.as_os_str().is_empty()).map(Path::to_path_buf)
            }
            _ => None,
        };
        s.dir = initial.unwrap_or_else(default_dir);
        if is_save(&s) && s.name.is_empty() {
            s.name = "untitled.psd".to_string();
        }
    }

    let save = is_save(&s);
    let title = if save { "Save As — Android" } else { "Open — Android" };
    let mut answer: Option<Option<FileDialogAnswer>> = None;
    let mut navigate: Option<PathBuf> = None;

    let dir = s.dir.clone();
    let listing = entries(&dir, s.pending.as_ref().and_then(|p| p.extensions.as_ref()));

    egui::Window::new(title)
        .collapsible(false)
        .resizable(true)
        .default_width(620.0)
        .default_height(460.0)
        .show(ctx, |ui| {
            // Roots
            ui.horizontal_wrapped(|ui| {
                for (label, path) in roots() {
                    let ok = readable(&path);
                    let here = path == dir;
                    let text = if ok { label } else { format!("{label} \u{1F512}") };
                    if ui.add_enabled(ok && !here, egui::Button::new(text)).clicked() {
                        navigate = Some(path);
                    }
                }
            });
            ui.separator();

            // Current path + up
            ui.horizontal(|ui| {
                if ui.button("\u{2B06} Up").clicked() {
                    if let Some(parent) = dir.parent() {
                        navigate = Some(parent.to_path_buf());
                    }
                }
                if ui.button("\u{1F4C1} App folder").clicked() {
                    navigate = Some(default_dir());
                }
                ui.label(egui::RichText::new(dir.display().to_string()).monospace());
            });

            if let Some(note) = &s.note {
                ui.colored_label(egui::Color32::from_rgb(220, 160, 60), note);
            }

            ui.separator();
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| match &listing {
                Err(e) => {
                    ui.label("Cannot read this folder.");
                    ui.label(egui::RichText::new(e.clone()).monospace().weak());
                    ui.label("Grant \"All files access\" in Settings \u{25B8} Apps \u{25B8} PhotoCraft \u{25B8} Special access to browse shared storage.");
                }
                Ok((dirs, files)) => {
                    if dirs.is_empty() && files.is_empty() {
                        ui.label("(empty \u{2014} no matching files)");
                    }
                    for d in dirs {
                        if ui.button(format!("\u{1F4C1} {}", file_label(d))).clicked() {
                            navigate = Some(d.clone());
                        }
                    }
                    for f in files {
                        let chosen = s.picked.iter().any(|p| p == f);
                        let mark = if chosen { "\u{2611}" } else { "\u{2610}" };
                        let label = if s.pending.as_ref().is_some_and(|p| p.multiple) {
                            format!("{mark} {}", file_label(f))
                        } else {
                            format!("   {}", file_label(f))
                        };
                        if ui.button(label).clicked() {
                            if save {
                                s.name = file_label(f);
                            } else if s.pending.as_ref().is_some_and(|p| p.multiple) {
                                if chosen {
                                    s.picked.retain(|p| p != f);
                                } else {
                                    s.picked.push(f.clone());
                                }
                            } else {
                                answer = Some(Some(FileDialogAnswer::Paths(vec![f.to_string_lossy().to_string()])));
                            }
                        }
                    }
                }
            });

            ui.separator();
            ui.horizontal(|ui| {
                if save {
                    ui.label("File name:");
                    ui.add(egui::TextEdit::singleline(&mut s.name).desired_width(320.0));
                    if ui.button("Save").clicked() {
                        let name = s.name.trim();
                        if name.is_empty() {
                            s.note = Some("Enter a file name.".to_string());
                        } else {
                            let target = dir.join(name);
                            answer = Some(Some(FileDialogAnswer::SaveTo(target.to_string_lossy().to_string())));
                        }
                    }
                } else {
                    let n = s.picked.len();
                    let multi = s.pending.as_ref().is_some_and(|p| p.multiple);
                    let enabled = !multi || n > 0;
                    let label = if multi { format!("Open {n} file(s)") } else { "Open".to_string() };
                    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                        let paths: Vec<String> = s.picked.iter().map(|p| p.to_string_lossy().to_string()).collect();
                        answer = Some(Some(FileDialogAnswer::Paths(paths)));
                    }
                }
                if ui.button("Cancel").clicked() {
                    answer = Some(None);
                }
            });

            if dir == default_dir() {
                ui.label(egui::RichText::new("Tip: push files here over USB, or grant \"All files access\" to browse the whole tablet.").weak().small());
            }
        });

    if let Some(dir) = navigate {
        s.dir = dir;
        s.note = None;
    }
    if let Some(ans) = answer {
        if let Some(pending) = s.pending.take() {
            if let Some(reply) = pending.reply {
                reply.send(ans);
            }
        }
        s.picked.clear();
    }
    ctx.request_repaint();
}
