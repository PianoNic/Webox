//! Link previews for links in received messages, unfurled by the client itself.
//!
//! Every message with links is unfurled on the receiving client, per URL (index 0, 1, ...), with a preview only when there is
//! enough info. The setting: "Show Link Previews" ("Link-Vorschau anzeigen"), on by default (settings key `show_link_previews`).
//!
//! The pipeline:
//! - Only http(s) URLs to public host names are fetched (no IP literals, no localhost / intranet names).
//! - The page is downloaded by the client itself, with no Webex token and no cookies. Limits: MAX_PAGE, 6 s, 5 redirects.
//! - The parser reads `<title`, `<meta ` and `<link ` up to `</head`, keys `og:title` `twitter:title` `title`,
//!   `og:description` `twitter:description` `description`, `og:image` then `twitter:image`, `og:site_name` then
//!   `twitter:site` (og first).
//! - "Sufficient preview info": a title and a description, and the description has at least 10 words, or at least 20
//!   characters with CJK in it. The image is optional; with none the card keeps its empty 200 x 105
//!   `attachment-unfurl-border` box.
//! - Image: only jpg / png / gif, else the card has no picture.
//! Never runs with --sample (offline).
use crate::*;
use std::sync::mpsc::{channel, Receiver, Sender};

const MAX_PAGE: u64 = 768 * 1024;
const MAX_IMAGE: u64 = 3 * 1024 * 1024;
const MAX_URLS_PER_MESSAGE: usize = 3;
const MAX_MESSAGES: usize = 60; // newest messages of the open space that are looked at
const MAX_FETCHES: usize = 4; // page fetches running at once (each on its own short-lived thread)
const MAX_REDIRECTS: u32 = 5;

/// Fetch results: (url, preview or None, image bytes).
type Done = (String, Option<cloud::LinkPreview>, Option<Vec<u8>>);

#[derive(Default)]
pub(crate) struct Unfurl { cache: HashMap<String, Option<cloud::LinkPreview>>, pending: HashSet<String>, chan: Option<(Sender<Done>, Receiver<Done>)> }

/// Only public web pages: http(s), a host name with a dot, no IP literal, no localhost / .local / .internal / .lan names.
pub(crate) fn fetchable(u: &str) -> bool {
    let rest = match u.strip_prefix("https://").or_else(|| u.strip_prefix("http://")) { Some(r) => r, None => return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("").rsplit('@').next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("").to_lowercase();
    if host.is_empty() || !host.contains('.') || host.starts_with('[') { return false; }
    if host.split('.').all(|p| p.chars().all(|c| c.is_ascii_digit())) { return false; }
    !(host == "localhost" || [".local", ".internal", ".lan", ".localhost", ".corp", ".home"].iter().any(|s| host.ends_with(s)))
}

/// HTML character references in attribute values / <title>: numeric (&#183; &#xB7;) and the common named ones.
fn decode(s: &str) -> String {
    if !s.contains('&') { return s.to_string(); }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let end = rest[1..].find(|c: char| c == ';' || c == '&' || c.is_whitespace()).map(|e| e + 1).filter(|&e| e <= 12 && rest[e..].starts_with(';'));
        let ch = end.and_then(|e| {
            let name = &rest[1..e];
            if let Some(n) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) { return u32::from_str_radix(n, 16).ok().and_then(char::from_u32) }
            if let Some(n) = name.strip_prefix('#') { return n.parse::<u32>().ok().and_then(char::from_u32) }
            Some(match name { "amp" => '&', "quot" => '"', "apos" => '\'', "lt" => '<', "gt" => '>', "nbsp" => ' ', "middot" => '·', "ndash" => '–', "mdash" => '—',
                "hellip" => '…', "laquo" => '«', "raquo" => '»', "lsquo" => '‘', "rsquo" => '’', "ldquo" => '“', "rdquo" => '”', "bull" => '•', "copy" => '©', "reg" => '®', "trade" => '™',
                "auml" => 'ä', "ouml" => 'ö', "uuml" => 'ü', "Auml" => 'Ä', "Ouml" => 'Ö', "Uuml" => 'Ü', "szlig" => 'ß', "eacute" => 'é', "egrave" => 'è', "agrave" => 'à', _ => return None })
        });
        match (ch, end) { (Some(c), Some(e)) => { out.push(c); rest = &rest[e + 1..]; } _ => { out.push('&'); rest = &rest[1..]; } }
    }
    out.push_str(rest);
    out
}

