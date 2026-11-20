//! The plugin repository index (docs/plugins.md): either a plain array of plugins (the SchulyPlugins / Aniyomi shape) or an
//! object `{ "name", "description", "homepage", "formatVersion", "plugins": [...] }`. Entries that do not validate are skipped
//! (counted in `skipped`), so one broken entry never hides a whole repository.
use collabhost_plugin_api::{CONTRACT_MAJOR, CONTRACT_MINOR};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct FileRef { pub file: String, pub sha256: String, pub size: u64 }

#[derive(Clone, Debug, PartialEq)]
pub struct IndexPlugin {
    pub name: String,
    /// Reverse-DNS id, e.g. `dev.webox.plugins.slash-commands`; the install folder name.
    pub pkg: String,
    pub version: String,
    /// plugin-api contract "major.minor".
    pub contract: String,
    pub description: String,
    pub authors: Vec<String>,
    pub icon: Option<String>,
    pub homepage: Option<String>,
    /// Platform key (`windows-x86_64`, `linux-x86_64`, `macos-aarch64`, ...) -> binary.
    pub files: BTreeMap<String, FileRef>,
    pub min_webox: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct RepoIndex {
    pub name: Option<String>,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub plugins: Vec<IndexPlugin>,
    pub skipped: usize,
}

fn text(v: &Value, k: &str) -> Option<String> { v.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from) }

pub fn parse(json: &str) -> Result<RepoIndex, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Index ist kein gültiges JSON: {e}"))?;
    let (mut idx, list) = match &v {
        Value::Array(a) => (RepoIndex::default(), a.clone()),
        Value::Object(_) => {
            let list = v.get("plugins").and_then(Value::as_array).cloned().ok_or("Index ohne \"plugins\"-Liste")?;
            (RepoIndex { name: text(&v, "name"), description: text(&v, "description"), homepage: text(&v, "homepage"), ..Default::default() }, list)
        }
        _ => return Err("Index muss eine Liste oder ein Objekt sein".into()),
    };
    let mut seen = std::collections::HashSet::new();
    for e in &list {
        match plugin(e) {
            Some(p) if seen.insert(p.pkg.clone()) => idx.plugins.push(p),
            _ => idx.skipped += 1,
        }
    }
    Ok(idx)
}

fn plugin(v: &Value) -> Option<IndexPlugin> {
    let pkg = text(v, "pkg")?;
    let version = text(v, "version")?;
    let contract = text(v, "contract")?;
    if !valid_pkg(&pkg) || semver::Version::parse(&version).is_err() || parse_contract(&contract).is_none() { return None; }
    let authors = match v.get("authors") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => vec![],
    };
    let mut files = BTreeMap::new();
    for (k, f) in v.get("files").and_then(Value::as_object)? {
        let (Some(file), Some(sha)) = (text(f, "file"), text(f, "sha256")) else { continue };
        if !valid_sha256(&sha) || !valid_file_ref(&file) { continue; }
        files.insert(k.clone(), FileRef { file, sha256: sha.to_ascii_lowercase(), size: f.get("size").and_then(Value::as_u64).unwrap_or(0) });
    }
    if files.is_empty() { return None; }
    Some(IndexPlugin {
        name: text(v, "name").unwrap_or_else(|| pkg.clone()), pkg, version, contract,
        description: text(v, "description").unwrap_or_default(), authors,
        icon: text(v, "icon"), homepage: text(v, "homepage"), files, min_webox: text(v, "minWebox"),
    })
}

/// Reverse-DNS id: lowercase letters, digits, `.`, `-`, `_`; at least two labels; no empty label (so never `..`).
pub fn valid_pkg(p: &str) -> bool {
    p.len() <= 128 && p.split('.').count() >= 2
        && p.split('.').all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'))
}
pub fn valid_sha256(s: &str) -> bool { s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) }
/// A binary is either a path relative to the index (`/`-separated names, none starting with `.`, so never `..`) or an
/// absolute https URL. The last segment is the file name it is installed under.
fn valid_file_ref(f: &str) -> bool { f.starts_with("https://") || (f.len() <= 400 && f.split('/').all(valid_file_name)) }
pub fn valid_file_name(f: &str) -> bool {
    !f.is_empty() && f.len() <= 200 && !f.starts_with('.') && f.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

pub fn parse_contract(s: &str) -> Option<(u16, u16)> {
    let (a, b) = s.split_once('.')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// Whether this Webox can load a plugin built for `contract` (major must match, minor may not be newer).
pub fn check_contract(major: u16, minor: u16) -> Result<(), String> {
    if major != CONTRACT_MAJOR {
        Err(format!("für Plugin-Schnittstelle {major}.{minor} gebaut, Webox hat {CONTRACT_MAJOR}.{CONTRACT_MINOR}"))
    } else if minor > CONTRACT_MINOR {
        Err(format!("braucht Plugin-Schnittstelle {major}.{minor}, Webox hat {CONTRACT_MAJOR}.{CONTRACT_MINOR}; Webox aktualisieren"))
    } else { Ok(()) }
}

/// This build's platform key in `files`.
pub fn platform() -> String {
    let os = match std::env::consts::OS { "macos" => "macos", "windows" => "windows", o => o };
    format!("{os}-{}", std::env::consts::ARCH)
}

/// The binary for this platform, or why the plugin can not be installed here.
pub fn compatible<'a>(p: &'a IndexPlugin, webox_version: &str) -> Result<&'a FileRef, String> {
    let (ma, mi) = parse_contract(&p.contract).ok_or("ungültige Schnittstellen-Version")?;
    check_contract(ma, mi)?;
    if let Some(min) = &p.min_webox {
        if let (Ok(min), Ok(have)) = (semver::Version::parse(min), semver::Version::parse(webox_version)) {
            if have < min { return Err(format!("braucht Webox {min} oder neuer")); }
        }
    }
    p.files.get(&platform()).ok_or_else(|| format!("nicht für diese Plattform ({}) verfügbar", platform()))
}

/// Whether version `a` is newer than `b` (semver; unparsable versions are never newer).
pub fn newer(a: &str, b: &str) -> bool {
    match (semver::Version::parse(a), semver::Version::parse(b)) { (Ok(a), Ok(b)) => a > b, _ => false }
}
