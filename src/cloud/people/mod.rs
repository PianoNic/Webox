//! Profile pictures (avatar service) and presence (apheleia snapshot + usersub live updates). Read-only.
//!
//! Both services key people by their Webex UUID (`entryUUID` of a conversation participant), not by the
//! email-style participant `id`; `Session.uuids` maps the participant id to that UUID.
use super::*;
use std::io::Read;

mod subs;
pub use subs::{evicted, subscribe_body, Subs};

/// Picture size asked from the avatar service (px). 192 covers the 96 pt profile card at 200 %, so large avatars are not
/// upscaled. `AVATAR_SIZE_FALLBACK` is used when the answer has no entry for the larger size.
pub const AVATAR_SIZE: u32 = 192;
pub const AVATAR_SIZE_FALLBACK: u32 = 80;
pub const AVATAR_MAX_BYTES: u64 = 2 * 1024 * 1024;
const AVATAR_BATCH: usize = 40;
const PRESENCE_BATCH: usize = 100;

/// `req` with the bearer when `url` passes the same host gate as `Session::http` (`bearer_host`); anything else goes without it.
fn with_bearer(req: ureq::Request, url: &str, token: &str) -> ureq::Request {
    if super::session::bearer_host(url) { req.set("Authorization", &format!("Bearer {token}")) } else { req }
}

fn is_uuid(s: &str) -> bool { s.len() == 36 && s.matches('-').count() == 4 }

/// "2026-11-13T13:37:17.527Z" → Unix milliseconds (0 if unparseable).
pub fn iso_ms(s: &str) -> i64 {
    let n = |r: std::ops::Range<usize>| s.get(r).and_then(|x| x.parse::<i64>().ok());
    let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(se)) = (n(0..4), n(5..7), n(8..10), n(11..13), n(14..16), n(17..19)) else { return 0 };
    let ms = s.get(20..23).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
    // Days from civil (Howard Hinnant).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    (((days * 24 + h) * 60 + mi) * 60 + se) * 1000 + ms
}

/// When a status ends and its text, from an apheleia status record (`expires` / `expiresTime`, ISO) or a live event.
/// For "active"/"inactive" `expires` is only the record's time-to-live, so it is ignored there; DND, meeting, out of
/// office… carry the real end.
pub fn status_detail(status: &str, s: &Value) -> (Option<i64>, String) {
    let until = if matches!(status, "" | "active" | "inactive") { None } else {
        ["expiresTime", "expires", "until", "endTime", "expirationTime"].iter().find_map(|k| match &s[*k] {
            Value::String(t) => Some(iso_ms(t)).filter(|m| *m > 0),
            Value::Number(n) => n.as_i64().filter(|m| *m > 1_000_000_000_000), // Unix ms
            _ => None,
        })
    };
    let text = ["statusText", "customStatusText", "message", "text", "statusMessage", "displayName", "content", "description"].iter()
        .map(|k| st(s, k)).find(|t| !t.is_empty()).unwrap_or_default();
    (until, text)
}

/// Maps the presence services' status names to the strings the UI's `presence_text()` knows.
pub fn normalize_status(s: &str) -> String {
    match s.to_ascii_lowercase().as_str() {
        "active" | "online" | "available" => "active".into(),
        "inactive" | "idle" | "unavailable" => "inactive".into(),
        // Away is its own state in the presence model, not Inactive: the keepalive stops for it.
        "away" => "away".into(),
        "dnd" | "donotdisturb" | "do_not_disturb" => "DoNotDisturb".into(),
        // The presence model keeps Busy apart from Dnd (its own badge, busy-presence-regular).
        "busy" => "busy".into(),
        "ooo" | "outofoffice" | "out_of_office" => "OutOfOffice".into(),
        "meeting" | "call" | "presenting" => s.to_ascii_lowercase(),
        "quiet" | "quiethours" | "quiet_hours" => "quiet".into(),
        "calendaritem" | "calendar" | "calendar_item" => "calendar".into(),
        "unknown" | "offline" | "pending" | "" => String::new(),
        _ => s.to_string(),
    }
}

