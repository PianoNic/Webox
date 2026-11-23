//! Files and attachments: the state the UI and the cloud thread share while files go up or come down.
//!
//! Uploads (composer "Datei anhängen", drag & drop, pasted images, screen captures) and downloads (file cards in messages, the
//! "Inhalt" tab, the image viewer's "Speichern unter") run on worker threads of the cloud layer (`cloud/files/`). They report
//! progress here; the UI reads it every frame and asks for a repaint through the stored `egui::Context`. Cancelling sets an
//! `AtomicBool` the worker checks between chunks.
//!
//! Limits:
//! - Size: device policy `file-upload-size-limit` (in MB), default `MAX_FILE_SIZE_IN_MB = 2048`; the files service's
//!   initialize answer can lower it (`fileUploadSizeLimit`).
//! - Desktop uploads allowed: device policy `bfs-upload-allow-desktop`.
//! - Blocked file types: decided by the service (policies `bfs-upload-block-unknown-file-type`, `bfs-upload-allow-file-type-enabled`);
//!   a refused upload is reported as "Gesperrte Datei" / "Dieser Dateityp ist aufgrund der {{0}}-Richtlinie nicht zulässig."
//!   There is no client-side list.
//! - Files per message: capped at `MAX_FILES` ("Pro Nachricht sind maximal %1 Dateien erlaubt").
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Default upload size limit in MB when the device policy does not set one.
pub const DEFAULT_LIMIT_MB: u64 = 2048;
/// Maximum number of files per message.
pub const MAX_FILES: usize = 10;

#[derive(Clone, Debug, PartialEq)]
pub enum Phase { Preparing, Uploading, Sharing, Done, Failed(String), Cancelled }

/// What happens once a download has finished.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Then { Nothing, Open, ShowInFolder, /// fetched for the inline video player (into the playback cache, no toast)
    Play }

#[derive(Clone, Debug)]
pub struct Upload { pub id: String, pub space: String, pub names: Vec<String>, pub sent: u64, pub total: u64, pub phase: Phase, pub cancel: Arc<AtomicBool>, pub started: f64 }

#[derive(Clone, Debug)]
pub struct Download { pub key: String, pub name: String, pub path: PathBuf, pub got: u64, pub total: u64, pub phase: Phase, pub then: Then, pub cancel: Arc<AtomicBool> }

struct Policy { limit_mb: u64, allow_desktop: bool }

