//! Builds the store's rows from what Webox already keeps in memory (no Webex request). `Source` names every piece of app
//! state the model reads; `src/app/data_sync.rs` fills it from `App`, the tests from fixtures. Each `*_rows` function builds
//! the complete rows of one kind (messages: of one space), in source order.
use super::model::Kind;
use super::store::Row;
use crate::cloud::locus::controls::Roster;
use crate::cloud::meetcall::{CalEvent, CallRecord};
use crate::cloud::{Meeting, Message, Person, ReadMark, Space, Team};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// What the space list does not carry: notification setting, lock, moderators (person ids or uuids) and the full member list
/// once it was loaded.
#[derive(Clone, Debug, Default)]
pub struct SpaceExt { pub notify: &'static str, pub locked: bool, pub moderators: Vec<String>, pub people: Option<Vec<Person>> }

/// A team's facts from the conversation list.
#[derive(Clone, Debug, Default)]
pub struct TeamExt { pub general: String, pub archived: bool }

/// The current call as the window has it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallView {
    pub id: String,
    /// "dialing", "ringing", "incoming", "connecting", "connected", "ended".
    pub state: &'static str,
    pub incoming: bool, pub name: String, pub email: String,
    pub started_ms: Option<i64>,
    pub roster: Roster,
    pub muted: bool, pub muted_by_host: bool, pub unmute_allowed: bool, pub asked_to_unmute: bool,
    pub video: bool, pub video_sending: bool, pub sharing: bool,
    /// The camera sent, the size and rate it is sent at (while sending), and whether the other side's camera is on.
    pub camera: String, pub video_size: (u32, u32), pub video_fps: u32, pub remote_video: bool,
    pub remote_share: Option<String>,
}

/// A toast on screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NotifView { pub id: u64, pub kind: &'static str, pub at_ms: Option<i64>, pub space: Option<String>, pub title: String, pub body: String }

/// The window's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppView {
    pub page: &'static str, pub selected: Option<String>, pub team: Option<String>,
    pub connected: bool, pub signed_in: bool, pub status: String, pub call_active: bool,
}

/// One plugin of the list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PluginView { pub key: String, pub name: String, pub version: String, pub author: String, pub enabled: bool, pub builtin: bool, pub error: Option<String>,
    /// Its setting values, passwords left out.
    pub settings: Option<serde_json::Map<String, Value>> }

/// Everything the model is built from. Closures reach state that lives in UI-thread-local storage.
pub struct Source<'a> {
    pub me: &'a Person,
    pub spaces: &'a [Space],
    pub teams: &'a [Team],
    pub people: &'a [Person],
    pub meetings: &'a [Meeting],
    pub messages: &'a HashMap<String, Vec<Message>>,
    pub settings: &'a Value,
    pub drafts: &'a HashMap<String, String>,
    pub flags: &'a HashSet<(String, String)>,
    pub last_active: &'a HashMap<String, i64>,
    pub presence_info: &'a HashMap<String, (Option<i64>, String)>,
    pub outside_status: &'a HashMap<String, String>,
    pub read_marks: &'a HashMap<String, Vec<ReadMark>>,
    /// Directory search results kept by the data hub.
    pub directory: &'a [Person],
    /// Whether a profile picture (`photo`) or an image (`image`) with that cache key is loaded.
    pub has_photo: &'a dyn Fn(&str) -> bool,
    pub has_image: &'a dyn Fn(&str) -> bool,
    pub space_ext: &'a dyn Fn(&str) -> SpaceExt,
    pub team_ext: &'a dyn Fn(&str) -> Option<TeamExt>,
    pub typers: &'a dyn Fn(&str) -> Vec<Person>,
    pub call: Option<CallView>,
    pub call_records: Option<&'a [CallRecord]>,
    pub calendar: Option<&'a [CalEvent]>,
    pub notifications: Vec<NotifView>,
    pub app: AppView,
    pub plugins: Vec<PluginView>,
    /// My own cover picture is loaded (`Resp::Cover`).
    pub my_cover: bool,
    pub now_ms: i64,
}

/// Fills one row by field name (an unknown name is a bug: it panics, and the tests build every kind).
struct R { kind: Kind, row: Row }
impl R {
    fn new(kind: Kind, id: &str) -> R { let mut row = vec![Value::Null; kind.def().fields.len()]; row[0] = json!(id); R { kind, row } }
    fn set(&mut self, f: &str, v: impl Into<Value>) -> &mut Self { self.row[self.kind.f(f)] = v.into(); self }
    fn opt<T: Into<Value>>(&mut self, f: &str, v: Option<T>) -> &mut Self { if let Some(v) = v { self.set(f, v); } self }
    fn done(self) -> (String, Row) { (self.row[0].as_str().unwrap_or_default().to_string(), self.row) }
}
fn nonempty(s: &str) -> Option<&str> { (!s.is_empty()).then_some(s) }
fn iso_ms(s: &str) -> Option<i64> { crate::i18n::parse_iso_ms(s) }

/// Up to two letters: the first of the first and of the last word.
pub fn initials(name: &str) -> String {
    let w: Vec<&str> = name.split_whitespace().filter(|w| w.chars().next().is_some_and(char::is_alphanumeric)).collect();
    let first = |s: &str| s.chars().next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
    match w.as_slice() { [] => String::new(), [one] => first(one), [a, .., b] => first(a) + &first(b) }
}

/// The store's presence state for Webox's status string.
pub fn presence_state(s: &str) -> &'static str {
    // the spellings cloud/people normalises to (and the raw service names)
    match s.to_ascii_lowercase().as_str() {
        "active" | "online" | "available" => "active", "inactive" | "idle" | "unavailable" => "inactive", "away" => "away",
        "donotdisturb" | "dnd" | "do_not_disturb" => "dnd", "busy" => "busy", "quiet" | "quiethours" | "quiet_hours" => "quiet",
        "outofoffice" | "ooo" | "out_of_office" => "ooo", "call" => "call", "meeting" => "meeting", "presenting" => "presenting",
        "calendar" | "calendaritem" | "calendar_item" => "calendar", _ => "unknown",
    }
}

