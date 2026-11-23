//! One TLS trust store and proxy handling for every outgoing HTTPS call (ureq agents) and the Mercury websocket: the bundled
//! Mozilla roots plus the system's (SSL_CERT_FILE / SSL_CERT_DIR, /etc/ssl/certs, the OS store on Windows / macOS), so a
//! company network with a TLS-inspecting proxy works; HTTPS_PROXY / ALL_PROXY (and NO_PROXY for the websocket) are honoured.
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use base64::{engine::general_purpose::STANDARD, Engine};
use rustls::{ClientConfig, RootCertStore};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);

/// The shared client config (ring provider, webpki roots + system roots), built once.
pub fn config() -> Arc<ClientConfig> {
    static C: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    C.get_or_init(|| {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let native = rustls_native_certs::load_native_certs();
        let (ok, bad) = roots.add_parsable_certificates(native.certs);
        super::start::dbg_conn(&format!("tls: {} bundled + {ok} system roots ({bad} unusable, {} load errors)", webpki_roots::TLS_SERVER_ROOTS.len(), native.errors.len()));
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        Arc::new(ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions().expect("tls versions")
            .with_root_certificates(roots).with_no_client_auth())
    }).clone()
}

/// A ureq agent builder with the shared trust store and the proxy from the environment; call sites add their timeouts.
pub fn agent() -> ureq::AgentBuilder { ureq::AgentBuilder::new().tls_config(config()).try_proxy_from_env(true) }

/// `host:port` of an HTTP proxy and its optional `Proxy-Authorization` value.
#[derive(Debug, PartialEq)]
pub struct Proxy { pub host: String, pub port: u16, pub auth: Option<String> }

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let (mut o, mut i) = (Vec::with_capacity(b.len()), 0);
    while i < b.len() {
        match (b[i], b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(v)) => { o.push(v); i += 3 }
            (c, _) => { o.push(c); i += 1 }
        }
    }
    String::from_utf8_lossy(&o).into_owned()
}

/// `[scheme://][user:pass@]host[:port]` of an HTTP proxy (any scheme but socks is spoken as plain HTTP CONNECT).
pub fn parse_proxy(s: &str) -> Option<Proxy> {
    let s = s.trim();
    if s.is_empty() || s.to_ascii_lowercase().starts_with("socks") { return None }
    let u = url::Url::parse(&if s.contains("://") { s.to_string() } else { format!("http://{s}") }).ok()?;
    let host = u.host_str()?.trim_matches(['[', ']']).to_string();
    let auth = (!u.username().is_empty()).then(|| format!("Basic {}", STANDARD.encode(format!("{}:{}", pct_decode(u.username()), pct_decode(u.password().unwrap_or(""))))));
    Some(Proxy { host, port: u.port_or_known_default()?, auth })
}

/// Whether NO_PROXY (comma list: `*`, `host`, `.domain`, `*.domain`, optional `:port`) exempts `host`.
pub fn no_proxy_match(host: &str, list: &str) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    list.split(',').map(|e| e.trim().to_ascii_lowercase()).filter(|e| !e.is_empty()).any(|e| {
        if e == "*" { return true }
        let e = if e.starts_with('[') { e.trim_matches(['[', ']']).to_string() } else { e.rsplit_once(':').filter(|(h, _)| !h.contains(':')).map_or(e.clone(), |(h, _)| h.to_string()) };
        let e = e.trim_start_matches("*.").trim_start_matches('.');
        host == e || host.strip_suffix(e).is_some_and(|p| p.ends_with('.'))
    })
}

fn env(k: &str) -> Option<String> { std::env::var(k).ok().filter(|v| !v.trim().is_empty()) }

/// The proxy for an HTTPS connection to `host` from HTTPS_PROXY / ALL_PROXY, unless NO_PROXY exempts it.
fn proxy_for(host: &str) -> Option<Proxy> {
    if env("NO_PROXY").or_else(|| env("no_proxy")).is_some_and(|l| no_proxy_match(host, &l)) { return None }
    ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"].iter().find_map(|k| env(k)).and_then(|v| parse_proxy(&v))
}

fn io_err(m: String) -> tungstenite::Error { tungstenite::Error::Io(std::io::Error::new(std::io::ErrorKind::Other, m)) }

