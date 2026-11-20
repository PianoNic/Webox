//! Sending and receiving files the way the Webex service expects it.
//!
//! SEND (`Req::SendFiles`; only ever on the user's own click):
//!  1. `PUT <conversation url>/space` and `PUT <conversation url>/space/hidden` (answers `{spaceUrl}`)
//!  2. per file: SCR (random AES-256-GCM key, 96-bit iv, aad = ISO time), the file encrypted with it, tag kept in the SCR
//!  3. `POST <spaceUrl>/upload_sessions?transcode=true` `{uploadProtocol:"content-length", fileSize, claimedFileType}`
//!     -> `{uploadUrl, finishUploadUrl, url, fileUploadSizeLimit?}`; over the limit: `DELETE url` and stop
//!  4. `PUT uploadUrl` with the encrypted bytes, no Authorization header (the storage host answers), `x-trans-id`
//!  5. `POST finishUploadUrl?transcode=true` `{fileSize, fileHash}` (sha256 hex of the encrypted bytes) -> `{downloadUrl, ...}`
//!  6. images: a thumbnail (<= 640 x 960) the same way, but into the hidden space, no claimedFileType
//!  7. `POST conversation/content?transcode=true&async=false` (see `GIF_SHARE_PATH`) with the share
//!     activity: object `{objectType:"content", contentCategory, displayName?, content?, mentions?, files:{items:[{objectType:"file",
//!     displayName (JWE), fileSize, mimeType, url, scr (JWE of the SCR), encryptionKeyUrl, width?, height?, image?{width,height,url,scr}}]}}`.
//!     `encryptionKeyUrl` is set per file item, as in received shares (gif_share_body).
//!
//! RECEIVE (`Req::DownloadFile`): the file's `scr` (JWE, kms key of the file / activity) -> SCR -> `POST https://<files host>/v1/download/endpoints`
//! `{endpoints:[loc]}` -> signed URL -> GET -> AES-256-GCM open, as `images.rs` does it for thumbnails.
//! Giphy shares point at Giphy's public CDN: plain GET, nothing to decrypt.
use super::*;
use crate::app::files::{self as shared, Phase, Then};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Largest thumbnail size in px (width / height).
pub const THUMB_MAX_W: u32 = 640;
pub const THUMB_MAX_H: u32 = 960;
/// Query of the initialize and finalize phases.
const TRANSCODE: &str = "transcode=true";

/// What is needed to fetch one attachment again: its file `scr` (JWE), the key that opens it, name / type / size.
#[derive(Clone, Debug, Default)]
#[allow(dead_code)] // `name`: kept with the descriptor (diagnostics never log it)
pub(super) struct FileDesc { pub name: String, pub mime: String, pub size: u64, pub scr: String, pub key_url: String }

static DESCS: Mutex<Option<HashMap<String, FileDesc>>> = Mutex::new(None);
static SPACE_URLS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

pub(super) fn remember(key: &str, d: FileDesc) { DESCS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(key.to_string(), d); }
fn desc(key: &str) -> Option<FileDesc> { DESCS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(key).cloned()) }

/// A fresh SCR: 256-bit key, 96-bit iv, aad = the creation time as ISO string, enc A256GCM.
pub(super) struct NewScr { pub key: [u8; 32], pub iv: [u8; 12], pub aad: String }
impl NewScr {
    pub fn new(aad: &str) -> Self {
        use rand::RngCore;
        let (mut key, mut iv) = ([0u8; 32], [0u8; 12]);
        rand::thread_rng().fill_bytes(&mut key);
        rand::thread_rng().fill_bytes(&mut iv);
        NewScr { key, iv, aad: aad.to_string() }
    }
    /// AES-256-GCM over the whole file with the aad; the ciphertext is uploaded without the tag, the tag goes
    /// into the SCR. Encrypts in place, returns the tag.
    pub fn seal(&self, data: &mut Vec<u8>) -> Result<Vec<u8>, String> {
        use aes_gcm::aead::{AeadInPlace, KeyInit};
        let c = aes_gcm::Aes256Gcm::new_from_slice(&self.key).map_err(|e| e.to_string())?;
        let tag = c.encrypt_in_place_detached(aes_gcm::Nonce::from_slice(&self.iv), self.aad.as_bytes(), data).map_err(|e| e.to_string())?;
        Ok(tag.to_vec())
    }
    /// SCR JSON in key order: enc, key, iv, aad, loc, tag (base64url without padding).
    pub fn json(&self, loc: &str, tag: &[u8]) -> Value {
        json!({"enc": "A256GCM", "key": jose::b64(&self.key), "iv": jose::b64(&self.iv), "aad": self.aad, "loc": loc, "tag": jose::b64(tag)})
    }
}