/// People by id and by lower-case e-mail, built once per sync: every relation to a person goes through it.
#[derive(Default)]
pub struct People {
    /// (id, person, contact, directory), in order: me, contacts, the people of my spaces, call participants, directory
    /// results, then authors and reactors of loaded messages.
    list: Vec<(String, Person, bool, bool)>,
    by_id: HashMap<String, usize>,
    by_mail: HashMap<String, usize>,
}
impl People {
    pub fn of(src: &Source) -> People {
        let mut p = People::default();
        p.add(src.me, false, false);
        for x in src.people { p.add(x, true, false); }
        for s in src.spaces {
            for x in &s.participants { p.add(x, true, false); }
            if let Some(full) = (src.space_ext)(&s.id).people { for x in &full { p.add(x, true, false); } }
        }
        if let Some(c) = &src.call {
            for m in &c.roster.members { p.add(&Person { id: String::new(), name: m.name.clone(), email: m.email.clone(), status: String::new() }, false, false); }
        }
        for x in src.directory { p.add(x, false, true); }
        for l in src.messages.values() {
            for m in l {
                if !m.email.is_empty() && m.notice.is_empty() { p.add(&Person { id: String::new(), name: m.author.clone(), email: m.email.clone(), status: String::new() }, false, false); }
                for r in m.reactions.iter().flat_map(|r| &r.users) {
                    if !r.email.is_empty() || !r.id.is_empty() { p.add(&Person { id: if r.email.is_empty() { r.id.clone() } else { String::new() }, name: r.name.clone(), email: r.email.clone(), status: String::new() }, false, false); }
                }
            }
        }
        p
    }
    /// Adds a person, or merges into the one with the same id or e-mail (a real id wins over `email:`; a contact stays one).
    fn add(&mut self, x: &Person, contact: bool, directory: bool) {
        let mail = x.email.to_lowercase();
        let found = nonempty(&x.id).and_then(|id| self.by_id.get(id)).or_else(|| nonempty(&mail).and_then(|m| self.by_mail.get(m))).copied();
        match found {
            Some(i) => {
                let e = &mut self.list[i];
                if e.1.name.is_empty() { e.1.name = x.name.clone(); }
                if e.1.email.is_empty() && !x.email.is_empty() { e.1.email = x.email.clone(); self.by_mail.insert(mail, i); }
                if e.1.status.is_empty() { e.1.status = x.status.clone(); }
                if e.0.starts_with("email:") && !x.id.is_empty() { e.0 = x.id.clone(); e.1.id = x.id.clone(); self.by_id.insert(x.id.clone(), i); }
                e.2 |= contact;
                e.3 = e.3 && directory && !e.2;
            }
            None => {
                let id = if !x.id.is_empty() { x.id.clone() } else if !mail.is_empty() { format!("email:{mail}") } else { return };
                let i = self.list.len();
                self.by_id.insert(id.clone(), i);
                if !mail.is_empty() { self.by_mail.insert(mail, i); }
                self.list.push((id, x.clone(), contact, directory && !contact));
            }
        }
    }
    /// The store id of a person given by id, uuid or e-mail.
    pub fn id(&self, id_or_mail: &str) -> Option<&str> {
        let k = id_or_mail.to_lowercase();
        self.by_id.get(id_or_mail).or_else(|| self.by_mail.get(&k)).map(|&i| self.list[i].0.as_str())
    }
    fn by_name(&self, name: &str) -> Option<&str> { self.list.iter().find(|p| !name.is_empty() && p.1.name == name).map(|p| p.0.as_str()) }
    fn ids_of<'a>(&'a self, l: impl Iterator<Item = &'a str>) -> Vec<Value> { l.filter_map(|x| self.id(x)).map(|x| json!(x)).collect() }
}

fn photo_ref(src: &Source, id: &str, email: &str) -> Option<String> {
    [id, &email.to_lowercase()].into_iter().find(|k| !k.is_empty() && (src.has_photo)(k)).map(|_| format!("img:person:{id}"))
}
fn cover_ref(src: &Source, id: &str, email: &str) -> Option<String> {
    [id, &email.to_lowercase()].into_iter().find(|k| !k.is_empty() && (src.has_image)(&format!("cover:{k}"))).map(|_| format!("img:cover:{id}"))
}
fn is_me(src: &Source, p: &Person) -> bool {
    (!p.id.is_empty() && p.id == src.me.id) || (!p.email.is_empty() && p.email.eq_ignore_ascii_case(&src.me.email))
}
/// The other person of a direct space.
fn peer_of<'a>(src: &Source, s: &'a Space, people: &'a People) -> Option<&'a str> {
    if !s.direct { return None; }
    s.participants.iter().find(|p| !is_me(src, p)).and_then(|p| people.id(if p.id.is_empty() { &p.email } else { &p.id })).or_else(|| people.by_name(&s.title))
}
/// Whether `m` mentions me (a person mention of my id in the HTML, else my name in the text).
pub fn mentions_me(m: &Message, me: &Person) -> bool {
    if m.mine { return false; }
    if !m.html.is_empty() { return !me.id.is_empty() && m.html.contains(&format!("data-object-id=\"{}\"", me.id)); }
    !me.name.is_empty() && m.text.to_lowercase().contains(&me.name.to_lowercase())
}

pub fn person_rows(src: &Source, people: &People) -> Vec<(String, Row)> {
    let directs: HashMap<&str, (&str, bool)> = src.spaces.iter().filter_map(|s| peer_of(src, s, people).map(|p| (p, (s.id.as_str(), s.favorite)))).collect();
    people.list.iter().map(|(id, p, contact, directory)| {
        let mut r = R::new(Kind::Person, id);
        let me = id == &people.list[0].0;
        let cover = cover_ref(src, id, &p.email).or_else(|| (me && src.my_cover).then(|| format!("img:cover:{id}")));
        r.set("name", p.name.clone()).set("initials", initials(&p.name)).opt("avatar", photo_ref(src, id, &p.email)).opt("cover", cover)
            .opt("email", nonempty(&p.email)).set("presence", id.clone()).opt("direct", directs.get(id.as_str()).map(|d| d.0))
            .set("favorite", directs.get(id.as_str()).is_some_and(|d| d.1)).set("contact", *contact && !me).set("directory", *directory).set("me", me);
        r.done()
    }).collect()
}

pub fn presence_rows(src: &Source, people: &People) -> Vec<(String, Row)> {
    people.list.iter().map(|(id, p, _, _)| {
        let mail = p.email.to_lowercase();
        let look = |m: &dyn Fn(&str) -> bool| [id.as_str(), p.id.as_str(), mail.as_str()].into_iter().find(|k| !k.is_empty() && m(k)).map(String::from);
        let status = if id == &people.list[0].0 { src.me.status.clone() } else {
            look(&|k| src.outside_status.get(k).is_some_and(|s| !s.is_empty())).and_then(|k| src.outside_status.get(&k).cloned())
                .filter(|_| p.status.is_empty()).unwrap_or_else(|| p.status.clone())
        };
        let state = presence_state(&status);
        let last = look(&|k| src.last_active.contains_key(k)).and_then(|k| src.last_active.get(&k).copied());
        let info = look(&|k| src.presence_info.contains_key(k)).and_then(|k| src.presence_info.get(&k).cloned());
        let mut r = R::new(Kind::Presence, id);
        r.set("person", id.clone()).set("state", state)
            .set("online", matches!(state, "active" | "busy" | "call" | "meeting" | "presenting" | "calendar"))
            .set("in_call", matches!(state, "call" | "meeting" | "presenting"))
            .opt("last_active", last.filter(|l| *l > 0))
            .opt("until", info.as_ref().and_then(|i| i.0)).opt("text", info.as_ref().and_then(|i| nonempty(&i.1).map(String::from)));
        r.done()
    }).collect()
}

