//! Launcher behaviour (command line, host log, autostart) plus settings and opening links, for Windows and Linux
//! (including NixOS).
use serde_json::{json, Value};
use std::path::PathBuf;

#[cfg(windows)]
pub const AUTOSTART_ARGS: &str = "/minimized /autostartedWithWindows=true"; // the arguments Webex uses for autostart
pub const HOST_LOG: &str = "webox-host-log.txt";

#[derive(Default, Debug)]
pub struct Args { pub minimized: bool, pub autostarted: bool, pub protocol_uri: Option<String>, pub screenshot: Option<PathBuf>, pub signin: Option<String>, pub sample: bool }

pub fn args() -> Args {
    let v: Vec<String> = std::env::args().collect();
    let mut a = Args::default();
    let mut i = 1;
    while i < v.len() {
        let s = v[i].as_str();
        let low = s.to_lowercase();
        if low == "/minimized" || low == "--minimized" { a.minimized = true }
        else if low.starts_with("/autostartedwithwindows") || low == "--autostarted" { a.autostarted = !low.ends_with("=false") }
        else if low == "/protocoluri" || low == "--protocol-uri" { i += 1; a.protocol_uri = v.get(i).cloned() }
        else if let Some(u) = s.strip_prefix("/protocolUri=") { a.protocol_uri = Some(u.into()) }
        else if s == "--screenshot" { i += 1; a.screenshot = v.get(i).map(PathBuf::from) }
        // Render offline with the built-in sample spaces (for layout comparison, no network).
        else if low == "--sample" { a.sample = true }
        // Start the native sign-in automatically once the window exists. Optional email argument;
        // without one the saved sign-in email (webexusername/email setting) is used.
        else if low == "--signin" { a.signin = Some(v.get(i + 1).map(String::as_str).filter(|e| !e.starts_with('-')).unwrap_or("").into()) }
        else if s.contains("://") { a.protocol_uri = Some(s.into()) }
        i += 1;
    }
    a
}

/// The space a link like `collabhost://space/<id>` or `webexteams://im?space=<id>` points at.
pub fn space_from_uri(uri: &str) -> Option<String> {
    let rest = uri.split_once("://")?.1;
    if let Some(q) = rest.split_once("space=") { return Some(q.1.split('&').next()?.to_string()); }
    rest.strip_prefix("space/").map(|s| s.trim_end_matches('/').to_string())
}

/// `$XDG_CONFIG_HOME` (only when absolute, as the spec demands), else `~/.config`.
#[cfg(not(windows))]
pub fn xdg_config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").filter(|h| !h.is_empty()).map(|h| PathBuf::from(h).join(".config")))
}

/// The settings folder (tokens, plugin settings, the webview profile). Owner-only (0700) on unix. Worked out once per run:
/// the last-resort fallback is a fresh folder, which every caller must share.
pub fn config_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        #[cfg(windows)]
        let base = std::env::var_os("APPDATA").map(PathBuf::from);
        #[cfg(target_os = "macos")]
        let base = crate::platform::macos::app_support(); // ~/Library/Application Support/WebOx
        #[cfg(not(any(windows, target_os = "macos")))]
        let base = xdg_config_home();
        let Some(base) = base else { return fallback_config_dir() };
        let d = base.join(if cfg!(any(windows, target_os = "macos")) { "WebOx" } else { "webox" });
        // The app was called CollabHost before: carry its settings, sign-in and plugins over once.
        let old = base.join(if cfg!(any(windows, target_os = "macos")) { "CollabHost" } else { "collabhost" });
        if !d.exists() && old.is_dir() { let _ = std::fs::rename(&old, &d); }
        let _ = create_private_dir(&d);
        // An existing folder (older versions made it 0755) is tightened; a symlinked one (dotfiles) is followed.
        #[cfg(unix)] {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if let Ok(m) = std::fs::metadata(&d) { if m.is_dir() && m.mode() & 0o077 != 0 && my_uid().is_none_or(|u| m.uid() == u) { let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)); } }
        }
        d
    }).clone()
}