/// usersub's numeric `presenceState`: Unknown, Pending, Active, Inactive, Dnd, Quiet, Busy, OutOfOffice, Call, Meeting,
/// Presenting, CalendarItem, Away (0..=12). Away maps like apheleia's "away" (see `normalize_status`): its own state,
/// not Inactive.
pub fn state_status(n: i64) -> String {
    match n {
        2 => "active", 3 => "inactive", 4 => "DoNotDisturb", 5 => "quiet", 6 => "busy", 7 => "OutOfOffice", 8 => "call",
        9 => "meeting", 10 => "presenting", 11 => "calendar", 12 => "away", _ => "",
    }.into()
}

/// Which order an update carries: usersub's `sequence` (one less than or equal to the previous is ignored) or
/// apheleia's `statusTime`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Order { #[default] None, Sequence(i64), StatusTime(i64) }

/// One person's presence from a live event or the snapshot, keyed by the person's uuid. `status: None` = the update says
/// nothing about the status (only activity / timestamps).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Update { pub uuid: String, pub status: Option<String>, pub last: i64, pub until: Option<i64>, pub text: String, pub order: Order }

fn num(v: &Value) -> Option<i64> { v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())) }
fn ms_of(v: &Value) -> i64 { match v { Value::String(s) => iso_ms(s), v => v.as_i64().unwrap_or(0) } }

/// Parses a presence event (`usersub.userstate`, `apheleia.subscription_update`, or a list of either) into updates.
/// Shapes: usersub `{user, sequence, ttl, compositions:[{type: availability, composition:{type}}, {type: lastActivity,
/// composition:{lastActiveTime}}, workStatus, customStatus, …]}` (most events carry only timestamps, no availability);
/// apheleia `{subject, status, category, statusTime, expiresTime, expiresTTL, lastActive}`; parsed records
/// `{contactId, presenceState, lastActiveTime, statusTime, expiresTime, sequence}`, also as `userStates[]`.
pub fn parse_presence(data: &Value) -> Vec<Update> {
    let mut out = vec![];
    let mut items: Vec<&Value> = vec![];
    for key in ["userStates", "subscriptionUpdates", "updates", "presenceUpdates", "statusList"] {
        if let Some(a) = data[key].as_array() { items.extend(a); }
    }
    if items.is_empty() { items.push(data); }
    for s in items {
        let uuid = [st(s, "user"), st(s, "contactId"), st(s, "subject"), st(s, "userId")].into_iter().find(|x| !x.is_empty()).unwrap_or_default();
        if uuid.is_empty() { continue }
        let order = match (num(&s["sequence"]), ms_of(&s["statusTime"])) { (Some(n), _) if n > 0 => Order::Sequence(n), (_, t) if t > 0 => Order::StatusTime(t), _ => Order::None };
        if let Some(comps) = s["compositions"].as_array() {
            let find = |t: &str| comps.iter().find(|c| c["type"] == t).map(|c| &c["composition"]);
            let last = find("lastActivity").map(|c| ms_of(&c["lastActiveTime"])).unwrap_or(0);
            let status = find("availability").map(|c| normalize_status(&st(c, "type")));
            let (mut until, mut text) = match (&status, find("availability")) { (Some(st), Some(c)) => status_detail(st, c), _ => (None, String::new()) };
            // customStatus / workStatus carry the user's own text and an end when one was set (field names read defensively).
            for t in ["workStatus", "customStatus"] {
                if let Some(c) = find(t) { let (u, tx) = status_detail("custom", c); until = until.or(u); if text.is_empty() { text = tx; } }
            }
            if status.is_none() && last == 0 { continue } // nothing the UI shows (timestamps, capabilities, cover choice)
            out.push(Update { uuid, status, last, until, text, order });
            continue;
        }
        let status = match s["presenceState"].as_i64() { Some(n) => state_status(n), None => normalize_status(&st(s, "status")) };
        let last = match &s["lastActiveTime"] { Value::Null => iso_ms(&st(s, "lastActive")), v => ms_of(v) };
        let (until, text) = status_detail(&status, s);
        out.push(Update { uuid, status: Some(status), last, until, text, order });
    }
    out
}

/// After a transient status's end has passed and no update came, the snapshot is asked again at most this often.
pub const PRESENCE_RECHECK_MS: i64 = 60_000;