/// The newest message of a list that is not a system line.
fn last_message(l: &[Message]) -> Option<&Message> { l.iter().rev().find(|m| m.notice.is_empty() && !m.id.is_empty()) }

/// Whether a loaded message after `last_seen` mentions me.
fn mentioned(src: &Source, s: &Space) -> bool {
    if !s.unread { return false; }
    let seen = iso_ms(&s.last_seen).unwrap_or(0);
    src.messages.get(&s.id).is_some_and(|l| l.iter().any(|m| m.published_ms > seen && mentions_me(m, src.me)))
}

pub fn space_rows(src: &Source, people: &People) -> Vec<(String, Row)> {
    src.spaces.iter().map(|s| {
        let ext = (src.space_ext)(&s.id);
        let peer = peer_of(src, s, people);
        let avatar = if (src.has_image)(&format!("spaceavatar:{}", s.id)) { Some(format!("img:space:{}", s.id)) } else {
            peer.and_then(|p| people.by_id.get(p).map(|&i| &people.list[i].1)).and_then(|p| photo_ref(src, peer.unwrap_or(""), &p.email))
        };
        let members: Vec<Value> = match &ext.people {
            Some(full) => people.ids_of(full.iter().map(|p| if p.id.is_empty() { p.email.as_str() } else { p.id.as_str() })),
            None => people.ids_of(s.participants.iter().map(|p| if p.id.is_empty() { p.email.as_str() } else { p.id.as_str() })),
        };
        let reason = if s.closed { Some("closed") } else if s.announcement { Some("announcement") } else { None };
        let mut r = R::new(Kind::Space, &s.id);
        r.set("title", s.title.clone()).set("kind", if s.direct { "direct" } else { "group" }).opt("team", nonempty(&s.team)).opt("peer", peer)
            .opt("avatar", avatar).opt("cover", (src.has_image)(&format!("spacecover:{}", s.id)).then(|| format!("img:spacecover:{}", s.id)))
            .set("favorite", s.favorite).set("hidden", s.hidden).set("public", s.public).set("unread", s.unread).set("mentioned", mentioned(src, s))
            .opt("last_activity", iso_ms(&s.last)).opt("last_seen", iso_ms(&s.last_seen))
            .set("notify", ext.notify).set("muted", ext.notify == "off").set("writable", reason.is_none()).opt("read_only_reason", reason)
            .set("locked", ext.locked).set("moderators", people.ids_of(ext.moderators.iter().map(String::as_str)))
            .opt("member_count", ext.people.as_ref().map(|l| l.len() as u64)).set("members", members)
            .opt("meeting", (s.meeting || !s.locus_url.is_empty()).then(|| json!({ "active": s.meeting, "locus": s.locus_url })))
            .set("typing", people.ids_of((src.typers)(&s.id).iter().map(|p| if p.id.is_empty() { p.email.as_str() } else { p.id.as_str() })).into_iter().collect::<Vec<_>>())
            .opt("draft", src.drafts.get(&s.id).filter(|d| !d.trim().is_empty()).cloned())
            .opt("last_message", src.messages.get(&s.id).and_then(|l| last_message(l)).map(|m| m.id.clone()));
        r.done()
    }).collect()
}

pub fn team_rows(src: &Source) -> Vec<(String, Row)> {
    src.teams.iter().map(|t| {
        let ext = (src.team_ext)(&t.id).unwrap_or_default();
        let spaces: Vec<&Space> = src.spaces.iter().filter(|s| s.team == t.id).collect();
        let mut r = R::new(Kind::Team, &t.id);
        r.set("name", t.name.clone()).opt("color", t.color.map(|(r, g, b)| format!("#{r:02X}{g:02X}{b:02X}"))).set("initials", initials(&t.name))
            .opt("general", nonempty(&ext.general)).set("archived", ext.archived)
            .set("spaces", spaces.iter().map(|s| json!(s.id)).collect::<Vec<_>>())
            .set("unread", spaces.iter().any(|s| s.unread)).set("mentioned", spaces.iter().any(|s| mentioned(src, s)));
        r.done()
    }).collect()
}

/// Whether a system line is the tombstone of a deleted message (the service's wording in the active language).
fn is_tombstone(notice: &str) -> bool {
    let (a, b) = ("\u{1}", "\u{2}");
    let templates = [
        crate::trq!("You deleted your message.").into_owned(), crate::trq!("You deleted {{0}}'s message.", a),
        crate::trq!("{{0}} deleted their own message.", a), crate::trq!("{{0}} deleted {{1}}'s message.", a, b),
        crate::trq!("You deleted your reply").into_owned(), crate::trq!("You deleted {{0}}'s reply", a),
        crate::trq!("{{0}} deleted their own reply", a), crate::trq!("{{0}} deleted {{1}}'s reply", a, b),
    ];
    templates.iter().any(|t| {
        let pieces: Vec<&str> = t.split(['\u{1}', '\u{2}']).collect();
        let (first, last) = (pieces[0], pieces[pieces.len() - 1]);
        if !notice.starts_with(first) || !notice.ends_with(last) || notice.len() < first.len() + last.len() { return false; }
        let mut rest = &notice[first.len()..notice.len() - last.len()];
        for p in &pieces[1..pieces.len() - 1] { match rest.find(p) { Some(i) => rest = &rest[i + p.len()..], None => return false } }
        pieces.len() > 1 || notice == t
    })
}

/// (author, text) of a quote (`<blockquote class='messageQuote'>author<br/>text</blockquote>`), as plain text.
pub fn quote_of(html: &str) -> Option<(String, String)> {
    let start = html.find("<blockquote class='messageQuote'>").or_else(|| html.find("<blockquote class=\"messageQuote\">"))?;
    let inner = &html[start..];
    let inner = &inner[inner.find('>')? + 1..];
    let inner = &inner[..inner.find("</blockquote>")?];
    let (who, body) = inner.split_once("<br/>").or_else(|| inner.split_once("<br>")).unwrap_or(("", inner));
    let plain = |s: &str| {
        let mut out = String::new();
        let mut tag = false;
        for c in s.replace("<br/>", "\n").replace("<br>", "\n").chars() { match c { '<' => tag = true, '>' => tag = false, c if !tag => out.push(c), _ => {} } }
        out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&").trim().to_string()
    };
    Some((plain(who), plain(body)))
}

