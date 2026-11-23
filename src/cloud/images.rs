//! Inline images in messages. A file in a "share" activity has a thumbnail `image {url, scr, width, height}`;
//! `scr` is a JWE (alg dir, key = the file's KMS key) whose plaintext is the SCR
//! `{enc:"A256GCM", key, iv, aad, loc, tag}`. The blob at `loc` is fetched through the files service
//! (`POST https://<files host>/v1/download/endpoints {"endpoints":[loc]}` → signed URL) and AES-256-GCM
//! decrypted with that key.
use super::*;
use std::io::Read;

pub const IMAGE_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Size cap for the original file of an image/GIF (full-resolution viewer).
pub const FULL_IMAGE_MAX_BYTES: u64 = 25 * 1024 * 1024;

/// The original file behind an inline thumbnail (its own `url` + `scr` + key), kept per thumbnail key.
pub(super) struct FullDesc { pub scr: String, pub key_url: String, pub mime: String }

/// `fallback` is the still thumbnail's scr, used when the primary (an animated GIF file) can't be fetched or is too large.
pub(super) struct ImgJob { pub key: String, pub scr: String, pub key_url: String, pub fallback: Option<String> }

#[derive(Clone)]
struct Scr { enc: String, key: Vec<u8>, iv: Vec<u8>, aad: Vec<u8>, loc: String, tag: Vec<u8> }

/// Waits before the retries of a failed image download (seconds). A key is marked as fetched when its job starts, so without a
/// retry one failed attempt would leave the picture grey for the whole session. Right after a share the files service may not
/// hand out a fresh upload yet, while the same file downloads fine shortly after.
pub(super) const RETRY_DELAYS: [u64; 5] = [2, 5, 10, 20, 40];

/// Whether a failed download is worth another try later: everything but a refused location, an unknown cipher and the size cap
/// (those don't change by waiting).
pub(super) fn transient(err: &str) -> bool {
    !(err.starts_with("content location is not a Webex host") || err.starts_with("unsupported content cipher") || err.contains("larger than the size cap")
        || err == "download url is not https")
}

/// One attempt: the file, then (for animated GIFs) the still when the file isn't usable.
fn fetch_one(agent: &ureq::Agent, token: &str, scr: &Scr, fallback: Option<&Scr>, cap: u64) -> Result<Vec<u8>, String> {
    let res = download(agent, token, scr, cap);
    match (res, fallback) {
        (Err(e), Some(fb)) => { dbg_conn(&format!("gif file not usable ({e}); using the still")); download(agent, token, fb, cap).map_err(|e2| if transient(&e) { e } else { e2 }) }
        (r, _) => r,
    }
}

/// Hosts that may receive the bearer token (content downloads included).
pub(super) fn token_host(url: &str) -> bool { token_host_of(url).is_some() }
/// The parsed host of a `token_host` URL (requests built from it, like `/v1/download/endpoints`, go where the check looked).
pub(super) fn token_host_of(url: &str) -> Option<String> {
    url_host(url, &["https"]).filter(|h| WEBEX_DOMAINS.iter().chain(["webexcontent.com"].iter()).any(|d| subdomain_of(h, d)))
}

/// Public media CDNs whose files are fetched with a plain GET and never get the token (GIF shares).
pub(super) fn giphy_host(url: &str) -> bool {
    url_host(url, &["https", "http"]).is_some_and(|h| h == "giphy.com" || subdomain_of(&h, "giphy.com"))
}

fn parse_scr(plain: &[u8]) -> Result<Scr, String> {
    let v: Value = serde_json::from_slice(plain).map_err(|e| e.to_string())?;
    Ok(Scr { enc: st(&v, "enc"), key: jose::unb64(&st(&v, "key"))?, iv: jose::unb64(&st(&v, "iv"))?, aad: st(&v, "aad").into_bytes(),
        loc: st(&v, "loc"), tag: jose::unb64(&st(&v, "tag")).unwrap_or_default() })
}

