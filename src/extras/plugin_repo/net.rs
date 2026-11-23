//! Network side of the plugin repositories: the index (cached with ETag / If-Modified-Since) and the binary download (HTTPS only,
//! size cap, timeouts, redirects followed by hand: only to the same host or GitHub's raw / release CDN, streamed into a file while
//! hashing, refused and deleted on a sha256 or size mismatch). Runs on worker threads, never on the UI thread.
use super::index::FileRef;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use url::Url;

pub const INDEX_MAX: u64 = 4 << 20;
pub const BINARY_MAX: u64 = 64 << 20;
const MAX_HOPS: usize = 5;
/// Where GitHub serves raw files and release assets; a binary download may be redirected there, nowhere else.
pub const GITHUB_CDN: &[&str] = &["raw.githubusercontent.com", "github.com", "objects.githubusercontent.com",
    "release-assets.githubusercontent.com", "github-releases.githubusercontent.com", "codeload.github.com"];

/// Transport rules. `strict()` everywhere; tests may allow plain http to the loopback address for an in-process fixture.
#[derive(Clone, Copy, Debug)]
pub struct Policy { pub allow_loopback_http: bool }
impl Policy {
    pub const fn strict() -> Policy { Policy { allow_loopback_http: false } }
    fn url_ok(&self, u: &Url) -> Result<(), String> {
        match u.scheme() {
            "https" => Ok(()),
            "http" if self.allow_loopback_http && matches!(u.host_str(), Some("127.0.0.1") | Some("localhost")) => Ok(()),
            s => Err(format!("nur HTTPS erlaubt (nicht {s}://)")),
        }
    }
}

pub fn parse_url(s: &str, p: Policy) -> Result<Url, String> {
    let u = Url::parse(s.trim()).map_err(|e| format!("ungültige Adresse: {e}"))?;
    p.url_ok(&u)?;
    if !u.username().is_empty() || u.password().is_some() { return Err("Adressen mit Zugangsdaten sind nicht erlaubt".into()); }
    Ok(u)
}

/// The binary URL: an absolute https URL from the index, or a file name next to the index.
pub fn resolve(index_url: &str, file: &str, p: Policy) -> Result<Url, String> {
    let base = parse_url(index_url, p)?;
    let u = if file.contains("://") { Url::parse(file).map_err(|e| e.to_string())? } else { base.join(file).map_err(|e| e.to_string())? };
    p.url_ok(&u)?;
    Ok(u)
}

/// A redirect may stay on the host it started on or go to GitHub's CDN.
pub fn redirect_ok(origin_host: &str, to: &Url) -> bool {
    to.host_str().is_some_and(|h| h.eq_ignore_ascii_case(origin_host) || GITHUB_CDN.iter().any(|g| h.eq_ignore_ascii_case(g)))
}

pub struct Got { pub status: u16, pub etag: Option<String>, pub last_modified: Option<String>, pub body: Vec<u8>, pub len: Option<u64> }

fn agent(url: &Url, timeout: Duration) -> ureq::Agent {
    crate::cloud::tls::agent_for(url.as_str()).redirects(0).timeout_connect(Duration::from_secs(10)).timeout(timeout)
        .user_agent(concat!("Webox/", env!("CARGO_PKG_VERSION"), " plugin-repo")).build()
}

/// GET with hand-followed redirects. Calls `sink` with the body reader of the final 2xx response (or returns 304 / errors).
fn get(url: &Url, headers: &[(&str, String)], p: Policy, timeout: Duration, mut sink: impl FnMut(ureq::Response) -> Result<Got, String>) -> Result<Got, String> {
    let a = agent(url, timeout);
    let origin = url.host_str().unwrap_or_default().to_string();
    let mut cur = url.clone();
    for _ in 0..=MAX_HOPS {
        let mut req = a.get(cur.as_str());
        for (k, v) in headers { req = req.set(k, v); }
        let resp = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(code, _)) => return Err(format!("Server antwortet mit HTTP {code}")),
            Err(e) => return Err(format!("Verbindung fehlgeschlagen: {}", e.kind())),
        };
        let st = resp.status();
        if (300..400).contains(&st) && st != 304 {
            let loc = resp.header("location").ok_or("Weiterleitung ohne Ziel")?;
            let next = cur.join(loc).map_err(|e| format!("ungültige Weiterleitung: {e}"))?;
            p.url_ok(&next)?;
            if !redirect_ok(&origin, &next) { return Err(format!("Weiterleitung zu {} abgelehnt", next.host_str().unwrap_or("?"))); }
            cur = next;
            continue;
        }
        if st == 304 { return Ok(Got { status: 304, etag: None, last_modified: None, body: vec![], len: None }); }
        return sink(resp);
    }
    Err("zu viele Weiterleitungen".into())
}

