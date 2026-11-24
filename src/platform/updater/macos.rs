//! Self-update on macOS: the whole `Webox.app` is replaced. The release's `webox-<v>-macos-universal.zip` is checked against
//! its SHA-256, unpacked with `ditto` into a folder beside the running bundle (same volume, so the swap is two renames), the new
//! program is tried (`--version`) and its signature checked, then `Webox.app` becomes `Webox.app.old` and the new bundle moves
//! in. Any failure puts the old bundle back. `Webox.app.old` is deleted at the next start; the restart goes through `open`.
use super::release::Release;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicU64;

/// The bundle the running program is in: `<x>/Webox.app` for `<x>/Webox.app/Contents/MacOS/webox`.
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let app = exe.parent()?.parent()?.parent()?;
    let ok = exe.parent()?.file_name()? == "MacOS" && exe.parent()?.parent()?.file_name()? == "Contents"
        && app.extension().is_some_and(|e| e.eq_ignore_ascii_case("app"));
    ok.then(|| app.to_path_buf())
}

pub fn old_bundle(app: &Path) -> PathBuf { suffixed(app, ".old") }
fn staging(app: &Path, version: &str) -> PathBuf { suffixed(app, &format!(".update-{version}")) }
fn suffixed(p: &Path, s: &str) -> PathBuf {
    let mut n = p.file_name().unwrap_or_default().to_os_string();
    n.push(s);
    p.with_file_name(n)
}

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let st = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map_err(|e| format!("{what}: {e}"))?;
    if st.success() { Ok(()) } else { Err(format!("{what} fehlgeschlagen ({st})")) }
}

/// Swaps `new` in for `app`; `app` becomes `<app>.old`. The old bundle comes back when the second rename fails.
pub fn swap_bundle(app: &Path, new: &Path) -> Result<(), String> {
    let old = old_bundle(app);
    if old.exists() { std::fs::remove_dir_all(&old).map_err(|e| format!("Alte Sicherung lässt sich nicht löschen: {e}"))?; }
    std::fs::rename(app, &old).map_err(|e| format!("Webox.app lässt sich nicht umbenennen: {e}"))?;
    if let Err(e) = std::fs::rename(new, app) {
        let back = std::fs::rename(&old, app);
        return Err(match back {
            Ok(()) => format!("Neue Version lässt sich nicht einsetzen: {e}"),
            Err(b) => format!("Neue Version lässt sich nicht einsetzen ({e}), Wiederherstellung fehlgeschlagen: {b}"),
        });
    }
    Ok(())
}

pub fn run_install(rel: &Release, done: &AtomicU64, total: &AtomicU64) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("Programmdatei nicht gefunden: {e}"))?;
    let app = bundle_of(&exe).ok_or("Webox läuft nicht aus Webox.app: bitte die App von der Release-Seite installieren.")?;
    let parent = app.parent().ok_or("Programmordner unbekannt")?;
    if !super::swap::folder_writable(&app) {
        return Err(format!("Der Ordner {} ist schreibgeschützt. Laden Sie die neue Version von der Release-Seite.", parent.display()));
    }
    let asset = super::release::platform_asset(&rel.assets, "macos", std::env::consts::ARCH).ok_or("Für macOS gibt es in dieser Version keinen Download.")?;
    let sums = super::release::checksum_asset(&rel.assets, &asset.name).ok_or("Die Prüfsumme fehlt in dieser Version; Update abgebrochen.")?;
    let sha = super::release::parse_checksum(&super::net::text(&sums.url).map_err(|f| f.text())?, &asset.name)
        .ok_or("Die Prüfsumme ist nicht lesbar; Update abgebrochen.")?;
    let version = rel.version.to_string();
    let stage = staging(&app, &version);
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| format!("Neue Version kann nicht abgelegt werden: {e}"))?;
    let res = (|| {
        let zip = stage.join(&asset.name);
        super::net::download(&asset.url, &zip, asset.size, done, total).map_err(|f| f.text())?;
        super::set(|s| s.phase = super::Phase::Installing);
        let got = super::swap::sha256_file(&zip)?;
        if !got.eq_ignore_ascii_case(sha.trim()) { return Err("Prüfsumme (SHA-256) stimmt nicht; Update verworfen".to_string()); }
        run(Command::new("/usr/bin/ditto").arg("-x").arg("-k").arg(&zip).arg(&stage), "Entpacken")?;
        let new = stage.join("Webox.app");
        if !new.join("Contents/MacOS/webox").is_file() { return Err("Im Download fehlt Webox.app".into()); }
        let _ = run(Command::new("/usr/bin/xattr").args(["-dr", "com.apple.quarantine"]).arg(&new), "xattr");
        run(Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"]).arg(&new), "Signaturprüfung")?;
        super::swap::probe_version(&new.join("Contents/MacOS/webox"), &version)?;
        if super::IN_CALL.load(std::sync::atomic::Ordering::Relaxed) { return Err("Während eines Anrufs wird nicht aktualisiert.".into()); }
        swap_bundle(&app, &new)
    })();
    let _ = std::fs::remove_dir_all(&stage);
    res
}

