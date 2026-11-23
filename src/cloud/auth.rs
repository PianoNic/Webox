//! Native sign-in with the Webex desktop OAuth client, so people sign in without a developer registration.
//! The client credentials are not in the source tree (`secrets.toml`, see `crate::cloud::secrets`).
//! - Authorize: `response_type=code&code_challenge_method=S256&cisService=webex&cisKeepMeSignedInOption=1&oauthProviderSupported=true&token_version=2&isHideAuthzCode=true`
//!   with `redirect_uri=urn:ietf:wg:oauth:2.0:oob`.
//! - Token exchange: POST `/idb/oauth2/v1/access_token?self_contained_token=true&token_version=2`
//!   with `Authorization: Basic base64(client_id:client_secret)`, `User-Agent: sparkwindows/<ver>`,
//!   `TrackingID: CLIENT_<uuid>`, `Content-Type: application/x-www-form-urlencoded; charset=utf-8`,
//!   and a body of `grant_type=authorization_code`, `redirect_uri`, `code`, `code_verifier`,
//!   `device_id` - no cookies and no `client_id` in the body (the credentials are the header).
//!
//! Flow: enter email → home-cluster discovery (`discover_idbroker`) → Webex's sign-in page loads in the
//! embedded webview (`isHideAuthzCode=true`) → the webview captures the one-time code idbroker hands back →
//! `exchange` POSTs it to the same cluster's `/access_token`. (A browser+paste path remains as a fallback
//! when no embedded webview is available.)
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

/// The US/global broker. Accounts on a regional home cluster (EU, etc.) sign in and exchange on that
/// cluster instead — discovered per-email by `discover_idbroker`. This is the default and the fallback
/// when discovery fails. All of authorize/access_token/revoke must hit the SAME cluster: the code is
/// issued by the regional broker, so the token exchange has to POST back to it.
pub const US_BROKER: &str = "https://idbroker.webex.com";
const AUTHORIZE_PATH: &str = "/idb/oauth2/v1/authorize";
const TOKEN_PATH: &str = "/idb/oauth2/v1/access_token";
const REVOKE_PATH: &str = "/idb/oauth2/v1/revoke";
/// Pre-auth home-cluster discovery (no token): the "limited" user-to-cluster catalog, queried with the email's hash
/// to learn which idbroker the account lives on.
const U2C_CATALOG: &str = "https://u2c.wbx2.com/u2c/api/v1/limited/catalog";

/// The Webex desktop OAuth client_id. Not in the source tree: `build.rs` embeds it from `secrets.toml`
/// (see `crate::cloud::secrets`).
pub fn client_id() -> String { crate::cloud::secrets::client_id().unwrap_or_default() }
/// The same client's secret (also from `secrets.toml`). This client authenticates at the token endpoint via HTTP
/// Basic, not via a body `client_id`; without it idbroker answers `400 invalid_request`.
pub fn client_secret() -> String { crate::cloud::secrets::client_secret().unwrap_or_default() }
/// Both credentials are present, so a sign-in can start.
pub fn configured() -> bool { crate::cloud::secrets::client_id().is_some() && crate::cloud::secrets::client_secret().is_some() }
/// The native User-Agent for idbroker calls (sparkwindows/<version>).
pub const USER_AGENT: &str = "sparkwindows/46.11.0.36194";

/// `Basic base64(client_id:client_secret)` for the token endpoint.
pub fn basic_auth() -> String {
    format!("Basic {}", crate::jose::b64_std(format!("{}:{}", client_id(), client_secret()).as_bytes()))
}
/// The desktop client is registered out-of-band: idbroker returns the code on a page instead of
/// redirecting anywhere (the embedded webview reads it; otherwise Webox shows it to paste).
pub const REDIRECT_OOB: &str = "urn:ietf:wg:oauth:2.0:oob";
/// The scope string requested on authorize.
pub const SCOPES: &str = "Identity:Organization Identity:SCIM spark:all webexsquare:get_conversation spark:kms";

#[derive(Clone, Debug, Default)]
pub struct Token { pub access: String, pub refresh: String, pub expires_at: u64, pub issued_at: u64 }

/// Renew after this share of the token's lifetime; `SSO_TTL_REFRESH_PERCENTAGE` 1–99 overrides it (value / 100),
/// anything else is rejected as invalid.
pub const DEFAULT_TTL_REFRESH: f64 = 0.75;
pub fn ttl_refresh_fraction(percentage: Option<u64>) -> f64 {
    match percentage { Some(p) if (1..=99).contains(&p) => p as f64 / 100.0, _ => DEFAULT_TTL_REFRESH }
}

fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

/// Sign-in diagnostics → `%APPDATA%/WebOx/auth-debug.log`, only when `WEBOX_DEBUG_LOG=1` (off by default).
/// `pub(crate)` so the webview capture paths can log what idbroker actually delivers in true mode.
pub(crate) fn dbg_auth(msg: String) {
    if !crate::cloud::debug_log_enabled() { return; }
    if let Some(d) = std::env::var_os("APPDATA") {
        use std::io::Write;
        let p = std::path::Path::new(&d).join("WebOx").join("auth-debug.log");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) { let _ = writeln!(f, "{msg}"); }
    }
}

/// Lowercase hex SHA-256, as idbroker/u2c want the email and domain hashed.
fn sha256_hex(s: &str) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Finds the idbroker base URL for this account's home cluster (e.g. `https://idbroker-eu.webex.com`).
/// No token needed: GET the limited u2c catalog by the lowercase-email SHA-256 and read the `idbroker`
/// service's first `baseUrl`. Any failure (network, HTTP 400 for an unknown/garbage email, parse,
/// empty) falls back to the US broker. A short timeout keeps the sign-in click responsive.
pub fn discover_idbroker(email: &str) -> String {
    let email = email.trim().to_lowercase();
    if email.is_empty() { return US_BROKER.to_string(); }
    let url = format!("{U2C_CATALOG}?emailhash={}", sha256_hex(&email));
    let found = (|| -> Option<String> {
        let v: Value = ureq::get(&url).timeout(std::time::Duration::from_secs(6)).call().ok()?.into_json().ok()?;
        let svc = v["services"].as_array()?.iter().find(|s| s["serviceName"] == "idbroker")?;
        let base = svc["serviceUrls"][0]["baseUrl"].as_str()?.trim_end_matches('/');
        (!base.is_empty()).then(|| base.to_string())
    })();
    found.unwrap_or_else(|| US_BROKER.to_string())
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| match b { b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(), _ => format!("%{b:02X}") }).collect()
}