/// What has been applied per person: the last order seen (updates at or before it are ignored) and when a transient
/// status (call, meeting, DND …) ends. The snapshot is asked again once the status's own end (`expiresTime`) has passed
/// without an update, so a call that ended without an event doesn't stay forever.
#[derive(Default, Debug)]
pub struct Book { seq: HashMap<String, i64>, time: HashMap<String, i64>, ends: HashMap<String, i64>, last: HashMap<String, i64> }

impl Book {
    /// Whether `u` is newer than what was applied; remembers its order and the status's end. Like the desktop client
    /// ("PresenceIgnored"), an update whose last activity is older than one already applied is dropped: the snapshot and
    /// the live events have separate orders, and an older record must not bring back an older state.
    pub fn accept(&mut self, u: &Update, now: i64) -> bool {
        if u.last > 0 && self.last.get(&u.uuid).is_some_and(|l| u.last < *l) { return false }
        let fresh = match u.order {
            Order::Sequence(n) => { let e = self.seq.entry(u.uuid.clone()).or_insert(0); if n <= *e { false } else { *e = n; true } }
            Order::StatusTime(t) => { let e = self.time.entry(u.uuid.clone()).or_insert(0); if t < *e { false } else { *e = t; true } }
            Order::None => true,
        };
        if !fresh { return false }
        if u.last > 0 { self.last.insert(u.uuid.clone(), u.last); }
        if let Some(status) = &u.status {
            match u.until.filter(|_| !matches!(status.as_str(), "" | "active" | "inactive")) {
                Some(end) => { self.ends.insert(u.uuid.clone(), end.max(now + if end <= now { PRESENCE_RECHECK_MS } else { 0 })); }
                None => { self.ends.remove(&u.uuid); }
            }
        }
        true
    }
    /// People whose transient status has ended without an update; each is asked again at most every `PRESENCE_RECHECK_MS`.
    pub fn due(&mut self, now: i64) -> Vec<String> {
        let mut d: Vec<String> = self.ends.iter().filter(|(_, e)| **e <= now).map(|(k, _)| k.clone()).collect();
        d.sort();
        for k in &d { self.ends.insert(k.clone(), now + PRESENCE_RECHECK_MS); }
        d
    }
}

/// The status last sent to the UI per participant id (an update without a status re-sends it with the new activity time).
static LAST_STATUS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, String>>> = std::sync::LazyLock::new(Default::default);

/// Sends `Resp::Presence` and remembers each person's status.
pub(super) fn send_presence(out: &std::sync::mpsc::Sender<Resp>, v: Vec<(String, String, i64)>) {
    if v.is_empty() { return }
    if let Ok(mut m) = LAST_STATUS.lock() { for (id, s, _) in &v { m.insert(id.clone(), s.clone()); } }
    let _ = out.send(Resp::Presence(v));
}

static BOOK: std::sync::Mutex<Option<Book>> = std::sync::Mutex::new(None);
fn book<R>(f: impl FnOnce(&mut Book) -> R) -> R { let mut g = BOOK.lock().unwrap_or_else(|e| e.into_inner()); f(g.get_or_insert_with(Book::default)) }
fn now_ms() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64) }

impl Session {
    pub(super) fn remember_uuid(&mut self, id: &str, p: &Value) {
        let u = st(p, "entryUUID");
        if is_uuid(&u) { self.uuids.insert(id.to_string(), u); }
    }

    fn uuid_of(&self, p: &Person) -> Option<String> {
        if self.is_me(p) && is_uuid(&self.user) { return Some(self.user.clone()); }
        self.uuids.get(&p.id).cloned().or_else(|| is_uuid(&p.id).then(|| p.id.clone()))
    }