#[derive(Default)]
struct State {
    uploads: Vec<Upload>,
    downloads: Vec<Download>,
    /// Results of the open-file dialog: (space id, chosen files).
    picks: Vec<(String, Vec<PathBuf>)>,
    /// Results of a save dialog: (attachment key, chosen path).
    saves: Vec<(String, PathBuf)>,
    ctx: Option<eframe::egui::Context>,
    policy: Option<(u64, bool)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

pub fn set_ctx(ctx: &eframe::egui::Context) { with(|s| if s.ctx.is_none() { s.ctx = Some(ctx.clone()) }); }
pub fn repaint() { if let Some(c) = with(|s| s.ctx.clone()) { c.request_repaint(); } }

/// Policies from the device registration (cloud.rs `private_setup` calls this with the device answer).
pub fn set_policy(dev: &serde_json::Value) {
    let limit = find(dev, "file-upload-size-limit").and_then(|v| v.trim().parse::<u64>().ok()).filter(|v| *v > 0);
    let allow = find(dev, "bfs-upload-allow-desktop").map(|v| v.trim() != "false");
    with(|s| s.policy = Some((limit.unwrap_or(DEFAULT_LIMIT_MB), allow.unwrap_or(true))));
}
/// Like `cloud::find_policy` (`{"<name>": v}` or `{"…": "<name>", "value": v}` anywhere), but numbers and booleans count too.
fn find(v: &serde_json::Value, name: &str) -> Option<String> {
    use serde_json::Value;
    let text = |x: &Value| match x { Value::String(s) => Some(s.clone()), Value::Number(n) => Some(n.to_string()), Value::Bool(b) => Some(b.to_string()), _ => None };
    match v {
        Value::Object(m) => {
            if let Some(s) = m.get(name).and_then(text) { return Some(s); }
            if m.values().any(|x| x.as_str() == Some(name)) { if let Some(s) = m.get("value").and_then(text) { return Some(s); } }
            m.values().find_map(|x| find(x, name))
        }
        Value::Array(a) => a.iter().find_map(|x| find(x, name)),
        _ => None,
    }
}
fn policy() -> Policy { with(|s| s.policy).map_or(Policy { limit_mb: DEFAULT_LIMIT_MB, allow_desktop: true }, |(l, a)| Policy { limit_mb: l, allow_desktop: a }) }
pub fn limit_bytes() -> u64 { policy().limit_mb * 1024 * 1024 }
pub fn uploads_allowed() -> bool { policy().allow_desktop }

// Uploads.
pub fn upload_begin(space: &str, names: Vec<String>, total: u64, now: f64) -> (String, Arc<AtomicBool>) {
    let id = format!("upload:{}", crate::jose::uuid());
    let cancel = Arc::new(AtomicBool::new(false));
    with(|s| s.uploads.push(Upload { id: id.clone(), space: space.into(), names, sent: 0, total, phase: Phase::Preparing, cancel: cancel.clone(), started: now }));
    repaint();
    (id, cancel)
}
pub fn upload_cancel_flag(id: &str) -> Option<Arc<AtomicBool>> { with(|s| s.uploads.iter().find(|u| u.id == id).map(|u| u.cancel.clone())) }
pub fn upload_add(id: &str, n: u64) { with(|s| if let Some(u) = s.uploads.iter_mut().find(|u| u.id == id) { u.sent = (u.sent + n).min(u.total); if u.phase == Phase::Preparing { u.phase = Phase::Uploading } }); repaint(); }
pub fn upload_phase(id: &str, p: Phase) { with(|s| if let Some(u) = s.uploads.iter_mut().find(|u| u.id == id) { if u.phase != Phase::Cancelled { u.phase = p } }); repaint(); }
pub fn upload_cancel(id: &str) { with(|s| if let Some(u) = s.uploads.iter_mut().find(|u| u.id == id) { u.cancel.store(true, Ordering::SeqCst); u.phase = Phase::Cancelled; }); repaint(); }
pub fn uploads() -> Vec<Upload> { with(|s| s.uploads.clone()) }
pub fn upload_remove(id: &str) { with(|s| s.uploads.retain(|u| u.id != id)); }

// Downloads.
pub fn download_begin(key: &str, name: &str, path: &Path, then: Then) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    with(|s| {
        s.downloads.retain(|d| d.key != key);
        s.downloads.push(Download { key: key.into(), name: name.into(), path: path.into(), got: 0, total: 0, phase: Phase::Preparing, then, cancel: cancel.clone() });
    });
    repaint();
    cancel
}
pub fn download_progress(key: &str, got: u64, total: u64) { with(|s| if let Some(d) = s.downloads.iter_mut().find(|d| d.key == key) { d.got = got; d.total = total; d.phase = Phase::Uploading; }); repaint(); }
pub fn download_phase(key: &str, p: Phase) { with(|s| if let Some(d) = s.downloads.iter_mut().find(|d| d.key == key) { d.phase = p }); repaint(); }
pub fn download(key: &str) -> Option<Download> { with(|s| s.downloads.iter().find(|d| d.key == key).cloned()) }
pub fn downloads() -> Vec<Download> { with(|s| s.downloads.clone()) }
pub fn download_cancel(key: &str) { with(|s| if let Some(d) = s.downloads.iter_mut().find(|d| d.key == key) { d.cancel.store(true, Ordering::SeqCst); d.phase = Phase::Cancelled; }); repaint(); }

// Native dialogs (own threads, results collected by the UI).
/// The composer's "Datei anhängen": dialog title "Datei zum Hochladen auswählen", filters "Alle (*)" and "Bilder".
/// Several files may be chosen.
pub fn pick_files_async(space: &str) {
    let space = space.to_string();
    std::thread::spawn(move || {
        let r = rfd::FileDialog::new().set_title("Datei zum Hochladen auswählen").add_filter("Alle (*)", &["*"])
            .add_filter("Bilder", &["png", "jpg", "jpeg", "gif", "bmp", "webp"]).pick_files();
        if let Some(v) = r { with(|s| s.picks.push((space, v))); repaint(); }
    });
}
/// Queues files for the composer of `space` as if chosen in the open-file dialog (a favourite uploaded GIF picked in the GIF picker).
pub fn push_picks(space: &str, paths: Vec<PathBuf>) { if !paths.is_empty() { with(|s| s.picks.push((space.to_string(), paths))); repaint(); } }
pub fn take_picks(space: &str) -> Vec<PathBuf> {
    with(|s| { let mut out = vec![]; s.picks.retain(|(sp, v)| if sp == space { out.extend(v.iter().cloned()); false } else { true }); out })
}
/// "Speichern unter": a save dialog in the download folder, pre-filled with the name.
pub fn save_as_async(key: &str, name: &str, dir: &Path) {
    let (key, name, dir) = (key.to_string(), name.to_string(), dir.to_path_buf());
    std::thread::spawn(move || {
        let r = rfd::FileDialog::new().set_title("Speichern unter").set_directory(&dir).set_file_name(&name).save_file();
        if let Some(p) = r { with(|s| s.saves.push((key, p))); repaint(); }
    });
}
pub fn take_saves() -> Vec<(String, PathBuf)> { with(|s| std::mem::take(&mut s.saves)) }

// Helpers.
/// Settings "Standardmäßiger Dateispeicherort" (`download_dir`), else the OS download folder (as the settings page shows it).
pub fn download_dir(settings: &serde_json::Value) -> PathBuf {
    if let Some(d) = settings["download_dir"].as_str().filter(|d| !d.is_empty()) { return PathBuf::from(d); }
    #[cfg(windows)]
    if let Ok(p) = std::env::var("USERPROFILE") { return PathBuf::from(p).join("Downloads"); }
    if let Ok(p) = std::env::var("HOME") { return PathBuf::from(p).join("Downloads"); }
    PathBuf::from("Downloads")
}

/// A file name that is safe on disk: no path parts, no reserved characters, no invisible bidi / format characters (U+202E
/// would show "evil\u{202e}fdp.exe" as "evilexe.pdf"), no trailing dots or spaces (Windows drops them, so "evil.exe ." would open
/// as evil.exe), and no Windows device name (CON, NUL, COM1, ... with any extension) without a leading `_`.
pub fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let s: String = base.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() || invisible_format(c) { '_' } else { c }).collect();
    let s = s.trim_start().trim_end_matches(|c: char| c == '.' || c.is_whitespace());
    if s.is_empty() { "Datei".into() } else if device_name(s) { format!("_{s}") } else { s.to_string() }
}