/// PKCE: `&code_challenge=…&code_challenge_method=S256`. Returns (verifier, challenge).
pub fn pkce() -> (String, String) {
    use rand::RngCore;
    use sha2::Digest;
    let mut b = [0u8; PKCE_VERIFIER_BYTES];
    rand::thread_rng().fill_bytes(&mut b);
    let verifier = crate::jose::b64(&b);
    let challenge = crate::jose::b64(&sha2::Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}
pub const PKCE_VERIFIER_BYTES: usize = 32; // 43-character verifier, the RFC 7636 minimum

/// The authorize URL on `base` (the account's home-cluster idbroker, from `discover_idbroker`), with
/// the native client's parameters. It sends only the **hashes** of the email and its domain
/// (`emailHash`/`domainHash`), never the plaintext address.
///
/// `isHideAuthzCode=true`: in this mode idbroker does not print the code; it hands it to the embedded host
/// (a navigation redirect, or an `idbroker.authorize.response` host-bridge message). The webview captures it
/// from the navigation URL / host-bridge message (see `webview.rs`). A code displayed with
/// `isHideAuthzCode=false` is not redeemable by this client (the token endpoint returns `400 invalid_request`).
pub fn authorize_url(email: &str, state: &str, challenge: &str, base: &str) -> String {
    let base = if base.is_empty() { US_BROKER } else { base.trim_end_matches('/') };
    let mut u = format!(
        "{base}{AUTHORIZE_PATH}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={state}\
         &code_challenge={challenge}&code_challenge_method=S256\
         &cisService=webex&cisKeepMeSignedInOption=1&oauthProviderSupported=true&token_version=2\
         &isHideAuthzCode=true",
        enc(&client_id()), enc(REDIRECT_OOB), enc(SCOPES));
    dbg_auth(format!("AUTHORIZE base={base} challenge={challenge} state={state}"));
    let email = email.trim().to_lowercase();
    if !email.is_empty() {
        u.push_str(&format!("&emailHash={}", sha256_hex(&email)));
        if let Some(domain) = email.split('@').nth(1).filter(|d| !d.is_empty()) {
            u.push_str(&format!("&domainHash={}", sha256_hex(domain)));
        }
    }
    u
}

/// The token endpoint for `base`, with its query params: `?self_contained_token=true&token_version=2`.
/// The in-webview `fetch` exchange POSTs here too, so this is the single source of truth for both paths.
pub fn token_url(base: &str) -> String {
    let base = if base.is_empty() { US_BROKER } else { base.trim_end_matches('/') };
    format!("{base}{TOKEN_PATH}?self_contained_token=true&token_version=2")
}

fn token_call(base: &str, form: &[(&str, &str)]) -> Result<Token, String> {
    // Basic client auth, the native sparkwindows User-Agent, a fresh CLIENT_<uuid> TrackingID, form-urlencoded
    // with charset - and no client_id in the body (the credentials are the Authorization header).
    // Without these headers idbroker answers `400 invalid_request` for every grant type.
    if !configured() { return Err(crate::cloud::secrets::NOT_CONFIGURED.into()); }
    let url = token_url(base);
    // Diagnostics log the parameter NAMES and value lengths only - never the code, verifier or
    // tokens themselves (the log lives in the user's profile directory).
    let keys = form.iter().map(|(k, v)| format!("{k}<{}>", v.len())).collect::<Vec<_>>().join(" ");
    dbg_auth(format!("POST {url}\n  {keys}"));
    let body = form.iter().map(|(k, v)| format!("{k}={}", enc(v))).collect::<Vec<_>>().join("&");
    let req = ureq::post(&url)
        .set("Authorization", &basic_auth())
        .set("User-Agent", USER_AGENT)
        .set("TrackingID", &format!("CLIENT_{}", crate::jose::uuid()))
        .set("Content-Type", "application/x-www-form-urlencoded; charset=utf-8");
    let v: Value = match req.send_string(&body) {
        Ok(r) => { let t = r.into_string().unwrap_or_default(); dbg_auth(format!("  OK: {} bytes", t.len())); /* never the body: it carries the tokens */ serde_json::from_str(&t).map_err(|e| e.to_string())? }
        Err(ureq::Error::Status(c, r)) => { let t = r.into_string().unwrap_or_default(); dbg_auth(format!("  ERR {c}: {t}")); return Err(format!("login service said {c}: {}", t.chars().take(200).collect::<String>())); }
        Err(e) => { dbg_auth(format!("  NET ERR: {e}")); return Err(e.to_string()); }
    };
    let access = v["access_token"].as_str().ok_or("no access token in the answer")?.to_string();
    let issued = now();
    Ok(Token { access, refresh: v["refresh_token"].as_str().unwrap_or("").into(), expires_at: issued + v["expires_in"].as_u64().unwrap_or(3600), issued_at: issued })
}

/// The auth-code exchange: `grant_type=authorization_code`, `redirect_uri`, `code`, `code_verifier`, `device_id`.
/// `base` must be the same cluster the code was issued on.
pub fn exchange(code: &str, verifier: &str, base: &str, device_id: &str) -> Result<Token, String> {
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("redirect_uri", REDIRECT_OOB),
        ("code", code.trim()),
        ("code_verifier", verifier),
    ];
    if !device_id.is_empty() { form.push(("device_id", device_id)); }
    token_call(base, &form)
}

/// Renewal with the saved refresh token. A plain refresh grant (no `scope=` / `upgrade_scope=`) re-issues the
/// code exchange's original scopes.
pub fn refresh(refresh: &str, base: &str, _device_id: &str) -> Result<Token, String> {
    let form = [("grant_type", "refresh_token"), ("refresh_token", refresh)];
    let mut t = token_call(base, &form)?;
    if t.refresh.is_empty() { t.refresh = refresh.to_string(); }
    Ok(t)
}

/// A second, reduced-scope access token just for the key service: a refresh grant with `scope=spark:kms`.
pub fn refresh_kms(refresh: &str, base: &str) -> Result<Token, String> {
    let form = [("grant_type", "refresh_token"), ("refresh_token", refresh), ("scope", "spark:kms")];
    token_call(base, &form)
}

pub fn revoke(token: &str, base: &str) {
    let base = if base.is_empty() { US_BROKER } else { base.trim_end_matches('/') };
    let _ = ureq::post(&format!("{base}{REVOKE_PATH}"))
        .set("Authorization", &basic_auth())
        .set("User-Agent", USER_AGENT)
        .set("TrackingID", &format!("CLIENT_{}", crate::jose::uuid()))
        .set("Content-Type", "application/x-www-form-urlencoded; charset=utf-8")
        .send_string(&format!("token={}", enc(token)));
}

