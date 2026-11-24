//! Embedded, in-window sign-in — the webview part of the native OAuth flow in `auth.rs`.
//!
//! Its only job: drive the Webex sign-in page and scrape back the one-time OOB **authorization
//! code**. `webox` keeps the PKCE verifier and does the token exchange (`auth::exchange`). No
//! password or token ever passes through here; the webview just hands over the code idbroker shows.
//!
//! Per-platform, using each OS's *own* native webview library directly (no tauri / wry / tao):
//! - **Windows**: Microsoft **Edge WebView2** via `webview2-com` + the `windows` crate, embedded as
//!   a child of the app's existing Win32 window (same-window sign-in).
//! - **Linux/Nix**: **WebKitGTK** (`webkit2gtk`, webkit2gtk-4.1) in a chromeless window we own.
//!   eframe runs on winit, not GTK, so a GTK child *inside* the winit window isn't possible; a
//!   dedicated top-level window is the fallback for this platform.
//! - **macOS**: **WKWebView** (`objc2-web-kit`) in a window of its own, like WebKitGTK.
//! - **Other**: no embed → `start` returns `Err`, and the caller uses the browser+paste path.
//!
//! idbroker authorizes with `isHideAuthzCode=true`. In that mode idbroker does not render the code; it
//! hands it to the embedded host, so capture is deliberately broad, catching the code from any of:
//!   * a **navigation** to a URL carrying `code=…` (WebView2 `NavigationStarting`/`FrameNavigationStarting`
//!     — fires even for the OOB/callback redirect),
//!   * a **host-bridge / postMessage** delivery (`window.chrome.webview.postMessage`, `window.postMessage`,
//!     or a `message` event, including `idbroker.authorize.response`) — the injected SCRAPER wraps those
//!     and re-posts, and the Rust side also parses a raw JSON `code` / `code=` out of whatever arrives, and
//!   * the **DOM** (kept as a last-resort fallback).
//! `webox` keeps the PKCE verifier and does the token exchange (`auth::exchange`); no password or token
//! passes through the webview.