/// `href` resolved against the page's (final, after redirects) URL: absolute, protocol-relative, root-relative or relative.
pub(crate) fn resolve(base: &str, href: &str) -> String {
    let href = href.trim();
    if href.is_empty() || href.starts_with("http://") || href.starts_with("https://") || href.starts_with("data:") { return href.to_string(); }
    let scheme = if base.starts_with("http://") { "http:" } else { "https:" };
    if href.starts_with("//") { return format!("{scheme}{href}"); }
    let after = base.split_once("://").map_or("", |x| x.1);
    let host_end = after.find(['/', '?', '#']).unwrap_or(after.len());
    let origin = &base[..base.len() - after.len() + host_end];
    if href.starts_with('/') { return format!("{origin}{href}"); }
    let path = after[host_end..].split(['?', '#']).next().unwrap_or("");
    if href.starts_with('?') { return format!("{origin}{}{href}", if path.is_empty() { "/" } else { path }); }
    let dir = &path[..path.rfind('/').map_or(0, |i| i + 1)];
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    let (rel, tail) = match href.find(['?', '#']) { Some(i) => (&href[..i], &href[i..]), None => (href, "") };
    let segs: Vec<&str> = rel.split('/').collect();
    for (i, s) in segs.iter().enumerate() {
        match *s { "." => {} ".." => { parts.pop(); } "" if i + 1 < segs.len() => {} s => parts.push(s) }
    }
    format!("{origin}/{}{tail}", parts.join("/"))
}

/// Accepted unfurl pictures: jpg / png / gif (by content).
pub(crate) fn allowed_image(b: &[u8]) -> bool {
    b.starts_with(&[0xFF, 0xD8, 0xFF]) || b.starts_with(b"\x89PNG\r\n\x1a\n") || b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")
}

/// "Sufficient preview info": a title and a description of at least 10 words, or of at least 20 characters that contain
/// CJK (CJK text has no spaces between words).
pub(crate) fn sufficient(title: &str, description: &str) -> bool {
    if title.is_empty() || description.is_empty() { return false; }
    if description.split_whitespace().count() >= 10 { return true; }
    let cjk = |c: char| matches!(c as u32, 0x1100..=0x11FF | 0x2E80..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FFFF);
    description.chars().count() >= 20 && description.chars().any(cjk)
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let low = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(p) = low[from..].find(name) {
        let at = from + p;
        let before_ok = at == 0 || low.as_bytes()[at - 1].is_ascii_whitespace();
        let after = low[at + name.len()..].trim_start();
        if before_ok && after.starts_with('=') {
            let vstart = tag.len() - after.len() + 1;
            let v = tag[vstart..].trim_start();
            let q = v.chars().next()?;
            return if q == '"' || q == '\'' { v[1..].find(q).map(|e| &v[1..1 + e]) } else { Some(v.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or("")) };
        }
        from = at + name.len();
    }
    None
}

/// OpenGraph / twitter / <title> / description of a page (`html` as served at `page_url`, the URL after redirects) -> the
/// preview for the link `url` + its image URL (absolute, "" for none); `None` without sufficient preview info.
pub(crate) fn parse_page(url: &str, page_url: &str, html: &str) -> Option<(cloud::LinkPreview, String)> {
    let mut cut = html.len().min(300_000);
    while !html.is_char_boundary(cut) { cut -= 1; }
    let head = &html[..cut];
    let low = head.to_ascii_lowercase();
    let head = &head[..low.find("</head").unwrap_or(low.len())]; // the parser stops at </head
    let low = &low[..head.len()];
    let mut meta: HashMap<String, String> = HashMap::new();
    let mut i = 0;
    while let Some(p) = low[i..].find("<meta") {
        let s = i + p;
        let e = low[s..].find('>').map_or(low.len(), |e| s + e);
        let tag = &head[s..e];
        if let (Some(k), Some(v)) = (attr(tag, "property").or_else(|| attr(tag, "name")), attr(tag, "content")) {
            meta.entry(k.to_ascii_lowercase()).or_insert_with(|| decode(v.trim()));
        }
        i = e;
    }
    let title_tag = low.find("<title").and_then(|s| { let a = low[s..].find('>')? + s + 1; let b = low[a..].find("</title")? + a; Some(decode(head[a..b].trim())) });
    let pick = |ks: &[&str]| ks.iter().find_map(|k| meta.get(*k).filter(|v| !v.is_empty()).cloned());
    let one_line = |s: String| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = one_line(pick(&["og:title", "twitter:title"]).or(title_tag).unwrap_or_default());
    let description = one_line(pick(&["og:description", "twitter:description", "description"]).unwrap_or_default());
    if !sufficient(&title, &description) { return None; }
    // With neither og:site_name nor twitter:site the host is shown as the site name.
    let host = url.split("://").nth(1).unwrap_or("").split(['/', '?', '#']).next().unwrap_or("").rsplit('@').next().unwrap_or("").to_string();
    let site = pick(&["og:site_name", "twitter:site"]).unwrap_or(host);
    let img = pick(&["og:image", "twitter:image"]).map(|i| resolve(page_url, &i)).unwrap_or_default();
    Some((cloud::LinkPreview { url: url.to_string(), title, description, site, image_key: String::new() }, img))
}