fn file_kind(mime: &str, giphy: bool) -> &'static str {
    if giphy || mime == "image/gif" { "gif" } else if mime.starts_with("image/") { "image" } else if mime.starts_with("video/") { "video" } else { "file" }
}

pub fn message_rows(src: &Source, people: &People, space: &str) -> Vec<(String, Row)> {
    let Some(list) = src.messages.get(space) else { return vec![] };
    let mut replies: HashMap<&str, u64> = HashMap::new();
    for m in list { if !m.parent.is_empty() { *replies.entry(m.parent.as_str()).or_default() += 1; } }
    let marks = src.read_marks.get(space).map_or(&[][..], |v| v.as_slice());
    let mut seen_ids = HashSet::new();
    list.iter().enumerate().filter_map(|(i, m)| {
        let id = if m.id.is_empty() { format!("{space}#{i}") } else { m.id.clone() };
        if !seen_ids.insert(id.clone()) { return None; }
        let kind = if m.deleted > 0 { "deleted" } else if !m.group.is_empty() { "join_leave" } else if !m.notice.is_empty() { if is_tombstone(&m.notice) { "deleted" } else { "system" } } else { "message" };
        let author = if m.notice.is_empty() || m.deleted > 0 { nonempty(&m.email).and_then(|e| people.id(e)).map(String::from).or_else(|| m.mine.then(|| people.list[0].0.clone())) } else { None };
        let image = |k: &str| (!k.is_empty() && (src.has_image)(k)).then(|| format!("img:file:{k}"));
        let files: Vec<Value> = m.images.iter().chain(&m.files).map(|a| {
            let mut f = json!({ "name": a.name, "mime": a.mime, "size": a.size, "kind": file_kind(&a.mime, a.giphy.is_some()) });
            if let Some(t) = image(&a.key) { f["thumb"] = json!(t); }
            if a.width > 0 { f["width"] = json!(a.width); }
            if a.height > 0 { f["height"] = json!(a.height); }
            f
        }).collect();
        let previews: Vec<Value> = m.previews.iter().map(|p| {
            let mut v = json!({ "url": p.url, "title": p.title, "description": p.description, "site": p.site });
            if let Some(t) = (!p.image_key.is_empty() && (src.has_image)(&p.image_key)).then(|| format!("img:preview:{}", p.image_key)) { v["image"] = json!(t); }
            v
        }).collect();
        let reactions: Vec<Value> = m.reactions.iter().map(|r| json!({ "name": r.name, "count": r.count, "mine": r.mine.is_some(),
            "people": r.users.iter().filter_map(|u| people.id(if u.email.is_empty() { &u.id } else { &u.email })).collect::<Vec<_>>() })).collect();
        let seen_by = people.ids_of(marks.iter().filter(|k| k.activity == m.id && !m.id.is_empty()).map(|k| if k.email.is_empty() { k.person.as_str() } else { k.email.as_str() }));
        let mut r = R::new(Kind::Message, &id);
        r.set("space", space).opt("author", author).set("author_name", m.author.clone()).set("mine", m.mine).opt("at", (m.published_ms > 0).then_some(m.published_ms))
            .set("kind", kind).set("text", if m.notice.is_empty() { m.text.clone() } else { m.notice.clone() }).opt("html", nonempty(&m.html))
            .opt("parent", nonempty(&m.parent)).opt("replies", replies.get(m.id.as_str()).copied().filter(|_| m.parent.is_empty() && !m.id.is_empty()))
            .set("edited", m.edited).set("kept", m.deleted > 0)
            .opt("history", (!m.history.is_empty()).then(|| m.history.iter().map(|h| h.iter().map(|s| s.0.as_str()).collect::<String>()).collect::<Vec<_>>()))
            .set("reactions", reactions).set("files", files).set("previews", previews).set("mentions_me", mentions_me(m, src.me))
            .set("flagged", !m.id.is_empty() && (src.flags.contains(&(space.to_string(), m.id.clone())) || src.flags.contains(&(String::new(), m.id.clone()))))
            .opt("quote", quote_of(&m.html).map(|(a, t)| json!({ "author": a, "text": t }))).set("seen_by", seen_by);
        Some(r.done())
    }).collect()
}

pub fn call_rows(src: &Source, people: &People) -> Vec<(String, Row)> {
    let Some(c) = &src.call else { return vec![] };
    let me = c.roster.me();
    let direct = crate::ui::call::direct_space_with(src.spaces, &c.email);
    let participants: Vec<Value> = c.roster.members.iter().map(|m| {
        let state = if m.joined() { "joined" } else if m.state == "LEFT" { "left" } else { "ringing" };
        let mut v = json!({ "id": m.id, "name": m.name, "state": state, "muted": m.muted, "hand": m.hand, "host": m.host, "me": m.me });
        if let Some(p) = people.id(&m.email).filter(|_| !m.email.is_empty()) { v["person"] = json!(p); }
        v
    }).collect();
    let mut my_side = json!({ "muted": c.muted, "muted_by_host": c.muted_by_host, "unmute_allowed": c.unmute_allowed, "asked_to_unmute": c.asked_to_unmute,
        "video": c.video, "video_sending": c.video_sending, "sharing": c.sharing, "hand": me.is_some_and(|m| m.hand), "host": c.roster.me_host });
    if !c.camera.is_empty() { my_side["camera"] = json!(c.camera); }
    if c.video_sending && c.video_size.0 > 0 { my_side["video_width"] = json!(c.video_size.0); my_side["video_height"] = json!(c.video_size.1); my_side["video_fps"] = json!(c.video_fps); }
    let mut r = R::new(Kind::Call, &c.id);
    r.set("kind", if c.roster.group { "space" } else { "direct" }).set("state", c.state).set("incoming", c.incoming)
        .opt("peer", people.id(&c.email).filter(|_| !c.email.is_empty() && !c.roster.group)).set("peer_name", c.name.clone())
        .opt("space", direct.filter(|_| !c.roster.group)).opt("started_at", c.started_ms).set("participants", participants)
        .set("me", my_side)
        .set("remote_video", c.remote_video)
        .opt("remote_share", c.remote_share.as_deref().and_then(|n| people.by_name(n))).opt("remote_share_name", c.remote_share.clone());
    vec![r.done()]
}

pub fn call_record_rows(src: &Source, people: &People) -> Vec<(String, Row)> {
    src.call_records.unwrap_or_default().iter().enumerate().map(|(i, c)| {
        let id = if c.session_id.is_empty() { format!("call:{i}") } else { c.session_id.clone() };
        let mut r = R::new(Kind::CallRecord, &id);
        r.set("at", c.start_ms).set("ended_at", c.end_ms).set("secs", c.secs).set("outgoing", c.outgoing).set("missed", c.missed())
            .opt("peer", nonempty(&c.email).and_then(|e| people.id(e)).or_else(|| nonempty(&c.other_id).and_then(|o| people.id(o))))
            .set("name", c.name.clone()).opt("space", crate::ui::call::direct_space_with(src.spaces, &c.email));
        r.done()
    }).collect()
}