/// Initialize body (`uploadProtocol` + size, and the claimed file type for real files).
pub(super) fn init_body(size: u64, claimed: Option<&str>) -> Value {
    let mut b = json!({"uploadProtocol": "content-length", "fileSize": size});
    if let Some(c) = claimed.filter(|c| !c.is_empty()) { b["claimedFileType"] = json!(c); }
    b
}
/// Finalize body.
pub(super) fn finish_body(size: u64, hash: &str) -> Value { json!({"fileSize": size, "fileHash": hash}) }
/// The claimed file type: the name from its last '.' (the whole name when there is no dot).
pub(super) fn claimed_type(name: &str) -> String { name.rfind('.').map_or(name.to_string(), |i| name[i..].to_string()) }
fn with_query(url: &str, q: &str) -> String { format!("{url}{}{q}", if url.contains('?') { '&' } else { '?' }) }
fn sha256_hex(b: &[u8]) -> String { use sha2::Digest; sha2::Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect() }

/// "images" / "videos" when every file is of that kind, else "documents".
pub(super) fn content_category(mimes: &[String]) -> &'static str {
    let first = mimes.first().map(|m| m.split('/').next().unwrap_or("")).unwrap_or("");
    if mimes.iter().any(|m| m.is_empty()) || !(first == "image" || first == "video") || mimes.iter().any(|m| m.split('/').next() != Some(first)) { return "documents"; }
    if first == "image" { "images" } else { "videos" }
}

/// One uploaded file as it goes into the share (already encrypted strings).
pub(super) struct Item { pub name_enc: String, pub size: u64, pub mime: String, pub url: String, pub scr_enc: String, pub dims: Option<(u32, u32)>, pub thumb: Option<(u32, u32, String, String)> }

pub(super) fn file_item(key_url: &str, it: &Item) -> Value {
    let mut f = json!({"objectType": "file", "displayName": it.name_enc, "fileSize": it.size, "mimeType": it.mime, "url": it.url, "scr": it.scr_enc, "encryptionKeyUrl": key_url});
    if let Some((w, h)) = it.dims { f["width"] = json!(w); f["height"] = json!(h); }
    if let Some((w, h, url, scr)) = &it.thumb { f["image"] = json!({"width": w, "height": h, "url": url, "scr": scr}); }
    f
}

/// The share activity: `text` is the composer's text as a `comment_object` (displayName / content /
/// mentions, already encrypted) or None; `parent` = thread root for a reply.
pub(super) fn share_body(key_url: &str, text: Option<Value>, items: Vec<Value>, mimes: &[String], parent: Option<&str>) -> Value {
    let mut object = json!({"objectType": "content", "contentCategory": content_category(mimes), "files": {"items": items}});
    if let Some(Value::Object(t)) = text { for (k, v) in t { if k != "objectType" { object[k] = v; } } }
    let mut b = json!({"verb": "share", "encryptionKeyUrl": key_url, "object": object});
    if let Some(p) = parent { b["parent"] = json!({"id": p, "type": "reply"}); }
    b
}

/// A thumbnail no larger than 640 x 960 (never enlarged) as PNG, plus both sizes.
/// Returns (png, thumb w, thumb h, file w, file h); None for non-images.
pub(super) fn thumbnail(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32, u32, u32)> {
    let img = image::load_from_memory(bytes).ok()?;
    let (w, h) = (img.width(), img.height());
    let t = if w > THUMB_MAX_W || h > THUMB_MAX_H { img.resize(THUMB_MAX_W, THUMB_MAX_H, image::imageops::FilterType::Triangle) } else { img };
    let mut out = std::io::Cursor::new(Vec::new());
    t.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some((out.into_inner(), t.width(), t.height(), w, h))
}