/// Zero-width and bidi control characters (they change how a name reads, not what it is).
fn invisible_format(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

/// A Windows device name, with or without extension ("nul.txt" and "COM1 .log" are the device too).
fn device_name(s: &str) -> bool {
    let stem = s.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || ["COM", "LPT"].iter().any(|p| stem.strip_prefix(p).is_some_and(|d| matches!(d, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")))
}

/// `dir/name`, or `dir/name (1).ext`, `(2)`, ... when that exists (Windows' own naming for copies).
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let name = safe_name(name);
    let p = dir.join(&name);
    if !p.exists() { return p; }
    let (stem, ext) = match name.rfind('.') { Some(i) if i > 0 => (&name[..i], &name[i..]), _ => (name.as_str(), "") };
    (1..10_000).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).unwrap_or(p)
}

/// MIME type: by content for images (sniffs the bytes), else by extension.
pub fn mime_of(name: &str, head: &[u8]) -> String {
    if let Ok(f) = image::guess_format(head) {
        let m = match f { image::ImageFormat::Png => "image/png", image::ImageFormat::Jpeg => "image/jpeg", image::ImageFormat::Gif => "image/gif",
            image::ImageFormat::WebP => "image/webp", image::ImageFormat::Bmp => "image/bmp", _ => "" };
        if !m.is_empty() { return m.into(); }
    }
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "pdf" => "application/pdf", "txt" | "log" => "text/plain", "csv" => "text/csv", "md" => "text/markdown", "html" | "htm" => "text/html",
        "json" => "application/json", "xml" => "application/xml", "zip" => "application/zip", "7z" => "application/x-7z-compressed", "gz" => "application/gzip",
        "doc" => "application/msword", "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel", "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint", "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "mp3" => "audio/mpeg", "wav" => "audio/wav", "m4a" => "audio/mp4", "mp4" => "video/mp4", "mov" => "video/quicktime", "webm" => "video/webm",
        "png" => "image/png", "jpg" | "jpeg" => "image/jpeg", "gif" => "image/gif", "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }.into()
}

/// Types "Öffnen" starts with their default program: documents, pictures, sound, video, archives.
const OPEN_DIRECT: &[&str] = &["pdf", "txt", "log", "csv", "md", "json", "xml", "png", "jpg", "jpeg", "gif", "webp", "bmp", "mp3", "wav", "m4a",
    "ogg", "flac", "mp4", "mov", "webm", "mkv", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "zip", "7z", "gz", "tar"];

/// Whether a file from the chat may be started with its default program. Everything not in `OPEN_DIRECT` (programs, scripts,
/// shortcuts, macro documents, unknown types) is only shown in its folder. The extension is the one of the name as the disk
/// keeps it (`safe_name`: trailing dots and spaces dropped like Windows does), in lower case.
pub fn opens_directly(name: &str) -> bool {
    safe_name(name).to_lowercase().rsplit_once('.').is_some_and(|(_, ext)| OPEN_DIRECT.contains(&ext))
}

/// Opens a downloaded file with its default program (Windows: the shell's open verb via explorer); other types only in their
/// folder (`opens_directly`).
pub fn open_file(p: &Path) {
    if !opens_directly(&p.to_string_lossy()) { show_in_folder(p); return; }
    #[cfg(windows)] let _ = std::process::Command::new("explorer").arg(p).spawn();
    #[cfg(not(windows))] let _ = std::process::Command::new("xdg-open").arg(p).spawn();
}

/// Mark-of-the-Web on a file downloaded from a chat: the `Zone.Identifier` stream (Internet zone) that SmartScreen, Office's
/// Protected View and the shell's "open file" warning look at. NTFS only; errors (FAT, some network shares) are ignored. Only
/// for a file that exists (writing the stream of a missing file would create it). Elsewhere a no-op.
pub fn mark_downloaded(p: &Path) {
    #[cfg(windows)]
    if p.is_file() {
        let mut ads = p.as_os_str().to_os_string();
        ads.push(":Zone.Identifier");
        let _ = std::fs::write(PathBuf::from(ads), "[ZoneTransfer]\r\nZoneId=3\r\n");
    }
    #[cfg(not(windows))]
    let _ = p;
}

/// A per-user private folder for temporary files (the decrypted video cache, staged pastes and GIFs), created when missing and
/// checked on every call. Unix: `<temp>/webox-<uid>/<sub>`; both levels must be real
/// folders (no symlinks) of this user without any group / other permission, so other local users can neither read the files
/// nor plant folders, files or links in their place. Windows: `<temp>/webox-<sub>` (the temp folder is per user there).
pub fn private_tmp(sub: &str) -> std::io::Result<PathBuf> {
    #[cfg(unix)]
    {
        let uid = current_uid();
        let base = std::env::temp_dir().join(format!("webox-{uid}")); // not XDG_RUNTIME_DIR: that is RAM, and the video cache holds up to 2 GB
        private_dir(&base, uid)?;
        let d = base.join(sub);
        private_dir(&d, uid)?;
        Ok(d)
    }
    #[cfg(not(unix))]
    {
        let d = std::env::temp_dir().join(format!("webox-{sub}"));
        std::fs::create_dir_all(&d)?;
        Ok(d)
    }
}

/// Creates `d` with mode 0700 (if missing), then checks what is there: a folder, not a symlink, owned by `uid`, no group / other
/// permission bits.
#[cfg(unix)]
fn private_dir(d: &Path, uid: u32) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    match std::fs::DirBuilder::new().mode(0o700).create(d) {
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return Err(e),
        _ => {}
    }
    let m = std::fs::symlink_metadata(d)?;
    if m.file_type().is_dir() && m.uid() == uid && m.mode() & 0o077 == 0 { return Ok(()); }
    Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, format!("{} is not a private folder of this user", d.display())))
}

