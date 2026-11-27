//! Browser meeting helper: a native-messaging host named `com.webex.meeting` (stdio, 4-byte little-endian length +
//! JSON) that the Webex browser extensions call to `ping` and to `join`/`launch` a meeting.
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::Path;

pub const HOST_NAME: &str = "com.webex.meeting"; // chrome.json / edge.json / firefox.json
const CHROME_ORIGINS: [&str; 1] = ["chrome-extension://jlhmfgmfgeifomenelglieieghnjghma/"];
const EDGE_ORIGINS: [&str; 3] = ["chrome-extension://cmihkeafcknlomclapaddfljaeegfbdl/", "chrome-extension://jlhmfgmfgeifomenelglieieghnjghma/", "chrome-extension://ikdddppdhmjcdfgilpnbkdeggoiicjgo/"];
const FIREFOX_IDS: [&str; 2] = ["ciscowebexstart@cisco.com", "ciscowebexstart1@cisco.com"];
const MAX_MESSAGE: u32 = 1024 * 1024; // browsers send at most 4 GB; 1 MB is plenty

/// Browsers start a host with the caller's origin (Chrome/Edge) or the manifest path and
/// extension id (Firefox) as arguments.
pub fn is_host_call(args: &[String]) -> bool {
    args.iter().skip(1).any(|a| a.starts_with("chrome-extension://") || FIREFOX_IDS.contains(&a.as_str()) || a == "--native-messaging")
}