/// A request from a worker thread: the bearer token only goes to Webex hosts (`images::token_host`).
fn call(agent: &ureq::Agent, token: &str, method: &str, url: &str, body: Option<Value>) -> Result<Value, (u16, String)> {
    if !url.starts_with("https://") { return Err((0, "not https".into())); }
    let mut r = agent.request(method, url).set("TrackingID", &format!("CLIENT_{}", jose::uuid()));
    if images::token_host(url) { r = r.set("Authorization", &format!("Bearer {token}")); }
    let resp = match body { Some(b) => r.send_json(b), None => r.call() };
    match resp {
        Ok(r) => Ok(r.into_json::<Value>().unwrap_or(Value::Null)),
        Err(ureq::Error::Status(c, r)) => Err((c, r.into_string().unwrap_or_default().chars().take(300).collect())),
        Err(e) => Err((0, e.to_string())),
    }
}

/// Reads the bytes for the upload PUT and counts them as progress; stops when the user cancels.
struct Counting<'a> { data: &'a [u8], pos: usize, id: String, cancel: Arc<AtomicBool> }
impl Read for Counting<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::SeqCst) { return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled")); }
        let n = buf.len().min(64 * 1024).min(self.data.len() - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        if n > 0 { shared::upload_add(&self.id, n as u64); }
        Ok(n)
    }
}

/// Phases 3-5 for one blob. Returns the `downloadUrl`. `progress`: count this blob into the upload's progress.
fn upload_blob(agent: &ureq::Agent, token: &str, space_url: &str, cdata: &[u8], claimed: Option<&str>, id: &str, cancel: &Arc<AtomicBool>, progress: bool) -> Result<String, String> {
    let size = cdata.len() as u64;
    let s = call(agent, token, "POST", &with_query(&format!("{space_url}/upload_sessions"), TRANSCODE), Some(init_body(size, claimed)))
        .map_err(|(c, b)| { dbg_conn(&format!("upload session refused: http {c}")); refusal(c, &b) })?;
    // The service's own limit (MB) wins; over it the session is deleted.
    let limit = s["fileUploadSizeLimit"].as_u64().unwrap_or(shared::DEFAULT_LIMIT_MB) * 1024 * 1024;
    if size > limit {
        if let Some(u) = s["url"].as_str() { let _ = call(agent, token, "DELETE", u, None); }
        return Err(TOO_LARGE.into());
    }
    let (up, fin) = (st(&s, "uploadUrl"), st(&s, "finishUploadUrl"));
    if up.is_empty() { return Err("Expected a uploadUrl response".into()); }
    if fin.is_empty() { return Err("Expected a finishUploadUrl response".into()); }
    if !up.starts_with("https://") || !fin.starts_with("https://") { return Err("upload url is not https".into()); }
    let reader = Counting { data: cdata, pos: 0, id: if progress { id.to_string() } else { String::new() }, cancel: cancel.clone() };
    let put = agent.put(&up).set("Content-Length", &size.to_string()).set("x-trans-id", &jose::uuid()).send(reader);
    if cancel.load(Ordering::SeqCst) { return Err(CANCELLED.into()); }
    put.map_err(|e| match e { ureq::Error::Status(c, _) => format!("upload http {c}"), e => format!("upload {}", e.kind()) })?;
    let done = call(agent, token, "POST", &with_query(&fin, TRANSCODE), Some(finish_body(size, &sha256_hex(cdata)))).map_err(|(c, b)| refusal(c, &b))?;
    let url = st(&done, "downloadUrl");
    if url.is_empty() { return Err("There was an error ending the upload session, downloadUrl not found".into()); }
    Ok(url)
}

const CANCELLED: &str = "cancelled";
/// "File too large" = "Datei zu groß".
const TOO_LARGE: &str = "Datei zu groß";
/// The service refusing a file: blocked types come back as 4xx, reported as "Gesperrte Datei" (QObject "Blocked file").
fn refusal(code: u16, body: &str) -> String {
    let b = body.to_lowercase();
    if code == 413 || b.contains("too large") || b.contains("size limit") { TOO_LARGE.into() }
    else if (400..500).contains(&code) && (b.contains("block") || b.contains("extension") || b.contains("not allowed") || b.contains("type")) { "Gesperrte Datei".into() }
    else if code == 0 { "Keine Verbindung".into() } else { format!("http {code}") }
}

