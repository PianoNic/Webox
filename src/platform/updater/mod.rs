//! App updates from the GitHub releases: the check (by hand, and once a day in the background), and on Windows the download,
//! checksum test and program swap (`swap`). Linux only reports a new version: packages update through Nix or the distribution.
//! macOS only reports it too and links to the GitHub download (`webox-<v>-macos-arm64.zip`; no self-replacing of the .app).
//! The state lives here; the UI reads it every frame (`status`) and the workers wake the window when it changes.
pub mod release;
pub mod net;
pub mod swap;
pub mod zip;
#[cfg(windows)]
mod registry;
#[cfg(test)]
mod tests;

use release::Release;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Command-line flag of the process started after a swap: it deletes `webox.exe.old`.
pub const UPDATED_FLAG: &str = "--updated";
/// Interval of the automatic check.
pub const AUTO_INTERVAL_MS: i64 = 24 * 3_600_000;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Phase {
    #[default] Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Installing,
    /// The new program is in place; the app restarts as soon as no call is running.
    Ready,
    Failed(String),
}

#[derive(Clone, Debug, Default)]
pub struct Status { pub phase: Phase, pub release: Option<Release> }

impl Status {
    /// A newer version is known (found, downloading or installed): the rail shows the update button.
    pub fn offered(&self) -> bool { self.release.is_some() && matches!(self.phase, Phase::Available | Phase::Downloading | Phase::Installing | Phase::Ready) }
}

static STATE: Mutex<Status> = Mutex::new(Status { phase: Phase::Idle, release: None });
static DONE: AtomicU64 = AtomicU64::new(0);
static TOTAL: AtomicU64 = AtomicU64::new(0);
/// Set by the UI every frame; the installer refuses to swap during a call.
pub static IN_CALL: AtomicBool = AtomicBool::new(false);
/// Sample and screenshot runs never check in the background.
pub static NO_AUTO_CHECK: AtomicBool = AtomicBool::new(false);

pub fn status() -> Status { STATE.lock().map(|s| s.clone()).unwrap_or_default() }
/// Bytes downloaded / expected.
pub fn progress() -> (u64, u64) { (DONE.load(Ordering::Relaxed), TOTAL.load(Ordering::Relaxed)) }
fn set(f: impl FnOnce(&mut Status)) { if let Ok(mut s) = STATE.lock() { f(&mut s); } crate::ui::wake(); }

/// Screenshot runs (`WEBOX_SHOT_UPDATE`): a made-up release 99.0.0, found or half downloaded. Never touches the network.
pub fn sample(downloading: bool) {
    let notes = "## Neu\n- Beispiel: schnellere Suche\n- Beispiel: neue Tastenkombinationen\n## Behoben\n- Beispiel: Anzeige von Bildern".to_string();
    let assets = ["webox-99.0.0-windows-x64.zip", "webox-99.0.0-linux-x86_64.tar.gz"].iter()
        .map(|n| release::Asset { name: n.to_string(), url: format!("https://github.com/PianoNic/Webox/releases/download/v99.0.0/{n}"), size: 0 }).collect();
    let rel = Release { version: semver::Version::new(99, 0, 0), notes, page: release::RELEASES_PAGE.into(), assets };
    if downloading { DONE.store(12_400_000, Ordering::Relaxed); TOTAL.store(31_000_000, Ordering::Relaxed); }
    set(|s| { s.release = Some(rel); s.phase = if downloading { Phase::Downloading } else { Phase::Available }; });
}

/// Windows replaces its own program file; elsewhere the system's package manager does.
pub const fn self_update() -> bool { cfg!(windows) }

/// Running from the Nix store: the flake / NixOS module decides the version, so no release nag in the background.
pub fn from_nix_store() -> bool {
    static R: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *R.get_or_init(|| std::env::current_exe().is_ok_and(|e| e.starts_with("/nix/store")))
}

/// The running program, for the swap.
fn exe() -> Result<PathBuf, String> { std::env::current_exe().map_err(|e| format!("Programmdatei nicht gefunden: {e}")) }

