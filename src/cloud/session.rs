//! The session's HTTP calls, token renewal, device registration, own profile and the private setup.
use super::*;

impl Session {
    /// Participant ids in the conversation list are email-style (not the device's user uuid), so "me" is
    /// recognised by either the uuid or the account email.
    pub(super) fn is_me(&self, p: &Person) -> bool {
        p.id == self.user
            || (!self.my_email.is_empty() && (p.id.eq_ignore_ascii_case(&self.my_email) || p.email.eq_ignore_ascii_case(&self.my_email)))
    }
    pub(super) fn me_person(&self) -> Option<Person> {
        (!self.user.is_empty()).then(|| Person { id: if self.my_email.is_empty() { self.user.clone() } else { self.my_email.clone() }, email: self.my_email.clone(), ..Default::default() })
    }
    pub(super) fn say(&self, s: &str) { dbg_conn(s); let _ = self.out.send(Resp::Status(s.into())); }
    pub(super) fn url(&self, svc: &str, path: &str) -> String { format!("{}{path}", self.svc.get(svc).cloned().unwrap_or_default()) }

    /// A request to a Webex service. A 401 renews the bearer once and replays the request once. The key-service token is
    /// renewed with the sign-in (`renew_token`), not here.
    pub(super) fn http(&self, method: &str, url: &str, body: Option<Value>) -> Result<Value, String> {
        match self.http_once(method, url, body.clone(), None) {
            Err(e) if http_status(&e) == Some(401) && !url.contains("/kms/messages") => match force_refresh(&self.token) {
                Some(t) => { dbg_conn("http 401: bearer renewed, replaying once"); self.http_once(method, url, body, Some(&t)) }
                None => Err(e),
            },
            r => r,
        }
    }

    pub(super) fn http_once(&self, method: &str, url: &str, body: Option<Value>, bearer: Option<&str>) -> Result<Value, String> {
        let tok = bearer.unwrap_or(if url.contains("/kms/messages") { &self.kms_token } else { &self.token });
        // TrackingID `CLIENT_<uuid>`; the same native User-Agent as the idbroker calls; `cisco-device-url` on every service
        // request once the device is registered (all services but idbroker/oauth/saml).
        let mut r = self.agent.request(method, url).set("Authorization", &format!("Bearer {tok}"))
            .set("TrackingID", &format!("CLIENT_{}", jose::uuid())).set("User-Agent", crate::cloud::auth::USER_AGENT);
        if !self.device.is_empty() && webex_host(url) && !url.contains("idbroker") { r = r.set("cisco-device-url", &self.device); }
        let resp = match body { Some(b) => r.send_json(b), None => r.call() };
        match resp {
            Ok(r) => Ok(r.into_json::<Value>().unwrap_or(Value::Null)),
            Err(ureq::Error::Status(code, r)) => Err(format!("{method} {url} → {code} {}", r.into_string().unwrap_or_default().chars().take(200).collect::<String>())),
            Err(e) => Err(e.to_string()),
        }
    }

    /// `Req::Token`: the renewed bearer for every later request and socket authorisation; the key-service token is renewed
    /// with it.
    pub(super) fn renew_token(&mut self, access: String, kms: Option<(String, String)>) {
        if let Ok(mut t) = LIVE_TOKEN.write() { *t = access.clone(); }
        if let (Some(src), Ok(mut r)) = (kms.clone(), REFRESH_SRC.write()) { *r = Some(src); }
        let kms_was_main = self.kms_token == self.token;
        self.token = access;
        match kms.map(|(r, base)| crate::cloud::auth::refresh_kms(&r, &base)) {
            Some(Ok(t)) => self.kms_token = t.access,
            Some(Err(e)) => { dbg_conn(&format!("KMS token renewal failed: {}", e.chars().take(80).collect::<String>())); if kms_was_main { self.kms_token = self.token.clone(); } }
            None => if kms_was_main { self.kms_token = self.token.clone(); },
        }
        dbg_conn("sign-in renewed in place");
    }