fn read_capped(r: ureq::Response, cap: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut b = Vec::new();
    r.into_reader().take(cap).read_to_end(&mut b).ok()?;
    Some(b)
}

fn agent() -> ureq::Agent {
    // A browser user agent, as some sites serve their meta tags only to browsers.
    ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(6)).redirects(MAX_REDIRECTS)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36").build()
}

/// The page behind `url` (following redirects, html only, at most MAX_PAGE): (final URL, html).
fn fetch_page(url: &str) -> Option<(String, String)> {
    let r = agent().get(url).set("Accept", "text/html,application/xhtml+xml").call().ok()?;
    let fin = r.get_url().to_string();
    if !r.content_type().contains("html") || !fetchable(&fin) { return None; }
    Some((fin, String::from_utf8_lossy(&read_capped(r, MAX_PAGE)?).into_owned()))
}

/// The preview image: jpg / png / gif only, at most MAX_IMAGE, and it must decode.
fn fetch_image(img: &str) -> Option<Vec<u8>> {
    if !fetchable(img) { return None; }
    let r = agent().get(img).set("Accept", "image/png,image/jpeg,image/gif,image/*;q=0.5").call().ok()?;
    if !fetchable(r.get_url()) { return None; }
    read_capped(r, MAX_IMAGE).filter(|b| allowed_image(b) && image::load_from_memory(b).is_ok())
}

fn fetch(url: &str) -> Done {
    let Some((mut pv, img)) = fetch_page(url).and_then(|(fin, h)| parse_page(url, &fin, &h)) else { return (url.to_string(), None, None) };
    let bytes = fetch_image(&img);
    if bytes.is_some() { pv.image_key = format!("unfurl:{url}"); }
    (url.to_string(), Some(pv), bytes)
}

impl App {
    fn previews_on(&self) -> bool { self.settings["show_link_previews"].as_bool().unwrap_or(true) }

