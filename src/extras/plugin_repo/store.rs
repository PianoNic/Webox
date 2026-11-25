//! Installed repository plugins on disk: `<root>/<pkg>/<version>/<file>` plus `<root>/installed.json` (which version of which
//! package, its pinned sha256 and the repository it came from). `root` is `%APPDATA%\WebOx\plugins` on Windows and
//! `$XDG_DATA_HOME/webox/plugins` (`~/.local/share/webox/plugins`) elsewhere. Installs land in a staging folder first and are
//! renamed into place; installed.json is replaced atomically (write + rename), so a crash leaves the previous version usable.
use super::index::{valid_file_name, valid_pkg};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const INSTALLED: &str = "installed.json";

#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub pkg: String,
    pub name: String,
    pub version: String,
    pub file: String,
    pub sha256: String,
    /// Index URL of the repository it was installed from.
    pub repo: String,
}

pub fn install_root(config: &Path) -> PathBuf {
    if cfg!(windows) { return config.join("plugins"); }
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    data.map_or_else(|| config.join("plugins"), |d| d.join("webox").join("plugins"))
}

fn ok(i: &Installed) -> bool { valid_pkg(&i.pkg) && semver::Version::parse(&i.version).is_ok() && valid_file_name(&i.file) }

pub fn read(root: &Path) -> Vec<Installed> {
    let Some(v) = std::fs::read_to_string(root.join(INSTALLED)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return vec![] };
    let s = |e: &Value, k: &str| e[k].as_str().unwrap_or_default().to_string();
    v["plugins"].as_array().into_iter().flatten().map(|e| Installed {
        pkg: s(e, "pkg"), name: s(e, "name"), version: s(e, "version"), file: s(e, "file"), sha256: s(e, "sha256"), repo: s(e, "repo"),
    }).filter(ok).collect()
}

pub fn write(root: &Path, list: &[Installed]) -> Result<(), String> {
    let v = json!({ "plugins": list.iter().map(|i| json!({ "pkg": i.pkg, "name": i.name, "version": i.version, "file": i.file,
        "sha256": i.sha256, "repo": i.repo })).collect::<Vec<_>>() });
    write_atomic(&root.join(INSTALLED), serde_json::to_string_pretty(&v).unwrap_or_default().as_bytes())
}

/// Write + rename. Owner-only on unix from creation (0700 new folders, 0600 file): plugin secrets may fall back to these files.
pub fn write_atomic(p: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(d) = p.parent() { crate::system::create_private_dir(d).map_err(|e| e.to_string())?; }
    let tmp = p.with_extension(format!("tmp{}", std::process::id()));
    // Unix: private (0600) from the first byte (the plugin-settings file may hold secrets when there is no keyring).
    crate::system::write_private(&tmp, bytes).map_err(|e| { let _ = std::fs::remove_file(&tmp); e.to_string() })?;
    std::fs::rename(&tmp, p).map_err(|e| { let _ = std::fs::remove_file(&tmp); e.to_string() })
}

pub fn path_of(root: &Path, i: &Installed) -> PathBuf { root.join(&i.pkg).join(&i.version).join(&i.file) }

pub fn sha256_file(p: &Path) -> Result<String, String> {
    let mut f = std::fs::File::open(p).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).map_err(|e| e.to_string())?;
    Ok(super::net::hex(&h.finalize()))
}

/// The file of an installed plugin, after checking it still has the pinned hash (a changed file is never loaded).
pub fn verify(root: &Path, i: &Installed) -> Result<PathBuf, String> {
    if !ok(i) { return Err(crate::trq!("invalid entry in installed.json").into_owned()); }
    let p = path_of(root, i);
    match sha256_file(&p) {
        Ok(h) if h.eq_ignore_ascii_case(&i.sha256) => Ok(p),
        Ok(_) => Err(crate::trq!("The file was changed after installation (SHA-256 does not match); not loaded").into_owned()),
        Err(e) => Err(crate::trq!("File missing: {{0}}", e)),
    }
}