/// Starts the new bundle (a new instance through Launch Services, with `--updated`).
pub fn restart(args: &[String]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let app = bundle_of(&exe).ok_or("kein App-Bundle")?;
    Command::new("/usr/bin/open").arg("-n").arg(&app).arg("--args").arg(super::UPDATED_FLAG).args(args).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Undoes a swap: the new bundle goes, `Webox.app.old` comes back.
pub fn rollback() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let app = bundle_of(&exe).ok_or("kein App-Bundle")?;
    let old = old_bundle(&app);
    if !old.exists() { return Err("Keine Sicherung vorhanden".into()); }
    let _ = std::fs::remove_dir_all(&app);
    std::fs::rename(&old, &app).map_err(|e| format!("Wiederherstellung fehlgeschlagen: {e}"))
}

/// At start: deletes `Webox.app.old` and unfinished staging folders.
pub fn cleanup() {
    let Some(app) = std::env::current_exe().ok().and_then(|e| bundle_of(&e)) else { return };
    let _ = std::fs::remove_dir_all(old_bundle(&app));
    let (Some(dir), Some(name)) = (app.parent(), app.file_name().map(|n| n.to_string_lossy().into_owned())) else { return };
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.file_name().to_string_lossy().starts_with(&format!("{name}.update-")) { let _ = std::fs::remove_dir_all(e.path()); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_bundle_of_the_program() {
        assert_eq!(bundle_of(Path::new("/Applications/Webox.app/Contents/MacOS/webox")), Some(PathBuf::from("/Applications/Webox.app")));
        assert_eq!(bundle_of(Path::new("/Users/a/target/debug/webox")), None);
        assert_eq!(old_bundle(Path::new("/Applications/Webox.app")), PathBuf::from("/Applications/Webox.app.old"));
    }

    #[test]
    fn swaps_bundles_and_rolls_back() {
        let d = std::env::temp_dir().join(format!("webox-bundle-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (app, new) = (d.join("Webox.app"), d.join("Webox.app.update-9.9.9/Webox.app"));
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        std::fs::write(app.join("Contents/v"), "old").unwrap();
        std::fs::create_dir_all(new.join("Contents")).unwrap();
        std::fs::write(new.join("Contents/v"), "new").unwrap();
        swap_bundle(&app, &new).unwrap();
        assert_eq!(std::fs::read_to_string(app.join("Contents/v")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(old_bundle(&app).join("Contents/v")).unwrap(), "old");
        // a missing new bundle leaves the old one in place
        assert!(swap_bundle(&app, &d.join("missing.app")).is_err());
        assert_eq!(std::fs::read_to_string(app.join("Contents/v")).unwrap(), "new");
        let _ = std::fs::remove_dir_all(&d);
    }
}