    /// Once per frame for the open space: start unfurling new links, collect results, attach previews to messages.
    pub(crate) fn unfurl_tick(&mut self, ctx: &egui::Context, space: &str) {
        if self.client.is_none() || self.shot.is_some() || !self.previews_on() { return; }
        let st = &mut self.msgx.unfurl;
        let (tx, rx) = st.chan.get_or_insert_with(channel);
        while let Ok((url, pv, img)) = rx.try_recv() {
            st.pending.remove(&url);
            if let (Some(p), Some(b)) = (&pv, img) { self.photos.add_images(vec![(p.image_key.clone(), b)]); }
            st.cache.insert(url, pv);
        }
        let tx = tx.clone();
        let Some(list) = self.messages.get_mut(space) else { return };
        let n = list.len();
        for m in list.iter_mut().skip(n.saturating_sub(MAX_MESSAGES)).filter(|m| m.notice.is_empty() && m.previews.is_empty()) {
            let urls: Vec<String> = super::messages::url_spans(&m.text).into_iter().map(|(a, b)| m.text[a..b].to_string())
                .map(|u| if u.starts_with("www.") { format!("https://{u}") } else { u }).filter(|u| fetchable(u)).take(MAX_URLS_PER_MESSAGE).collect();
            let mut got = Vec::new();
            for u in urls {
                match st.cache.get(&u) {
                    Some(Some(p)) => got.push(p.clone()),
                    Some(None) => {}
                    // At most MAX_FETCHES at once; the rest start on a later frame (each result asks for a repaint).
                    None => if st.pending.len() < MAX_FETCHES && st.pending.insert(u.clone()) { let tx = tx.clone(); let c = ctx.clone(); std::thread::spawn(move || { let _ = tx.send(fetch(&u)); c.request_repaint(); }); },
                }
            }
            if !got.is_empty() { m.previews = got; m.rev = 0; }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_web_pages_are_fetched() {
        for ok in ["https://gitlab.com/x", "http://www.example.org/a?b", "https://user@sub.example.com:8443/p"] { assert!(fetchable(ok), "{ok}"); }
        for no in ["file:///c:/x", "https://localhost/x", "http://10.0.0.1/x", "https://intranet/x", "https://nas.local/", "https://[::1]/", "javascript:alert(1)", "https://printer.lan"] { assert!(!fetchable(no), "{no}"); }
    }

    const DESC: &str = "A small made-up tool that copies files between two example machines over the network";

    /// A code-hosting project page (synthetic content, real meta structure).
    #[test]
    fn project_page_gives_image_title_description_and_site() {
        let h = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/unfurl/project_page.html")).unwrap();
        let (p, img) = parse_page("https://code.example.com/example-user/filedrop", "https://code.example.com/example-user/filedrop", &h).unwrap();
        assert_eq!(p.title, "example-user / filedrop · ExampleHub");
        assert_eq!(p.description, DESC);
        assert_eq!(p.site, "ExampleHub");
        assert_eq!(img, "https://code.example.com/assets/card-default.png");
    }

    /// A sign-in page reached by a redirect: only a <title>, no description -> not sufficient, no card.
    #[test]
    fn sign_in_page_without_description_gives_no_card() {
        let h = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/unfurl/sign_in.html")).unwrap();
        assert!(parse_page("https://dev.example.com/example-org/", "https://login.example.net/_signin?realm=dev.example.com", &h).is_none());
    }

    #[test]
    fn fallbacks_and_entities() {
        // twitter:* when og:* is missing, <title> last; twitter:site as the site name; numeric / named references.
        let h = format!(r#"<head><title>Tab &#8211; text</title><meta name="twitter:description" content="{DESC}"><meta name="twitter:site" content="@example">
            <meta name="twitter:image" content="img/card.jpg?v=2"></head><body><meta property="og:image" content="/late.png"></body>"#);
        let (p, img) = parse_page("https://www.example.org/a/b/page", "https://www.example.org/a/b/page", &h).unwrap();
        assert_eq!((p.title.as_str(), p.site.as_str()), ("Tab – text", "@example"));
        assert_eq!(img, "https://www.example.org/a/b/img/card.jpg?v=2", "tags after </head> are not read");
        let h = format!(r#"<meta property="og:title" content="A &amp; B &middot; C &#xB7; D &bogus; E"><meta name="description" content="{DESC}">"#);
        let (p, img) = parse_page("https://www.example.org/", "https://www.example.org/", &h).unwrap();
        assert_eq!((p.title.as_str(), p.site.as_str(), img.as_str()), ("A & B · C · D &bogus; E", "www.example.org", ""));
    }

    #[test]
    fn sufficiency_rule() {
        assert!(sufficient("T", DESC));
        assert!(!sufficient("", DESC));
        assert!(!sufficient("T", ""));
        assert!(!sufficient("T", "one two three four five six seven eight nine"));
        assert!(sufficient("T", "一 二 三 四 五 六 七 八 九 十"));
        assert!(sufficient("T", "これは例のためのとても短い説明文でありますよ")); // 20+ characters with CJK, no spaces
        assert!(!sufficient("T", "これは短い説明です"));
        assert!(!sufficient("T", "abcdefghijklmnopqrstuvwxyz")); // long, but one word and no CJK
    }

    #[test]
    fn urls_are_resolved_against_the_final_page() {
        let b = "https://www.example.org/dir/sub/page.html?q=1#x";
        assert_eq!(resolve(b, "https://cdn.example.net/i.png"), "https://cdn.example.net/i.png");
        assert_eq!(resolve(b, "//cdn.example.net/i.png"), "https://cdn.example.net/i.png");
        assert_eq!(resolve("http://example.org/", "//cdn.example.net/i.png"), "http://cdn.example.net/i.png");
        assert_eq!(resolve(b, "/i.png"), "https://www.example.org/i.png");
        assert_eq!(resolve(b, "i.png"), "https://www.example.org/dir/sub/i.png");
        assert_eq!(resolve(b, "./a/i.png"), "https://www.example.org/dir/sub/a/i.png");
        assert_eq!(resolve(b, "../../i.png?v=1"), "https://www.example.org/i.png?v=1");
        assert_eq!(resolve("https://example.org", "i.png"), "https://example.org/i.png");
    }

    #[test]
    fn only_jpg_png_gif_are_allowed() {
        assert!(allowed_image(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]));
        assert!(allowed_image(b"\x89PNG\r\n\x1a\n...."));
        assert!(allowed_image(b"GIF89a...."));
        assert!(!allowed_image(b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(!allowed_image(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"));
    }
}