/// A loci meeting's start ("HH:MM" today, local time) in ms.
fn hhmm_today(s: &str, now_ms: i64) -> Option<i64> {
    if let Some(ms) = iso_ms(s) { return Some(ms); }
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (i64, i64) = (h.parse().ok()?, m.parse().ok()?);
    if !(0..24).contains(&h) || !(0..60).contains(&m) { return None; }
    let off = crate::i18n::local_offset_secs(now_ms / 1000) as i64 * 1000;
    let day = (now_ms + off).div_euclid(86_400_000) * 86_400_000;
    Some(day + (h * 60 + m) * 60_000 - off)
}

pub fn meeting_rows(src: &Source) -> Vec<(String, Row)> {
    let mut out = vec![];
    let mut seen = HashSet::new();
    for m in src.meetings {
        let id = format!("loci:{}", if m.link.is_empty() { &m.title } else { &m.link });
        if !seen.insert(id.clone()) { continue; }
        let mut r = R::new(Kind::Meeting, &id);
        r.set("title", m.title.clone()).opt("start", hhmm_today(&m.start, src.now_ms)).set("join", format!("join:{id}"))
            .set("recurring", false).set("running", true).set("source", "loci");
        out.push(r.done());
    }
    for e in src.calendar.unwrap_or_default() {
        if e.id.is_empty() || !seen.insert(e.id.clone()) { continue; }
        let space = src.spaces.iter().find(|s| !e.space_url.is_empty() && e.space_url.ends_with(&s.id)).map(|s| s.id.clone());
        let mut r = R::new(Kind::Meeting, &e.id);
        r.set("title", e.title.clone()).opt("start", (e.start_ms > 0).then_some(e.start_ms)).opt("end", (e.end_ms > 0).then_some(e.end_ms))
            .opt("organizer", nonempty(&e.organizer)).opt("location", nonempty(&e.location)).set("join", format!("join:{}", e.id)).opt("space", space)
            .set("recurring", e.recurring).opt("attendee_count", (e.attendees > 0).then_some(e.attendees as u64))
            .set("running", e.start_ms <= src.now_ms && src.now_ms < e.end_ms).set("source", "calendar");
        out.push(r.done());
    }
    out
}

pub fn notification_rows(src: &Source) -> Vec<(String, Row)> {
    src.notifications.iter().map(|n| {
        let mut r = R::new(Kind::Notification, &format!("toast:{}", n.id));
        r.set("kind", n.kind).opt("at", n.at_ms).opt("space", n.space.clone()).set("title", n.title.clone()).opt("body", nonempty(&n.body));
        r.done()
    }).collect()
}

/// Display settings a plugin or theme may read: (key, kind). Tokens, ids, paths and the e-mail are never in it.
pub const SETTINGS: &[(&str, &str)] = &[
    ("theme", "choice"), ("density", "choice"), ("zoom", "number"), ("language", "choice"), ("rail_wide", "bool"), ("compact_view", "bool"),
    ("show_presence", "bool"), ("msg_font_size", "number"), ("gif_autoplay", "bool"), ("gifs_pause_unfocused", "bool"),
    ("show_link_previews", "bool"), ("emoji_tone", "choice"), ("markdown_mode", "bool"), ("composer_rich_toolbar", "bool"),
    ("always_dark_calls", "bool"), ("call_separate_windows", "bool"), ("call_autohide_controls", "bool"), ("emoticons_to_emojis", "bool"),
    ("emoji_autocomplete", "bool"), ("quiet_hours", "bool"),
];

pub fn setting_rows(src: &Source) -> Vec<(String, Row)> {
    SETTINGS.iter().filter_map(|(k, kind)| {
        let v = src.settings.get(*k)?;
        if !(v.is_boolean() || v.is_number() || v.is_string()) { return None; }
        let mut r = R::new(Kind::Setting, k);
        r.set("value", v.clone()).set("kind", *kind);
        Some(r.done())
    }).collect()
}

pub fn reminder_rows(src: &Source) -> Vec<(String, Row)> {
    let mut out = vec![];
    for v in src.settings["message_reminders"].as_array().into_iter().flatten() {
        let (Some(space), Some(msg), Some(due)) = (v["space"].as_str(), v["msg"].as_str(), v["due"].as_i64()) else { continue };
        let mut r = R::new(Kind::Reminder, &format!("reminder:{msg}:{due}"));
        r.set("kind", "reminder").set("space", space).set("message", msg).set("at", due).set("text", v["text"].as_str().unwrap_or_default())
            .opt("author", v["author"].as_str().filter(|a| !a.is_empty()));
        out.push(r.done());
    }
    for v in src.settings["scheduled_messages"].as_array().into_iter().flatten() {
        let (Some(id), Some(space), Some(due)) = (v["id"].as_str(), v["space"].as_str(), v["due"].as_i64()) else { continue };
        let mut r = R::new(Kind::Reminder, &format!("scheduled:{id}"));
        r.set("kind", "scheduled").set("space", space).set("at", due).set("text", v["text"].as_str().unwrap_or_default());
        out.push(r.done());
    }
    out
}

pub fn plugin_rows(src: &Source) -> Vec<(String, Row)> {
    src.plugins.iter().map(|p| {
        let mut r = R::new(Kind::Plugin, &p.key);
        r.set("name", p.name.clone()).set("version", p.version.clone()).set("author", p.author.clone()).set("enabled", p.enabled).set("builtin", p.builtin).opt("error", p.error.clone())
            .opt("settings", p.settings.clone().filter(|s| !s.is_empty()).map(Value::Object));
        r.done()
    }).collect()
}

pub fn me_rows(src: &Source, people: &People) -> Vec<(String, Row)> {
    let s = src.settings;
    let audio = crate::media::audio::Choice::load(s);
    let device = s["device"].as_str().and_then(|d| d.trim_end_matches('/').rsplit('/').next()).filter(|d| !d.is_empty());
    let mut r = R::new(Kind::Me, "me");
    r.set("person", people.list[0].0.clone()).opt("device", device)
        .set("audio", json!({ "input": audio.input, "output": audio.output, "ringtone": audio.ringtone, "volume": audio.volume }))
        .opt("custom_status", s["custom_status"]["text"].as_str().filter(|t| !t.is_empty()).map(|t| {
            let mut v = json!({ "text": t });
            if let Some(u) = s["custom_status"]["until"].as_i64() { v["until"] = json!(u); }
            v
        }))
        .opt("quiet_hours", s["quiet_hours"].as_bool().unwrap_or(false).then(|| json!({ "start": s["quiet_start"].as_str().unwrap_or("18:00"),
            "end": s["quiet_end"].as_str().unwrap_or("08:00"), "days": [s["quiet_day"].as_u64().unwrap_or(0)] })))
        .set("dnd", presence_state(&src.me.status) == "dnd");
    vec![r.done()]
}