    /// Apheleia snapshot: `POST /compositions {"subjects":[uuid…]}` → `statusList[{subject,status,lastActive,…}]`.
    pub(super) fn presence(&self, people: &[Person]) -> Vec<(String, String, i64)> {
        let mut last_active = vec![];
        let mut details: Vec<(String, Option<i64>, String)> = vec![];
        let subjects: Vec<(usize, String)> = people.iter().enumerate().filter_map(|(i, p)| self.uuid_of(p).map(|u| (i, u))).collect();
        let mut seen: HashMap<String, usize> = HashMap::new();
        for chunk in subjects.chunks(PRESENCE_BATCH) {
            let ids: Vec<&str> = chunk.iter().map(|(_, u)| u.as_str()).collect();
            match self.http("POST", &self.url("apheleia", "/compositions"), Some(json!({"subjects": ids}))) {
                Ok(v) => for s in v["statusList"].as_array().into_iter().flatten() {
                    *seen.entry(format!("{}/{}", st(s, "status"), st(s, "category"))).or_default() += 1;
                    // A live update newer than this snapshot record wins (the book keeps the order).
                    let Some(u) = parse_presence(s).into_iter().next() else { continue };
                    if !book(|b| b.accept(&u, now_ms())) { continue }
                    let status = u.status.clone().unwrap_or_default();
                    for (i, _) in chunk.iter().filter(|(_, x)| *x == u.uuid) {
                        details.push((people[*i].id.clone(), u.until, u.text.clone()));
                        last_active.push((people[*i].id.clone(), status.clone(), u.last));
                    }
                },
                Err(e) => dbg_conn(&format!("presence: {}", e.chars().take(100).collect::<String>())),
            }
        }
        if !details.is_empty() { let _ = self.out.send(Resp::PresenceDetail(details)); }
        dbg_conn(&format!("presence statuses (status/category: count): {seen:?}"));
        last_active
    }

    /// Everyone whose presence is followed: me and every participant of every space (the same set
    /// `spaces()` asks for), for the periodic usersub renewal.
    pub(super) fn presence_people(&self) -> Vec<Person> {
        let mut want: Vec<Person> = self.me_person().into_iter().collect();
        want.extend(self.spaces.values().flat_map(|s| s.participants.iter().filter(|p| !self.is_me(p)).cloned()));
        let mut seen = std::collections::HashSet::new();
        want.retain(|p| seen.insert(p.id.clone()));
        want
    }

    /// Opens / renews the live presence subscription (usersub, needs the device URL) so mercury delivers
    /// `usersub.userstate` events, the way the desktop client asks (people/subs.rs): ttl 600, and `refresh` for contacts
    /// new to the subscription or whose subscription lapsed, so their current state is pushed again. Read-only: it only
    /// asks to be told about these contacts.
    pub(super) fn subscribe_presence(&mut self, people: &[Person]) {
        let mut ids: Vec<String> = people.iter().filter_map(|p| self.uuid_of(p)).collect();
        ids.sort();
        ids.dedup();
        for chunk in ids.chunks(PRESENCE_BATCH) {
            let now = now_ms();
            let users: Vec<(String, bool)> = chunk.iter().map(|u| (u.clone(), self.subs.refresh(u, now))).collect();
            let refresh = users.iter().filter(|(_, r)| *r).count();
            let url = self.url("usersub", "/subscribe");
            let post = |body: Value| with_bearer(self.agent.post(&url), &url, &self.token)
                .set("Cisco-Device-URL", &self.device).set("Cisco-Usersub-Device-Caps", "subs:2")
                .set("TrackingID", &format!("CLIENT_{}", jose::uuid())).send_json(body);
            let mut r = post(subscribe_body(&users));
            // Should the service ever refuse the desktop client's body, the older `contactIds` form still subscribes.
            if matches!(r, Err(ureq::Error::Status(400, _))) {
                dbg_conn("usersub subscribe: users body refused (400), sending contactIds");
                r = post(json!({"contactIds": chunk}));
            }
            match r {
                Ok(r) => { self.subs.subscribed(chunk, now); dbg_conn(&format!("usersub subscribe {} contacts ({refresh} refresh) -> {}", chunk.len(), r.status())) }
                Err(e) => dbg_conn(&format!("usersub subscribe failed: {}", e.to_string().chars().take(100).collect::<String>())),
            }
        }
    }