fn read_capped(r: impl Read, cap: u64, mut each: impl FnMut(&[u8]) -> Result<(), String>, progress: Option<&AtomicU64>) -> Result<u64, String> {
    let mut r = r.take(cap + 1);
    let (mut buf, mut total) = (vec![0u8; 64 * 1024], 0u64);
    loop {
        let n = r.read(&mut buf).map_err(|e| format!("Übertragung abgebrochen: {e}"))?;
        if n == 0 { return Ok(total); }
        total += n as u64;
        if total > cap { return Err(format!("Datei größer als erlaubt ({} MB)", cap >> 20)); }
        each(&buf[..n])?;
        if let Some(pr) = progress { pr.store(total, Ordering::Relaxed); }
    }
}

/// Cached copy of an index: the body plus its validators.
#[derive(Default, Clone)]
pub struct Cache { pub etag: Option<String>, pub last_modified: Option<String>, pub body: String }

fn cache_file(dir: &Path, url: &str) -> std::path::PathBuf {
    let h = Sha256::digest(url.as_bytes());
    dir.join(format!("{}.json", hex(&h[..8])))
}
pub fn read_cache(dir: &Path, url: &str) -> Option<Cache> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(cache_file(dir, url)).ok()?).ok()?;
    if v["url"] != url { return None; }
    Some(Cache { etag: v["etag"].as_str().map(String::from), last_modified: v["lastModified"].as_str().map(String::from), body: v["body"].as_str()?.to_string() })
}
fn write_cache(dir: &Path, url: &str, c: &Cache) {
    let _ = std::fs::create_dir_all(dir);
    let v = serde_json::json!({ "url": url, "etag": c.etag, "lastModified": c.last_modified, "body": c.body });
    let _ = super::store::write_atomic(&cache_file(dir, url), v.to_string().as_bytes());
}

/// Fetches an index, using the cache's validators; returns the body (fresh or, on 304, cached).
pub fn fetch_index(url: &str, cache_dir: &Path, p: Policy) -> Result<String, String> {
    let u = parse_url(url, p)?;
    let cached = read_cache(cache_dir, url);
    let mut h = vec![("Accept", "application/json".to_string())];
    if let Some(c) = &cached {
        if let Some(e) = &c.etag { h.push(("If-None-Match", e.clone())); }
        if let Some(m) = &c.last_modified { h.push(("If-Modified-Since", m.clone())); }
    }
    let got = get(&u, &h, p, Duration::from_secs(20), |resp| {
        let (etag, lm) = (resp.header("etag").map(String::from), resp.header("last-modified").map(String::from));
        let mut body = vec![];
        read_capped(resp.into_reader(), INDEX_MAX, |b| { body.extend_from_slice(b); Ok(()) }, None)?;
        Ok(Got { status: 200, etag, last_modified: lm, body, len: None })
    })?;
    if got.status == 304 { return cached.map(|c| c.body).ok_or_else(|| "304 ohne zwischengespeicherte Kopie".into()); }
    let body = String::from_utf8(got.body).map_err(|_| "Index ist kein UTF-8".to_string())?;
    write_cache(cache_dir, url, &Cache { etag: got.etag, last_modified: got.last_modified, body: body.clone() });
    Ok(body)
}

/// Downloads `url` into `dest`, hashing on the fly. On any mismatch with `want` (sha256, size) `dest` is deleted and the
/// download fails: an unverified file is never left where it could be loaded.
pub fn download(url: &Url, want: &FileRef, dest: &Path, p: Policy, progress: &AtomicU64, total: &AtomicU64) -> Result<(), String> {
    let cap = if want.size > 0 { want.size.min(BINARY_MAX) } else { BINARY_MAX };
    let res = (|| {
        let mut f = std::fs::File::create(dest).map_err(|e| format!("Datei kann nicht angelegt werden: {e}"))?;
        let mut hasher = Sha256::new();
        get(url, &[], p, Duration::from_secs(300), |resp| {
            let len = resp.header("content-length").and_then(|v| v.parse::<u64>().ok());
            if len.is_some_and(|l| l > cap) { return Err(format!("Datei größer als erlaubt ({} MB)", cap >> 20)); }
            total.store(len.unwrap_or(want.size), Ordering::Relaxed);
            let n = read_capped(resp.into_reader(), cap, |b| { hasher.update(b); f.write_all(b).map_err(|e| e.to_string()) }, Some(progress))?;
            Ok(Got { status: 200, etag: None, last_modified: None, body: vec![], len: Some(n) })
        }).and_then(|g| {
            if g.status == 304 { return Err("unerwartete Antwort 304".into()); }
            if want.size > 0 && g.len != Some(want.size) { return Err(format!("Größe stimmt nicht ({} statt {} Bytes)", g.len.unwrap_or(0), want.size)); }
            f.sync_all().map_err(|e| e.to_string())?;
            let got = hex(&hasher.finalize());
            if !got.eq_ignore_ascii_case(&want.sha256) { return Err("Prüfsumme (SHA-256) stimmt nicht mit dem Index überein; Datei verworfen".into()); }
            Ok(())
        })
    })();
    if res.is_err() { let _ = std::fs::remove_file(dest); }
    res
}

pub fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }
