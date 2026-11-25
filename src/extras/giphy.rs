//! Giphy client for the composer's GIF picker (Giphy is called directly, not via Webex).
//!
//! Host `https://api.giphy.com/v1/gifs/`, query key `api_key`, trending on open with `offset 0, limit 30`,
//! renditions `fixed_width_downsampled` (grid thumbnails), `downsized_medium` (preview / the file that is shared) and
//! `original_still`. No `rating` is sent, so Giphy returns results of every rating (no content filter).
//!
//! API key: not in the source tree. `build.rs` embeds it from `secrets.toml` (`giphy_api_key`, see `crate::cloud::secrets`),
//! and the env var `WEBOX_GIPHY_API_KEY` overrides it. Without one the picker shows its error state. The key is never
//! printed or logged.
use serde_json::Value;
use std::sync::mpsc::Sender;

pub const API_BASE: &str = "https://api.giphy.com/v1/gifs/";
/// Stickers (same key): `/v1/stickers/trending` and `/v1/stickers/search`. A shared sticker is sent exactly like a GIF.
pub const STICKER_BASE: &str = "https://api.giphy.com/v1/stickers/";
pub const PAGE_LIMIT: u32 = 30;
/// Cap for one downloaded thumbnail.
pub const THUMB_MAX_BYTES: u64 = 2 * 1024 * 1024;
/// Search is sent this long after the last keystroke.
pub const DEBOUNCE_SECS: f64 = 0.3;
pub const TERMS_URL: &str = "https://support.giphy.com/hc/en-us/articles/360020027752-GIPHY-User-Terms-of-Service";

pub fn api_key() -> Option<String> {
    let k = crate::cloud::secrets::giphy_api_key()?;
    let k = k.trim().to_string();
    (!k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric())).then_some(k)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind { Trending, Search(String), StickerTrending, StickerSearch(String) }

impl Kind {
    #[allow(dead_code)]
    pub fn is_sticker(&self) -> bool { matches!(self, Kind::StickerTrending | Kind::StickerSearch(_)) }
}

/// One result: the grid thumbnail (`fixed_width_downsampled`) and the file that is shared (`downsized_medium`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gif { pub id: String, pub title: String, pub thumb_url: String, pub thumb_w: u32, pub thumb_h: u32, pub media_url: String, pub media_w: u32, pub media_h: u32 }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page { pub gifs: Vec<Gif>, pub total: u32, pub offset: u32, pub count: u32 }

#[derive(Clone, Debug, PartialEq)]
pub enum FetchError { NoKey, Network, Http(u16), Parse }

pub enum Event { Page { ticket: u64, res: Result<Page, FetchError> }, Thumb { id: String, bytes: Option<Vec<u8>> }, Categories(Result<Vec<Category>, FetchError>) }

/// A home-screen tile of the picker: Giphy's top-level category (`/v1/gifs/categories`: `name`, `name_encoded`, `gif.images.*`).
/// The sub-categories (786 of them) carry only a name, no picture, so they are not used as tiles.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Category { pub name: String, pub query: String, pub thumb_url: String }

pub fn categories_url(key: &str) -> String {
    format!("{API_BASE}categories?api_key={}", pct(key))
}

/// Parses `/v1/gifs/categories`: `data[] = { name, name_encoded, subcategories[], gif{ images{ fixed_width_downsampled{url} } } }`.
pub fn parse_categories(body: &str) -> Result<Vec<Category>, FetchError> {
    let v: Value = serde_json::from_str(body).map_err(|_| FetchError::Parse)?;
    let data = v["data"].as_array().ok_or(FetchError::Parse)?;
    let mut out = vec![];
    for d in data {
        let Some(name) = d["name"].as_str().map(str::trim).filter(|n| !n.is_empty()) else { continue };
        let im = &d["gif"]["images"];
        let thumb = ["fixed_width_downsampled", "fixed_width", "fixed_height_downsampled", "original"].iter().find_map(|k| im[*k]["url"].as_str());
        let Some(thumb) = thumb else { continue };
        out.push(Category { name: name.to_string(), query: name.to_string(), thumb_url: thumb.to_string() });
    }
    Ok(out)
}

/// Loads the home-screen categories on a worker thread (read-only GET).
pub fn fetch_categories(tx: Sender<Event>) {
    std::thread::spawn(move || {
        let res = (|| {
            let key = api_key().ok_or(FetchError::NoKey)?;
            let r = agent().get(&categories_url(&key)).call().map_err(|e| match e { ureq::Error::Status(c, _) => FetchError::Http(c), _ => FetchError::Network })?;
            let mut body = String::new();
            std::io::Read::read_to_string(&mut std::io::Read::take(r.into_reader(), 4 * 1024 * 1024), &mut body).map_err(|_| FetchError::Network)?;
            parse_categories(&body)
        })();
        let _ = tx.send(Event::Categories(res));
    });
}

/// Percent-encodes a query value (RFC 3986 unreserved characters stay).
pub fn pct(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { o.push(b as char) } else { o.push_str(&format!("%{b:02X}")) }
    }
    o
}

/// Request URL. Giphy's `lang` only exists for search. No `rating`: Giphy then returns every rating.
pub fn build_url(kind: &Kind, key: &str, lang: &str, limit: u32, offset: u32) -> String {
    match kind {
        Kind::Trending => format!("{API_BASE}trending?api_key={}&limit={limit}&offset={offset}", pct(key)),
        Kind::Search(q) => format!("{API_BASE}search?api_key={}&q={}&limit={limit}&offset={offset}&lang={}", pct(key), pct(q), pct(lang)),
        Kind::StickerTrending => format!("{STICKER_BASE}trending?api_key={}&limit={limit}&offset={offset}", pct(key)),
        Kind::StickerSearch(q) => format!("{STICKER_BASE}search?api_key={}&q={}&limit={limit}&offset={offset}&lang={}", pct(key), pct(q), pct(lang)),
    }
}

fn num(v: &Value) -> u32 { v.as_u64().map(|n| n as u32).or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0) }

/// Parses a `trending` / `search` response (`data[].images.*{url,width,height}` — width/height are strings in the real API).
pub fn parse_page(body: &str) -> Result<Page, FetchError> {
    let v: Value = serde_json::from_str(body).map_err(|_| FetchError::Parse)?;
    let data = v["data"].as_array().ok_or(FetchError::Parse)?;
    let mut gifs = vec![];
    for d in data {
        let im = &d["images"];
        let thumb = if im["fixed_width_downsampled"]["url"].is_string() { &im["fixed_width_downsampled"] } else { &im["fixed_width"] };
        let media = if im["downsized_medium"]["url"].is_string() { &im["downsized_medium"] } else { &im["original"] };
        let (Some(tu), Some(mu)) = (thumb["url"].as_str(), media["url"].as_str()) else { continue };
        let (tw, th) = (num(&thumb["width"]), num(&thumb["height"]));
        if tw == 0 || th == 0 { continue }
        gifs.push(Gif { id: d["id"].as_str().unwrap_or(tu).to_string(), title: d["title"].as_str().unwrap_or("").trim().to_string(),
            thumb_url: tu.to_string(), thumb_w: tw, thumb_h: th, media_url: mu.to_string(), media_w: num(&media["width"]), media_h: num(&media["height"]) });
    }
    let p = &v["pagination"];
    Ok(Page { total: num(&p["total_count"]), offset: num(&p["offset"]), count: if p["count"].is_null() { gifs.len() as u32 } else { num(&p["count"]) }, gifs })
}

/// Thumbnails and shared files come from Giphy's public CDN over TLS only.
pub fn media_host_ok(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else { return false };
    if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() || u.port().is_some() { return false }
    matches!(u.host(), Some(url::Host::Domain(h)) if h == "giphy.com" || h.strip_suffix(".giphy.com").is_some_and(|p| !p.is_empty()))
}

fn agent() -> &'static ureq::Agent {
    static A: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    A.get_or_init(|| crate::cloud::tls::agent_for(API_BASE).timeout_connect(std::time::Duration::from_secs(8)).timeout(std::time::Duration::from_secs(20)).build())
}