/// Injected into every document before it loads. It logs how idbroker delivers the code (every host-bridge/postMessage
/// call, every message event, and page-state probes; only with the debug log on), and captures the code from the
/// idbroker shapes (`authorizationCode`/`authCode`/`code` keys, or `code=` inside a `RedirectUri`). Every hook and probe
/// is wrapped in try/catch so it can never throw into — and blank — idbroker's page.
/// Diagnostics reach Rust as `webox-log:<tag>:<payload>` messages (logged there); the code reaches Rust
/// as `webox-code:<code>`. A clean bridge handle (`__webox_post`) keeps our own sends from recursing
/// through the wrapper or being mistaken for idbroker's delivery.
const SCRAPER: &str = r#"(function(){try{
  var sent=false;
  function bridge(){ try{ return window.chrome&&window.chrome.webview; }catch(e){ return null; } }
  function send(s){
    try{ var p=window.__webox_post; if(p){ p(s); return; } var b=bridge(); if(b&&b.postMessage){ b.postMessage(s); return; } }catch(e){}
    try{ window.webkit.messageHandlers.webox.postMessage(s); return; }catch(e){}
    try{ if(/^webox-code:/.test(s)) location.hash=s; }catch(e){}
  }
  function raw(x){ try{ return (typeof x==='string')?x:JSON.stringify(x); }catch(e){ try{ return String(x); }catch(e2){ return '[unstringifiable]'; } } }
  function log(tag,x){ try{ var r=raw(x); send('webox-log:'+tag+':'+(r.length>8000?r.slice(0,8000)+'…<'+r.length+'>':r)); }catch(e){} }
  // The code is handed straight to the native side (auth::exchange): the live capture of the real
  // app's own token call shows the redeem is a cookie-less native POST with Basic client auth -
  // there is no session binding, so the webview need only capture the code, exactly like the
  // original's webview hands it to SystemService.
  function post(c){ if(sent||!c) return; sent=true; try{ log('CODE',c.length+'chars'); send('webox-code:'+c); }catch(e){} }
  // Precise extraction from idbroker.authorize.response shapes (keys REd out of spark-windows-desktop-ui.dll).
  function pick(o){ try{
    if(!o||typeof o!=='object') return null;
    var keys=['authorizationCode','authCode','authorization_code','code','Code'];
    for(var i=0;i<keys.length;i++){ var v=o[keys[i]]; if(typeof v==='string'&&v.length>=12) return v; }
    for(var rk in o){ try{ if(/redirect|uri|url/i.test(rk)){ var m=/[?&#]code=([^&#"'\s]{12,})/.exec(String(o[rk])); if(m) return decodeURIComponent(m[1]); } }catch(e){} }
    return null;
  }catch(e){ return null; } }
  function scan(x){ try{
    if(x==null) return null;
    if(typeof x==='object') return pick(x);
    if(typeof x==='string'){
      var s=x.trim();
      if(s.charAt(0)==='{'||s.charAt(0)==='['){ try{ var c=pick(JSON.parse(s)); if(c) return c; }catch(e){} }
      // `code=<token>` after any delimiter INCLUDING whitespace/tag/quote (the hidden-code page is
      // `<p hidden=""> code=<token></p>`), stopping before markup/quote/space so we never keep the prefix.
      var m=/(?:[?&#\s>"'(]|^)code=([A-Za-z0-9._~+/-]{12,})/.exec(x); if(m){ try{ return decodeURIComponent(m[1]); }catch(e){ return m[1]; } }
      var jm=/["'](?:authorizationCode|authCode|code)["']\s*[:=]\s*["']([^"']{12,})["']/.exec(x); if(jm) return jm[1];
      // idbroker auth-code shape: <base64url>_<ORG>_<uuid> (observed: …_PE93_ea1e0902-…). Charset excludes
      // '=' so a leading `code=` can never be swallowed into the token.
      var im=/([A-Za-z0-9._~+/-]{20,}_[A-Za-z0-9]{2,10}_[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})/.exec(x); if(im) return im[1];
    }
  }catch(e){} return null; }
  function handle(tag,x){ try{ log(tag,x); var c=scan(x); if(c){ log('CODE-via-'+tag,c.length+'chars'); post(c); } }catch(e){ log('scan-threw',tag+' '+e); } }
  // Hook the WebView2 host bridge idbroker posts the code to; keep a clean handle for our own sends.
  try{ var b=bridge(); if(b&&b.postMessage&&!window.__webox_post){ var ob=b.postMessage.bind(b); window.__webox_post=ob; try{ b.postMessage=function(m){ try{ handle('bridge.postMessage',m); }catch(e){} return ob(m); }; }catch(e){ log('bridge-unwritable',''+e); } } }catch(e){ log('hook-bridge',''+e); }
  try{ var ow=window.postMessage.bind(window); window.postMessage=function(m,t,x){ try{ handle('window.postMessage',m); }catch(e){} return ow(m,t,x); }; }catch(e){ log('hook-wpm',''+e); }
  try{ window.addEventListener('message',function(ev){ try{ handle('message-event',ev&&ev.data); }catch(e){} },true); }catch(e){ log('hook-msgev',''+e); }
  // Page-state probes: blank-because-empty vs blank-because-crashed (bodyLen, htmlLen, url, title).
  function probe(n){ try{ var bl=(document.body&&document.body.innerText||'').length, hl=(document.documentElement&&document.documentElement.innerHTML||'').length; log('page','{"at":"'+n+'","url":"'+((location.origin||'')+(location.pathname||'')).slice(0,300)+'","title":"'+(document.title||'').slice(0,160)+'","bodyLen":'+bl+',"htmlLen":'+hl+'}');
    // The isHideAuthzCode=true response ("Authorization Code Page") is a tiny page with the code in the
    // markup (hidden input / inline script), not the visible body. Dump the whole small page once so we
    // can see exactly where the code lives, and scan the full markup for it.
    try{ var oh=document.documentElement.outerHTML||''; if(oh.length<4000){ log('html@'+n, oh); var c=scan(oh); if(c){ log('CODE-via-html',c.length+'chars'); post(c); } } }catch(e){}
  }catch(e){} }
  try{ document.addEventListener('DOMContentLoaded',function(){ probe('DOMContentLoaded'); }); }catch(e){}
  try{ window.addEventListener('load',function(){ probe('load'); setTimeout(function(){probe('load+500');},500); setTimeout(function(){probe('load+2000');},2000); setTimeout(function(){probe('load+5000');},5000); }); }catch(e){}
  // URL/DOM fallback, repeated briefly.
  function tick(){ if(sent) return; try{ var c=scan(location.href); if(c){ post(c); return; } var e=document.querySelector('[name="code"],[name="authorizationCode"],#code,#authorizationCode'); if(e){ var cc=scan(e.value||e.textContent||''); if(cc) post(cc); } }catch(e){} }
  try{ document.addEventListener('DOMContentLoaded',tick); }catch(e){}
  var n=0,iv=setInterval(function(){ try{ tick(); }catch(e){} if(sent||++n>60) clearInterval(iv); },700);
}catch(e){ try{ window.chrome.webview.postMessage('webox-log:scraper-fatal:'+e); }catch(e2){} }})();"#;

/// Pulls the code out of a posted message (`webox-code:<code>`) or a hash-carrying URL
/// (`…#webox-code:<percent-encoded>`). Returns `None` when the marker isn't present.
pub fn extract(s: &str) -> Option<String> {
    const MARKER: &str = "webox-code:";
    let start = s.find(MARKER)? + MARKER.len();
    let rest = &s[start..];
    let rest = rest.split(['&', ' ', '#']).next().unwrap_or(rest);
    Some(percent_decode(rest.trim()))
}

/// Broad capture for `isHideAuthzCode=true`: our own `webox-code:` marker, a `code=<token>` URL/body param, or a
/// JSON `"code":"<token>"` field. Used on raw navigation URLs and host-bridge/postMessage payloads. Requires a ≥20-char
/// OOB code charset so short params like `country_code=CH` or `error_code=…` don't false-match.
pub fn capture(s: &str) -> Option<String> {
    extract(s).or_else(|| boundary_code(s)).or_else(|| json_code(s))
}

const CODE_CHARS: fn(char) -> bool = |c| matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | '.' | '_' | '~' | '-');

/// `code=<token>` where `code` is at a word boundary (not `country_code=`, `postal_code=`, …).
fn boundary_code(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut from = 0;
    while let Some(rel) = s[from..].find("code=") {
        let pos = from + rel;
        let prev_ident = pos > 0 && matches!(bytes[pos - 1], b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_');
        if !prev_ident {
            let tail: String = s[pos + 5..].chars().take_while(|&c| CODE_CHARS(c) || c == '%').collect();
            let tok = percent_decode(&tail);
            if tok.len() >= 20 { return Some(tok); }
        }
        from = pos + 5;
    }
    None
}

/// A code value under one of idbroker's JSON/host-bridge keys: `authorizationCode`, `authCode`, `code`.
fn json_code(s: &str) -> Option<String> {
    for key in ["authorizationCode", "authCode", "authorization_code", "\"code\"", "'code'"] {
        if let Some(idx) = s.find(key) {
            let rest = s[idx + key.len()..].trim_start_matches(['"', '\'', ':', '=', ' ']);
            let tok: String = rest.chars().take_while(|&c| CODE_CHARS(c)).collect();
            if tok.len() >= 20 { return Some(tok); }
        }
    }
    None
}

/// Minimal `%XX` decoding (enough for a `location.hash` round-trip of the code charset).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) { out.push(v); i += 3; continue; }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests;

/// What the webview hands back: the authorization **code** for the native PKCE exchange, or that the user closed the sign-in
/// window (Linux and macOS, a window of its own). A ready-made token is never accepted from a page: any page could post one, while a
/// foreign code fails the exchange (it lacks our PKCE verifier).
pub enum Captured { Code(String), #[cfg(unix)] Closed }

/// Classify a message the SCRAPER posted back: the code for the native out-of-process exchange.
fn classify(s: &str) -> Option<Captured> {
    if let Some(i) = s.find("webox-code:") {
        let code = s[i + 11..].split(['"', '&', ' ', '#']).next().unwrap_or("").trim().to_string();
        if !code.is_empty() { return Some(Captured::Code(code)); }
    }
    None
}

/// A URL for the debug log: without query and fragment (they carry `code=…`).
fn log_url(u: &str) -> &str { u.split(['?', '#']).next().unwrap_or("") }

/// A page message for the debug log: every auth code in it (our marker, `code=…`, a JSON code key, any long token-like
/// run) is replaced by its length.
fn redact(s: &str) -> String {
    let mut out = s.to_string();
    for _ in 0..4 {
        let Some(c) = capture(&out).filter(|c| c.len() >= 12 && c.chars().all(CODE_CHARS) && out.contains(c.as_str())) else { break };
        out = out.replace(c.as_str(), &format!("<code {}>", c.len()));
    }
    // Percent-encoded or unlabelled codes: no run of 32+ token characters survives.
    let (mut res, mut run) = (String::with_capacity(out.len()), String::new());
    for ch in out.chars().chain(std::iter::once('\0')) {
        if CODE_CHARS(ch) || ch == '%' { run.push(ch); continue; }
        if run.chars().count() >= 32 { res.push_str(&format!("<{} chars>", run.chars().count())); } else { res.push_str(&run); }
        run.clear();
        if ch != '\0' { res.push(ch); }
    }
    res
}

// Windows.
#[cfg(windows)]
mod platform_win;
#[cfg(windows)]
use platform_win as platform;

// Linux / BSD.
#[cfg(all(unix, not(target_os = "macos")))]
mod platform_gtk;
#[cfg(all(unix, not(target_os = "macos")))]
use platform_gtk as platform;

// macOS.
#[cfg(target_os = "macos")]
mod platform_mac;
#[cfg(target_os = "macos")]
use platform_mac as platform;

// Other platforms.
#[cfg(not(any(windows, unix)))]
mod platform {
    //! Anything else: no embedded webview; the caller falls back to browser + paste.
    use std::sync::mpsc::Receiver;
    pub struct WebView(#[allow(dead_code)] Receiver<super::Captured>);
    pub const EMBEDDED: bool = false;
    impl WebView {
        pub fn start(_parent: Option<isize>, _url: &str, _udd: &std::path::Path) -> Result<Self, String> {
            Err("embedded sign-in isn't available on this platform".into())
        }
        pub fn poll(&self) -> Option<super::Captured> { None }
        pub fn set_bounds(&self, _x: i32, _y: i32, _w: i32, _h: i32) {}
    }
}

pub use platform::{WebView, EMBEDDED};