/// Checks for a newer release on a worker. `background`: from the daily timer (an "up to date" or an error stays silent).
pub fn check(background: bool) {
    {
        let Ok(mut s) = STATE.lock() else { return };
        if matches!(s.phase, Phase::Checking | Phase::Downloading | Phase::Installing | Phase::Ready) { return; }
        if background && s.phase == Phase::Available { return; }
        if !background { s.phase = Phase::Checking; }
    }
    crate::ui::wake();
    std::thread::spawn(move || {
        let res = net::latest().map_err(|f| f.text()).and_then(|body| release::parse(&body));
        set(|s| match res {
            Ok(Some(r)) if release::is_newer(VERSION, &r.version) => {
                s.release = Some(r);
                s.phase = Phase::Available;
            }
            Ok(_) if background => {}
            Ok(_) => s.phase = Phase::UpToDate,
            // "No release yet" means this build is the newest there is.
            Err(e) if e == net::Fail::NoRelease.text() => if !background { s.phase = Phase::UpToDate },
            Err(_) if background => {}
            Err(e) => s.phase = Phase::Failed(e),
        });
    });
}

/// Downloads and installs the found release on a worker (Windows). Ends in `Ready` or `Failed`; the old program stays on failure.
pub fn install() {
    let Some(rel) = ({
        let Ok(mut s) = STATE.lock() else { return };
        if s.phase != Phase::Available || IN_CALL.load(Ordering::Relaxed) { return; }
        s.phase = Phase::Downloading;
        s.release.clone()
    }) else { return };
    DONE.store(0, Ordering::Relaxed);
    TOTAL.store(0, Ordering::Relaxed);
    crate::ui::wake();
    std::thread::spawn(move || {
        let res = run_install(&rel);
        set(|s| s.phase = match res { Ok(()) => Phase::Ready, Err(e) => Phase::Failed(e) });
    });
}

fn run_install(rel: &Release) -> Result<(), String> {
    let exe = exe()?;
    if !swap::folder_writable(&exe) {
        return Err("Der Programmordner ist schreibgeschützt. Installieren Sie die neue Version mit dem Setup von der Release-Seite.".into());
    }
    let asset = release::platform_asset(&rel.assets, std::env::consts::OS, std::env::consts::ARCH)
        .ok_or("Für dieses System gibt es in dieser Version keinen Download.")?;
    let sums = release::checksum_asset(&rel.assets, &asset.name).ok_or("Die Prüfsumme fehlt in dieser Version; Update abgebrochen.")?;
    let sha = release::parse_checksum(&net::text(&sums.url).map_err(|f| f.text())?, &asset.name)
        .ok_or("Die Prüfsumme ist nicht lesbar; Update abgebrochen.")?;
    let dir = std::env::temp_dir().join(format!("webox-update-{}", rel.version));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let zip = dir.join(&asset.name);
    let res = net::download(&asset.url, &zip, asset.size, &DONE, &TOTAL).map_err(|f| f.text()).and_then(|_| {
        set(|s| s.phase = Phase::Installing);
        let version = rel.version.to_string();
        let new = swap::stage(&zip, &sha, &exe, |p| swap::probe_version(p, &version))?;
        if IN_CALL.load(Ordering::Relaxed) { let _ = std::fs::remove_file(&new); return Err("Während eines Anrufs wird nicht aktualisiert.".into()); }
        swap::swap(&exe, &new)
    });
    let _ = std::fs::remove_dir_all(&dir);
    res
}

/// Starts the new program (with `--updated`). On failure the old program is put back and the error returned.
pub fn restart() -> Result<(), String> {
    let exe = exe()?;
    let args: Vec<String> = std::env::args().skip(1).filter(|a| a != UPDATED_FLAG).collect();
    match std::process::Command::new(&exe).arg(UPDATED_FLAG).args(args).spawn() {
        Ok(_) => Ok(()),
        Err(e) => {
            let back = swap::rollback(&exe);
            let msg = format!("Neustart fehlgeschlagen: {e}");
            set(|s| s.phase = Phase::Failed(msg.clone()));
            back.and(Err(msg))
        }
    }
}

/// At every start: removes what an update left (the old program may still be closing, so it retries for a few seconds) and
/// on Windows keeps the installer's version entry in "Apps" in step with this program.
pub fn after_start() {
    #[cfg(not(windows))]
    { let cfg = crate::system::config_dir(); std::thread::spawn(move || crate::platform::nmhost::refresh(&cfg)); } // browser helper manifests follow the launcher path
    let Ok(exe) = exe() else { return };
    #[cfg(windows)]
    registry::sync_display_version(&exe, VERSION);
    let updated = std::env::args().any(|a| a == UPDATED_FLAG);
    if !updated && !swap::old_path(&exe).exists() && !swap::new_path(&exe).exists() { return; }
    std::thread::spawn(move || {
        for _ in 0..40 {
            if swap::cleanup(&exe) { return; }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });
}