/// Loads one page on a worker thread; the answer arrives as `Event::Page` with the same `ticket`.
pub fn fetch_page(tx: Sender<Event>, ticket: u64, kind: Kind, offset: u32) {
    std::thread::spawn(move || {
        let res = (|| {
            let key = api_key().ok_or(FetchError::NoKey)?;
            let url = build_url(&kind, &key, "de", PAGE_LIMIT, offset);
            let r = agent().get(&url).call().map_err(|e| match e { ureq::Error::Status(c, _) => FetchError::Http(c), _ => FetchError::Network })?;
            let mut body = String::new();
            std::io::Read::read_to_string(&mut std::io::Read::take(r.into_reader(), 4 * 1024 * 1024), &mut body).map_err(|_| FetchError::Network)?;
            parse_page(&body)
        })();
        let _ = tx.send(Event::Page { ticket, res });
    });
}

/// Downloads one thumbnail (<= 2 MB, giphy CDN, https only); `bytes` is `None` on any failure.
pub fn fetch_thumb(tx: Sender<Event>, id: String, url: String) {
    std::thread::spawn(move || {
        let bytes = (|| {
            if !media_host_ok(&url) { return None }
            let r = agent().get(&url).call().ok()?;
            let mut b = vec![];
            std::io::Read::read_to_end(&mut std::io::Read::take(r.into_reader(), THUMB_MAX_BYTES + 1), &mut b).ok()?;
            (!b.is_empty() && b.len() as u64 <= THUMB_MAX_BYTES).then_some(b)
        })();
        let _ = tx.send(Event::Thumb { id, bytes });
    });
}

/// Search-as-you-type delay: `set` records the newest text, `due` hands it out once `DEBOUNCE_SECS` passed without a change.
#[derive(Default, Clone)]
pub struct Debounce { pending: Option<(String, f64)>, last: String }
impl Debounce {
    pub fn set(&mut self, text: &str, now: f64) {
        if self.pending.as_ref().map_or(self.last == text, |p| p.0 == text) { return }
        self.pending = Some((text.to_string(), now));
    }
    pub fn due(&mut self, now: f64) -> Option<String> {
        let (t, at) = self.pending.as_ref()?;
        if now - at < DEBOUNCE_SECS { return None }
        let t = t.clone();
        self.pending = None;
        self.last = t.clone();
        Some(t)
    }
    /// Seconds until `due` fires (for the repaint timer).
    pub fn wait(&self, now: f64) -> Option<f64> { self.pending.as_ref().map(|(_, at)| (DEBOUNCE_SECS - (now - at)).max(0.0)) }
    pub fn reset(&mut self) { self.pending = None; self.last.clear(); }
}

/// Offline sample results (screenshot / `--sample` mode): small generated animated GIFs, no network.
pub fn sample_gifs() -> Vec<(Gif, Vec<u8>)> {
    use image::{codecs::gif::GifEncoder, Delay, Frame, Rgba, RgbaImage};
    let hs = [150u32, 112, 200, 130, 96, 170, 124, 188, 140, 104, 160, 118];
    let titles = ["Thumbs up", "Applause", "Dancing", "Head shake", "Laughing", "Wow", "Celebrate", "Wink", "Thank you", "Mic drop", "Coffee", "Good night"];
    hs.iter().enumerate().map(|(i, &h)| {
        let w = 200u32;
        let mut out = vec![];
        {
            let mut enc = GifEncoder::new(&mut out);
            let frames: Vec<Frame> = (0..4u32).map(|f| {
                let hue = (i as f32 * 0.37 + f as f32 * 0.06).fract();
                let rgb = |o: f32| { let k = (hue + o).fract(); let t = (k * 6.0 - 3.0).abs() - 1.0; (80.0 + 150.0 * t.clamp(0.0, 1.0)) as u8 };
                let img = RgbaImage::from_fn(w, h, |x, y| {
                    let band = (((x + y) as f32 / 18.0 + f as f32 * 1.2).sin() * 0.5 + 0.5) * 0.35 + 0.65;
                    let c = |o: f32| (rgb(o) as f32 * band) as u8;
                    Rgba([c(0.0), c(0.33), c(0.66), 255])
                });
                Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(180, 1))
            }).collect();
            let _ = enc.encode_frames(frames);
        }
        let g = Gif { id: format!("sample-gif-{i}"), title: titles[i].to_string(), thumb_url: String::new(), thumb_w: w, thumb_h: h, media_url: format!("https://media.giphy.com/media/sample-gif-{i}/giphy.gif"), media_w: 480, media_h: h * 480 / w };
        (g, out)
    }).collect()
}