/// Everything a `Req::SendFiles` carries.
pub struct SendFiles { pub space: String, pub upload: String, pub files: Vec<PathBuf>, pub text: String, pub markdown: bool, pub mentions: Vec<crate::richtext::MentionRef>, pub parent: Option<String> }

impl Session {
    /// `PUT <conversation url>/space` (or `/space/hidden`) -> `spaceUrl`, cached per conversation.
    fn space_url(&self, conv: &str, hidden: bool) -> Result<String, String> {
        let k = format!("{conv}|{hidden}");
        if let Some(u) = SPACE_URLS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&k).cloned()) { return Ok(u); }
        let url = self.url("conversation", &format!("/conversations/{conv}/space{}", if hidden { "/hidden" } else { "" }));
        let v = self.http("PUT", &url, None)?;
        let u = st(&v, "spaceUrl");
        if u.is_empty() { return Err("Expected a spaceUrl response".into()); }
        SPACE_URLS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(k, u.clone());
        Ok(u)
    }

    /// `Req::SendFiles`: key, space urls and the text are prepared here; the uploads and the share run on their own thread so the
    /// job loop keeps serving the UI. Progress / result go to `crate::app::files` (the composer's upload strip shows them).
    pub(super) fn send_files(&mut self, r: SendFiles) -> Result<(), String> {
        let fail = |e: String| { shared::upload_phase(&r.upload, Phase::Failed(e.clone())); e };
        let Some(cancel) = shared::upload_cancel_flag(&r.upload) else { return Ok(()) };
        if cancel.load(Ordering::SeqCst) { return Ok(()); }
        let key_url = self.key_url(&r.space).map_err(fail)?;
        let key = self.key(&key_url).map_err(fail)?;
        let images = r.files.iter().any(|p| { let n = p.to_string_lossy().to_lowercase(); [".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp"].iter().any(|e| n.ends_with(e)) });
        let space_url = self.space_url(&r.space, false).map_err(fail)?;
        let hidden = if images { Some(self.space_url(&r.space, true).map_err(fail)?) } else { None };
        // The text goes into the share object exactly like a posted comment (displayName / content / mentions).
        let text = if r.text.trim().is_empty() { None } else {
            let resolved: Vec<crate::richtext::MentionRef> = r.mentions.iter().filter_map(|m| if m.all { Some(m.clone()) } else { self.person_uuid_pub(&m.id).map(|u| crate::richtext::MentionRef { id: u, ..m.clone() }) }).collect();
            let rendered = crate::richtext::render(&r.text, r.markdown, &resolved);
            Some(activities::comment_object(&rendered, &r.text, &mut |t| jose::encrypt_dir(&key, &key_url, t.as_bytes())).map_err(fail)?)
        };
        let (agent, token, user, jobs) = (self.agent.clone(), self.token.clone(), self.user.clone(), self.calls.jobs.clone());
        let target = self.url("conversation", activities::GIF_SHARE_PATH);
        std::thread::spawn(move || {
            let res = upload_all(&agent, &token, &r, &key, &key_url, &space_url, hidden.as_deref(), &cancel).and_then(|(items, mimes)| {
                if cancel.load(Ordering::SeqCst) { return Err(CANCELLED.to_string()); }
                shared::upload_phase(&r.upload, Phase::Sharing);
                let mut body = share_body(&key_url, text, items, &mimes, r.parent.as_deref());
                body["target"] = json!({"id": r.space, "objectType": "conversation"});
                body["clientTempId"] = json!(jose::uuid());
                body["objectType"] = json!("activity");
                body["actor"] = json!({"objectType": "person", "id": user});
                call(&agent, &token, "POST", &target, Some(body)).map(|_| ()).map_err(|(c, b)| { dbg_conn(&format!("file share rejected: http {c}")); refusal(c, &b) })
            });
            match res {
                Ok(()) => { shared::upload_phase(&r.upload, Phase::Done); if let Some(tx) = jobs { let _ = tx.send(Job::Ui(Req::Messages(r.space.clone()))); } }
                Err(e) if e == CANCELLED || cancel.load(Ordering::SeqCst) => shared::upload_phase(&r.upload, Phase::Cancelled),
                Err(e) => { dbg_conn(&format!("file upload failed: {}", e.chars().take(80).collect::<String>())); shared::upload_phase(&r.upload, Phase::Failed(e)) }
            }
        });
        Ok(())
    }

    fn person_uuid_pub(&self, id: &str) -> Option<String> {
        if id.len() == 36 && id.matches('-').count() == 4 { return Some(id.to_string()); }
        self.uuids.get(id).cloned().or_else(|| (id.eq_ignore_ascii_case(&self.my_email) && !self.user.is_empty()).then(|| self.user.clone()))
    }

    /// `Req::DownloadFile`: opens the file's scr here (needs the key service), downloads + decrypts on its own thread, writes
    /// `path` (via `path.part`), then opens it / shows it in its folder when asked.
    pub(super) fn download_file(&mut self, key: &str, path: &Path, then: Then) -> Result<(), String> {
        let fail = |e: String| { shared::download_phase(key, Phase::Failed(e.clone())); e };
        let d = desc(key).ok_or_else(|| fail("Datei nicht gefunden".into()))?;
        let cancel = shared::download(key).map(|x| x.cancel).unwrap_or_default();
        let kid = Some(st(&jose::header(&d.scr), "kid")).filter(|k| !k.is_empty()).unwrap_or(d.key_url.clone());
        let plain = self.key(&kid).and_then(|k| jose::decrypt_dir(&k, &d.scr)).map_err(fail)?;
        let scr: Value = serde_json::from_slice(&plain).map_err(|e| fail(e.to_string()))?;
        let (agent, token, key, path) = (self.agent.clone(), self.token.clone(), key.to_string(), path.to_path_buf());
        std::thread::spawn(move || {
            // decrypted into `path.part` as it arrives; renamed only once the tag checked out
            let part = PathBuf::from(format!("{}.part", path.display()));
            let r = fetch_to(&agent, &token, &scr, &key, &cancel, &part).and_then(|()| std::fs::rename(&part, &path).map_err(|e| e.to_string()));
            if r.is_err() { let _ = std::fs::remove_file(&part); }
            match r {
                Ok(()) => { shared::download_phase(&key, Phase::Done); finish(&path, then); }
                Err(_) if cancel.load(Ordering::SeqCst) => shared::download_phase(&key, Phase::Cancelled),
                Err(e) => { dbg_conn(&format!("file download failed: {}", e.chars().take(80).collect::<String>())); shared::download_phase(&key, Phase::Failed(e)) }
            }
        });
        Ok(())
    }
}

