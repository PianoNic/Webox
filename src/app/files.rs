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

/// A file name that is safe on disk (no path parts, no reserved characters).
pub fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let s: String = base.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    let s = s.trim().trim_end_matches('.').to_string();
    if s.is_empty() || s == "." || s == ".." { "Datei".into() } else { s }
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

/// Files that would run code when "opened": these are only shown in their folder, never started from the chat.
pub fn runs_code(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    ["exe", "com", "bat", "cmd", "msi", "msp", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "scr", "pif", "lnk", "hta", "cpl", "jar", "reg", "appx", "msix"].contains(&ext.as_str())
}

/// Opens a downloaded file with its default program (Windows: the shell's open verb via explorer).
pub fn open_file(p: &Path) {
    if runs_code(&p.to_string_lossy()) { show_in_folder(p); return; }
    #[cfg(windows)] let _ = std::process::Command::new("explorer").arg(p).spawn();
    #[cfg(not(windows))] let _ = std::process::Command::new("xdg-open").arg(p).spawn();
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
        let p = std::path::Path::new(r"C:\Users\A B\Downloads\Spesen, final (1).xltx");
        assert_eq!(explorer_select_arg(p), "/select,\"C:\\Users\\A B\\Downloads\\Spesen, final (1).xltx\"");
        assert_eq!(file_uri(std::path::Path::new("/home/a b/x,y'z.txt")), "file:///home/a%20b/x%2Cy%27z.txt");
        assert_eq!(file_uri(std::path::Path::new(r"C:\a b\c.txt")), "file:///C:/a%20b/c.txt");
        assert_eq!(gvariant_uris(&["file:///a".into(), "file:///b'c".into()]), "['file:///a', 'file:///b\\'c']");
        assert!(runs_code("setup.EXE") && !runs_code("a.pdf"));
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
