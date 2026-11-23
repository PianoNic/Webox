//! Sign-in and tokens: renewal, the idbroker base, the device id, the webview and browser flows, sign out.
use super::*;
use crate::cloud::auth;

impl App {
    /// Renews once 75 % of the token's lifetime has passed (SSO_TTL_REFRESH_PERCENTAGE 1–99 overrides).
    pub(crate) fn renewal_due(&self) -> bool {
        let pct = self.settings["SSO_TTL_REFRESH_PERCENTAGE"].as_u64();
        auth::needs_refresh(self.settings["issued_at"].as_u64().unwrap_or(0), self.settings["expires_at"].as_u64().unwrap_or(0), auth::ttl_refresh_fraction(pct))
    }

    /// The account's home-cluster idbroker base, discovered at sign-in and saved; US default if unset.
    /// authorize, token exchange, refresh and revoke must all use this same cluster.
    pub(crate) fn idbroker_base(&self) -> String {
        self.settings["idbroker_base"].as_str().filter(|s| !s.is_empty()).unwrap_or(auth::US_BROKER).to_string()
    }

    /// Stable per-install device id sent on the token exchange/refresh (`&device_id=`). Generated once (64 hex) and
    /// persisted so a later refresh uses the same id.
    pub(crate) fn oauth_device_id(&mut self) -> String {
        if let Some(d) = self.settings["oauth_device_id"].as_str().filter(|s| !s.is_empty()) { return d.to_string(); }
        let id = format!("{:032x}{:032x}", rand::random::<u128>(), rand::random::<u128>());
        self.settings["oauth_device_id"] = json!(id);
        system::save_settings(&self.cfg, &self.settings);
        id
    }

    /// Renews the sign-in with the saved refresh token on a background thread. False when it can't.
    pub(crate) fn try_refresh(&mut self) -> bool {
        let refresh = self.settings["refresh_token"].as_str().unwrap_or("").to_string();
        if refresh.is_empty() || self.refreshed { return false; }
        self.refreshed = true;
        let base = self.idbroker_base();
        let device_id = self.oauth_device_id();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || { let _ = tx.send(auth::refresh(&refresh, &base, &device_id)); crate::ui::wake(); });
        self.pending = Some(rx);
        true
    }

    pub(crate) fn save_token(&mut self, t: &auth::Token) {
        self.settings["token"] = json!(t.access);
        self.settings["refresh_token"] = json!(t.refresh);
        self.settings["expires_at"] = json!(t.expires_at);
        self.settings["issued_at"] = json!(t.issued_at);
        self.settings["sample"] = json!(false);
        system::save_settings(&self.cfg, &self.settings);
    }

    pub(crate) fn poll_login(&mut self) {
        let Some(rx) = &self.pending else { return };
        let Ok(res) = rx.try_recv() else { return };
        self.pending = None;
        match res {
            Ok(t) => {
                self.save_token(&t); self.need_login = false; self.login_error.clear(); self.refreshed = false;
                // A renewal while signed in: the running session takes the new bearer in place; only a fresh sign-in starts a new session.
                if let Some(c) = &self.client {
                    c.req(Req::Token { access: t.access.clone(), kms: (!t.refresh.is_empty()).then(|| (t.refresh.clone(), self.idbroker_base())) });
                } else { self.sign_in(t.access); }
            }
            Err(e) => { self.login_error = e; self.need_login = true; self.client = None; }
        }
    }

    /// Opens the embedded webview on `url` (the caller already set the matching PKCE verifier).
    /// Returns false — caller falls back to the browser + paste flow — when the platform has no
    /// webview or it can't be created (e.g. the Edge WebView2 runtime isn't installed).
    pub(crate) fn start_web_signin(&mut self, url: &str) -> bool {
        let udd = self.cfg.join("EBWebView"); // our own WebView2/WebKit profile
        match platform::webview::WebView::start(self.hwnd, url, &udd) {
            Ok(wv) => { self.webview = Some(wv); self.login_error.clear(); self.login_page = LoginPage::Web; true }
            Err(e) => { self.plugin_log.push(format!("embedded sign-in unavailable: {e}")); false }
        }
    }

    /// Starts the native sign-in for `email` (the login screen's Next, and `--signin`): remember the
    /// address, discover its home cluster, mint PKCE, and open the embedded webview on the authorize
    /// URL — falling back to the browser + paste flow when there's no webview.
    pub(crate) fn kick_signin(&mut self, email: &str) {
        let email = email.trim().to_string();
        self.settings["email"] = json!(email);
        self.email_input = email.clone();
        self.login_error.clear();
        if !auth::configured() { self.login_error = crate::cloud::secrets::NOT_CONFIGURED.into(); return; }
        // Home-cluster discovery first: non-US orgs (e.g. an EU org → idbroker-eu) must sign in on their own cluster, or
        // idbroker rejects the right password. Discover once, persist, and use the same base for authorize and the token exchange.
        let base = auth::discover_idbroker(&email);
        self.settings["idbroker_base"] = json!(base);
        system::save_settings(&self.cfg, &self.settings);
        // Sign in inside the window (embedded webview). The same PKCE verifier/URL is reused for the browser + paste fallback
        // if there's no webview.
        let (v, ch) = auth::pkce();
        self.pkce_verifier = v;
        let url = auth::authorize_url(&email, &jose::uuid(), &ch, &base);
        if !self.start_web_signin(&url) {
            system::open(&url);
            self.login_page = LoginPage::Code;
        }
    }

    /// Polls the embedded webview. Once it hands back the code, run the out-of-process PKCE exchange
    /// (which `poll_login` then finishes). `Closed` (the user shut the Linux sign-in window) goes back to the email step.
    pub(crate) fn poll_webview(&mut self) {
        let Some(cap) = self.webview.as_ref().and_then(|w| w.poll()) else { return };
        self.webview = None;
        self.login_error.clear();
        match cap {
            #[cfg(all(unix, not(target_os = "macos")))]
            platform::webview::Captured::Closed => self.login_page = LoginPage::Email,
            platform::webview::Captured::Code(code) => {
                self.login_page = LoginPage::Code;
                let (base, dev) = (self.idbroker_base(), self.oauth_device_id());
                self.pending = Some(auth::exchange_async(code, self.pkce_verifier.clone(), base, dev));
            }
        }
    }

    pub(crate) fn sign_in(&mut self, token: String) {
        self.settings["token"] = json!(token);
        system::save_settings(&self.cfg, &self.settings);
        self.client = Some(cloud::start(token, self.settings["device"].as_str().map(String::from),
            self.settings["refresh_token"].as_str().filter(|r| !r.is_empty()).map(|r| (r.to_string(), self.idbroker_base()))));
        self.status = "Signing in…".into();
        self.spaces.clear(); self.messages.clear(); self.selected = None; self.teams.clear(); self.people.clear(); self.meetings.clear();
    }

    pub(crate) fn sign_out(&mut self) {
        self.client = None;
        let base = self.idbroker_base();
        if let Some(tok) = self.settings["token"].as_str().map(String::from) { std::thread::spawn(move || auth::revoke(&tok, &base)); }
        for k in ["token", "refresh_token"] { self.settings[k] = json!(""); }
        self.settings["sample"] = json!(false);
        system::save_settings(&self.cfg, &self.settings);
        *self = App::new(system::Args { screenshot: self.shot.take(), ..Default::default() });
    }
}