fn stamp() -> u128 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()) }

/// A fresh staging folder for a download of `pkg`.
pub fn stage_dir(root: &Path, pkg: &str) -> Result<PathBuf, String> {
    if !valid_pkg(pkg) { return Err(crate::trq!("invalid package ID").into_owned()); }
    let d = root.join(pkg).join(format!(".staging-{}", stamp()));
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

/// Result of moving a verified download into place: the plugin file, and where an existing copy of the same version was parked.
pub struct Commit { pub path: PathBuf, backup: Option<PathBuf> }

/// Renames the staging folder holding `staged` to `<root>/<pkg>/<version>`.
pub fn commit(root: &Path, staged: &Path, i: &Installed) -> Result<Commit, String> {
    if !ok(i) { return Err(crate::trq!("invalid package details").into_owned()); }
    let stage = staged.parent().ok_or_else(|| crate::trq!("no staging folder").into_owned())?;
    if staged.file_name().and_then(|n| n.to_str()) != Some(i.file.as_str()) || stage.parent() != Some(&root.join(&i.pkg)) {
        return Err(crate::trq!("The staging file is not where it was expected").into_owned());
    }
    let target = root.join(&i.pkg).join(&i.version);
    let backup = if target.exists() {
        let b = root.join(&i.pkg).join(format!(".old-{}", stamp()));
        std::fs::rename(&target, &b).map_err(|e| crate::trq!("the old version cannot be moved: {{0}}", e))?;
        Some(b)
    } else { None };
    if let Err(e) = std::fs::rename(stage, &target) {
        if let Some(b) = &backup { let _ = std::fs::rename(b, &target); }
        return Err(crate::trq!("Installation failed: {{0}}", e));
    }
    Ok(Commit { path: target.join(&i.file), backup })
}

/// Undoes a commit whose plugin then failed to load.
pub fn rollback(root: &Path, i: &Installed, c: Commit) {
    let target = root.join(&i.pkg).join(&i.version);
    let _ = std::fs::remove_dir_all(&target);
    if let Some(b) = c.backup { let _ = std::fs::rename(b, &target); }
}

/// Records the new version in installed.json and removes what it replaced.
pub fn finish(root: &Path, i: &Installed, old: Option<&Installed>, c: Commit) -> Result<(), String> {
    let mut list: Vec<Installed> = read(root).into_iter().filter(|x| x.pkg != i.pkg).collect();
    list.push(i.clone());
    list.sort_by(|a, b| a.pkg.cmp(&b.pkg));
    write(root, &list)?;
    if let Some(b) = c.backup { let _ = std::fs::remove_dir_all(b); }
    if let Some(o) = old.filter(|o| o.version != i.version) { let _ = std::fs::remove_dir_all(root.join(&o.pkg).join(&o.version)); }
    Ok(())
}

/// Deletes a package and its installed.json entry.
pub fn remove(root: &Path, pkg: &str) -> Result<(), String> {
    if !valid_pkg(pkg) { return Err(crate::trq!("invalid package ID").into_owned()); }
    let list: Vec<Installed> = read(root).into_iter().filter(|x| x.pkg != pkg).collect();
    write(root, &list)?;
    let d = root.join(pkg);
    if d.exists() { std::fs::remove_dir_all(&d).map_err(|e| crate::trq!("The folder cannot be deleted: {{0}}", e))?; }
    Ok(())
}

/// Leftovers of interrupted installs (`.staging-*`, `.old-*`), removed at start before anything is loaded.
pub fn clean(root: &Path) {
    let Ok(rd) = std::fs::read_dir(root) else { return };
    for pkg in rd.flatten().filter(|e| e.path().is_dir()) {
        let Ok(inner) = std::fs::read_dir(pkg.path()) else { continue };
        for d in inner.flatten() {
            let n = d.file_name().to_string_lossy().to_string();
            if n.starts_with(".staging-") || n.starts_with(".old-") { let _ = std::fs::remove_dir_all(d.path()); }
        }
    }
}