fn read_msg(r: &mut impl Read) -> Option<Value> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).ok()?;
    let n = u32::from_le_bytes(len);
    if n > MAX_MESSAGE { return None; }
    let mut buf = vec![0u8; n as usize];
    r.read_exact(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

fn write_msg(w: &mut impl Write, v: &Value) -> std::io::Result<()> {
    let b = v.to_string().into_bytes();
    w.write_all(&(b.len() as u32).to_le_bytes())?;
    w.write_all(&b)?;
    w.flush()
}

/// What to do with one message; `open` gets the meeting link.
pub fn handle(msg: &Value, open: &mut dyn FnMut(&str)) -> Value {
    let kind = ["type", "cmd", "command", "action", "method"].iter().find_map(|k| msg[*k].as_str()).unwrap_or("").to_lowercase();
    let url = ["joinURL", "joinUrl", "url", "meetingUrl", "ReturnURL"].iter().find_map(|k| msg[*k].as_str().or_else(|| msg["data"][*k].as_str()));
    match (kind.as_str(), url) {
        ("ping", _) | ("version", _) => json!({"result": "success", "version": env!("CARGO_PKG_VERSION"), "installed": true}),
        (k, Some(u)) if k.contains("join") || k.contains("launch") || k.contains("start") || k.is_empty() => {
            // Parsed and handed on in its normalised form: no raw spaces, quotes or control characters reach the system's opener.
            match url::Url::parse(u).ok().filter(|p| u.starts_with("https://") && p.host_str().is_some_and(|h| !h.is_empty())) {
                Some(p) => { open(p.as_str()); json!({"result": "success", "joined": true}) }
                None => json!({"result": "failure", "reason": "only https meeting links are opened"}),
            }
        }
        _ => json!({"result": "failure", "reason": format!("unsupported message '{kind}'")}),
    }
}

pub fn run(config: &Path) {
    let (mut inp, mut out) = (std::io::stdin().lock(), std::io::stdout().lock());
    while let Some(msg) = read_msg(&mut inp) {
        let mut open = |u: &str| { record(config, u); crate::system::open(u); };
        if write_msg(&mut out, &handle(&msg, &mut open)).is_err() { break; }
    }
}

/// Records a joined meeting (Site, Date) in the MeetingHistory table of CiscoMeetings.db.
pub fn record(config: &Path, url: &str) {
    let p = config.join("meeting-history.json");
    let mut list: Vec<Value> = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let site = url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(url);
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    list.insert(0, json!({"Site": site, "Url": url, "Date": secs}));
    list.truncate(50);
    let _ = std::fs::write(p, serde_json::to_string_pretty(&list).unwrap_or_default());
}

pub fn history(config: &Path) -> Vec<(String, String)> {
    let list: Vec<Value> = std::fs::read_to_string(config.join("meeting-history.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    list.iter().map(|v| (v["Site"].as_str().unwrap_or("").to_string(), v["Url"].as_str().unwrap_or("").to_string())).collect()
}

fn manifest(firefox: bool, origins: &[&str]) -> Value {
    let exe = crate::system::launcher_path().display().to_string(); // by its profile path on Nix: the store path changes with every update
    let mut m = json!({"name": HOST_NAME, "description": "WebOx meeting helper", "path": exe, "type": "stdio"});
    if firefox { m["allowed_extensions"] = json!(FIREFOX_IDS) } else { m["allowed_origins"] = json!(origins) }
    m
}

/// Registers the helper for Chrome, Edge and Firefox (on Windows replacing an existing Webex helper's registration).
pub fn register(config: &Path, on: bool) -> Result<(), String> {
    let set: [(&str, bool, &[&str]); 3] = [("chrome", false, &CHROME_ORIGINS), ("edge", false, &EDGE_ORIGINS), ("firefox", true, &[])];
    #[cfg(windows)]
    for (b, ff, origins) in set {
        let file = config.join(format!("{b}.json"));
        let key = match b { "chrome" => r"HKCU\Software\Google\Chrome\NativeMessagingHosts", "edge" => r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts", _ => r"HKCU\Software\Mozilla\NativeMessagingHosts" };
        let key = format!(r"{key}\{HOST_NAME}");
        let mut c = std::process::Command::new("reg");
        if on {
            std::fs::write(&file, serde_json::to_string_pretty(&manifest(ff, origins)).unwrap()).map_err(|e| e.to_string())?;
            c.args(["add", &key, "/ve", "/t", "REG_SZ", "/d", &file.display().to_string(), "/f"]);
        } else { let _ = std::fs::remove_file(&file); c.args(["delete", &key, "/f"]); }
        let ok = c.output().map_err(|e| e.to_string())?.status.success();
        if on && !ok { return Err(format!("couldn't register for {b}")); }
    }
    #[cfg(not(windows))]
    {
        let _ = config;
        use std::path::PathBuf;
        let home = PathBuf::from(std::env::var_os("HOME").ok_or("no home folder")?);
        // macOS: each browser's folder under Application Support.
        #[cfg(target_os = "macos")]
        let dirs: Vec<(PathBuf, bool, &[&str])> = {
            let s = home.join("Library/Application Support");
            vec![(s.join("Google/Chrome/NativeMessagingHosts"), false, &set[0].2[..]), (s.join("Chromium/NativeMessagingHosts"), false, &set[0].2[..]),
                (s.join("Microsoft Edge/NativeMessagingHosts"), false, &set[1].2[..]), (s.join("Mozilla/NativeMessagingHosts"), true, &[][..])]
        };
        #[cfg(not(target_os = "macos"))]
        let cfg = crate::system::xdg_config_home().unwrap_or_else(|| home.join(".config"));
        #[cfg(not(target_os = "macos"))]
        let dirs: Vec<(PathBuf, bool, &[&str])> = vec![
            (cfg.join("google-chrome/NativeMessagingHosts"), false, &set[0].2[..]), (cfg.join("chromium/NativeMessagingHosts"), false, &set[0].2[..]),
            (cfg.join("microsoft-edge/NativeMessagingHosts"), false, &set[1].2[..]), (home.join(".mozilla/native-messaging-hosts"), true, &[][..]),
        ];
        for (d, ff, origins) in dirs {
            let file = d.join(format!("{HOST_NAME}.json"));
            if on {
                std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
                std::fs::write(&file, serde_json::to_string_pretty(&manifest(ff, origins)).unwrap()).map_err(|e| e.to_string())?;
            } else { let _ = std::fs::remove_file(&file); }
        }
    }
    Ok(())
}

/// Linux/Nix, at every start: rewrites the manifests when the helper is switched on, so they follow the launcher (a store path goes
/// stale after an update or a garbage collection). Only from a Nix build, so a development binary never takes the registration.
#[cfg(not(windows))]
pub fn refresh(config: &Path) {
    if crate::system::launcher_exe().starts_with("/nix/store") && crate::system::load_settings(config)["browser_helper"].as_bool() == Some(true) {
        let _ = register(config, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol() {
        let mut opened = vec![];
        let mut open = |u: &str| opened.push(u.to_string());
        assert_eq!(handle(&json!({"type": "ping"}), &mut open)["result"], "success");
        assert_eq!(handle(&json!({"type": "join", "joinURL": "https://site.example/j/1"}), &mut open)["result"], "success");
        assert_eq!(handle(&json!({"type": "join", "joinURL": "file:///x"}), &mut open)["result"], "failure");
        assert_eq!(opened, ["https://site.example/j/1"]);
        let mut buf = vec![];
        write_msg(&mut buf, &json!({"a": 1})).unwrap();
        assert_eq!(read_msg(&mut buf.as_slice()), Some(json!({"a": 1})));
        assert!(is_host_call(&["x".into(), "chrome-extension://abc/".into()]));
    }
}