/// After a download: "Öffnen" / "In Ordner anzeigen".
pub(crate) fn finish(path: &Path, then: Then) {
    match then { Then::Open => shared::open_file(path), Then::ShowInFolder => shared::show_in_folder(path), Then::Nothing | Then::Play => {} }
}

/// Uploads every file (and the thumbnails of images); returns the file items (encrypted) and their mime types.
fn upload_all(agent: &ureq::Agent, token: &str, r: &SendFiles, key: &[u8], key_url: &str, space_url: &str, hidden: Option<&str>, cancel: &Arc<AtomicBool>) -> Result<(Vec<Value>, Vec<String>), String> {
    let (mut items, mut mimes) = (vec![], vec![]);
    for p in &r.files {
        if cancel.load(Ordering::SeqCst) { return Err(CANCELLED.into()); }
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Datei".into());
        let meta = std::fs::metadata(p).map_err(|_| format!("{name}: Datei nicht gefunden"))?;
        if meta.len() > shared::limit_bytes() { return Err(TOO_LARGE.into()); }
        let mut data = std::fs::read(p).map_err(|e| format!("{name}: {e}"))?;
        let size = data.len() as u64;
        let mime = shared::mime_of(&name, &data[..data.len().min(64)]);
        let thumb = if mime.starts_with("image/") && hidden.is_some() { thumbnail(&data) } else { None };
        let scr = NewScr::new(&activities::iso_now());
        let tag = scr.seal(&mut data)?;
        let url = upload_blob(agent, token, space_url, &data, Some(&claimed_type(&name)), &r.upload, cancel, true)?;
        drop(data);
        let scr_enc = jose::encrypt_dir_nokid(key, scr.json(&url, &tag).to_string().as_bytes())?;
        let mut item = Item { name_enc: jose::encrypt_dir(key, key_url, name.as_bytes())?, size, mime: mime.clone(), url, scr_enc, dims: None, thumb: None };
        if let (Some((mut png, tw, th, w, h)), Some(hs)) = (thumb, hidden) {
            item.dims = Some((w, h));
            let tscr = NewScr::new(&activities::iso_now());
            let ttag = tscr.seal(&mut png)?;
            // A failed thumbnail does not stop the file.
            match upload_blob(agent, token, hs, &png, None, &r.upload, cancel, false) {
                Ok(turl) => item.thumb = Some((tw, th, turl.clone(), jose::encrypt_dir_nokid(key, tscr.json(&turl, &ttag).to_string().as_bytes())?)),
                Err(e) if e == CANCELLED => return Err(e),
                Err(e) => dbg_conn(&format!("thumbnail upload failed: {}", e.chars().take(60).collect::<String>())),
            }
        }
        items.push(file_item(key_url, &item));
        mimes.push(mime);
    }
    Ok((items, mimes))
}

