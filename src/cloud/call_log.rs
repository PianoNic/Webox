//! The call log: always on (no environment variable), so every real call leaves evidence.
//!
//! What goes in: call events only. Every `call:` line of the debug log (offer / answer summaries without addresses,
//! ICE / DTLS / peer states, ROAP messages, Locus states with their reason fields, errors, the end reason) and the Locus
//! media keep-alive. Never tokens, names, addresses or message content: the `call:` lines are written for that (`scrub`,
//! `sdp::summary`, enum-like reason values only).
//!
//! Where: `call.log` in the settings folder (`%APPDATA%\WebOx`, `~/.config/webox`, …). At about 1 MB it is renamed to
//! `call.log.1` (the previous `.1` goes), so at most two files of about 1 MB each are kept.
//!
//! Format: one line per event, `2026-11-25T10:42:25.174Z call: <event>`.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The size at which `call.log` is rotated.
pub const MAX_BYTES: u64 = 1_000_000;
pub const FILE: &str = "call.log";

static LOCK: Mutex<()> = Mutex::new(());

/// The call log's path.
pub fn path() -> PathBuf { crate::system::config_dir().join(FILE) }

/// Appends one line (a timestamp in front) to the call log; failures are ignored (logging never breaks a call).
pub fn write(line: &str) {
    if cfg!(test) { return; } // the loopback tests' calls are not the user's
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    let _g = LOCK.lock();
    append(&path(), &format!("{} {}", stamp(now), clean(line)), MAX_BYTES);
}

/// `line` on one line: no line breaks (an SDP or an error text can't split a record), at most 600 characters.
pub fn clean(line: &str) -> String {
    line.chars().map(|c| if c == '\r' || c == '\n' { ' ' } else { c }).take(600).collect()
}

/// UTC time with milliseconds, `2026-11-25T10:42:25.174Z`.
pub fn stamp(ms: i64) -> String { crate::cloud::meetcall::iso_utc(ms) }

/// Appends `line` to `file`, first rotating it to `<file>.1` when it holds `max` bytes or more.
pub fn append(file: &Path, line: &str, max: u64) {
    if std::fs::metadata(file).is_ok_and(|m| m.len() >= max) {
        let old = file.with_extension("log.1");
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(file, &old);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_log_rotates_and_keeps_two_files() {
        let dir = std::env::temp_dir().join(format!("webox-call-log-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("call.log");
        let _ = std::fs::remove_file(&f);
        let _ = std::fs::remove_file(dir.join("call.log.1"));
        for i in 0..30 { append(&f, &format!("line {i:03} xxxxxxxxxxxxxxxxxxxx"), 200); }
        let cur = std::fs::read_to_string(&f).unwrap();
        let old = std::fs::read_to_string(dir.join("call.log.1")).unwrap();
        assert!(cur.len() < 240 && old.len() < 240, "each file stays about at the limit");
        assert!(cur.contains("line 029"), "the newest line is in the current file");
        assert!(!old.contains("line 029") && old.contains("line 0"), "the previous file holds older lines");
        assert!(!dir.join("call.log.2").exists(), "only two files");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn call_log_lines_are_single_lines_with_utc_milliseconds() {
        assert_eq!(clean("a\r\nb\nc"), "a  b c");
        assert_eq!(clean(&"x".repeat(900)).len(), 600);
        assert_eq!(stamp(1_764_067_345_174), "2025-11-25T10:42:25.174Z");
        assert_eq!(stamp(1_764_067_345_004), "2025-11-25T10:42:25.004Z");
        assert_eq!(stamp(0), "1970-01-01T00:00:00.000Z");
    }
}