#[cfg(target_os = "linux")]
fn current_uid() -> u32 { unsafe { libc::getuid() } }
#[cfg(all(unix, not(target_os = "linux")))]
fn current_uid() -> u32 {
    extern "C" { fn getuid() -> u32; }
    unsafe { getuid() }
}
/// "In Ordner anzeigen": the file's folder with the file selected.
/// Windows: `SHOpenFolderAndSelectItems` (reuses an Explorer window that already shows the folder), on its own thread with COM
/// initialised; if that fails, `explorer.exe /select,"<path>"`. Linux: org.freedesktop.FileManager1.ShowItems over D-Bus
/// (gdbus), else the folder via xdg-open.
pub fn show_in_folder(p: &Path) {
    let p = p.to_path_buf();
    std::thread::spawn(move || {
        #[cfg(windows)]
        {
            if !shell_select(&p) { explorer_select(&p); }
        }
        #[cfg(not(windows))]
        {
            let ok = std::process::Command::new("gdbus").args(["call", "--session", "--dest", "org.freedesktop.FileManager1", "--object-path", "/org/freedesktop/FileManager1",
                "--method", "org.freedesktop.FileManager1.ShowItems", &gvariant_uris(&[file_uri(&p)]), ""]).status().is_ok_and(|s| s.success());
            if !ok { let _ = std::process::Command::new("xdg-open").arg(p.parent().unwrap_or(&p)).spawn(); }
        }
    });
}

/// `SHOpenFolderAndSelectItems` with the file's own ITEMIDLIST (an absolute pidl selects that item in its parent folder).
#[cfg(windows)]
fn shell_select(p: &Path) -> bool {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "shell32")]
    extern "system" {
        fn ILCreateFromPathW(path: *const u16) -> *mut c_void;
        fn ILFree(pidl: *mut c_void);
        fn SHOpenFolderAndSelectItems(folder: *const c_void, count: u32, items: *const *const c_void, flags: u32) -> i32;
    }
    let wide: Vec<u16> = p.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        let pidl = ILCreateFromPathW(wide.as_ptr());
        if pidl.is_null() { return false; }
        let hr = SHOpenFolderAndSelectItems(pidl, 0, std::ptr::null(), 0);
        ILFree(pidl);
        hr >= 0
    }
}

#[cfg(windows)]
fn explorer_select(p: &Path) {
    use std::os::windows::process::CommandExt;
    // raw: Rust's own quoting would wrap the whole `/select,…` in quotes, which explorer does not parse as a selection
    let _ = std::process::Command::new("explorer.exe").raw_arg(explorer_select_arg(p)).spawn();
}

/// `/select,"<path>"`: explorer's own syntax; the quotes keep spaces and commas in the path together (Windows paths cannot
/// contain `"`).
#[cfg(windows)]
pub(crate) fn explorer_select_arg(p: &Path) -> String { format!("/select,\"{}\"", p.display().to_string().replace('"', "")) }

/// `file://` URI of an absolute path, every byte outside the unreserved set and `/` percent-encoded.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn file_uri(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !s.starts_with('/') { out.push('/'); }
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) { out.push(b as char) } else { out.push_str(&format!("%{b:02X}")) }
    }
    out
}

/// The `as` (array of strings) argument of `gdbus call`: `['uri', ...]` with `'` and `\` escaped.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn gvariant_uris(uris: &[String]) -> String {
    format!("[{}]", uris.iter().map(|u| format!("'{}'", u.replace('\\', "\\\\").replace('\'', "\\'"))).collect::<Vec<_>>().join(", "))
}

/// Size text under a file, as Webex writes it: below 1024 "%1 bytes" (de "%1 Byte"); below 1024² whole KB, truncated
/// (de "%1\u{a0}KB"); below 1024³ MB with one decimal and trailing zeros (and the point) removed, always with a '.'
/// ("1.5 MB"); from 1 GiB "{{0}} GB" with the same one-decimal rule.
/// 0 = size unknown in our data: no text.
pub fn size_text(b: u64) -> String {
    if b == 0 { return String::new(); }
    let one = |v: f64| { let s = format!("{v:.1}"); s.strip_suffix(".0").map(str::to_string).unwrap_or(s) };
    if b < 1024 { format!("{b} Byte") }
    else if b < 1 << 20 { format!("{}\u{a0}KB", one((b >> 10) as f64)) }
    else if b < 1 << 30 { format!("{} MB", one(b as f64 / (1u64 << 20) as f64)) }
    else { format!("{} GB", one(b as f64 / (1u64 << 30) as f64)) }
}

/// Today as "yyyy-MM-dd_HH-mm-ss" (local time), the name pattern for captured images; the ".png" suffix is ours.
pub fn capture_name() -> String {
    let utc = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (y, mo, d, h, mi, s) = civil(utc + crate::i18n::local_offset_secs(utc) as i64);
    format!("{y:04}-{mo:02}-{d:02}_{h:02}-{mi:02}-{s:02}.png")
}