    pub(super) fn connect(&mut self, device: Option<String>, jobs: Sender<Job>, kms_tx: Sender<String>, gen: u64) -> Result<(), String> {
        self.private_setup(device, jobs, kms_tx, gen)?;
        self.me();
        self.say("Connected");
        self.spaces()?;
        self.load_cover();
        self.meetings().ok();
        self.call_history().ok(); // call history and calendar sync after login
        self.calendar().ok();
        self.call_req(locus::CallReq::Cleanup).ok(); // leave any call this device was left joined to by an earlier run
        Ok(())
    }

    /// Downloads the cover picture from the avatar service (avatar-a.wbx2.com/avatar/api/v1/coverImages/…).
    pub(super) fn load_cover(&self) {
        use std::io::Read;
        // The user's own selection (GET avatar/coverImages/users/<uuid>, e.g. "Sky") wins; colleagues answer
        // `{"avatarUrl":"webex://coverImage?selection=default"}` (no custom cover). Without a selection the global
        // default from the device policy `cover-image-selection` is used.
        let mut sources: Vec<(&str, String)> = vec![];
        if !self.user.is_empty() { sources.push(("own selection", self.url("avatar", &format!("/coverImages/users/{}", self.user)))); }
        if !self.cover.is_empty() && webex_host(&self.cover) { sources.push(("global default", self.cover.clone())); }
        let mut sent_own = false;
        for (what, url) in sources {
            let a = match self.http("GET", &url, None) { Ok(a) => a, Err(e) => { dbg_conn(&format!("cover ({what}): {}", e.chars().take(60).collect::<String>())); continue } };
            let urls = &a["imageUrls"];
            let pick = ["Dark.png", "Light.png", "thumbnail"].iter().map(|k| st(urls, k)).find(|u| !u.is_empty())
                .or_else(|| urls.as_object().and_then(|m| m.values().find_map(|x| x.as_str().map(String::from))));
            let Some(u) = pick.filter(|u| u.starts_with("https://")) else { dbg_conn(&format!("cover ({what}): no image url (selection default)")); continue };
            let mut req = self.agent.get(&u);
            if images::token_host(&u) { req = req.set("Authorization", &format!("Bearer {}", self.token)); }
            let mut buf = vec![];
            let ok = match req.call() {
                Ok(r) => r.into_reader().take(COVER_MAX_BYTES).read_to_end(&mut buf).is_ok() && !buf.is_empty(),
                Err(e) => { dbg_conn(&format!("cover ({what}) image: {}", match e { ureq::Error::Status(c, _) => format!("http {c}"), _ => "transport".into() })); false }
            };
            dbg_conn(&format!("cover ({what}, {:?}): {} bytes, decodes: {}", st(&a, "imageName"), buf.len(), image::load_from_memory(&buf).is_ok()));
            if !ok { continue }
            if what == "own selection" { let _ = self.out.send(Resp::Cover(buf)); sent_own = true; continue }
            if !sent_own { let _ = self.out.send(Resp::Cover(buf.clone())); }
            let _ = self.out.send(Resp::Covers(vec![("@default".into(), buf)]));
            return;
        }
    }
    /// Your own name from the identity service (SCIM Users/me).
    pub(super) fn me(&mut self) {
        let url = self.svc.get("identity").map(|u| format!("{u}/identity/scim/v1/Users/me")).unwrap_or_else(|| IDENTITY_ME.to_string());
        if let Ok(v) = self.http("GET", &url, None) {
            let name = [st(&v, "displayName"), format!("{} {}", st(&v["name"], "givenName"), st(&v["name"], "familyName")).trim().to_string()]
                .into_iter().find(|n| !n.is_empty()).unwrap_or_default();
            let email = st(&v, "userName");
            if let Some(o) = v["meta"]["organizationID"].as_str() { self.svc.insert(header_plus::ORG_KEY.into(), o.into()); } // for the directory search
            self.my_email = email.clone();
            let _ = self.out.send(Resp::Me(Person { id: self.user.clone(), name, email, status: "active".into() }));
        }
    }
    pub(super) fn private_setup(&mut self, device: Option<String>, jobs: Sender<Job>, kms_tx: Sender<String>, gen: u64) -> Result<(), String> {
        self.say("Finding services…");
        if let Ok(cat) = self.http("GET", U2C_CATALOG, None) {
            if let Some(m) = cat["serviceLinks"].as_object() {
                for (k, v) in m { if let Some(u) = v.as_str() { self.svc.insert(k.clone(), u.trim_end_matches('/').to_string()); } }
            }
        }
        self.say("Registering this device…");
        let body = json!({"deviceName": "webox", "deviceType": "DESKTOP", "localizedModel": "webox", "model": "webox",
            "name": format!("WebOx on {}", std::env::consts::OS), "systemName": std::env::consts::OS, "systemVersion": env!("CARGO_PKG_VERSION")});
        // Refresh = PUT on the saved device url; a new registration only when that device is gone (404), so a failed PUT
        // (expired token, 5xx, timeout) never registers one more device for the account.
        let saved = device.filter(|d| !d.is_empty() && webex_host(d));
        let dev = match saved {
            Some(d) => match self.http("PUT", &d, Some(body.clone())) {
                Ok(v) => v,
                Err(e) if device_gone(&e) => { dbg_conn("device registration gone (404): registering anew"); self.http("POST", &self.url("wdm", "/devices"), Some(body))? }
                Err(e) => return Err(e),
            },
            None => self.http("POST", &self.url("wdm", "/devices"), Some(body))?,
        };
        // Feature toggles arrive with the registration.
        dbg_conn(&format!("feature toggles: developer {} entitlement {} user {}", dev["features"]["developer"].as_array().map_or(0, |a| a.len()),
            dev["features"]["entitlement"].as_array().map_or(0, |a| a.len()), dev["features"]["user"].as_array().map_or(0, |a| a.len())));
        features::store(features::Features::from_device(&dev));
        self.device = st(&dev, "url");
        self.user = st(&dev, "userId");
        self.calls.device_type = st(&dev, "deviceType"); // Locus wants the type the registration has
        self.calls.jobs = Some(jobs.clone());
        // The policy "cover-image-selection" comes with this answer.
        self.cover = find_policy(&dev, COVER_POLICY).unwrap_or_default();
        crate::app::files::set_policy(&dev); // file-upload-size-limit, bfs-upload-allow-desktop
        dbg_conn(&format!("cover policy: {}", if self.cover.is_empty() { "not in the device answer" } else { "found" }));
        if self.cover.is_empty() {
            // Which keys carry anything about covers? (key names only)
            fn paths(v: &Value, p: &str, out: &mut Vec<String>) {
                match v {
                    Value::Object(m) => for (k, x) in m { let q = format!("{p}/{k}"); if k.to_lowercase().contains("cover") { out.push(q.clone()); } paths(x, &q, out); },
                    Value::Array(a) => for x in a { paths(x, p, out); },
                    Value::String(s) if s.contains("coverImages") || s == COVER_POLICY => out.push(format!("{p}=<string with cover>")),
                    _ => {}
                }
            }
            let mut found = vec![]; paths(&dev, "", &mut found); found.truncate(8);
            dbg_conn(&format!("cover-related keys in device answer: {found:?}"));
        }
        let _ = self.out.send(Resp::Device(self.device.clone()));
        if let Some(m) = dev["services"].as_object() {
            for (k, v) in m {
                if let (Some(name), Some(u)) = (k.strip_suffix("ServiceUrl"), v.as_str()) { self.svc.insert(name.to_string(), u.trim_end_matches('/').to_string()); }
            }
        }
        let ws = st(&dev, "webSocketUrl");
        if ws.is_empty() { return Err("the device registration has no webSocketUrl".into()); }
        let out = self.out.clone();
        std::thread::spawn(move || mercury(ws, gen, jobs, kms_tx, out)); // bearer read live, ends with the session

        self.say("Opening the key service…");
        let info = self.http("GET", &self.url("encryption", &format!("/kms/{}", self.user)), None)?;
        let jwk = match &info["rsaPublicKey"] { Value::String(s) => serde_json::from_str(s).map_err(|e| e.to_string())?, v => v.clone() };
        self.kms = Some(Kms { cluster: st(&info, "kmsCluster"), kid: st(&jwk, "kid"), rsa: jose::rsa_from_jwk(&jwk)?, eph: None });
        let mut last = String::new();
        for _ in 0..4 { // the socket must be up before the key service can answer (4 attempts)
            match self.ecdhe() { Ok(()) => { last.clear(); break } Err(e) => { last = e; std::thread::sleep(Duration::from_secs(2)) } }
        }
        if last.is_empty() { Ok(()) } else { Err(last) }
    }
}
