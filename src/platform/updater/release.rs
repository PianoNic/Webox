//! The GitHub release answer: the version, the release notes, this platform's download and its SHA-256 checksum file.
use semver::Version;
use serde_json::Value;

/// The latest published release (GitHub leaves drafts and pre-releases out of `releases/latest`).
pub const LATEST_URL: &str = "https://api.github.com/repos/PianoNic/Webox/releases/latest";
/// Where the releases are listed, for the "Versionshinweise" link when there is no release page in the answer.
pub const RELEASES_PAGE: &str = "https://github.com/PianoNic/Webox/releases";

#[derive(Clone, Debug, PartialEq)]
pub struct Asset { pub name: String, pub url: String, pub size: u64 }

#[derive(Clone, Debug)]
pub struct Release { pub version: Version, pub notes: String, pub page: String, pub assets: Vec<Asset> }

/// "v0.2.0", "0.2.0", "release-0.2.0" -> 0.2.0.
pub fn parse_version(tag: &str) -> Option<Version> {
    Version::parse(tag.trim().trim_start_matches(|c: char| !c.is_ascii_digit())).ok()
}

/// The release in a `releases/latest` answer. `None` for a draft, a pre-release or a tag that is not a version.
pub fn parse(json: &str) -> Result<Option<Release>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Antwort nicht lesbar: {e}"))?;
    if v["draft"].as_bool().unwrap_or(false) || v["prerelease"].as_bool().unwrap_or(false) { return Ok(None); }
    let tag = v["tag_name"].as_str().unwrap_or_default();
    let Some(version) = parse_version(tag).filter(|v| v.pre.is_empty()) else { return Ok(None) };
    let assets = v["assets"].as_array().map(|a| a.iter().filter_map(|x| Some(Asset {
        name: x["name"].as_str()?.to_string(),
        url: x["browser_download_url"].as_str()?.to_string(),
        size: x["size"].as_u64().unwrap_or(0),
    })).collect()).unwrap_or_default();
    Ok(Some(Release {
        version,
        notes: v["body"].as_str().unwrap_or_default().to_string(),
        page: v["html_url"].as_str().filter(|u| u.starts_with("https://")).unwrap_or(RELEASES_PAGE).to_string(),
        assets,
    }))
}

/// True when `rel` is newer than the running version `cur`.
pub fn is_newer(cur: &str, rel: &Version) -> bool {
    rel.pre.is_empty() && parse_version(cur).is_some_and(|c| *rel > c)
}

/// The release workflow's architecture word: `x64` on Windows, `uname -m` on Linux.
pub fn arch_word(os: &str, arch: &str) -> String {
    match (os, arch) { ("windows", "x86_64") => "x64".into(), ("windows", "aarch64") => "arm64".into(), (_, a) => a.into() }
}

/// This platform's archive: `webox-<version>-windows-x64.zip` or `webox-<version>-linux-<arch>.tar.gz`.
pub fn platform_asset<'a>(assets: &'a [Asset], os: &str, arch: &str) -> Option<&'a Asset> {
    let a = arch_word(os, arch);
    let tail = match os { "windows" => format!("-windows-{a}.zip"), "linux" => format!("-linux-{a}.tar.gz"), _ => return None };
    assets.iter().find(|x| x.name.starts_with("webox-") && x.name.ends_with(&tail))
}

/// The checksum file for `file`: `<file>.sha256`, else a combined `SHA256SUMS`.
pub fn checksum_asset<'a>(assets: &'a [Asset], file: &str) -> Option<&'a Asset> {
    let own = format!("{file}.sha256");
    assets.iter().find(|x| x.name == own).or_else(|| assets.iter().find(|x| x.name == "SHA256SUMS"))
}

/// The hash for `file` in `sha256sum` output ("<hex>  <name>" or "<hex> *<name>"); a single bare hash counts too.
pub fn parse_checksum(text: &str, file: &str) -> Option<String> {
    let is_hex = |h: &str| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit());
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    for l in &lines {
        let mut it = l.splitn(2, char::is_whitespace);
        let (h, name) = (it.next().unwrap_or(""), it.next().map(|n| n.trim().trim_start_matches('*')));
        if is_hex(h) && name == Some(file) { return Some(h.to_ascii_lowercase()); }
    }
    match lines.as_slice() { [only] if is_hex(only) => Some(only.to_ascii_lowercase()), _ => None }
}

/// The first lines of the release notes for the dialog (markdown headings and list marks dropped).
pub fn excerpt(notes: &str, max_lines: usize, max_chars: usize) -> String {
    let mut out = String::new();
    for l in notes.lines().map(str::trim).filter(|l| !l.is_empty()).take(max_lines) {
        let l = l.trim_start_matches('#').trim();
        let l = l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")).map(|s| format!("• {s}")).unwrap_or_else(|| l.to_string());
        if out.chars().count() + l.chars().count() > max_chars { out.push('…'); break; }
        if !out.is_empty() { out.push('\n'); }
        out.push_str(&l);
    }
    out
}