    /// Fetches the profile pictures of `people` in the background and sends them as `Resp::Avatars`
    /// keyed by participant id, uuid and email (lower-case). Only people not asked for before.
    pub(super) fn load_avatars(&mut self, people: &[Person]) {
        let mut want: Vec<(String, Vec<String>)> = vec![]; // uuid → the keys the UI may look it up by
        for p in people {
            let Some(u) = self.uuid_of(p) else { continue };
            if !self.avatars_done.insert(u.clone()) { continue }
            let mut keys = vec![u.clone(), p.id.clone()];
            if !p.email.is_empty() { keys.push(p.email.to_lowercase()); }
            if p.id == self.user || self.is_me(p) { keys.push(self.my_email.to_lowercase()); }
            keys.sort(); keys.dedup(); keys.retain(|k| !k.is_empty());
            want.push((u, keys));
        }
        if want.is_empty() { return; }
        let (agent, token, base, out) = (self.agent.clone(), self.token.clone(), self.url("avatar", "/profiles/urls"), self.out.clone());
        std::thread::spawn(move || {
            let mut got = 0usize;
            let mut outcome: std::collections::BTreeMap<String, usize> = Default::default();
            for chunk in want.chunks(AVATAR_BATCH) {
                let body: Vec<Value> = chunk.iter().map(|(u, _)| json!({"uuid": u, "sizes": [AVATAR_SIZE, AVATAR_SIZE_FALLBACK]})).collect();
                let r = with_bearer(agent.post(&base), &base, &token).set("TrackingID", &format!("CLIENT_{}", jose::uuid())).send_json(Value::Array(body));
                let r = match r { Ok(r) => r, Err(e) => { *outcome.entry(format!("urls request failed: {}", match e { ureq::Error::Status(c, _) => c.to_string(), _ => "transport".into() })).or_default() += chunk.len(); continue } };
                let v: Value = r.into_json().unwrap_or(Value::Null);
                let mut batch = vec![];
                for (u, keys) in chunk {
                    let item = [AVATAR_SIZE, AVATAR_SIZE_FALLBACK].iter().map(|s| &v[u.as_str()][s.to_string()]).find(|i| !i.is_null()).unwrap_or(&Value::Null);
                    let url = st(item, "url");
                    if item.is_null() { *outcome.entry("no entry in answer".into()).or_default() += 1; continue }
                    if item["defaultAvatar"].as_bool().unwrap_or(false) { *outcome.entry("default avatar (initials)".into()).or_default() += 1; continue }
                    if !url.starts_with("https://") { *outcome.entry("no https url".into()).or_default() += 1; continue }
                    // The token goes only to Webex's own hosts; the CDN hosts the API returns get a plain GET.
                    let mut req = agent.get(&url);
                    if webex_host(&url) { req = req.set("Authorization", &format!("Bearer {token}")); }
                    let resp = match req.call() { Ok(r) => r, Err(e) => { *outcome.entry(format!("download failed: {}", match e { ureq::Error::Status(c, _) => c.to_string(), _ => "transport".into() })).or_default() += 1; continue } };
                    let mut buf = vec![];
                    if resp.into_reader().take(AVATAR_MAX_BYTES).read_to_end(&mut buf).is_ok() && !buf.is_empty() {
                        got += 1;
                        *outcome.entry("downloaded".into()).or_default() += 1;
                        for k in keys { batch.push((k.clone(), buf.clone())); }
                    }
                }
                if !batch.is_empty() { let _ = out.send(Resp::Avatars(batch)); }
            }
            dbg_conn(&format!("avatars: {got} of {} downloaded; outcomes {outcome:?}", want.len()));
        });
    }