/// Offline sample home screen: category names with (generated) sample GIFs standing in for the previews (the second tuple item is
/// the index into `sample_gifs()` whose bytes the preview uses).
pub fn sample_categories() -> Vec<(Category, usize)> {
    ["hello", "lol", "love", "happy birthday", "wow", "thanks", "dance", "coffee"].iter().enumerate()
        .map(|(i, n)| (Category { name: n.to_string(), query: n.to_string(), thumb_url: String::new() }, i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shape of Giphy's public API docs (`/v1/gifs/trending`): numbers are strings, one entry lacks the renditions.
    const SAMPLE: &str = r#"{"data":[
      {"type":"gif","id":"xT9IgzoKnwFNmISR8I","title":"Excited Happy GIF by Pudgy Penguins","images":{
         "original_still":{"url":"https://media2.giphy.com/media/xT9IgzoKnwFNmISR8I/giphy_s.gif","width":"480","height":"270"},
         "downsized_medium":{"url":"https://media2.giphy.com/media/xT9IgzoKnwFNmISR8I/giphy.gif?cid=1","width":"480","height":"270","size":"900000"},
         "fixed_width_downsampled":{"url":"https://media2.giphy.com/media/xT9IgzoKnwFNmISR8I/200w_d.gif","width":"200","height":"113","size":"30000"}}},
      {"type":"gif","id":"abc","title":"  ","images":{
         "original":{"url":"https://media0.giphy.com/media/abc/giphy.gif","width":"300","height":"300"},
         "fixed_width":{"url":"https://media0.giphy.com/media/abc/200w.gif","width":200,"height":200}}},
      {"type":"gif","id":"broken","title":"x","images":{}}
    ],"pagination":{"total_count":2300,"count":3,"offset":30},"meta":{"status":200,"msg":"OK"}}"#;

    #[test]
    fn parses_trending_response() {
        let p = parse_page(SAMPLE).unwrap();
        assert_eq!((p.total, p.offset, p.count), (2300, 30, 3));
        assert_eq!(p.gifs.len(), 2, "entry without renditions is skipped");
        let g = &p.gifs[0];
        assert_eq!((g.thumb_w, g.thumb_h, g.media_w, g.media_h), (200, 113, 480, 270));
        assert!(g.thumb_url.ends_with("200w_d.gif") && g.media_url.contains("giphy.gif") && g.title.starts_with("Excited"));
        // Fallbacks: fixed_width / original when the downsampled renditions are missing; numeric width/height accepted.
        let g = &p.gifs[1];
        assert_eq!((g.thumb_w, g.thumb_h, g.media_w), (200, 200, 300));
        assert_eq!(g.title, "");
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_page("not json"), Err(FetchError::Parse));
        assert_eq!(parse_page(r#"{"meta":{"status":429}}"#), Err(FetchError::Parse));
    }

    #[test]
    fn builds_urls() {
        assert_eq!(build_url(&Kind::Trending, "K3Y", "de", 30, 0), "https://api.giphy.com/v1/gifs/trending?api_key=K3Y&limit=30&offset=0");
        let u = build_url(&Kind::Search("cat & dog ä".into()), "K3Y", "de", 30, 60);
        assert_eq!(u, "https://api.giphy.com/v1/gifs/search?api_key=K3Y&q=cat%20%26%20dog%20%C3%A4&limit=30&offset=60&lang=de");
        assert!(!u.contains("rating="), "no content filter");
    }

    #[test]
    fn sticker_urls() {
        assert_eq!(build_url(&Kind::StickerTrending, "K3Y", "de", 30, 0), "https://api.giphy.com/v1/stickers/trending?api_key=K3Y&limit=30&offset=0");
        let u = build_url(&Kind::StickerSearch("hello world".into()), "K3Y", "de", 30, 30);
        assert_eq!(u, "https://api.giphy.com/v1/stickers/search?api_key=K3Y&q=hello%20world&limit=30&offset=30&lang=de");
        assert!(Kind::StickerSearch("x".into()).is_sticker() && !Kind::Trending.is_sticker());
        assert_eq!(categories_url("K3Y"), "https://api.giphy.com/v1/gifs/categories?api_key=K3Y");
    }

    /// Shape of `GET /v1/gifs/categories` (each entry: name, name_encoded, subcategories[{name,name_encoded}], gif{images}).
    #[test]
    fn parses_categories() {
        let body = r#"{"data":[
          {"name":"Actions","name_encoded":"actions","subcategories":[{"name":"breaking up","name_encoded":"breaking-up"}],
           "gif":{"id":"3o6wr","images":{"fixed_width_downsampled":{"url":"https://media3.giphy.com/media/3o6wr/200w_d.gif","width":"200","height":"108"}}}},
          {"name":"Art & Design","name_encoded":"art-design","subcategories":[],"gif":{"images":{"fixed_width":{"url":"https://media0.giphy.com/media/x/200w.gif"}}}},
          {"name":"No picture","name_encoded":"np","subcategories":[]},
          {"name":" ","gif":{"images":{"fixed_width":{"url":"https://media0.giphy.com/media/y/200w.gif"}}}}
        ],"pagination":{"total_count":4,"count":4,"offset":0},"meta":{"status":200,"msg":"OK"}}"#;
        let c = parse_categories(body).unwrap();
        assert_eq!(c.len(), 2, "entries without a name or a picture are skipped");
        assert_eq!((c[0].name.as_str(), c[0].query.as_str()), ("Actions", "Actions"));
        assert!(c[0].thumb_url.ends_with("200w_d.gif"));
        assert!(c[1].thumb_url.ends_with("200w.gif"), "falls back to fixed_width");
        assert_eq!(parse_categories("nope"), Err(FetchError::Parse));
        assert_eq!(parse_categories(r#"{"meta":{"status":401}}"#), Err(FetchError::Parse));
    }

    #[test]
    fn only_giphy_https_hosts() {
        assert!(media_host_ok("https://media2.giphy.com/media/x/200w_d.gif"));
        assert!(media_host_ok("https://giphy.com/x"));
        assert!(!media_host_ok("http://media.giphy.com/x"));
        assert!(!media_host_ok("https://evilgiphy.com/x"));
        assert!(!media_host_ok("https://giphy.com.evil.example/x"));
        assert!(media_host_ok("https://Media.GIPHY.com/x"));
        for no in ["https://evil.example#.giphy.com", "https://evil.example?.giphy.com", "https://evil.example\\.giphy.com/x", "https://media.giphy.com@evil.example/x",
            "https://user@media.giphy.com/x", "https://u:p@media.giphy.com/x", "https://media.giphy.com./x", "https://giphy.com.evil.example./x", "https://.giphy.com/x",
            "https://media.giphy.com:8443/x", "https://evil.example/media.giphy.com", "giphy.com/x", "not a url"] { assert!(!media_host_ok(no), "{no}"); }
    }

    #[test]
    fn debounce_waits_for_a_pause() {
        let mut d = Debounce::default();
        d.set("k", 10.0);
        assert_eq!(d.due(10.1), None);
        d.set("ka", 10.2); // typing on restarts the timer
        assert_eq!(d.due(10.45), None);
        assert_eq!(d.due(10.51).as_deref(), Some("ka"));
        assert_eq!(d.due(11.0), None, "fires once");
        d.set("ka", 12.0); // unchanged text: nothing to do
        assert_eq!(d.due(13.0), None);
        d.set("", 14.0); // clearing the field is a change too
        assert_eq!(d.due(14.4).as_deref(), Some(""));
    }

    #[test]
    fn sample_gifs_decode_as_animated() {
        use image::AnimationDecoder;
        let s = sample_gifs();
        assert_eq!(s.len(), 12);
        let frames = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(&s[0].1)).unwrap().into_frames().count();
        assert_eq!(frames, 4);
    }
}