/// Name of an image pasted from the clipboard: "<guid>.PNG" with a lower-case guid, as Webex names it. Only the clipboard
/// bitmap is read; an HTML fragment's `src` URL is not used for the name.
pub fn pasted_image_name() -> String { format!("{}.PNG", crate::jose::uuid()) }

/// Seconds since 1970 -> (year, month, day, hour, minute, second), proleptic Gregorian (Howard Hinnant's civil_from_days).
fn civil(t: i64) -> (i64, i64, i64, i64, i64, i64) {
    let (days, rem) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    let z = days + 719_468; let era = z.div_euclid(146_097); let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153; let d = doy - (153 * mp + 2) / 5 + 1; let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d, rem / 3600, rem / 60 % 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_paths() {
        assert_eq!(safe_name("..\\..\\evil.exe"), "evil.exe");
        assert_eq!(safe_name("a:b?.txt"), "a_b_.txt");
        assert_eq!(safe_name(""), "Datei");
        let dir = std::env::temp_dir().join(format!("webox-files-test-{}", crate::jose::uuid()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "x.pdf"), dir.join("x.pdf"));
        std::fs::write(dir.join("x.pdf"), b"1").unwrap();
        assert_eq!(unique_path(&dir, "x.pdf"), dir.join("x (1).pdf"));
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn mime_and_sizes() {
        assert_eq!(mime_of("a.png", b"\x89PNG\r\n\x1a\n...."), "image/png");
        assert_eq!(mime_of("bericht.PDF", b"%PDF-1.7"), "application/pdf");
        assert_eq!(mime_of("x.unknown", b""), "application/octet-stream");
        // Size texts as Webex shows them ("399 Byte", "87 Byte", "126 KB").
        assert_eq!(size_text(399), "399 Byte");
        assert_eq!(size_text(87), "87 Byte");
        assert_eq!(size_text(1023), "1023 Byte");
        assert_eq!(size_text(1024), "1\u{a0}KB");
        assert_eq!(size_text(129_400), "126\u{a0}KB"); // size >> 10, truncated
        assert_eq!(size_text(1_048_575), "1023\u{a0}KB");
        assert_eq!(size_text(1_048_576), "1 MB");
        assert_eq!(size_text(1_200_000), "1.1 MB"); // always '.'
        assert_eq!(size_text(10 * 1_048_576), "10 MB"); // trailing zeros removed, the 0 of 10 stays
        assert_eq!(size_text(3 << 30), "3 GB");
        assert_eq!(size_text(0), "");
    }
    #[test]
    fn show_in_folder_arguments() {
        #[cfg(windows)]
        let p = std::path::Path::new(r"C:\Users\A B\Downloads\Spesen, final (1).xltx");
        #[cfg(windows)]
        assert_eq!(explorer_select_arg(p), "/select,\"C:\\Users\\A B\\Downloads\\Spesen, final (1).xltx\"");
        assert_eq!(file_uri(std::path::Path::new("/home/a b/x,y'z.txt")), "file:///home/a%20b/x%2Cy%27z.txt");
        assert_eq!(file_uri(std::path::Path::new(r"C:\a b\c.txt")), "file:///C:/a%20b/c.txt");
        assert_eq!(gvariant_uris(&["file:///a".into(), "file:///b'c".into()]), "['file:///a', 'file:///b\\'c']");
    }
    #[test]
    fn names_windows_would_change_are_made_safe() {
        // trailing dots and spaces in any order: Windows drops them and would start evil.exe
        assert_eq!(safe_name("evil.exe ."), "evil.exe");
        assert_eq!(safe_name("evil.exe . . "), "evil.exe");
        assert_eq!(safe_name("  bericht.pdf\u{a0}"), "bericht.pdf");
        assert_eq!(safe_name("..."), "Datei");
        // bidi / zero-width characters
        assert_eq!(safe_name("evil\u{202E}fdp.exe"), "evil_fdp.exe");
        assert_eq!(safe_name("a\u{200B}b\u{FEFF}c\u{2066}d\u{061C}.txt"), "a_b_c_d_.txt");
        // device names, with or without extension, any case
        for (n, want) in [("CON", "_CON"), ("nul.txt", "_nul.txt"), ("Com1.log", "_Com1.log"), ("lpt9", "_lpt9"), ("aux.tar.gz", "_aux.tar.gz"), ("COM1 .txt", "_COM1 .txt")] {
            assert_eq!(safe_name(n), want, "{n}");
        }
        for n in ["console.txt", "COM10", "COM0", "lpt", "nulx.pdf", "prn-liste.pdf"] { assert_eq!(safe_name(n), n, "{n}"); }
    }
    #[test]
    fn only_known_document_types_are_opened() {
        for n in ["a.pdf", "Bericht.DOCX", "x.tar", "clip.mp4", "foto.JPG", "notiz.txt", r"C:\Users\a\Downloads\b.xlsx", "/home/a/c.png"] { assert!(opens_directly(n), "{n}"); }
        for n in ["setup.EXE", "evil.exe .", "evil.exe . ", "evil\u{202E}fdp.exe", "a.lnk", "a.js", "a.hta", "a.docm", "a.html", "a.svg", "a.desktop",
            "a.unknown", "noext", "pdf", "a.pdf.exe", "a.scr", r"C:\x\evil.exe ."] { assert!(!opens_directly(n), "{n}"); }
    }
    #[test]
    fn downloads_get_the_mark_of_the_web() {
        let dir = std::env::temp_dir().join(format!("webox-motw-test-{}", crate::jose::uuid()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.pdf");
        std::fs::write(&f, b"%PDF").unwrap();
        mark_downloaded(&f);
        #[cfg(windows)]
        assert_eq!(std::fs::read_to_string(format!("{}:Zone.Identifier", f.display())).unwrap(), "[ZoneTransfer]\r\nZoneId=3\r\n");
        assert_eq!(std::fs::read(&f).unwrap(), b"%PDF"); // the file itself is unchanged
        mark_downloaded(&dir.join("missing.pdf"));
        assert!(!dir.join("missing.pdf").exists()); // no empty file made for the stream
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[cfg(unix)]
    #[test]
    fn private_temp_folders_refuse_shared_or_linked_ones() {
        use std::os::unix::fs::PermissionsExt;
        let uid = current_uid();
        let root = std::env::temp_dir().join(format!("webox-private-test-{}", crate::jose::uuid()));
        std::fs::create_dir_all(&root).unwrap();
        let fresh = root.join("fresh");
        private_dir(&fresh, uid).unwrap();
        assert_eq!(std::fs::metadata(&fresh).unwrap().permissions().mode() & 0o777, 0o700);
        private_dir(&fresh, uid).unwrap(); // already there and private: fine
        assert!(private_dir(&fresh, uid.wrapping_add(1)).is_err()); // another user's
        let open = root.join("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_dir(&open, uid).is_err());
        let link = root.join("link");
        std::os::unix::fs::symlink(&fresh, &link).unwrap();
        assert!(private_dir(&link, uid).is_err()); // a link to a private folder is still a link
        let file = root.join("file");
        std::fs::write(&file, b"x").unwrap();
        assert!(private_dir(&file, uid).is_err());
        let p = private_tmp("selftest").unwrap();
        assert!(p.is_dir() && std::fs::metadata(&p).unwrap().permissions().mode() & 0o077 == 0);
        let _ = std::fs::remove_dir(&p);
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn capture_names_use_the_originals_pattern() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(1_700_000_000), (2023, 11, 14, 22, 13, 20)); // date -u -d @1700000000
        let n = capture_name();
        assert!(n.len() == "yyyy-MM-dd_HH-mm-ss.png".len() && n.ends_with(".png") && &n[10..11] == "_", "{n}");
    }
    #[test]
    fn pasted_images_get_a_guid_name() {
        let n = pasted_image_name();
        let (stem, ext) = n.rsplit_once('.').unwrap();
        assert_eq!(ext, "PNG");
        let parts: Vec<usize> = stem.split('-').map(str::len).collect();
        assert_eq!(parts, [8, 4, 4, 4, 12], "{n}");
        assert!(stem.chars().all(|c| c == '-' || c.is_ascii_digit() || ('a'..='f').contains(&c)), "{n}");
    }
    #[test]
    fn policy_from_device_answer() {
        set_policy(&serde_json::json!({"policies": [{"name": "file-upload-size-limit", "value": "100"}, {"name": "bfs-upload-allow-desktop", "value": "true"}]}));
        assert_eq!(limit_bytes(), 100 * 1024 * 1024);
        assert!(uploads_allowed());
        set_policy(&serde_json::json!({}));
        assert_eq!(limit_bytes(), DEFAULT_LIMIT_MB * 1024 * 1024);
    }
}