/// The Giphy id in a CDN media URL: `/media/<id>/giphy.gif` or `/media/v1.<token>/<id>/giphy.gif` (query and fragment ignored).
pub fn giphy_media_id(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let mut seg = path.split('/').skip_while(|s| *s != "media").skip(1);
    let first = seg.next().filter(|s| !s.is_empty())?;
    let id = if first.starts_with("v1.") { seg.next().filter(|s| !s.is_empty())? } else { first };
    // An id is alphanumeric; a file name here means the URL had no id segment.
    (id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') && !id.contains('.')).then(|| id.to_string())
}

impl Session {
    /// The Giphy source of a GIF share: decrypts the file's `scr` (the same key fetch_images uses) and keeps its `loc` when that is a
    /// Giphy CDN address with a recognisable id. `None` for everything else (uploaded GIFs have no public address).
    pub(super) fn giphy_of(&mut self, scr: &str, key_url: &str, name: &str) -> Option<Giphy> {
        let kid = Some(st(&jose::header(scr), "kid")).filter(|k| !k.is_empty()).unwrap_or_else(|| key_url.to_string());
        let plain = self.key(&kid).and_then(|k| jose::decrypt_dir(&k, scr)).ok()?;
        let loc = parse_scr(&plain).ok()?.loc;
        if !loc.starts_with("https://") || !giphy_host(&loc) { return None; }
        Some(Giphy { id: giphy_media_id(&loc)?, url: loc, title: name.trim().to_string() })
    }

    /// Full-resolution viewer: fetches and decrypts the original file behind the thumbnail `key` (images/GIFs only, <= 25 MB)
    /// and delivers it as `Resp::Images` under `full:<key>`. Once per key.
    pub(super) fn full_image(&mut self, key: &str) -> Result<(), String> {
        // No descriptor: the inline image already is the original file (GIFs, small pictures) – nothing more to load.
        let Some(d) = self.full_desc.get(key) else { return Ok(()) };
        if !d.mime.starts_with("image/") { return Ok(()) }
        let full_key = format!("full:{key}");
        if !self.images_done.insert(full_key.clone()) { return Ok(()) }
        let kid = Some(st(&jose::header(&d.scr), "kid")).filter(|k| !k.is_empty()).unwrap_or(d.key_url.clone());
        let scr_jwe = d.scr.clone();
        let scr = self.key(&kid).and_then(|k| jose::decrypt_dir(&k, &scr_jwe)).and_then(|p| parse_scr(&p)).map_err(|e| { self.images_done.remove(&full_key); e })?;
        let (agent, token, out) = (self.agent.clone(), self.token.clone(), self.out.clone());
        std::thread::spawn(move || match download(&agent, &token, &scr, FULL_IMAGE_MAX_BYTES) {
            Ok(b) => { dbg_conn(&format!("full image: {} bytes", b.len())); let _ = out.send(Resp::Images(vec![(full_key, b)])); }
            Err(e) => dbg_conn(&format!("full image failed: {e}")),
        });
        Ok(())
    }

    /// Starts the background download of every image queued by `activity()` that wasn't fetched before.
    pub(super) fn fetch_images(&mut self) {
        let jobs = std::mem::take(&mut self.img_jobs);
        let mut work: Vec<(String, Scr, Option<Scr>)> = vec![];
        let mut failed = 0;
        for j in jobs {
            if !self.images_done.insert(j.key.clone()) { continue }
            let kid = Some(st(&jose::header(&j.scr), "kid")).filter(|k| !k.is_empty()).unwrap_or(j.key_url.clone());
            match self.key(&kid).and_then(|k| jose::decrypt_dir(&k, &j.scr)).and_then(|p| parse_scr(&p)) {
                Ok(s) => {
                    let fb = j.fallback.as_ref().and_then(|f| self.key(&kid).and_then(|k| jose::decrypt_dir(&k, f)).and_then(|p| parse_scr(&p)).ok());
                    if let Some(f) = &fb { dbg_conn(&format!("gif job: file host {}, still host {}", s.loc.split('/').nth(2).unwrap_or("?"), f.loc.split('/').nth(2).unwrap_or("?"))); }
                    work.push((j.key, s, fb))
                }
                Err(e) => { failed += 1; dbg_conn(&format!("image scr failed: {}", e.chars().take(100).collect::<String>())); }
            }
        }
        if work.is_empty() { if failed > 0 { dbg_conn(&format!("images: {failed} scr failures")); } return; }
        let (agent, token, out) = (self.agent.clone(), self.token.clone(), self.out.clone());
        std::thread::spawn(move || {
            let (mut ok, mut bad) = (0, failed);
            let deliver = |key: String, bytes: Vec<u8>| match key.strip_prefix("spaceavatar:") {
                Some(space) => { let _ = out.send(Resp::SpaceAvatars(vec![(space.to_string(), bytes)])); }
                None => { let _ = out.send(Resp::Images(vec![(key, bytes)])); }
            };
            let cap_of = |key: &str| if key.starts_with("spaceavatar:") { 1024 * 1024 } else { IMAGE_MAX_BYTES };
            let mut again: Vec<(String, Scr, Option<Scr>)> = vec![];
            for (key, scr, fallback) in work {
                match fetch_one(&agent, &token, &scr, fallback.as_ref(), cap_of(&key)) {
                    Ok(bytes) => { ok += 1; deliver(key, bytes); }
                    Err(e) => {
                        bad += 1;
                        dbg_conn(&format!("image download failed: {e}"));
                        if transient(&e) { again.push((key, scr, fallback)); }
                    }
                }
            }
            dbg_conn(&format!("images: {ok} decrypted, {bad} failed"));
            // The failed ones once more after a while, with growing pauses (the batch above isn't held up by them).
            for wait in RETRY_DELAYS {
                if again.is_empty() { break }
                std::thread::sleep(Duration::from_secs(wait));
                let mut still: Vec<(String, Scr, Option<Scr>)> = vec![];
                for (key, scr, fallback) in again {
                    match fetch_one(&agent, &token, &scr, fallback.as_ref(), cap_of(&key)) {
                        Ok(bytes) => { dbg_conn("image download retry: ok"); deliver(key, bytes); }
                        Err(e) if transient(&e) => still.push((key, scr, fallback)),
                        Err(e) => dbg_conn(&format!("image download retry failed for good: {e}")),
                    }
                }
                again = still;
            }
            if !again.is_empty() { dbg_conn(&format!("images: {} still failing after the retries", again.len())); }
        });
    }
}

fn download(agent: &ureq::Agent, token: &str, scr: &Scr, cap: u64) -> Result<Vec<u8>, String> {
    if giphy_host(&scr.loc) {
        // A GIF share points at Giphy's public CDN: plain GET, no Authorization header, content is not encrypted.
        let url = scr.loc.replacen("http://", "https://", 1); // always over TLS
        let r = agent.get(&url).call().map_err(|e| format!("giphy {}", short(&e)))?;
        let mut blob = vec![];
        r.into_reader().take(cap + 1).read_to_end(&mut blob).map_err(|e| e.to_string())?;
        if blob.is_empty() { return Err("empty giphy file".into()); }
        if blob.len() < 64 { dbg_conn(&format!("giphy answer is only {} bytes: {:?}", blob.len(), String::from_utf8_lossy(&blob))); }
        if blob.len() as u64 > cap { return Err("image larger than the size cap".into()); }
        return Ok(blob);
    }
    let Some(host) = token_host_of(&scr.loc) else {
        return Err(format!("content location is not a Webex host ({})", url_host(&scr.loc, &["https", "http"]).unwrap_or_else(|| "?".into())));
    };
    let ep = agent.post(&format!("https://{host}/v1/download/endpoints")).set("Authorization", &format!("Bearer {token}"))
        .send_json(json!({"endpoints": [scr.loc]})).map_err(|e| format!("endpoints {}", short(&e)))?;
    let ep: Value = ep.into_json().map_err(|e| e.to_string())?;
    let url = Some(st(&ep["endpoints"], &scr.loc)).filter(|u| !u.is_empty()).unwrap_or_else(|| scr.loc.clone());
    if !url.starts_with("https://") { return Err("download url is not https".into()); }
    let mut req = agent.get(&url);
    if token_host(&url) { req = req.set("Authorization", &format!("Bearer {token}")); }
    let r = req.call().map_err(|e| format!("blob {}", short(&e)))?;
    let mut blob = vec![];
    r.into_reader().take(cap + 1).read_to_end(&mut blob).map_err(|e| e.to_string())?;
    if blob.is_empty() { return Err("empty blob".into()); }
    if blob.len() as u64 > cap { return Err("image larger than the size cap".into()); }
    if !scr.enc.eq_ignore_ascii_case("A256GCM") { return Err(format!("unsupported content cipher {}", scr.enc)); }
    jose::gcm_open(&scr.key, &scr.iv, &scr.aad, &blob, &scr.tag)
}

fn short(e: &ureq::Error) -> String {
    match e { ureq::Error::Status(c, _) => format!("http {c}"), ureq::Error::Transport(t) => format!("transport {:?}", t.kind()) }
}

#[cfg(test)]
mod giphy_id_tests {
    use super::giphy_media_id as id;

    #[test]
    fn ids_come_out_of_media_urls() {
        assert_eq!(id("https://media3.giphy.com/media/xT9IgzoKnwFNmISR8I/giphy.gif").as_deref(), Some("xT9IgzoKnwFNmISR8I"));
        assert_eq!(id("https://media0.giphy.com/media/v1.Y2lkPTc5MGI3NjEx/3o6wrib/giphy.gif?cid=1&rid=giphy.gif").as_deref(), Some("3o6wrib"));
        assert_eq!(id("https://media.giphy.com/media/abc-DEF_9/200w_d.gif#x").as_deref(), Some("abc-DEF_9"));
        assert_eq!(id("https://media.giphy.com/media/giphy.gif"), None); // no id segment
        assert_eq!(id("https://media.giphy.com/media/v1.tok/"), None);
        assert_eq!(id("https://example.com/files/abc/giphy.gif"), None); // not a media path
        assert_eq!(id(""), None);
    }
}

#[cfg(test)]
mod host_tests {
    use super::{giphy_host, token_host, token_host_of};

    #[test]
    fn token_hosts_are_parsed_hosts() {
        assert!(token_host("https://files-api-a.wbx2.com/v1/spaces/x"));
        assert!(token_host("https://boxes.WEBEXCONTENT.com/blob"));
        assert_eq!(token_host_of("https://files-api-a.wbx2.com:443/v1/x").as_deref(), Some("files-api-a.wbx2.com"));
        for u in ["https://evil.tld?.webexcontent.com", "https://evil.tld#.wbx2.com", "https://evil.tld\\.webex.com/x", "https://a.wbx2.com@evil.tld/",
            "https://a.wbx2.com:8443/", "http://a.webexcontent.com/", "https://webexcontent.com/", "https://u@a.wbx2.com/"] {
            assert!(!token_host(u), "{u}");
        }
    }

    #[test]
    fn giphy_hosts() {
        for u in ["https://media3.giphy.com/media/x/giphy.gif", "http://giphy.com/x", "https://MEDIA.GIPHY.COM/x"] { assert!(giphy_host(u), "{u}"); }
        for u in ["https://evil.tld?.giphy.com", "https://evil.tld#giphy.com", "https://media.giphy.com@evil.tld/", "https://notgiphy.com/", "ftp://giphy.com/", "https://giphy.com:444/"] {
            assert!(!giphy_host(u), "{u}");
        }
    }
}

#[cfg(test)]
mod retry_tests {
    use super::{transient, RETRY_DELAYS};

    /// A download that failed for a reason that can pass (the service not handing out a fresh upload yet, a dropped connection,
    /// a damaged transfer) is tried again; a refused location, an unknown cipher or the size cap are not.
    #[test]
    fn failed_downloads_are_retried_when_it_can_help() {
        for e in ["endpoints http 404", "endpoints http 423", "blob http 403", "blob transport Io", "empty blob", "content decryption failed", "giphy http 503"] {
            assert!(transient(e), "{e}");
        }
        for e in ["content location is not a Webex host (x.example)", "unsupported content cipher A128CBC", "image larger than the size cap", "download url is not https"] {
            assert!(!transient(e), "{e}");
        }
    }

    #[test]
    fn retries_back_off_and_end() {
        assert!(RETRY_DELAYS.windows(2).all(|w| w[0] < w[1]));
        assert!(RETRY_DELAYS.iter().sum::<u64>() <= 120); // given up within two minutes
    }
}