fn tcp(host: &str, port: u16) -> std::io::Result<TcpStream> {
    let mut last = None;
    for a in (host, port).to_socket_addrs()? {
        match TcpStream::connect_timeout(&a, HANDSHAKE_TIMEOUT) { Ok(s) => return Ok(s), Err(e) => last = Some(e) }
    }
    Err(last.unwrap_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("no address for {host}"))))
}

/// `CONNECT host:port` through the proxy; leaves `s` positioned at the start of the tunnel.
fn tunnel(s: &mut TcpStream, p: &Proxy, host: &str, port: u16) -> std::io::Result<()> {
    let auth = if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") };
    let mut req = format!("CONNECT {auth} HTTP/1.1\r\nHost: {auth}\r\n");
    if let Some(a) = &p.auth { req += &format!("Proxy-Authorization: {a}\r\n"); }
    s.write_all((req + "\r\n").as_bytes())?;
    let mut head = Vec::new();
    let mut b = [0u8; 1]; // byte by byte: never read into the TLS bytes behind the header
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 16 * 1024 || s.read(&mut b)? == 0 { return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "proxy closed the CONNECT answer")) }
        head.push(b[0]);
    }
    let line = String::from_utf8_lossy(head.split(|&c| c == b'\r').next().unwrap_or_default()).into_owned();
    if line.split_whitespace().nth(1) != Some("200") { return Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, format!("proxy refused CONNECT: {line}"))) }
    Ok(())
}

/// Connects the websocket `url` (ws / wss) with the shared trust store, through the environment's proxy when there is one.
/// Blocking; the stream has a handshake timeout, the caller sets its own read timeout afterwards.
pub fn connect_ws(url: &str) -> tungstenite::Result<(tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>, tungstenite::handshake::client::Response)> {
    let u = url::Url::parse(url).map_err(|e| io_err(format!("bad websocket url: {e}")))?;
    let host = u.host_str().ok_or_else(|| io_err("websocket url without host".into()))?.trim_matches(['[', ']']).to_string();
    let tls = u.scheme() == "wss";
    let port = u.port_or_known_default().unwrap_or(if tls { 443 } else { 80 });
    let proxy = if tls { proxy_for(&host) } else { None };
    let s = match &proxy {
        Some(p) => { let mut s = tcp(&p.host, p.port)?; s.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?; s.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?; tunnel(&mut s, p, &host, port)?; s }
        None => tcp(&host, port)?,
    };
    s.set_nodelay(true)?;
    s.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    s.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;
    tungstenite::client_tls_with_config(url, s, None, Some(tungstenite::Connector::Rustls(config()))).map_err(|e| match e {
        tungstenite::HandshakeError::Failure(f) => f,
        tungstenite::HandshakeError::Interrupted(_) => io_err("websocket handshake interrupted".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_proxy_lists() {
        assert!(no_proxy_match("mercury.wbx2.com", "localhost, .wbx2.com"));
        assert!(no_proxy_match("wbx2.com", "*.wbx2.com"));
        assert!(no_proxy_match("a.b.example.org", "example.org:443"));
        assert!(no_proxy_match("anything", "*"));
        assert!(no_proxy_match("::1", "[::1]"));
        assert!(!no_proxy_match("notwbx2.com", "wbx2.com"));
        assert!(!no_proxy_match("example.org", ""));
    }
    #[test]
    fn proxy_urls() {
        assert_eq!(parse_proxy("http://proxy.corp:3128"), Some(Proxy { host: "proxy.corp".into(), port: 3128, auth: None }));
        assert_eq!(parse_proxy("proxy.corp:8080").map(|p| (p.host, p.port)), Some(("proxy.corp".into(), 8080)));
        assert_eq!(parse_proxy("http://proxy.corp").map(|p| p.port), Some(80));
        let p = parse_proxy("http://us%40er:p%3Aw@10.0.0.1:3128/").unwrap();
        assert_eq!(p.auth.as_deref(), Some(format!("Basic {}", STANDARD.encode("us@er:p:w")).as_str()));
        assert_eq!(parse_proxy("socks5://x:1"), None);
        assert_eq!(parse_proxy(" "), None);
    }
}
