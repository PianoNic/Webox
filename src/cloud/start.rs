//! Starting a session: the job loop, the connection trace, service defaults, policies and forced token renewal.
use super::*;

pub fn start(token: String, device: Option<String>, kms_src: Option<(String, String)>) -> Client {
    let (tx, jobs) = channel::<Job>();
    let (out, from_cloud) = channel::<Resp>();
    // Every answer / live event wakes the UI as it arrives. Ends when the session's senders are gone or the UI dropped the client.
    let (to_ui, rx) = channel::<Resp>();
    std::thread::spawn(move || forward(from_cloud, to_ui, crate::ui::wake));
    let job_tx = tx.clone();
    // A new session: earlier sockets / job loops end, the socket takes this bearer.
    let gen = SESSION_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    if let Ok(mut t) = LIVE_TOKEN.write() { *t = token.clone(); }
    if let Ok(mut r) = REFRESH_SRC.write() { *r = kms_src.clone(); }
    std::thread::spawn(move || {
        let (kms_tx, kms_rx) = channel::<String>();
        let agent = super::tls::agent().timeout(Duration::from_secs(30)).build();
        // The key service gets its own reduced-scope token; fall back to the main one.
        let kms_token = kms_src.and_then(|(r, base)| match crate::cloud::auth::refresh_kms(&r, &base) {
            Ok(t) => { dbg_conn("got KMS-scoped token"); Some(t.access) }
            Err(e) => { dbg_conn(&format!("KMS token refresh failed: {e}")); None }
        }).unwrap_or_else(|| token.clone());
        let kms_loop = kms_tx.clone();
        let mut s = Session { hist: Default::default(), space_cover_urls: Default::default(), space_covers_done: Default::default(), full_desc: Default::default(), read_marks: Default::default(), covers_done: Default::default(), oracle_done: false, img_jobs: vec![], images_done: Default::default(), uuids: HashMap::new(), avatars_done: Default::default(), my_email: String::new(), kms_loop, kms_token, agent, token, svc: defaults(), device: String::new(), user: String::new(),
            kms: None, keys: HashMap::new(), kms_rx, spaces: HashMap::new(), out: out.clone(), cover: String::new(), calls: Default::default(), keep: Default::default(), live: Default::default(), extra: Default::default(), typing: Default::default(), subs: Default::default() };
        if let Err(e) = s.connect(device, job_tx, kms_tx, gen) { dbg_conn(&format!("CONNECT FAILED: {e}")); let _ = out.send(Resp::AuthFailed(e)); return; }
        dbg_conn("connected, entering job loop");
        // A timed loop: periodic work (presence renewal) and the end of a replaced session.
        let renew_ms = PRESENCE_RENEW_SECS as i64 * 1000;
        let mut renew_presence = keepalive_now() + renew_ms;
        // Running / upcoming meetings: at start and every ~36 min (cloud/polls.rs).
        let loci_every = Duration::from_secs(polls::loci_period_s());
        let mut loci_at = Instant::now();
        loop {
            if !session_current(gen) { dbg_conn("session replaced, job loop ends"); break }
            // A 401 renewed the bearer inside `http()`: later requests (and threads cloning `s.token`) use the new one.
            let live = live_token();
            if !live.is_empty() && live != s.token { if s.kms_token == s.token { s.kms_token = live.clone(); } s.token = live; }
            // My writes whose live event didn't arrive in time: their space's newest page is loaded (the fallback, see after_write).
            if let Err(e) = s.echo_overdue() { dbg_conn(&format!("fallback reload failed: {}", e.chars().take(80).collect::<String>())); }
            s.keepalive_tick(); // my own Active heartbeat (cloud/keepalive.rs)
            s.list_reload_due(); // a team / archive change elsewhere: one debounced list reload (cloud/live_list/)
            if Instant::now() >= loci_at {
                loci_at = Instant::now() + loci_every;
                if let Err(e) = s.meetings() { dbg_conn(&format!("meetings poll failed: {}", http_status(&e).map_or("transport".into(), |c| c.to_string()))); }
            }
            // Presence renewal on its own clock: checked on every turn, not only when the queue was idle for a second (a busy
            // session starved it for hours). Contacts whose subscription lapsed or was dropped are subscribed with `refresh`,
            // which makes the server push their current state again (cloud/people/subs.rs).
            if presence_renew_due(renew_presence, keepalive_now()) {
                renew_presence = keepalive_now() + renew_ms;
                let mut want = s.presence_people();
                want.extend(s.extra_people()); // people followed outside the spaces (cloud/presence_sub.rs)
                s.subscribe_presence(&want);
            }
            let job = match jobs.recv_timeout(Duration::from_secs(1)) {
                Ok(j) => j,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    s.presence_recheck(); // a call / meeting / DND whose end passed without an update
                    continue
                }
                Err(_) => break,
            };
            let user = super::report::user_started(&job);
            let r = match job {
                Job::Ui(Req::Token { access, kms }) => { s.renew_token(access, kms); Ok(()) }
                // Back after a reconnect (sleep, network): statuses changed meanwhile without an event. Like the desktop client
                // (mercury lost / restored, resumed from sleep) nothing counts as subscribed any more, and everyone is
                // subscribed again right away, with `refresh`.
                Job::Catchup(since) => { s.subs.forget_all(); renew_presence = keepalive_now(); s.catchup(since) }
                Job::Ui(Req::Spaces) => s.spaces(),
                Job::Ui(Req::Messages(id)) => { s.register_typing(&id); let r = s.messages(&id); s.publish_typing(&id); if r.is_err() { let _ = s.out.send(Resp::MessagesFailed(id.clone())); } r } // shown: receive its typing (cloud/typing.rs)
                Job::Ui(Req::Typing(id)) => s.send_typing(&id),
                Job::Typing(d) => { s.typing_event(&d); Ok(()) }
                Job::TypingTtl(id, ttl) => { s.typing_ttl(&id, ttl); Ok(()) }
                Job::Ui(Req::Send(id, text)) => s.send(&id, &text).and_then(|_| s.after_write(&id)),
                Job::Ui(Req::Meetings) => s.meetings(),
                Job::Ui(Req::Delete(id, m)) => s.delete(&id, &m).and_then(|_| s.after_write(&id)),
                Job::Ui(Req::Ack(id, m)) => s.ack(&id, &m),
                Job::Ui(Req::React(id, m, n)) => s.react(&id, &m, &n).and_then(|_| s.after_write(&id)),
                Job::Ui(Req::FullImage(k)) => s.full_image(&k),
                Job::Ui(Req::MessagesBefore(id)) => s.older_messages(&id),
                Job::Ui(Req::Unreact(id, r)) => s.unreact(&id, &r).and_then(|_| s.after_write(&id)),
                Job::Ui(Req::ReactorNames(id, msg)) => s.reactor_names(&id, &msg),
                Job::Ui(Req::Favorite(id, on)) => s.favorite(&id, on).and_then(|_| s.spaces()),
                Job::Ui(Req::Hide(id)) => s.hide(&id).and_then(|_| s.spaces()),
                Job::Ui(Req::Leave(id)) => s.leave(&id).and_then(|_| s.spaces()),
                Job::Ui(Req::Rename(id, t)) => s.rename(&id, &t).and_then(|_| s.spaces()),
                Job::Ui(Req::AddPerson(id, p)) => s.add_person(&id, &p).and_then(|_| s.spaces()),
                Job::Ui(Req::Direct { who, text, invite }) => s.direct_with(&who, &text, invite).map(|_| ()),
                Job::Ui(Req::Group { title, who, text }) => s.group_with(title.as_deref(), &who, &text).map(|_| ()),
                Job::Ui(Req::DirectorySearch(q)) => s.directory_search(&q),
                Job::Directory(d) => { s.directory_event(&d); Ok(()) }
                Job::Activity(a) => s.incoming(&a),
                Job::Presence(d) => { s.presence_event(&d); Ok(()) }
                Job::Ui(Req::Call(c)) => s.call_req(c),
                Job::Ui(Req::SendGif { space, url, title, width, height }) => s.send_gif(&space, &url, &title, width, height).and_then(|_| s.after_write(&space)),
                Job::Ui(Req::Post { space, text, markdown, mentions, kind }) => {
                    let parent = match &kind { PostKind::New => None, PostKind::Reply(p) => Some((p.as_str(), "reply")), PostKind::Edit(m) => Some((m.as_str(), "edit")) };
                    s.post_rich(&space, &text, markdown, &mentions, parent).and_then(|_| s.after_write(&space))
                }
                Job::Locus(d) => s.locus_event(&d),
                Job::Ui(Req::SetPresence { status, ttl }) => { s.keep.on_manual(&status, ttl, keepalive_now()); s.set_presence(&status, ttl) }
                Job::Ui(Req::SetFeature { key, val }) => s.set_feature(&key, val),
                Job::Ui(Req::SendFiles(r)) => s.send_files(r),
                Job::Ui(Req::DownloadFile { key, path, then }) => s.download_file(&key, &path, then),
                Job::Ui(Req::Flag { space, msg, on, flag_url }) => s.flag(&space, &msg, on, &flag_url),
                Job::Ui(Req::Flags) => s.flags_load(),
                Job::Ui(Req::MarkUnread { space, msg }) => s.mark_unread(&space, &msg),
                Job::Ui(Req::PostQuote { space, text, markdown, mentions, quoted_html, quoted_text, quoted_files, author, author_email, parent }) =>
                    s.post_quote(&space, &text, markdown, &mentions, &quoted_html, &quoted_text, &quoted_files, &author, &author_email, parent.as_deref()).and_then(|_| s.after_write(&space)),
                Job::Ui(Req::CallHistory) => s.call_history(),
                Job::Ui(Req::DeleteCalls(ids)) => s.delete_calls(&ids),
                Job::Ui(Req::Calendar) => s.calendar(),
                Job::Ui(Req::Schedule(r)) => s.schedule(&r),
                Job::Ui(Req::PersonalRoom) => s.personal_room(),
                Job::Ui(Req::SpaceOp(op)) => s.space_op(op),
            };
            // background failures only reach the debug log; the user's own actions get a short message (report.rs)
            if let Err(e) = r {
                let (ui, log) = super::report::report(user, &e);
                dbg_conn(&log);
                if let Some(m) = ui { if out.send(Resp::Error(m)).is_err() { break; } }
            }
        }
    });
    Client { tx, rx }
}