/// Download + decrypt one file (any size) into `out`, with progress and cancel. Memory stays at one read buffer: the bytes are
/// decrypted as they arrive (`stream::GcmStream`) and the tag is checked at the end (an error leaves a file the caller deletes).
fn fetch_to(agent: &ureq::Agent, token: &str, scr: &Value, key: &str, cancel: &Arc<AtomicBool>, out: &Path) -> Result<(), String> {
    use std::io::Write;
    let loc = st(scr, "loc");
    let plain_get = images::giphy_host(&loc);
    let url = if plain_get { loc.replacen("http://", "https://", 1) } else {
        if !loc.starts_with("https://") || !images::token_host(&loc) { return Err("content location is not a Webex host".into()); }
        let host = loc["https://".len()..].split('/').next().unwrap_or("");
        let ep = call(agent, token, "POST", &format!("https://{host}/v1/download/endpoints"), Some(json!({"endpoints": [loc]}))).map_err(|(c, _)| format!("endpoints http {c}"))?;
        Some(st(&ep["endpoints"], &loc)).filter(|u| !u.is_empty()).unwrap_or(loc.clone())
    };
    if !url.starts_with("https://") { return Err("download url is not https".into()); }
    let mut req = agent.get(&url);
    if !plain_get && images::token_host(&url) { req = req.set("Authorization", &format!("Bearer {token}")); }
    let resp = req.call().map_err(|e| match e { ureq::Error::Status(c, _) => format!("http {c}"), e => e.kind().to_string() })?;
    let total = resp.header("content-length").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let mut gcm = if plain_get { None } else {
        if !st(scr, "enc").eq_ignore_ascii_case("A256GCM") { return Err("unsupported content cipher".into()); }
        let (k, iv, tag) = (jose::unb64(&st(scr, "key"))?, jose::unb64(&st(scr, "iv"))?, jose::unb64(&st(scr, "tag")).unwrap_or_default());
        Some((stream::GcmStream::new(&k, &iv, st(scr, "aad").as_bytes(), tag.is_empty())?, tag))
    };
    let mut rd = resp.into_reader().take(shared::DEFAULT_LIMIT_MB * 1024 * 1024 + 16);
    let mut file = std::io::BufWriter::new(std::fs::File::create(out).map_err(|e| e.to_string())?);
    let (mut buf, mut plain) = (vec![0u8; 256 * 1024], Vec::with_capacity(256 * 1024));
    let mut got = 0u64;
    loop {
        if cancel.load(Ordering::SeqCst) { return Err(CANCELLED.into()); }
        let n = rd.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        got += n as u64;
        match &mut gcm {
            Some((g, _)) => { plain.clear(); g.update(&buf[..n], &mut plain); file.write_all(&plain).map_err(|e| e.to_string())?; }
            None => file.write_all(&buf[..n]).map_err(|e| e.to_string())?,
        }
        shared::download_progress(key, got, total.max(got));
    }
    if got == 0 { return Err("leere Datei".into()); }
    if let Some((g, tag)) = gcm { g.finish(&tag)?; }
    file.flush().map_err(|e| e.to_string())
}

mod stream;

#[cfg(test)]
mod tests;
