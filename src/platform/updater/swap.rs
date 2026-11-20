//! Replacing the program file (Windows): check the downloaded zip against its SHA-256, unpack `webox.exe` next to the running
//! one, try it (`--version`), then rename the running file to `webox.exe.old` and move the new one in. Any failure puts the old
//! file back. Only the program file changes; `plugins\` and everything else in the folder stay as they are.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The program file inside the release zip.
pub const EXE_NAME: &str = "webox.exe";
/// Upper bound for the unpacked program (bytes).
const EXE_MAX: u64 = 512 << 20;

pub fn sha256_file(p: &Path) -> Result<String, String> {
    let mut f = std::fs::File::open(p).map_err(|e| format!("Datei nicht lesbar: {e}"))?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).map_err(|e| format!("Datei nicht lesbar: {e}"))?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// `webox.exe` -> `webox.exe.old` (the running file, kept until the next start deletes it).
pub fn old_path(exe: &Path) -> PathBuf { suffixed(exe, ".old") }
/// `webox.exe` -> `webox.exe.new` (the unpacked update before the swap).
pub fn new_path(exe: &Path) -> PathBuf { suffixed(exe, ".new") }
fn suffixed(exe: &Path, s: &str) -> PathBuf {
    let mut n = exe.file_name().unwrap_or_default().to_os_string();
    n.push(s);
    exe.with_file_name(n)
}

/// True when files can be created in the program folder (not the case for an all-users install without admin rights).
pub fn folder_writable(exe: &Path) -> bool {
    let probe = suffixed(exe, ".write-test");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Checks `zip` against `want_sha`, unpacks the program to `webox.exe.new` beside `exe` and runs `probe` on it.
/// The zip is never opened when the checksum differs; the unpacked file is deleted when anything fails.
pub fn stage(zip: &Path, want_sha: &str, exe: &Path, probe: impl Fn(&Path) -> Result<(), String>) -> Result<PathBuf, String> {
    let got = sha256_file(zip)?;
    if !got.eq_ignore_ascii_case(want_sha.trim()) { return Err("Prüfsumme (SHA-256) stimmt nicht; Update verworfen".into()); }
    let bytes = std::fs::read(zip).map_err(|e| format!("Download nicht lesbar: {e}"))?;
    let program = super::zip::extract(&bytes, EXE_NAME, EXE_MAX)?;
    let new = new_path(exe);
    let res = std::fs::write(&new, &program).map_err(|e| format!("Neue Version kann nicht abgelegt werden: {e}")).and_then(|_| probe(&new));
    if res.is_err() { let _ = std::fs::remove_file(&new); }
    res.map(|_| new)
}

/// Moves `new` into place of `exe`; the old file becomes `webox.exe.old`. Restores the old file when the second step fails.
pub fn swap(exe: &Path, new: &Path) -> Result<(), String> {
    let old = old_path(exe);
    if old.exists() { std::fs::remove_file(&old).map_err(|e| format!("Alte Sicherung lässt sich nicht löschen: {e}"))?; }
    std::fs::rename(exe, &old).map_err(|e| format!("Programmdatei lässt sich nicht umbenennen: {e}"))?;
    if let Err(e) = std::fs::rename(new, exe) {
        let back = std::fs::rename(&old, exe);
        let _ = std::fs::remove_file(new);
        return Err(match back {
            Ok(()) => format!("Neue Version lässt sich nicht einsetzen: {e}"),
            Err(b) => format!("Neue Version lässt sich nicht einsetzen ({e}), Wiederherstellung fehlgeschlagen: {b}"),
        });
    }
    Ok(())
}

/// Undoes a swap: the new file goes, `webox.exe.old` comes back.
pub fn rollback(exe: &Path) -> Result<(), String> {
    let old = old_path(exe);
    if !old.exists() { return Err("Keine Sicherung vorhanden".into()); }
    let _ = std::fs::remove_file(exe);
    std::fs::rename(&old, exe).map_err(|e| format!("Wiederherstellung fehlgeschlagen: {e}"))
}

/// Deletes what an earlier update left behind (`.old`, `.new`). False while the old file is still locked.
pub fn cleanup(exe: &Path) -> bool {
    let _ = std::fs::remove_file(new_path(exe));
    let old = old_path(exe);
    !old.exists() || std::fs::remove_file(&old).is_ok()
}

/// Starts `exe --version` and expects it to print `version` within 20 s.
pub fn probe_version(exe: &Path, version: &str) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let mut c = Command::new(exe).arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().map_err(|e| format!("Neue Version startet nicht: {e}"))?;
    let t0 = std::time::Instant::now();
    loop {
        match c.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if t0.elapsed().as_secs() < 20 => std::thread::sleep(std::time::Duration::from_millis(100)),
            _ => { let _ = c.kill(); return Err("Neue Version antwortet nicht".into()); }
        }
    }
    let mut out = String::new();
    if let Some(mut s) = c.stdout.take() { let _ = std::io::Read::read_to_string(&mut s, &mut out); }
    if out.split_whitespace().any(|w| w == version) { Ok(()) } else { Err(format!("Neue Version meldet „{}“ statt {version}", out.trim())) }
}