/// Hands every cloud answer / live event to the UI and wakes it at once (no batching).
pub(super) fn forward(from_cloud: Receiver<Resp>, to_ui: Sender<Resp>, wake: impl Fn()) {
    for r in from_cloud { if to_ui.send(r).is_err() { break } wake(); }
}

/// Unix ms now (the keepalive's clock).
pub(super) fn keepalive_now() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64) }

/// Whether the diagnostic logs are written: only when the environment variable `WEBOX_DEBUG_LOG` is `1` (read once).
pub fn debug_log_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("WEBOX_DEBUG_LOG").is_ok_and(|v| v == "1"))
}

/// Connection diagnostics → `connect-debug.log` in the settings folder (`%APPDATA%\WebOx`, `~/.config/webox`,
/// `~/Library/Application Support/WebOx`), only when `WEBOX_DEBUG_LOG=1` (off by default). Each line starts with the
/// UTC time of day (hh:mm:ss.mmm).
pub fn dbg_conn(s: &str) {
    if !debug_log_enabled() { return; }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(crate::system::config_dir().join("connect-debug.log")) {
        let ms = keepalive_now().rem_euclid(86_400_000);
        let _ = writeln!(f, "{:02}:{:02}:{:02}.{:03} {s}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    }
}

pub(super) fn defaults() -> HashMap<String, String> {
    [("conversation", "https://conv-a.wbx2.com/conversation/api/v1"), ("wdm", "https://wdm-a.wbx2.com/wdm/api/v1"),
     ("encryption", "https://encryption-a.wbx2.com/encryption/api/v1"), ("locus", "https://locus-a.wbx2.com/locus/api/v1"),
     ("apheleia", "https://apheleia-a.wbx2.com/apheleia/api/v1")]
        .into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

pub(super) fn st(v: &Value, k: &str) -> String { v[k].as_str().unwrap_or("").to_string() }

pub const COVER_POLICY: &str = "cover-image-selection";
pub const COVER_MAX_BYTES: u64 = 10 * 1024 * 1024; // cover download size cap

/// Finds a policy either as `{"<name>": "<value>"}` or as `{"…": "<name>", "value": "<value>"}`, anywhere in the device answer.
pub fn find_policy(v: &Value, name: &str) -> Option<String> {
    match v {
        Value::Object(m) => {
            if let Some(s) = m.get(name).and_then(Value::as_str) { return Some(s.to_string()); }
            if m.values().any(|x| x.as_str() == Some(name)) {
                if let Some(s) = m.get("value").and_then(Value::as_str) { return Some(s.to_string()); }
            }
            m.values().find_map(|x| find_policy(x, name))
        }
        Value::Array(a) => a.iter().find_map(|x| find_policy(x, name)),
        _ => None,
    }
}

/// `http()`'s error for a 404 ("PUT <url> → 404 …"): the saved device registration no longer exists.
pub(crate) fn device_gone(err: &str) -> bool { http_status(err) == Some(404) }
/// The HTTP status in an `http()` error ("GET <url> → 401 …").
pub(crate) fn http_status(err: &str) -> Option<u16> { err.split(" → ").nth(1)?.split_whitespace().next()?.parse().ok() }

/// Where a forced renewal gets its refresh token (refresh token, idbroker base); set by `start()` and
/// `Req::Token`.
pub(super) static REFRESH_SRC: std::sync::RwLock<Option<(String, String)>> = std::sync::RwLock::new(None);
pub(super) static LAST_FORCED: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
/// A bearer after a 401 with `old`: the one another request already renewed, else a fresh one (at most one renewal per
/// 30 s, so a burst of 401s - or a refresh token that no longer works - doesn't hammer the identity service).
pub(super) fn force_refresh(old: &str) -> Option<String> {
    let live = live_token();
    if !live.is_empty() && live != old { return Some(live) }
    {
        let mut last = LAST_FORCED.lock().ok()?;
        if last.is_some_and(|t| t.elapsed() < Duration::from_secs(30)) { return None }
        *last = Some(Instant::now());
    }
    let (r, base) = REFRESH_SRC.read().ok()?.clone()?;
    match crate::cloud::auth::refresh(&r, &base, "") {
        Ok(t) => { if let Ok(mut l) = LIVE_TOKEN.write() { *l = t.access.clone(); } Some(t.access) }
        Err(e) => { dbg_conn(&format!("forced renewal after 401 failed: {}", e.chars().take(80).collect::<String>())); None }
    }
}

/// The sign-in token is only ever sent to Webex's own servers.
pub fn webex_host(url: &str) -> bool { url_host(url, &["https"]).is_some_and(|h| WEBEX_DOMAINS.iter().any(|d| subdomain_of(&h, d))) }
/// Webex's own domains (only hosts below them count, not the bare domain).
pub(super) const WEBEX_DOMAINS: [&str; 3] = ["wbx2.com", "webex.com", "ciscospark.com"];
/// The public REST API (`meetcall::HYDRA`): the one bare host outside `WEBEX_DOMAINS` that gets the bearer.
pub(super) const HYDRA_HOST: &str = "webexapis.com";

/// The lower-case host a URL really connects to, parsed the way the HTTP client (ureq, url crate) parses it; `None` unless
/// the scheme is one of `schemes`, there is no user name / password and the port is the scheme's default. Cutting the text
/// at the first `/` is not enough: `https://evil.tld?.webex.com`, `…#.webex.com`, `…\.webex.com` and
/// `https://a.webex.com@evil.tld/` all connect to evil.tld.
pub(super) fn url_host(url: &str, schemes: &[&str]) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    if !schemes.contains(&u.scheme()) || !u.username().is_empty() || u.password().is_some() || u.port().is_some() { return None }
    u.host_str().map(str::to_ascii_lowercase).filter(|h| !h.is_empty())
}
/// `host` lies below `domain` (dot boundary, the bare domain itself doesn't count).
pub(super) fn subdomain_of(host: &str, domain: &str) -> bool { host.strip_suffix(domain).is_some_and(|p| p.len() > 1 && p.ends_with('.')) }
pub(super) fn hhmm(iso: &str) -> String { iso.get(11..16).unwrap_or(iso).to_string() }

#[cfg(test)]
mod renew_tests {
    use super::super::presence_renew_due;

    #[test]
    fn renewal_on_the_wall_clock() {
        let at = 1_000_000_000_000;
        assert!(!presence_renew_due(at, at - 1));
        assert!(presence_renew_due(at, at));
        assert!(presence_renew_due(at, at + 10 * 3_600_000), "after a night asleep: at once");
        assert!(!presence_renew_due(at, at - 540_000), "planned one period ahead");
        assert!(presence_renew_due(at, at - 540_001), "clock set back: don't wait for hours");
    }
}

#[cfg(test)]
mod host_tests {
    use super::{url_host, webex_host};

    #[test]
    fn real_webex_hosts_pass() {
        for u in ["https://conv-a.wbx2.com/conversation/api/v1", "https://idbroker-eu.webex.com/idb", "https://x.ciscospark.com", "https://LOCUS-A.WBX2.COM/locus", "https://conv-a.wbx2.com:443/x"] {
            assert!(webex_host(u), "{u}");
        }
    }

    /// The text after `https://` up to the first `/` is not the host the client connects to.
    #[test]
    fn look_alike_urls_are_refused() {
        for u in ["https://evil.tld?.webex.com", "https://evil.tld#.webex.com", "https://evil.tld\\.webex.com", "https://a.webex.com@evil.tld/",
            "https://u:p@a.webex.com/", "https://a.webex.com:8443/", "http://a.webex.com/", "https://webex.com/", "https://evilwebex.com/",
            "https://a.webex.com.evil.tld/", "webex.com", "", "https://[::1]/"] {
            assert!(!webex_host(u), "{u}");
        }
    }

    #[test]
    fn url_host_is_the_connected_host() {
        assert_eq!(url_host("https://evil.tld?.webex.com", &["https"]).as_deref(), Some("evil.tld"));
        assert_eq!(url_host("https://A.Webex.COM/x", &["https"]).as_deref(), Some("a.webex.com"));
        assert_eq!(url_host("http://media.giphy.com:80/x", &["http"]).as_deref(), Some("media.giphy.com"));
        assert_eq!(url_host("ftp://a.webex.com/", &["https"]), None);
    }
}