/// No APPDATA / XDG_CONFIG_HOME / HOME. Unix: never a predictable shared path another user could prepare; first
/// `$XDG_RUNTIME_DIR/webox`, then `<tmp>/webox-<uid>`, each only when it is a real folder of ours, else a fresh random one.
fn fallback_config_dir() -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let tmp = std::env::temp_dir();
        let named = std::env::var_os("XDG_RUNTIME_DIR").map(|r| PathBuf::from(r).join("webox")).into_iter()
            .chain(my_uid().map(|u| tmp.join(format!("webox-{u}"))));
        for d in named {
            let _ = create_private_dir(&d);
            if owned_private_dir(&d) { return d; }
        }
        let fresh = || tmp.join(format!("webox-{:016x}", rand::random::<u64>()));
        for _ in 0..8 {
            // create (not recursive) fails on anything already there, so the folder is new and ours
            let d = fresh();
            if std::fs::DirBuilder::new().mode(0o700).create(&d).is_ok() { return d; }
        }
        fresh() // eight collisions in a row: an unguessable name all the same
    }
    #[cfg(not(unix))]
    {
        let d = std::env::temp_dir().join("WebOx");
        let _ = std::fs::create_dir_all(&d);
        d
    }
}

/// This process' user id (Linux; other unixes have no libc dependency here and skip the owner checks that need it).
#[cfg(unix)]
fn my_uid() -> Option<u32> {
    #[cfg(target_os = "linux")] { Some(unsafe { libc::getuid() }) }
    #[cfg(not(target_os = "linux"))] { None }
}

/// True when `d` is a real folder (not a symlink), owned by us and closed to others (tightened to 0700 if needed).
/// Without a known uid only a folder that is already owner-only counts.
#[cfg(unix)]
fn owned_private_dir(d: &std::path::Path) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let Ok(m) = std::fs::symlink_metadata(d) else { return false };
    if !m.file_type().is_dir() { return false; }
    match my_uid() {
        Some(u) if m.uid() == u => m.mode() & 0o077 == 0 || std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700)).is_ok(),
        Some(_) => false,
        None => m.mode() & 0o077 == 0,
    }
}

/// `create_dir_all`, with new folders owner-only (0700) on unix.
pub fn create_private_dir(d: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)] { use std::os::unix::fs::DirBuilderExt; std::fs::DirBuilder::new().recursive(true).mode(0o700).create(d) }
    #[cfg(not(unix))] { std::fs::create_dir_all(d) }
}

/// Writes (create / truncate) a file that is owner-only (0600) on unix from the moment it exists; an existing file is tightened too.
pub fn write_private(p: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; o.mode(0o600); }
    let mut f = o.open(p)?;
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; f.set_permissions(std::fs::Permissions::from_mode(0o600))?; }
    f.write_all(bytes)
}

/// Writes the host log on every start: command line and start mode.
pub fn host_log(dir: &std::path::Path, a: &Args) {
    let cmd: Vec<String> = std::env::args().filter(|s| !s.eq_ignore_ascii_case(SKIP_DE_ELEVATION)).collect();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let lines = [
        format!("Command line: {} ", cmd.join(" ")),
        String::new(),
        format!("Autostarted: {}, minimized: {}", a.autostarted, a.minimized),
        String::new(),
        "Started in the launcher mode.".to_string(),
        format!("{} version: {}", cmd[0], env!("CARGO_PKG_VERSION")),
    ];
    let secret = [local_data_dir(), std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default()];
    let text: String = lines.iter().map(|l| log_line(l, now, &secret)).collect();
    let _ = std::fs::write(dir.join(HOST_LOG), utf16le(&text));
}

/// Appends lines to the host log in the same format (time stamp, masking, UTF-16LE, size cap).
pub fn host_log_append(dir: &std::path::Path, lines: &[String]) {
    if lines.is_empty() { return; }
    let p = dir.join(HOST_LOG);
    let mut bytes = std::fs::read(&p).unwrap_or_default();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secret = [local_data_dir(), std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default()];
    let text: String = lines.iter().map(|l| log_line(l, now, &secret)).collect();
    bytes.extend(utf16le(&text));
    bytes.truncate(LOG_MAX_CHARS * 2);
    let _ = std::fs::write(p, bytes);
}

/// Policy name read from HKLM\SOFTWARE\Policies\CiscoWebex.
pub const SIDE_LOADING_POLICY: &str = "enableDllSideLoadingProtection";

