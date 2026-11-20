//! Launcher behaviour (command line, host log, autostart) plus settings and opening links, for Windows and Linux
//! (including NixOS).
use serde_json::{json, Value};
use std::path::PathBuf;

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

pub fn config_dir() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    let base = base.unwrap_or_else(std::env::temp_dir);
    let d = base.join(if cfg!(windows) { "WebOx" } else { "webox" });
    // The app was called CollabHost before: carry its settings, sign-in and plugins over once.
    let old = base.join(if cfg!(windows) { "CollabHost" } else { "collabhost" });
    if !d.exists() && old.is_dir() { let _ = std::fs::rename(&old, &d); }
    let _ = std::fs::create_dir_all(&d);
    d
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
    let p = dir.join("settings.json");
    let _ = std::fs::write(&p, serde_json::to_string_pretty(v).unwrap_or_default());
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)); }
}

pub fn open(url: &str) {
    // Only web, mail, phone and meeting links are handed to the system.
    if !["https://", "http://", "sip:", "tel:", "mailto:"].iter().any(|p| url.starts_with(p)) { return; }
    #[cfg(windows)] let _ = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(not(windows))] let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

pub fn open_folder(p: &std::path::Path) {
    #[cfg(windows)] let _ = std::process::Command::new("explorer").arg(p).spawn();
    #[cfg(not(windows))] let _ = std::process::Command::new("xdg-open").arg(p).spawn();
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

/// Start with the computer: the Run key on Windows, an XDG autostart entry on Linux.
pub fn set_autostart(on: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        // In-process registry calls (no spawned `reg add` / `reg delete` to wait for).
        let ok = reg_run_value("WebOx", on.then(|| format!("\"{}\" {AUTOSTART_ARGS}", exe.display())).as_deref());
        if ok || !on { Ok(()) } else { Err("couldn't change the Run key".into()) }
    }
    #[cfg(not(windows))]
    {
        let _ = exe;
        let dir = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))).ok_or("no home folder")?.join("autostart");
        let file = dir.join("webox.desktop");
        if !on { let _ = std::fs::remove_file(&file); return Ok(()); }
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        // A Nix store path changes with every update: start through PATH then (the Nix profile / NixOS system put `webox` there).
        let exec = match launcher_exe() { p if p.starts_with("/nix/store") => "webox".to_string(), p => format!("\"{}\"", p.display()) };
        std::fs::write(&file, format!("[Desktop Entry]\nType=Application\nName=Webox\nIcon=webox\nExec={exec} --minimized --autostarted\nX-GNOME-Autostart-enabled=true\n"))
            .map_err(|e| e.to_string())
    }
}

/// The program to start for autostart / browser helpers: the Nix wrapper script when there is one (`WEBOX_WRAPPER`, set by the
/// flake's wrapper; the unwrapped binary would miss the library path), else this executable.
pub fn launcher_exe() -> PathBuf {
    std::env::var_os("WEBOX_WRAPPER").map(PathBuf::from).filter(|p| p.is_file())
        .or_else(|| std::env::current_exe().ok()).unwrap_or_else(|| PathBuf::from("webox"))
}

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
}
