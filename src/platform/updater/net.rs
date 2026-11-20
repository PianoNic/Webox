//! Requests to GitHub: the latest release (unauthenticated, generic user agent, nothing about the user is sent), the checksum
//! file and the archive. HTTPS only; redirects only to GitHub's hosts. Runs on worker threads.
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const USER_AGENT: &str = "Webox-Updater";
const JSON_MAX: u64 = 2 << 20;
/// Upper bound for a release archive (bytes).
pub const ARCHIVE_MAX: u64 = 512 << 20;
const HOSTS: &[&str] = &["api.github.com", "github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com",
    "github-releases.githubusercontent.com"];

#[derive(Debug, PartialEq)]
pub enum Fail { NoRelease, RateLimited, Offline, Status(u16), Other(String) }

impl Fail {
    pub fn text(&self) -> String {
        match self {
            Fail::NoRelease => "Es wurde noch keine Version veröffentlicht.".into(),
            Fail::RateLimited => "GitHub hat zu viele Anfragen gezählt. Versuchen Sie es in einer Stunde erneut.".into(),
            Fail::Offline => "Keine Verbindung zu GitHub. Prüfen Sie Ihre Internetverbindung.".into(),
            Fail::Status(c) => format!("GitHub antwortet mit HTTP {c}."),
            Fail::Other(s) => s.clone(),
        }
    }
}

/// Maps an HTTP error status of the API (403 / 429 with no requests left = rate limit, 404 = no release yet).
pub fn status_fail(code: u16, remaining: Option<&str>) -> Fail {
    match code {
        404 => Fail::NoRelease,
        429 => Fail::RateLimited,
        403 if remaining == Some("0") => Fail::RateLimited,
        c => Fail::Status(c),
    }
}

fn host_ok(url: &str) -> bool {
    url::Url::parse(url).ok().is_some_and(|u| u.scheme() == "https" && u.host_str().is_some_and(|h| HOSTS.iter().any(|x| h.eq_ignore_ascii_case(x))))
}

fn get(url: &str, accept: &str, timeout: Duration) -> Result<ureq::Response, Fail> {
    if !host_ok(url) { return Err(Fail::Other("Adresse außerhalb von GitHub abgelehnt".into())); }
    let agent = ureq::AgentBuilder::new().user_agent(USER_AGENT).timeout_connect(Duration::from_secs(10)).timeout(timeout).redirects(5).build();
    let resp = match agent.get(url).set("Accept", accept).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => return Err(status_fail(code, r.header("x-ratelimit-remaining"))),
        Err(_) => return Err(Fail::Offline),
    };
    if !host_ok(resp.get_url()) { return Err(Fail::Other("Weiterleitung außerhalb von GitHub abgelehnt".into())); }
    Ok(resp)
}

fn read_text(resp: ureq::Response, cap: u64) -> Result<String, Fail> {
    let mut s = String::new();
    resp.into_reader().take(cap).read_to_string(&mut s).map_err(|_| Fail::Offline)?;
    Ok(s)
}

/// The `releases/latest` answer.
pub fn latest() -> Result<String, Fail> {
    read_text(get(super::release::LATEST_URL, "application/vnd.github+json", Duration::from_secs(20))?, JSON_MAX)
}

/// A small text asset (the checksum file).
pub fn text(url: &str) -> Result<String, Fail> { read_text(get(url, "application/octet-stream", Duration::from_secs(30))?, 64 << 10) }

/// Streams an asset into `dest`, reporting bytes in `done` / `total`. A partial file is deleted.
pub fn download(url: &str, dest: &Path, size: u64, done: &AtomicU64, total: &AtomicU64) -> Result<(), Fail> {
    let res = (|| {
        let resp = get(url, "application/octet-stream", Duration::from_secs(600))?;
        let len = resp.header("content-length").and_then(|v| v.parse::<u64>().ok()).unwrap_or(size);
        if len > ARCHIVE_MAX { return Err(Fail::Other("Download größer als erlaubt".into())); }
        total.store(len, Ordering::Relaxed);
        let mut f = std::fs::File::create(dest).map_err(|e| Fail::Other(format!("Datei kann nicht angelegt werden: {e}")))?;
        let mut r = resp.into_reader().take(ARCHIVE_MAX + 1);
        let (mut buf, mut n) = (vec![0u8; 64 << 10], 0u64);
        loop {
            let k = r.read(&mut buf).map_err(|_| Fail::Offline)?;
            if k == 0 { break; }
            n += k as u64;
            if n > ARCHIVE_MAX { return Err(Fail::Other("Download größer als erlaubt".into())); }
            f.write_all(&buf[..k]).map_err(|e| Fail::Other(format!("Schreiben fehlgeschlagen: {e}")))?;
            done.store(n, Ordering::Relaxed);
        }
        f.sync_all().map_err(|e| Fail::Other(e.to_string()))?;
        if size > 0 && n != size { return Err(Fail::Other(format!("Download unvollständig ({n} von {size} Bytes)"))); }
        Ok(())
    })();
    if res.is_err() { let _ = std::fs::remove_file(dest); }
    res
}