/// 1 = on, 0 = off. Not set: off (Webox isn't code-signed). Windows: HKLM\SOFTWARE\Policies\WebOx;
/// Linux/NixOS: /etc/webox/policy.json.
pub fn side_loading_protection() -> bool {
    // Read in-process (RegGetValueW): spawning `reg query` here cost 0.1-0.9 s of startup, plus a console host in release builds.
    #[cfg(windows)]
    {
        reg_policy_value(r"SOFTWARE\Policies\WebOx", SIDE_LOADING_POLICY) == Some(1)
    }
    #[cfg(not(windows))]
    {
        std::fs::read_to_string("/etc/webox/policy.json").ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .is_some_and(|v| v[SIDE_LOADING_POLICY].as_u64() == Some(1))
    }
}

pub const SKIP_DE_ELEVATION: &str = "/skip-de-elevation"; // never logged
pub const LOG_MAX_CHARS: usize = 32768; // the host log buffer holds 32768 UTF-16 units

fn local_data_dir() -> String {
    std::env::var("LOCALAPPDATA").or_else(|_| std::env::var("XDG_DATA_HOME")).or_else(|_| std::env::var("HOME").map(|h| format!("{h}/.local/share"))).unwrap_or_default()
}

/// One host-log line: UTC time "yyyy-MM-dd HH:mm:ss.mmm ", the text with the local data folder and user name masked by
/// as many '*', then CRLF. Empty lines get no time.
pub fn log_line(text: &str, now: std::time::Duration, secret: &[String]) -> String {
    let mut t = text.to_string();
    for s in secret.iter().filter(|s| !s.is_empty()) { t = mask(&t, s); }
    if t.is_empty() { return "\r\n".into(); }
    let secs = now.as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}.{:03} {t}\r\n", rem / 3600, rem % 3600 / 60, rem % 60, now.subsec_millis())
}

fn mask(text: &str, needle: &str) -> String {
    let (lt, ln) = (text.to_lowercase(), needle.to_lowercase());
    if ln.is_empty() || lt.len() != text.len() { return text.to_string(); }
    let mut out = String::new();
    let mut i = 0;
    while let Some(p) = lt[i..].find(&ln) {
        out.push_str(&text[i..i + p]);
        out.push_str(&"*".repeat(needle.chars().count()));
        i += p + ln.len();
    }
    out + &text[i..]
}