pub fn app_rows(src: &Source) -> Vec<(String, Row)> {
    let a = &src.app;
    let mut r = R::new(Kind::App, "app");
    r.set("page", a.page).opt("selected", a.selected.clone()).opt("team", a.team.clone()).set("connected", a.connected).set("signed_in", a.signed_in)
        .set("status", a.status.clone()).set("unread_total", src.spaces.iter().filter(|s| s.unread && !s.hidden).count() as u64).set("call_active", a.call_active);
    vec![r.done()]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::cloud::{Attachment, LinkPreview, Reaction, Reactor};

    /// App state for the tests: me, two contacts, a direct and a group space in a team, messages, a call.
    pub(crate) struct Fixture {
        pub me: Person, pub spaces: Vec<Space>, pub teams: Vec<Team>, pub people: Vec<Person>, pub meetings: Vec<Meeting>,
        pub messages: HashMap<String, Vec<Message>>, pub settings: Value, pub drafts: HashMap<String, String>, pub flags: HashSet<(String, String)>,
        pub last_active: HashMap<String, i64>, pub presence_info: HashMap<String, (Option<i64>, String)>, pub outside: HashMap<String, String>,
        pub marks: HashMap<String, Vec<ReadMark>>, pub directory: Vec<Person>, pub call: Option<CallView>, pub records: Vec<CallRecord>, pub calendar: Vec<CalEvent>,
        pub notifications: Vec<NotifView>, pub app: AppView, pub plugins: Vec<PluginView>,
    }
    pub(crate) fn person(id: &str, name: &str, email: &str, status: &str) -> Person { Person { id: id.into(), name: name.into(), email: email.into(), status: status.into() } }
    pub(crate) fn message(id: &str, email: &str, author: &str, text: &str, ms: i64) -> Message {
        Message { id: id.into(), email: email.into(), author: author.into(), text: text.into(), published_ms: ms, ..Default::default() }
    }
    impl Fixture {
        pub(crate) fn new() -> Fixture {
            let me = person("u-me", "Alex Muster", "alex@x.com", "active");
            let anna = person("u-anna", "Anna Berg", "anna@x.com", "meeting");
            let ben = person("u-ben", "Ben Carter", "ben@x.com", "DoNotDisturb");
            let spaces = vec![
                Space { id: "s-anna".into(), title: "Anna Berg".into(), direct: true, last: "2026-11-24T10:00:00.000Z".into(), unread: true, favorite: true,
                    participants: vec![me.clone(), anna.clone()], last_seen: "2026-11-24T09:00:00.000Z".into(), ..Default::default() },
                Space { id: "s-dev".into(), title: "Development".into(), team: "t1".into(), last: "2026-11-23T10:00:00.000Z".into(),
                    participants: vec![me.clone(), anna.clone(), ben.clone()], meeting: true, locus_url: "https://locus/1".into(), announcement: true, ..Default::default() },
                Space { id: "s-old".into(), title: "Old".into(), hidden: true, ..Default::default() },
            ];
            let mut quoted = message("m3", "ben@x.com", "Ben Carter", "re", 3000);
            quoted.html = "<blockquote class='messageQuote'><spark-mention data-object-type=\"person\" data-object-id=\"u-anna\">Anna Berg</spark-mention><br/>see &amp; you</blockquote><p>re</p>".into();
            quoted.parent = "m2".into();
            let mut m2 = message("m2", "anna@x.com", "Anna Berg", "Hi Alex Muster", 2000);
            m2.reactions = vec![Reaction { name: "like".into(), count: 2, mine: Some("r1".into()), users: vec![
                Reactor { id: "uuid-me".into(), email: "alex@x.com".into(), ..Default::default() }, Reactor { id: "uuid-ben".into(), email: "ben@x.com".into(), ..Default::default() }] }];
            m2.images = vec![Attachment { name: "a.png".into(), mime: "image/png".into(), size: 10, key: "k1".into(), width: 4, height: 3, giphy: None }];
            m2.files = vec![Attachment { name: "b.pdf".into(), mime: "application/pdf".into(), size: 20, ..Default::default() }];
            m2.previews = vec![LinkPreview { url: "https://e.com".into(), title: "E".into(), description: "d".into(), site: "e.com".into(), image_key: "pk".into() }];
            m2.edited = true;
            let mine = Message { mine: true, ..message("m1", "alex@x.com", "Alex Muster", "hello", 1000) };
            let notice = Message { id: "m4".into(), notice: crate::trq!("{{0}} deleted their own message.", "Ben Carter"), ..Default::default() };
            let stranger = message("m5", "carl@y.com", "Carl", "hey", 4000);
            let mut messages = HashMap::new();
            messages.insert("s-dev".to_string(), vec![mine, m2, quoted, notice, stranger]);
            messages.insert("s-anna".to_string(), vec![message("d1", "anna@x.com", "Anna Berg", "ping Alex Muster", crate::i18n::parse_iso_ms("2026-11-24T09:30:00.000Z").unwrap())]);
            Fixture {
                me: me.clone(), spaces, teams: vec![Team { id: "t1".into(), name: "Product Team".into(), color: Some((0x2E, 0x8B, 0xE6)) }],
                people: vec![anna.clone(), ben.clone()], meetings: vec![Meeting { title: "Sync".into(), start: "16:00".into(), link: "https://m/1".into() }],
                messages,
                settings: json!({ "theme": "dark", "token": "secret", "zoom": 1.25, "custom_status": { "text": "Lunch", "until": 99 }, "quiet_hours": true,
                    "quiet_start": "20:00", "quiet_end": "07:00", "quiet_day": 2, "device": "https://wdm/devices/dev-1",
                    "message_reminders": [{ "space": "s-dev", "msg": "m2", "author": "Anna Berg", "text": "Hi", "due": 5000 }],
                    "scheduled_messages": [{ "id": "q1", "space": "s-anna", "text": "later", "markdown": false, "due": 6000, "mentions": [] }] }),
                drafts: [("s-anna".to_string(), "draft!".to_string())].into(), flags: [("s-dev".to_string(), "m2".to_string())].into(),
                last_active: [("u-anna".to_string(), 1234)].into(), presence_info: [("u-anna".to_string(), (Some(9999), "In Zurich".to_string()))].into(),
                outside: [("u-dir".to_string(), "active".to_string())].into(),
                marks: [("s-dev".to_string(), vec![ReadMark { person: "u-anna".into(), email: "anna@x.com".into(), activity: "m3".into(), time: String::new() }])].into(),
                directory: vec![person("u-dir", "Dora Dir", "dora@x.com", "")],
                call: Some(CallView { id: "call:1".into(), state: "connected", name: "Anna Berg".into(), email: "anna@x.com".into(), started_ms: Some(500), muted: true,
                    roster: Roster { members: vec![crate::cloud::locus::controls::Member { id: "p1".into(), name: "Anna Berg".into(), email: "anna@x.com".into(), state: "JOINED".into(), hand: true, ..Default::default() },
                        crate::cloud::locus::controls::Member { id: "p2".into(), name: "Alex Muster".into(), email: "alex@x.com".into(), state: "JOINED".into(), me: true, ..Default::default() }],
                        me_host: false, group: false }, ..Default::default() }),
                records: vec![CallRecord { session_id: "c1".into(), start_ms: 10, end_ms: 70, secs: 60, outgoing: false, disposition: "MISSED".into(), name: "Ben Carter".into(), email: "ben@x.com".into(), ..Default::default() }],
                calendar: vec![CalEvent { id: "e1".into(), title: "Planning".into(), start_ms: 1, end_ms: i64::MAX / 2, organizer: "Anna".into(), space_url: "https://conv/s-dev".into(), attendees: 5, ..Default::default() }],
                notifications: vec![NotifView { id: 7, kind: "mention", at_ms: Some(42), space: Some("s-dev".into()), title: "Anna Berg".into(), body: "Hi".into() }],
                app: AppView { page: "messaging", selected: Some("s-dev".into()), team: None, connected: true, signed_in: true, status: "Connected".into(), call_active: true },
                plugins: vec![PluginView { key: "logger".into(), name: "Logger".into(), version: "1.0".into(), enabled: true, ..Default::default() }],
            }
        }
        pub(crate) fn source(&self) -> Source<'_> {
            Source {
                me: &self.me, spaces: &self.spaces, teams: &self.teams, people: &self.people, meetings: &self.meetings, messages: &self.messages,
                settings: &self.settings, drafts: &self.drafts, flags: &self.flags, last_active: &self.last_active, presence_info: &self.presence_info,
                outside_status: &self.outside, read_marks: &self.marks, directory: &self.directory,
                has_photo: &|k| k == "u-anna", has_image: &|k| k == "k1" || k == "spaceavatar:s-dev" || k == "cover:anna@x.com",
                space_ext: &|id| if id == "s-dev" { SpaceExt { notify: "mentions", locked: true, moderators: vec!["u-anna".into()], people: None } } else { SpaceExt { notify: "global", ..Default::default() } },
                team_ext: &|id| (id == "t1").then(|| TeamExt { general: "s-dev".into(), archived: false }),
                typers: &|id| if id == "s-dev" { vec![person("u-ben", "Ben Carter", "ben@x.com", "")] } else { vec![] },
                call: self.call.clone(), call_records: Some(&self.records), calendar: Some(&self.calendar), notifications: self.notifications.clone(),
                app: self.app.clone(), plugins: self.plugins.clone(), my_cover: false, now_ms: 1_000_000,
            }
        }
    }

    /// The row of `id` as a field-name map.
    pub(crate) fn obj(kind: Kind, rows: &[(String, Row)], id: &str) -> serde_json::Map<String, Value> {
        let (_, r) = rows.iter().find(|r| r.0 == id).unwrap_or_else(|| panic!("{} {id} missing", kind.name()));
        kind.def().fields.iter().zip(r).filter(|(_, v)| !v.is_null()).map(|(f, v)| (f.name.to_string(), v.clone())).collect()
    }
    fn o(v: Value) -> serde_json::Map<String, Value> { v.as_object().unwrap().clone() }

    #[test]
    fn me_and_app() {
        let f = Fixture::new();
        let s = f.source();
        let p = People::of(&s);
        assert_eq!(obj(Kind::Me, &me_rows(&s, &p), "me"), o(json!({ "id": "me", "person": "u-me", "device": "dev-1",
            "audio": { "input": "", "output": "", "ringtone": "", "volume": crate::media::audio::Choice::load(&f.settings).volume },
            "custom_status": { "text": "Lunch", "until": 99 }, "quiet_hours": { "start": "20:00", "end": "07:00", "days": [2] }, "dnd": false })));
        assert_eq!(obj(Kind::App, &app_rows(&s), "app"), o(json!({ "id": "app", "page": "messaging", "selected": "s-dev", "connected": true, "signed_in": true,
            "status": "Connected", "unread_total": 1, "call_active": true })));
    }

    #[test]
    fn people_and_presence() {
        let f = Fixture::new();
        let s = f.source();
        let p = People::of(&s);
        let rows = person_rows(&s, &p);
        assert_eq!(obj(Kind::Person, &rows, "u-anna"), o(json!({ "id": "u-anna", "name": "Anna Berg", "initials": "AB", "avatar": "img:person:u-anna",
            "cover": "img:cover:u-anna", "email": "anna@x.com", "presence": "u-anna", "direct": "s-anna", "favorite": true, "contact": true, "directory": false, "me": false })));
        assert_eq!(obj(Kind::Person, &rows, "u-me")["me"], json!(true));
        assert_eq!(obj(Kind::Person, &rows, "u-dir")["directory"], json!(true));
        // a message author no list knows: kept by e-mail until the id is known
        assert_eq!(obj(Kind::Person, &rows, "email:carl@y.com")["contact"], json!(false));
        let pr = presence_rows(&s, &p);
        assert_eq!(obj(Kind::Presence, &pr, "u-anna"), o(json!({ "id": "u-anna", "person": "u-anna", "state": "meeting", "online": true, "in_call": true,
            "last_active": 1234, "until": 9999, "text": "In Zurich" })));
        assert_eq!(obj(Kind::Presence, &pr, "u-ben")["state"], json!("dnd"));
        assert_eq!(obj(Kind::Presence, &pr, "u-dir")["state"], json!("active"), "directory people's presence is followed");
        assert_eq!(obj(Kind::Presence, &pr, "email:carl@y.com")["state"], json!("unknown"));
    }

    #[test]
    fn spaces_and_teams() {
        let f = Fixture::new();
        let s = f.source();
        let p = People::of(&s);
        let rows = space_rows(&s, &p);
        assert_eq!(obj(Kind::Space, &rows, "s-anna"), o(json!({ "id": "s-anna", "title": "Anna Berg", "kind": "direct", "peer": "u-anna", "avatar": "img:person:u-anna",
            "favorite": true, "hidden": false, "public": false, "unread": true, "mentioned": true, "last_activity": crate::i18n::parse_iso_ms("2026-11-24T10:00:00.000Z"), "last_seen": crate::i18n::parse_iso_ms("2026-11-24T09:00:00.000Z"),
            "notify": "global", "muted": false, "writable": true, "locked": false, "moderators": [], "members": ["u-me", "u-anna"], "typing": [],
            "draft": "draft!", "last_message": "d1" })));
        let dev = obj(Kind::Space, &rows, "s-dev");
        assert_eq!((dev["kind"].clone(), dev["team"].clone(), dev["avatar"].clone(), dev["writable"].clone(), dev["read_only_reason"].clone()),
            (json!("group"), json!("t1"), json!("img:space:s-dev"), json!(false), json!("announcement")));
        assert_eq!((dev["notify"].clone(), dev["locked"].clone(), dev["moderators"].clone(), dev["typing"].clone()), (json!("mentions"), json!(true), json!(["u-anna"]), json!(["u-ben"])));
        assert_eq!(dev["meeting"], json!({ "active": true, "locus": "https://locus/1" }));
        assert_eq!(dev["last_message"], json!("m5"), "system lines are not the last message");
        assert_eq!(obj(Kind::Space, &rows, "s-old")["hidden"], json!(true));
        let t = team_rows(&s);
        assert_eq!(obj(Kind::Team, &t, "t1"), o(json!({ "id": "t1", "name": "Product Team", "color": "#2E8BE6", "initials": "PT", "general": "s-dev",
            "archived": false, "spaces": ["s-dev"], "unread": false, "mentioned": false })));
    }

    #[test]
    fn messages() {
        let f = Fixture::new();
        let s = f.source();
        let p = People::of(&s);
        let rows = message_rows(&s, &p, "s-dev");
        assert_eq!(rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), ["m1", "m2", "m3", "m4", "m5"]);
        assert_eq!(obj(Kind::Message, &rows, "m2"), o(json!({ "id": "m2", "space": "s-dev", "author": "u-anna", "author_name": "Anna Berg", "mine": false, "at": 2000,
            "kind": "message", "text": "Hi Alex Muster", "replies": 1, "edited": true, "kept": false,
            "reactions": [{ "name": "like", "count": 2, "mine": true, "people": ["u-me", "u-ben"] }],
            "files": [{ "name": "a.png", "mime": "image/png", "size": 10, "kind": "image", "thumb": "img:file:k1", "width": 4, "height": 3 },
                      { "name": "b.pdf", "mime": "application/pdf", "size": 20, "kind": "file" }],
            "previews": [{ "url": "https://e.com", "title": "E", "description": "d", "site": "e.com" }],
            "mentions_me": true, "flagged": true, "seen_by": [] })));
        let m3 = obj(Kind::Message, &rows, "m3");
        assert_eq!((m3["parent"].clone(), m3["quote"].clone(), m3["seen_by"].clone(), m3["mentions_me"].clone()),
            (json!("m2"), json!({ "author": "Anna Berg", "text": "see & you" }), json!(["u-anna"]), json!(false)));
        assert_eq!(obj(Kind::Message, &rows, "m1")["author"], json!("u-me"));
        let m4 = obj(Kind::Message, &rows, "m4");
        assert_eq!((m4["kind"].clone(), m4.get("author")), (json!("deleted"), None), "a tombstone is a deleted message without author");
        assert_eq!(obj(Kind::Message, &rows, "m5")["author"], json!("email:carl@y.com"));
        assert!(message_rows(&s, &p, "nope").is_empty());
        assert!(!is_tombstone("Anna joined the space."));
    }

    #[test]
    fn calls_meetings_and_the_rest() {
        let f = Fixture::new();
        let s = f.source();
        let p = People::of(&s);
        let c = obj(Kind::Call, &call_rows(&s, &p), "call:1");
        assert_eq!((c["kind"].clone(), c["state"].clone(), c["peer"].clone(), c["space"].clone(), c["started_at"].clone()),
            (json!("direct"), json!("connected"), json!("u-anna"), json!("s-anna"), json!(500)));
        assert_eq!(c["participants"][0], json!({ "id": "p1", "person": "u-anna", "name": "Anna Berg", "state": "joined", "muted": false, "hand": true, "host": false, "me": false }));
        assert_eq!(c["me"]["muted"], json!(true));
        assert_eq!(obj(Kind::CallRecord, &call_record_rows(&s, &p), "c1"), o(json!({ "id": "c1", "at": 10, "ended_at": 70, "secs": 60, "outgoing": false,
            "missed": true, "peer": "u-ben", "name": "Ben Carter" })));
        let m = meeting_rows(&s);
        assert_eq!(obj(Kind::Meeting, &m, "e1"), o(json!({ "id": "e1", "title": "Planning", "start": 1, "end": i64::MAX / 2, "organizer": "Anna", "join": "join:e1",
            "space": "s-dev", "recurring": false, "attendee_count": 5, "running": true, "source": "calendar" })));
        assert_eq!(obj(Kind::Meeting, &m, "loci:https://m/1")["running"], json!(true));
        assert!(obj(Kind::Meeting, &m, "loci:https://m/1")["start"].is_i64());
        assert_eq!(obj(Kind::Notification, &notification_rows(&s), "toast:7"), o(json!({ "id": "toast:7", "kind": "mention", "at": 42, "space": "s-dev", "title": "Anna Berg", "body": "Hi" })));
        let st = setting_rows(&s);
        assert_eq!(st.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), ["theme", "zoom", "quiet_hours"], "the whitelist only: no token, no device");
        assert_eq!(obj(Kind::Setting, &st, "zoom"), o(json!({ "id": "zoom", "value": 1.25, "kind": "number" })));
        let rm = reminder_rows(&s);
        assert_eq!(obj(Kind::Reminder, &rm, "reminder:m2:5000"), o(json!({ "id": "reminder:m2:5000", "kind": "reminder", "space": "s-dev", "message": "m2", "at": 5000, "text": "Hi", "author": "Anna Berg" })));
        assert_eq!(obj(Kind::Reminder, &rm, "scheduled:q1")["kind"], json!("scheduled"));
        assert_eq!(obj(Kind::Plugin, &plugin_rows(&s), "logger"), o(json!({ "id": "logger", "name": "Logger", "version": "1.0", "author": "", "enabled": true, "builtin": false })));
    }

    #[test]
    fn helpers() {
        assert_eq!(initials("anna maria berg"), "AB");
        assert_eq!(initials("  "), "");
        assert_eq!(presence_state("DoNotDisturb"), "dnd");
        assert_eq!(presence_state("OutOfOffice"), "ooo");
        assert_eq!(presence_state(""), "unknown");
        assert_eq!(quote_of("<p>x</p>"), None);
        assert!(hhmm_today("25:00", 0).is_none());
    }
}