/// True once `fraction` of the lifetime (issued → expires) has passed. A token with no known lifetime never renews by time.
pub fn needs_refresh_at(issued_at: u64, expires_at: u64, fraction: f64, now_s: u64) -> bool {
    if expires_at == 0 || expires_at <= issued_at { return false; }
    let due = issued_at as f64 + (expires_at - issued_at) as f64 * fraction;
    now_s as f64 >= due
}
pub fn needs_refresh(issued_at: u64, expires_at: u64, fraction: f64) -> bool { needs_refresh_at(issued_at, expires_at, fraction, now()) }

/// Exchanges the captured authorization code for a token on a background thread; the window polls the channel.
pub fn exchange_async(code: String, verifier: String, base: String, device_id: String) -> std::sync::mpsc::Receiver<Result<Token, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || { let _ = tx.send(exchange(&code, &verifier, &base, &device_id)); crate::ui::wake(); });
    rx
}

#[cfg(test)]
mod tests {
    #[test]
    fn urls() {
        let (v, c) = super::pkce();
        assert_eq!((v.len(), c.len()), (43, 43));
        let u = super::authorize_url("a@b.c", "s1", &c, super::US_BROKER);
        assert!(u.contains("&code_challenge_method=S256"));
        assert!(u.starts_with(&format!("https://idbroker.webex.com/idb/oauth2/v1/authorize?response_type=code&client_id={}&", super::enc(&super::client_id()))));
        assert!(u.contains("redirect_uri=urn%3Aietf%3Awg%3Aoauth%3A2.0%3Aoob"));
        assert!(u.contains("scope=Identity%3AOrganization%20Identity%3ASCIM%20spark%3Aall") && u.contains("spark%3Akms"));
        assert!(u.contains("cisService=webex") && u.contains("token_version=2") && u.contains("isHideAuthzCode=true"));
        // Hashes, never the plaintext address (only emailHash/domainHash are sent).
        assert!(u.contains("state=s1") && !u.contains("email=a%40b.c") && !u.contains("&email="));
        assert!(u.contains("&emailHash=") && u.contains("&domainHash="));
        // Lowercase hex SHA-256 of the address and of its domain (the format u2c/idbroker accept), domain lower-cased first.
        assert_eq!(super::sha256_hex("alex.muster@example.com"), "91b1b005417211d16741286e6ce90a74d2d0c75cc6ba2cf6ffde36cddbed816e");
        assert_eq!(super::sha256_hex("example.com"), "a379a6f6eeafb9a55e378c118034e2751e682fab9f2d30ab13d2125586ce1947");
        // A regional base threads through to both the host and the domainHash (case-insensitive email).
        let eu = super::authorize_url("User@Example.COM", "s", &c, "https://idbroker-eu.webex.com/");
        assert!(eu.starts_with("https://idbroker-eu.webex.com/idb/oauth2/v1/authorize?"));
        assert!(eu.contains("&domainHash=a379a6f6eeafb9a55e378c118034e2751e682fab9f2d30ab13d2125586ce1947"));
        // 24 h token: renew at 18 h (64800 s).
        assert!(!super::needs_refresh_at(0, 86_400, super::ttl_refresh_fraction(None), 64_799));
        assert!(super::needs_refresh_at(0, 86_400, super::ttl_refresh_fraction(None), 64_800));
        assert_eq!(super::ttl_refresh_fraction(Some(50)), 0.5);
        assert_eq!(super::ttl_refresh_fraction(Some(0)), 0.75);
        assert_eq!(super::token_url("https://idbroker-eu.webex.com/"), "https://idbroker-eu.webex.com/idb/oauth2/v1/access_token?self_contained_token=true&token_version=2");
        let b = super::basic_auth();
        assert!(b.starts_with("Basic "));
        let dec = base64::engine::Engine::decode(&base64::engine::general_purpose::STANDARD, b.strip_prefix("Basic ").unwrap()).unwrap();
        let pair = String::from_utf8(dec).unwrap();
        assert_eq!(pair, format!("{}:{}", super::client_id(), super::client_secret()));
    }

    #[test]
    #[ignore = "network: hits u2c.wbx2.com (run with --ignored)"]
    fn discovery() {
        // Set WEBOX_TEST_EU_EMAIL to an address on an EU-hosted org to check the regional answer;
        // unknown/garbage and empty fall back to the US broker.
        if let Ok(eu) = std::env::var("WEBOX_TEST_EU_EMAIL") {
            assert_eq!(super::discover_idbroker(&eu), "https://idbroker-eu.webex.com");
        }
        assert_eq!(super::discover_idbroker("no-such-user-zzz9@garbage.invalid"), super::US_BROKER);
        assert_eq!(super::discover_idbroker(""), super::US_BROKER);
    }
}