/// Today's date as "YYYY-MM-DD" (UTC), and n days before it.
pub fn today() -> String { days_ago(0) }
pub fn days_ago(n: i64) -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (y, m, d) = civil(secs.div_euclid(86_400) - n);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Date → days since 1970-01-01 (the inverse of `civil`).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
fn civil(z: i64) -> (i64, u32, u32) { // days since 1970-01-01 → date (Howard Hinnant's algorithm)
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The host log is UTF-16LE without a byte-order mark.
fn utf16le(s: &str) -> Vec<u8> { s.encode_utf16().take(LOG_MAX_CHARS).flat_map(u16::to_le_bytes).collect() }

pub fn load_settings(dir: &std::path::Path) -> Value {
    std::fs::read_to_string(dir.join("settings.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({}))
}

pub fn save_settings(dir: &std::path::Path, v: &Value) {
    let _ = write_private(&dir.join("settings.json"), serde_json::to_string_pretty(v).unwrap_or_default().as_bytes());
}

pub fn open(url: &str) {
    // Only web, mail, phone and meeting links are handed to the system.
    if !["https://", "http://", "sip:", "tel:", "mailto:"].iter().any(|p| url.starts_with(p)) { return; }
    #[cfg(windows)] spawn_detached(std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]));
    #[cfg(target_os = "macos")] crate::platform::macos::open(url);
    #[cfg(not(any(windows, target_os = "macos")))] spawn_detached(std::process::Command::new("xdg-open").arg(url));
}

pub fn open_folder(p: &std::path::Path) {
    #[cfg(windows)] spawn_detached(std::process::Command::new("explorer").arg(p));
    #[cfg(target_os = "macos")] crate::platform::macos::open(p);
    #[cfg(not(any(windows, target_os = "macos")))] spawn_detached(std::process::Command::new("xdg-open").arg(p));
}

/// Gives a child process the caller's own `LD_LIBRARY_PATH` back: the Nix wrapper prefixes its libraries (`WEBOX_ORIG_LD_LIBRARY_PATH`
/// is the value from before that), and `xdg-open`, the browser, `secret-tool` ... must not load them.
pub fn restore_env(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(not(windows))]
    if let Some(orig) = std::env::var_os("WEBOX_ORIG_LD_LIBRARY_PATH") {
        if orig.is_empty() { cmd.env_remove("LD_LIBRARY_PATH"); } else { cmd.env("LD_LIBRARY_PATH", orig); }
    }
    cmd
}

/// Starts `cmd` and reaps it on a thread of its own: a dropped `Child` that is never waited for stays a zombie until we exit.
pub fn spawn_detached(cmd: &mut std::process::Command) {
    if let Ok(mut c) = restore_env(cmd).spawn() { let _ = std::thread::Builder::new().name("webox-reap".into()).spawn(move || { let _ = c.wait(); }); }
}

/// Linux: makes the calling thread part of the D-Bus runtime. zbus / ashpd (so rfd's portal dialogs) spawn onto "the current tokio
/// runtime" and panic on a plain thread. Hold the guard while the dialog is open. None elsewhere.
pub fn enter_dbus_rt() -> Option<tokio::runtime::EnterGuard<'static>> {
    #[cfg(target_os = "linux")]
    { Some(crate::platform::linux::rt().enter()) }
    #[cfg(not(target_os = "linux"))]
    { None }
}

/// The value of `key` (e.g. `XDG_DOWNLOAD_DIR`) in the text of `user-dirs.dirs` (`KEY="$HOME/Herunterladen"`), absolute; None when
/// unset or pointing at `$HOME` itself (the spec's way to switch a folder off).
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn parse_user_dir(text: &str, key: &str, home: &std::path::Path) -> Option<PathBuf> {
    let v = text.lines().map(str::trim).filter(|l| !l.starts_with('#')).find_map(|l| l.strip_prefix(key)?.trim_start().strip_prefix('='))?;
    let v = v.trim().trim_matches('"');
    let p = match v.strip_prefix("$HOME") { Some(r) => home.join(r.trim_start_matches('/')), None => PathBuf::from(v) };
    (p.is_absolute() && p != home).then_some(p)
}

/// The OS download folder: Windows `%USERPROFILE%\Downloads`; elsewhere `XDG_DOWNLOAD_DIR` of `~/.config/user-dirs.dirs` (localised,
/// e.g. `Herunterladen`), else `~/Downloads`.
pub fn default_download_dir() -> PathBuf {
    #[cfg(windows)]
    if let Ok(p) = std::env::var("USERPROFILE") { return PathBuf::from(p).join("Downloads"); }
    #[cfg(not(windows))]
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_absolute()) {
        let text = xdg_config_home().and_then(|c| std::fs::read_to_string(c.join("user-dirs.dirs")).ok()).unwrap_or_default();
        return parse_user_dir(&text, "XDG_DOWNLOAD_DIR", &home).unwrap_or_else(|| home.join("Downloads"));
    }
    PathBuf::from("Downloads")
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain([0]).collect() }

/// A policy value under HKLM (64-bit view, as `reg query` read it): a REG_DWORD, or a REG_SZ holding a number
/// ("0x1" hex like reg printed it, or decimal). `None` when the key / value doesn't exist.
#[cfg(windows)]
fn reg_policy_value(key: &str, name: &str) -> Option<u32> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
    let (k, n) = (wide(key), wide(name));
    let mut dw = 0u32;
    let mut len = 4u32;
    let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, PCWSTR(k.as_ptr()), PCWSTR(n.as_ptr()), RRF_RT_REG_DWORD, None, Some(&mut dw as *mut u32 as *mut _), Some(&mut len)) };
    if r.is_ok() { return Some(dw); }
    let mut buf = [0u16; 64];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, PCWSTR(k.as_ptr()), PCWSTR(n.as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) };
    if r.is_err() { return None; }
    let s = String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]);
    let s = s.trim();
    s.strip_prefix("0x").map_or_else(|| s.parse().ok(), |h| u32::from_str_radix(h, 16).ok())
}

/// Sets (`Some`) or deletes (`None`) a string value of HKCU\...\Run. True on success.
#[cfg(windows)]
fn reg_run_value(name: &str, value: Option<&str>) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
    let (k, n) = (wide(r"Software\Microsoft\Windows\CurrentVersion\Run"), wide(name));
    unsafe {
        match value {
            Some(v) => {
                let d = wide(v);
                RegSetKeyValueW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), PCWSTR(n.as_ptr()), REG_SZ.0, Some(d.as_ptr() as *const _), (d.len() * 2) as u32).is_ok()
            }
            None => RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), PCWSTR(n.as_ptr())).is_ok(),
        }
    }
}