    /// The other person's own cover for a direct space (shown as the chat banner):
    /// `GET avatar/coverImages/users/<uuid>`; people without a choice answer `{"avatarUrl":"webex://coverImage?selection=default"}`.
    pub(super) fn load_person_cover(&mut self, space: &str) {
        let Some(sp) = self.spaces.get(space).filter(|s| s.direct) else { return };
        let Some(other) = sp.participants.iter().find(|p| !self.is_me(p)).cloned() else { return };
        let Some(uuid) = self.uuid_of(&other) else { return };
        if !self.covers_done.insert(uuid.clone()) { return }
        let mut keys = vec![uuid.clone(), other.id.clone()];
        if !other.email.is_empty() { keys.push(other.email.to_lowercase()); }
        keys.sort(); keys.dedup();
        let (agent, token, url) = (self.agent.clone(), self.token.clone(), self.url("avatar", &format!("/coverImages/users/{uuid}")));
        let out = self.out.clone();
        std::thread::spawn(move || {
            let a: Value = match with_bearer(agent.get(&url), &url, &token).call() {
                Ok(r) => r.into_json().unwrap_or(Value::Null),
                Err(e) => { dbg_conn(&format!("person cover: {}", match e { ureq::Error::Status(c, _) => format!("http {c}"), _ => "transport".into() })); return }
            };
            let urls = &a["imageUrls"];
            let pick = ["Dark.png", "Light.png", "thumbnail"].iter().map(|k| st(urls, k)).find(|u| u.starts_with("https://"));
            let Some(u) = pick else { dbg_conn("person cover: none chosen (default selection)"); return };
            let mut req = agent.get(&u);
            if images::token_host(&u) { req = req.set("Authorization", &format!("Bearer {token}")); }
            let mut buf = vec![];
            let got = req.call().ok().map(|r| r.into_reader().take(COVER_MAX_BYTES).read_to_end(&mut buf).is_ok() && !buf.is_empty()).unwrap_or(false);
            match got {
                true => {
                    dbg_conn(&format!("person cover ({:?}): {} bytes", st(&a, "imageName"), buf.len()));
                    let _ = out.send(Resp::Covers(keys.into_iter().map(|k| (k, buf.clone())).collect()));
                }
                false => dbg_conn("person cover: image download failed"),
            }
        });
    }

    /// A live `usersub.userstate` / `apheleia.subscription_update` → `Resp::Presence` (+ details). Updates older than one
    /// already applied are dropped; an update without a status only moves the last-activity time.
    pub(super) fn presence_event(&mut self, data: &Value) {
        let now = now_ms();
        // Subscriptions the server dropped: the next renewal subscribes them again, with their current state.
        let gone = evicted(data);
        if !gone.is_empty() { dbg_conn(&format!("usersub: {} subscriptions evicted", gone.len())); self.subs.forget(&gone); }
        let updates: Vec<Update> = parse_presence(data).into_iter().filter(|u| book(|b| b.accept(u, now))).collect();
        let (mapped, mapped_details) = self.map_updates(&updates);
        send_presence(&self.out, mapped);
        if !mapped_details.is_empty() { let _ = self.out.send(Resp::PresenceDetail(mapped_details)); }
    }

    /// Updates keyed by uuid → the UI's (participant id, status, last active) and (id, until, text). A status-less update
    /// keeps the person's known status (it is sent again unchanged) and only brings the activity time.
    fn map_updates(&self, updates: &[Update]) -> (Vec<(String, String, i64)>, Vec<(String, Option<i64>, String)>) {
        let ids_of = |uuid: &str| -> Vec<String> {
            let mut v: Vec<String> = self.uuids.iter().filter(|(_, u)| u.as_str() == uuid).map(|(id, _)| id.clone()).collect();
            if uuid == self.user { v.push(self.my_email.clone()); }
            v
        };
        let (mut mapped, mut details) = (vec![], vec![]);
        for u in updates {
            for id in ids_of(&u.uuid) {
                match &u.status {
                    Some(s) => { mapped.push((id.clone(), s.clone(), u.last)); details.push((id, u.until, u.text.clone())); }
                    None => if let Some(s) = self.known_status(&id) { mapped.push((id, s, u.last)); },
                }
            }
        }
        (mapped, details)
    }

    /// The status the space list last filed for a participant id (live updates change it in the UI only, so a status-less
    /// update re-sends what the UI has unless it changed since; `Resp::Presence` carries the status with the time).
    fn known_status(&self, id: &str) -> Option<String> {
        LAST_STATUS.lock().ok().and_then(|m| m.get(id).cloned())
    }

    /// Transient statuses (call, meeting, DND …) whose end passed without an update: ask the snapshot again for those people.
    pub(super) fn presence_recheck(&self) {
        let due = book(|b| b.due(now_ms()));
        if due.is_empty() { return }
        let people: Vec<Person> = self.presence_people().into_iter().filter(|p| self.uuid_of(p).is_some_and(|u| due.contains(&u))).collect();
        if people.is_empty() { return }
        let v = self.presence(&people);
        send_presence(&self.out, v);
    }
}

#[cfg(test)]
mod tests;