/// Held for the whole run: the installer (packaging/windows/webox.iss, `AppMutex`) asks to close Webox while it exists, and a
/// second start sees it and hands over to the running copy (main.rs). True when another Webox already holds it.
#[cfg(windows)]
pub fn hold_app_mutex() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    let name = wide("WeboxAppMutex");
    // The handle is never closed; Windows releases it when the process ends.
    let _ = unsafe { windows::Win32::System::Threading::CreateMutexW(None, false, PCWSTR(name.as_ptr())) };
    (unsafe { GetLastError() }) == ERROR_ALREADY_EXISTS
}

/// Whether Webox starts with the computer: the Run key on Windows (the installer's "start with Windows" writes the same value), an
/// XDG autostart entry on Linux (`webox.desktop` in the user's autostart folder, else the first one of `XDG_CONFIG_DIRS`, e.g.
/// from the NixOS module or home-manager; `Hidden=true` switches it off). `None` elsewhere.
pub fn autostart_registered() -> Option<bool> {
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
        let (k, n) = (wide(r"Software\Microsoft\Windows\CurrentVersion\Run"), wide("WebOx"));
        let mut len = 0u32;
        Some(unsafe { RegGetValueW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), PCWSTR(n.as_ptr()), RRF_RT_REG_SZ, None, None, Some(&mut len)) }.is_ok())
    }
    #[cfg(target_os = "linux")]
    {
        // The first folder that has the file decides (a user file overrides a system one, also by hiding it).
        let text = autostart_dirs().iter().find_map(|d| std::fs::read_to_string(d.join(AUTOSTART_FILE)).ok());
        Some(text.is_some_and(|t| !desktop_hidden(&t)))
    }
    #[cfg(target_os = "macos")]
    { Some(crate::platform::macos::login::registered()) }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    None
}

#[cfg(not(windows))]
#[cfg_attr(target_os = "macos", allow(dead_code))]
const AUTOSTART_FILE: &str = "webox.desktop";

/// The autostart folders in the order of precedence: the user's, then `XDG_CONFIG_DIRS` (default `/etc/xdg`).
#[cfg(not(windows))]
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn autostart_dirs() -> Vec<PathBuf> {
    let dirs = std::env::var_os("XDG_CONFIG_DIRS").filter(|v| !v.is_empty()).unwrap_or_else(|| "/etc/xdg".into());
    xdg_config_home().into_iter().chain(std::env::split_paths(&dirs).filter(|p| p.is_absolute())).map(|p| p.join("autostart")).collect()
}

/// `Hidden=true` (or the GNOME switch `X-GNOME-Autostart-enabled=false`) in the text of a desktop entry.
#[cfg(not(windows))]
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn desktop_hidden(text: &str) -> bool {
    text.lines().map(str::trim).any(|l| l.eq_ignore_ascii_case("Hidden=true") || l.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false"))
}

/// Start with the computer: the Run key on Windows, an XDG autostart entry on Linux, a login item on macOS
/// (`SMAppService.mainApp`, else a LaunchAgent: platform/macos/login.rs).
pub fn set_autostart(on: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    { crate::platform::macos::login::set(on) }
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        // In-process registry calls (no spawned `reg add` / `reg delete` to wait for).
        let ok = reg_run_value("WebOx", on.then(|| format!("\"{}\" {AUTOSTART_ARGS}", exe.display())).as_deref());
        if ok || !on { Ok(()) } else { Err("couldn't change the Run key".into()) }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let dir = xdg_config_home().ok_or("no home folder")?.join("autostart");
        let file = dir.join(AUTOSTART_FILE);
        // home-manager / the NixOS module own a symlink into the read-only store: leave it alone when switching on.
        let is_link = file.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink());
        if on && is_link && file.exists() { return Ok(()); }
        if !on {
            // A system entry (/etc/xdg/autostart) or a store symlink can't be deleted: a user file with Hidden=true overrides it.
            let system = autostart_dirs().iter().skip(1).any(|d| std::fs::read_to_string(d.join(AUTOSTART_FILE)).is_ok_and(|t| !desktop_hidden(&t)));
            if !system && !is_link { let _ = std::fs::remove_file(&file); return Ok(()); }
            // A live symlink is home-manager's: replacing it would block its next activation, so it stays in charge.
            if is_link && file.exists() { return Err(crate::trq!("Autostart is managed by Home Manager (programs.webox.autostart)").into_owned()); }
            if is_link { let _ = std::fs::remove_file(&file); } // dangling link (garbage-collected store path)
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            return std::fs::write(&file, "[Desktop Entry]\nType=Application\nName=Webox\nHidden=true\n").map_err(|e| e.to_string());
        }
        if is_link { let _ = std::fs::remove_file(&file); } // dangling link (garbage-collected store path)
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        // A Nix store path changes with every update: use the profile's path, else start through PATH (the Nix profile / NixOS system put
        // `webox` there). Either way quoted and escaped for the Exec key.
        let exec = match launcher_exe() { p if p.starts_with("/nix/store") => stable_launcher().map_or("webox".to_string(), |s| desktop_exec_arg(&s.to_string_lossy())), p => desktop_exec_arg(&p.to_string_lossy()) };
        std::fs::write(&file, format!("[Desktop Entry]\nType=Application\nName=Webox\nIcon=webox\nExec={exec} --minimized --autostarted\nX-GNOME-Autostart-enabled=true\n"))
            .map_err(|e| e.to_string())
    }
}

/// One quoted `Exec=` argument per the Desktop Entry spec: `"`, `` ` ``, `$` and `\` get a backslash inside the quotes, then the
/// value's own string escaping doubles every backslash, and `%` (a field code) becomes `%%`. A path with a line break or other
/// control character can't be written safely: `webox` (from PATH) then.
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
fn desktop_exec_arg(p: &str) -> String {
    if p.chars().any(char::is_control) { return "webox".into(); }
    let mut s = String::from("\"");
    for c in p.chars() {
        match c { '"' | '`' | '$' => { s.push_str("\\\\"); s.push(c); } '\\' => s.push_str("\\\\\\\\"), '%' => s.push_str("%%"), c => s.push(c) }
    }
    s.push('"');
    s
}

/// The program to start for autostart / browser helpers: the Nix wrapper script when there is one (`WEBOX_WRAPPER`, set by the
/// flake's wrapper; the unwrapped binary would miss the library path), else this executable.
pub fn launcher_exe() -> PathBuf {
    std::env::var_os("WEBOX_WRAPPER").map(PathBuf::from).filter(|p| p.is_file())
        .or_else(|| std::env::current_exe().ok()).unwrap_or_else(|| PathBuf::from("webox"))
}

/// `/nix/store/<hash>-<name>-<version>/<rest>` -> (`<name>`, `<rest>`): which program a store path is, regardless of its version.
#[cfg(not(windows))]
fn nix_program(p: &std::path::Path) -> Option<(String, PathBuf)> {
    let mut it = p.strip_prefix("/nix/store").ok()?.components();
    let name = it.next()?.as_os_str().to_str()?.get(33..)?; // "<32 chars hash>-"
    let end = name.char_indices().find(|&(i, c)| c == '-' && name[i + 1..].starts_with(|d: char| d.is_ascii_digit())).map_or(name.len(), |(i, _)| i);
    Some((name[..end].to_string(), it.as_path().to_path_buf()))
}

/// Whether `cand` is (a link to) the same program as `cur`, possibly an other version of it.
#[cfg(not(windows))]
fn same_program(cand: &std::path::Path, cur: &std::path::Path) -> bool {
    let (Ok(a), Ok(b)) = (cand.canonicalize(), cur.canonicalize()) else { return false };
    a == b || nix_program(&a).is_some_and(|x| nix_program(&b) == Some(x))
}

/// The launcher by a path that survives updates and garbage collection: when `launcher_exe()` is in the Nix store, the first of the
/// profile paths (NixOS user / system profile, nix-env, `nix profile`) that exists and is the same program. None elsewhere.
pub fn stable_launcher() -> Option<PathBuf> {
    #[cfg(not(windows))]
    {
        let cur = launcher_exe();
        if !cur.starts_with("/nix/store") { return None; }
        let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
        let mut cands: Vec<PathBuf> = vec![];
        if let Some(u) = std::env::var_os("USER").filter(|u| !u.is_empty()) { cands.push(PathBuf::from("/etc/profiles/per-user").join(u).join("bin/webox")); }
        if let Some(h) = &home { cands.push(h.join(".nix-profile/bin/webox")); }
        cands.push("/run/current-system/sw/bin/webox".into());
        if let Some(h) = &home { cands.push(h.join(".local/state/nix/profile/bin/webox")); }
        cands.into_iter().find(|c| c.is_file() && same_program(c, &cur))
    }
    #[cfg(windows)]
    None
}

/// `launcher_exe()` by its stable path when it has one.
pub fn launcher_path() -> PathBuf { stable_launcher().unwrap_or_else(launcher_exe) }

#[cfg(test)]
mod tests {
    #[test]
    fn uris() {
        assert_eq!(super::space_from_uri("webexteams://im?space=abc&x=1").as_deref(), Some("abc"));
        assert_eq!(super::space_from_uri("collabhost://space/xyz").as_deref(), Some("xyz"));
        assert_eq!(super::space_from_uri("nothing"), None);
        let t = std::time::Duration::from_millis(1_794_207_535_957); // 2026-11-09 06:58:55.957 UTC
        let l = super::log_line(r"Loaded C:\Users\you\AppData\Local\x", t, &[r"C:\Users\you\AppData\Local".into()]);
        assert_eq!(l, "2026-11-09 06:58:55.957 Loaded **************************\\x\r\n");
        assert_eq!(super::log_line("", t, &[]), "\r\n");
    }

    #[cfg(not(windows))]
    #[test]
    fn xdg_files() {
        use std::path::Path;
        let home = Path::new("/home/a");
        let t = "# c\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_DOWNLOAD_DIR=\"$HOME/Herunterladen\"\n";
        assert_eq!(super::parse_user_dir(t, "XDG_DOWNLOAD_DIR", home), Some("/home/a/Herunterladen".into()));
        assert_eq!(super::parse_user_dir("XDG_DOWNLOAD_DIR=\"/data/dl\"", "XDG_DOWNLOAD_DIR", home), Some("/data/dl".into()));
        assert_eq!(super::parse_user_dir("XDG_DOWNLOAD_DIR=\"$HOME/\"", "XDG_DOWNLOAD_DIR", home), None);
        assert_eq!(super::parse_user_dir(t, "XDG_MUSIC_DIR", home), None);
        assert!(super::desktop_hidden("[Desktop Entry]\nHidden=true\n") && !super::desktop_hidden("[Desktop Entry]\nHidden=false\n"));
        assert_eq!(super::nix_program(Path::new("/nix/store/0123456789abcdfghijklmnpqrsvwxyz-webox-0.3.1/bin/webox")), Some(("webox".into(), "bin/webox".into())));
    }

    #[test]
    fn desktop_exec_quoting() {
        assert_eq!(super::desktop_exec_arg("/opt/webox/webox"), "\"/opt/webox/webox\"");
        // On disk `\\"` / `\\$` / `\\\\` (string escaping over the quoting rule), `%%` for a literal percent.
        assert_eq!(super::desktop_exec_arg(r#"/a "b"/$x`y`\z%"#), r#""/a \\"b\\"/\\$x\\`y\\`\\\\z%%""#);
        assert_eq!(super::desktop_exec_arg("/a\nExec=evil"), "webox");
    }

    #[test]
    fn private_files() {
        let d = std::env::temp_dir().join(format!("webox-test-private-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        super::create_private_dir(&d.join("sub")).unwrap();
        super::write_private(&d.join("sub").join("f.json"), b"{}").unwrap();
        assert_eq!(std::fs::read(d.join("sub").join("f.json")).unwrap(), b"{}");
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&d).unwrap().permissions().mode() & 0o777, 0o700);
            assert_eq!(std::fs::metadata(d.join("sub").join("f.json")).unwrap().permissions().mode() & 0o777, 0o600);
            assert!(super::owned_private_dir(&d) || super::my_uid().is_none());
            // a symlink to a folder is not taken as the fallback folder
            std::os::unix::fs::symlink(&d, d.with_extension("link")).unwrap();
            assert!(!super::owned_private_dir(&d.with_extension("link")));
            let _ = std::fs::remove_file(d.with_extension("link"));
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
