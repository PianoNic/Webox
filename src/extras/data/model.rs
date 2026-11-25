//! The data model `webox-data/1` (docs/data-api.md section 2): every entity kind, its fields in a fixed order, their types,
//! relations and privacy scopes. This table is the one definition of the model: the store keeps an entity as a row of values
//! in this field order, the query compiler resolves names against it, and `data.model` / `docs/data-model.json` are generated
//! from it.
//!
//! Rules (additive only within `webox-data/1`): fields, kinds and enum values are only added, never removed, renamed or
//! retyped. `reserved` fields are part of the model but always absent until Webox can fill them.
use serde_json::{json, Value};

/// The model version a plugin or theme asks for.
pub const VERSION: &str = "webox-data/1";

/// An entity kind. The discriminant is the index into [`ENTITIES`] and the bit in change masks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind { Me, App, Person, Presence, Space, Team, Message, Call, CallRecord, Meeting, Notification, Setting, Reminder, Plugin }

impl Kind {
    pub const ALL: [Kind; 14] = [Kind::Me, Kind::App, Kind::Person, Kind::Presence, Kind::Space, Kind::Team, Kind::Message, Kind::Call,
        Kind::CallRecord, Kind::Meeting, Kind::Notification, Kind::Setting, Kind::Reminder, Kind::Plugin];
    pub fn idx(self) -> usize { self as usize }
    pub fn bit(self) -> u32 { 1 << self as u32 }
    pub fn def(self) -> &'static Entity { &ENTITIES[self as usize] }
    pub fn name(self) -> &'static str { self.def().name }
    pub fn by_name(n: &str) -> Option<Kind> { Kind::ALL.into_iter().find(|k| k.name() == n) }
    /// The index of field `name`, if this kind has it.
    pub fn field(self, name: &str) -> Option<usize> { self.def().fields.iter().position(|f| f.name == name) }
    /// The index of a field the code knows exists (a typo is a bug, caught by the model tests).
    pub fn f(self, name: &str) -> usize { self.field(name).unwrap_or_else(|| panic!("{} has no field {name}", self.name())) }
}

/// One field of an entity.
pub struct Field {
    pub name: &'static str,
    /// The type as the model shows it: `string`, `bool`, `ms`, `u32`, `u64`, `f32`, `"a"|"b"` (an open enum), `ImageRef`,
    /// `object`, `[T]`, `-> kind` (a relation: the target's id), `[-> kind]`.
    pub ty: &'static str,
    /// The kind a relation points to.
    pub rel: Option<Kind>,
    /// The privacy scope (docs/data-api.md section 6.1); empty = the entity's scope.
    pub scope: &'static str,
    /// May be absent (`null` in plugin JSON).
    pub opt: bool,
    /// In the model but always absent for now (data Webex or Webox does not have yet).
    pub reserved: bool,
    /// Read when asked (device lists, feature toggles, call statistics): never stored, never in `data.changed` diffs.
    pub volatile: bool,
    /// Keys of an `object` field, for path checks (`meeting.active`); empty = any key.
    pub keys: &'static [&'static str],
    pub doc: &'static str,
}

const fn f(name: &'static str, ty: &'static str, doc: &'static str) -> Field {
    Field { name, ty, rel: None, scope: "", opt: false, reserved: false, volatile: false, keys: &[], doc }
}
const fn o(name: &'static str, ty: &'static str, doc: &'static str) -> Field { Field { opt: true, ..f(name, ty, doc) } }
const fn r(name: &'static str, ty: &'static str, to: Kind, doc: &'static str) -> Field { Field { rel: Some(to), ..f(name, ty, doc) } }
const fn ro(name: &'static str, ty: &'static str, to: Kind, doc: &'static str) -> Field { Field { rel: Some(to), opt: true, ..f(name, ty, doc) } }
const fn reserved(name: &'static str, ty: &'static str, doc: &'static str) -> Field { Field { reserved: true, opt: true, ..f(name, ty, doc) } }
const fn sc(mut x: Field, scope: &'static str) -> Field { x.scope = scope; x }
const fn keys(mut x: Field, k: &'static [&'static str]) -> Field { x.keys = k; x }
const fn volatile(mut x: Field) -> Field { x.volatile = true; x.opt = true; x }

/// One entity kind of the model.
pub struct Entity {
    pub kind: Kind,
    pub name: &'static str,
    /// The default scope of its fields.
    pub scope: &'static str,
    pub fields: &'static [Field],
    pub doc: &'static str,
}

const PRESENCE_STATES: &str = r#""active"|"inactive"|"away"|"dnd"|"busy"|"quiet"|"ooo"|"call"|"meeting"|"presenting"|"calendar"|"unknown""#;

pub static ENTITIES: [Entity; 14] = [
    Entity { kind: Kind::Me, name: "me", scope: "me.basic", doc: "The signed-in user (one item, id \"me\").", fields: &[
        f("id", "string", "Always \"me\"."),
        r("person", "-> person", Kind::Person, "My own person entry (name, avatar, presence)."),
        sc(o("device", "string", "This device's registration id."), "me.devices"),
        sc(keys(f("audio", "object", "The chosen devices: {input, output, ringtone, volume}; inputs / outputs are read when asked."),
            &["input", "output", "ringtone", "volume", "inputs", "outputs"]), "me.devices"),
        sc(volatile(f("cameras", "[Device]", "Cameras, read when asked (never on the UI thread).")), "me.devices"),
        keys(o("custom_status", "object", "{text, until?}: local only, not sent to Webex."), &["text", "until"]),
        keys(o("quiet_hours", "object", "{start: \"HH:MM\", end: \"HH:MM\", days: [u8]} while quiet hours are on (0 = Monday)."), &["start", "end", "days"]),
        f("dnd", "bool", "My presence is Do not disturb."),
        sc(volatile(f("features", "object", "Feature toggles and client policies {key: bool | number | string}, read when asked.")), "me.features"),
    ] },
    Entity { kind: Kind::App, name: "app", scope: "app", doc: "The window's state (one item, id \"app\").", fields: &[
        f("id", "string", "Always \"app\"."),
        f("page", r#""messaging"|"meetings"|"calling"|"whiteboards"|"contacts"|"teams"|"voicemail"|"settings""#, "The page shown."),
        ro("selected", "-> space", Kind::Space, "The open space."),
        ro("team", "-> team", Kind::Team, "The team whose spaces are shown (Teams page)."),
        f("connected", "bool", "Signed in with a running connection."),
        f("signed_in", "bool", "A sign-in exists (false on the sign-in screen)."),
        f("status", "string", "The connection status line."),
        f("unread_total", "u32", "Unread spaces (hidden ones left out)."),
        f("call_active", "bool", "A call is shown (ringing, connecting or connected)."),
        reserved("width_class", r#""narrow"|"medium"|"wide""#, "Reserved for the theming engine."),
        reserved("focus_mode", "bool", "Reserved for the theming engine."),
    ] },
    Entity { kind: Kind::Person, name: "person", scope: "people.basic", doc: "Someone Webox knows: people of my spaces, directory results, call participants, me.", fields: &[
        f("id", "string", "Webex person id; \"email:<address>\" while only the e-mail is known (then a rekey follows)."),
        f("name", "string", "Display name."),
        f("initials", "string", "Up to two letters."),
        o("avatar", "ImageRef", "Profile picture handle, when loaded."),
        o("cover", "ImageRef", "Cover picture handle, when loaded."),
        sc(o("email", "string", "E-mail address."), "people.email"),
        reserved("title", "string", "Job title (not fetched yet)."),
        reserved("department", "string", "Department (not fetched yet)."),
        reserved("phone", "string", "Phone (not fetched yet)."),
        r("presence", "-> presence", Kind::Presence, "Presence (same id)."),
        ro("direct", "-> space", Kind::Space, "My 1:1 space with this person."),
        f("favorite", "bool", "The 1:1 space is a favourite."),
        reserved("external", "bool", "Outside my organisation (not known yet)."),
        f("contact", "bool", "In the people of my spaces (the Contacts page)."),
        f("directory", "bool", "Found through a directory search, in none of my spaces."),
        f("me", "bool", "This is me."),
    ] },
    Entity { kind: Kind::Presence, name: "presence", scope: "people.presence", doc: "A person's presence (id = the person's id).", fields: &[
        f("id", "string", "The person's id."),
        r("person", "-> person", Kind::Person, "Whose presence."),
        f("state", PRESENCE_STATES, "The state (open enum)."),
        f("online", "bool", "state is active, busy, call, meeting, presenting or calendar."),
        f("in_call", "bool", "state is call, meeting or presenting."),
        o("last_active", "ms", "Last active."),
        o("until", "ms", "End of DND / meeting / out of office."),
        o("text", "string", "Custom status or out-of-office text."),
    ] },
    Entity { kind: Kind::Space, name: "space", scope: "spaces.list", doc: "A space (conversation) of mine.", fields: &[
        f("id", "string", "Webex conversation id."),
        f("title", "string", "Title (the other person's name for 1:1 spaces)."),
        f("kind", r#""direct"|"group""#, "1:1 or group."),
        ro("team", "-> team", Kind::Team, "Its team."),
        ro("peer", "-> person", Kind::Person, "Direct spaces: the other person."),
        o("avatar", "ImageRef", "Space picture; for direct spaces the peer's avatar."),
        o("cover", "ImageRef", "Space cover picture."),
        f("favorite", "bool", "Favourite."),
        f("hidden", "bool", "Hidden from the list."),
        f("public", "bool", "Public space."),
        reserved("archived", "bool", "Archived (not known for spaces yet)."),
        f("unread", "bool", "Has unread activity."),
        o("mentioned", "bool", "A loaded message I have not seen mentions me."),
        reserved("unread_count", "u32", "Unread count (Webex offers none)."),
        o("last_activity", "ms", "Last activity."),
        o("last_seen", "ms", "Last activity I have seen."),
        f("notify", r#""global"|"all"|"mentions"|"off""#, "Notification setting."),
        f("muted", "bool", "notify is off."),
        f("writable", "bool", "I can post (not closed, not announcement-blocked)."),
        o("read_only_reason", r#""closed"|"announcement""#, "Why I cannot post."),
        f("locked", "bool", "Moderated (locked) space."),
        r("moderators", "[-> person]", Kind::Person, "Moderators."),
        r("members", "[-> person]", Kind::Person, "Known members (the first 10 until the full list was loaded)."),
        o("member_count", "u32", "Exact member count once the full list was loaded."),
        keys(o("meeting", "object", "{active: bool, locus: string} once the space ever had a meeting."), &["active", "locus"]),
        r("typing", "[-> person]", Kind::Person, "Who is typing."),
        sc(o("draft", "string", "My unsent draft."), "messages.content"),
        Field { scope: "messages.meta", ..ro("last_message", "-> message", Kind::Message, "The newest loaded message.") },
    ] },
    Entity { kind: Kind::Team, name: "team", scope: "spaces.list", doc: "A team of mine.", fields: &[
        f("id", "string", "Webex team id."),
        f("name", "string", "Name."),
        o("color", "\"#RRGGBB\"", "Team colour."),
        f("initials", "string", "Up to two letters."),
        ro("general", "-> space", Kind::Space, "The team's general space."),
        f("archived", "bool", "Archived."),
        r("spaces", "[-> space]", Kind::Space, "My spaces of the team."),
        f("unread", "bool", "One of its spaces is unread."),
        o("mentioned", "bool", "One of its spaces mentions me."),
    ] },
    Entity { kind: Kind::Message, name: "message", scope: "messages.content", doc: "A loaded message (also system lines and deleted ones a plugin keeps).", fields: &[
        sc(f("id", "string", "Webex activity id."), "messages.meta"),
        sc(r("space", "-> space", Kind::Space, "Its space."), "messages.meta"),
        sc(ro("author", "-> person", Kind::Person, "Who wrote it (absent for system lines)."), "messages.meta"),
        sc(f("author_name", "string", "The author's name as the message shows it."), "messages.meta"),
        sc(f("mine", "bool", "I wrote it."), "messages.meta"),
        sc(o("at", "ms", "Sent at."), "messages.meta"),
        sc(f("kind", r#""message"|"system"|"deleted"|"join_leave""#, "What it is."), "messages.meta"),
        f("text", "string", "Plain text."),
        o("html", "string", "Rich text as sent (sanitised by Webox's renderer)."),
        ro("parent", "-> message", Kind::Message, "The thread root it replies to."),
        o("replies", "u32", "Loaded replies (thread roots)."),
        f("edited", "bool", "Edited."),
        f("kept", "bool", "Deleted, kept on screen by a plugin."),
        o("history", "[[string]]", "Earlier texts a plugin attached, oldest first."),
        f("reactions", "[Reaction]", "Reactions: {name, count, mine, people: [-> person]}."),
        sc(f("files", "[File]", "Files and images: {name, mime, size, kind, thumb?, width?, height?}."), "files"),
        f("previews", "[Preview]", "Link previews: {url, title, description, site, image?}."),
        f("mentions_me", "bool", "It mentions me."),
        f("flagged", "bool", "I flagged it."),
        keys(o("quote", "object", "{author, text} of a quoted message."), &["author", "text"]),
        sc(r("seen_by", "[-> person]", Kind::Person, "People whose last read message this is."), "messages.receipts"),
    ] },
    Entity { kind: Kind::Call, name: "call", scope: "calls.state", doc: "The current call (0 or 1 today).", fields: &[
        f("id", "string", "Opaque call id."),
        f("kind", r#""direct"|"space"|"meeting""#, "1:1 or group call."),
        f("state", r#""dialing"|"ringing"|"incoming"|"connecting"|"connected"|"ended""#, "State."),
        f("incoming", "bool", "Someone called me."),
        ro("peer", "-> person", Kind::Person, "1:1 calls: the other person."),
        f("peer_name", "string", "The name the call shows."),
        ro("space", "-> space", Kind::Space, "The space of the call."),
        o("started_at", "ms", "When it connected."),
        f("participants", "[Participant]", "{id, person?, name, state, muted, hand, host, me}."),
        keys(f("me", "object", "My side: {muted, muted_by_host, unmute_allowed, asked_to_unmute, video, video_sending, camera?, video_width?, video_height?, video_fps?, sharing, hand, host}."),
            &["muted", "muted_by_host", "unmute_allowed", "asked_to_unmute", "video", "video_sending", "camera", "video_width", "video_height", "video_fps", "sharing", "hand", "host"]),
        f("remote_video", "bool", "The other side's camera is on."),
        ro("remote_share", "-> person", Kind::Person, "Who shares a screen."),
        o("remote_share_name", "string", "The sharer's name."),
        sc(volatile(f("stats", "object", "Media statistics while the statistics view runs, read when asked.")), "calls.stats"),
    ] },
    Entity { kind: Kind::CallRecord, name: "call_record", scope: "calls.history", doc: "Call history (loaded with the Calling page).", fields: &[
        f("id", "string", "Session id."),
        f("at", "ms", "Start."),
        f("ended_at", "ms", "End."),
        f("secs", "u32", "Duration."),
        f("outgoing", "bool", "I called."),
        f("missed", "bool", "Incoming and not answered."),
        ro("peer", "-> person", Kind::Person, "The other person, when known."),
        f("name", "string", "The other side's name or number."),
        ro("space", "-> space", Kind::Space, "My 1:1 space with them."),
    ] },
    Entity { kind: Kind::Meeting, name: "meeting", scope: "meetings.list", doc: "Running and starting-soon meetings, and calendar events.", fields: &[
        f("id", "string", "Calendar id, or the join link for running meetings."),
        f("title", "string", "Title."),
        o("start", "ms", "Start."),
        o("end", "ms", "End."),
        o("organizer", "string", "Organiser."),
        o("location", "string", "Location."),
        f("join", "JoinRef", "Opaque; only the meeting.join action accepts it."),
        ro("space", "-> space", Kind::Space, "The space it belongs to."),
        f("recurring", "bool", "Recurring."),
        o("attendee_count", "u32", "Invitees."),
        f("running", "bool", "Running now."),
        f("source", r#""loci"|"calendar""#, "Where Webox knows it from."),
    ] },
    Entity { kind: Kind::Notification, name: "notification", scope: "notifications", doc: "Toasts on screen.", fields: &[
        f("id", "string", "Opaque id."),
        f("kind", r#""message"|"mention"|"call"|"missed_call"|"meeting"|"reminder"|"info""#, "What it is about."),
        o("at", "ms", "When it appeared."),
        ro("space", "-> space", Kind::Space, "Its space."),
        ro("person", "-> person", Kind::Person, "Its person."),
        f("title", "string", "Title."),
        sc(o("body", "string", "Text."), "messages.content"),
    ] },
    Entity { kind: Kind::Setting, name: "setting", scope: "settings.read", doc: "Display settings (a whitelist; no tokens, ids, paths or e-mail).", fields: &[
        f("id", "string", "The key."),
        f("value", "bool | number | string", "Its value."),
        f("kind", r#""bool"|"number"|"choice"|"text""#, "Its type."),
    ] },
    Entity { kind: Kind::Reminder, name: "reminder", scope: "messages.content", doc: "Message reminders and scheduled messages (local).", fields: &[
        f("id", "string", "Opaque id."),
        f("kind", r#""reminder"|"scheduled""#, "A reminder of a message, or a message to send later."),
        r("space", "-> space", Kind::Space, "The space."),
        ro("message", "-> message", Kind::Message, "The message to remember."),
        f("at", "ms", "When it is due."),
        f("text", "string", "The text."),
        o("author", "string", "The remembered message's author."),
    ] },
    Entity { kind: Kind::Plugin, name: "plugin", scope: "app", doc: "Installed plugins.", fields: &[
        f("id", "string", "The plugin's key."),
        f("name", "string", "Name."),
        f("version", "string", "Version."),
        f("author", "string", "Author."),
        f("enabled", "bool", "Switched on."),
        f("builtin", "bool", "Compiled into Webox."),
        o("error", "string", "Why it does not load."),
        keys(o("settings", "object", "Its setting values (passwords left out)."), &[]),
    ] },
];

/// A collection a query starts from: its entity kind and whether it needs an argument.
pub struct Collection { pub name: &'static str, pub kind: Kind, pub arg: Option<&'static str>, pub doc: &'static str }

pub static COLLECTIONS: &[Collection] = &[
    Collection { name: "people", kind: Kind::Person, arg: None, doc: "Everyone the store knows." },
    Collection { name: "contacts", kind: Kind::Person, arg: None, doc: "The people of my spaces (the Contacts page)." },
    Collection { name: "spaces", kind: Kind::Space, arg: None, doc: "My spaces; hidden ones only when the query names `hidden`." },
    Collection { name: "teams", kind: Kind::Team, arg: None, doc: "My teams." },
    Collection { name: "messages", kind: Kind::Message, arg: Some("space id"), doc: "Loaded messages of one space, oldest first." },
    Collection { name: "thread", kind: Kind::Message, arg: Some("message id"), doc: "A thread root and its loaded replies." },
    Collection { name: "calls", kind: Kind::Call, arg: None, doc: "The current call (0 or 1)." },
    Collection { name: "call_history", kind: Kind::CallRecord, arg: None, doc: "Call history, newest first." },
    Collection { name: "meetings", kind: Kind::Meeting, arg: None, doc: "Running meetings and calendar events." },
    Collection { name: "notifications", kind: Kind::Notification, arg: None, doc: "Toasts on screen." },
    Collection { name: "settings", kind: Kind::Setting, arg: None, doc: "The display-settings whitelist." },
    Collection { name: "reminders", kind: Kind::Reminder, arg: None, doc: "Message reminders and scheduled messages." },
    Collection { name: "plugins", kind: Kind::Plugin, arg: None, doc: "Installed plugins." },
    Collection { name: "presence", kind: Kind::Presence, arg: None, doc: "Presence of everyone the store knows." },
    Collection { name: "me", kind: Kind::Me, arg: None, doc: "One item: me." },
    Collection { name: "app", kind: Kind::App, arg: None, doc: "One item: the window's state." },
];

pub fn collection(name: &str) -> Option<&'static Collection> { COLLECTIONS.iter().find(|c| c.name == name) }

/// Embedded object types (not entities: no id of their own, part of a field's value).
pub static TYPES: &[(&str, &str)] = &[
    ("Device", "{id, name, kind: \"audio_in\"|\"audio_out\"|\"camera\", default: bool}"),
    ("Reaction", "{name, count: u32, mine: bool, people: [-> person]}"),
    ("File", "{name, mime, size: u64, kind: \"image\"|\"video\"|\"gif\"|\"file\", thumb?: ImageRef, width?: u32, height?: u32}"),
    ("Preview", "{url, title, description, site, image?: ImageRef}"),
    ("Participant", "{id, person?: -> person, name, state: \"joined\"|\"ringing\"|\"left\", muted, hand, host, me}"),
    ("ImageRef", "An opaque image handle (\"img:...\"); never a URL or a token."),
    ("JoinRef", "An opaque meeting handle (\"join:...\"); only the meeting.join action accepts it."),
    ("ms", "Unix time in milliseconds (JSON number)."),
];

/// Functions the query language offers.
pub static FUNCTIONS: &[(&str, &str)] = &[
    ("len(list)", "Number of items of a list (or characters of a string)."),
    ("count(path)", "Same as len."),
    ("any(list, expr)", "Some item of the list matches expr (paths in expr are relative to the item)."),
    ("all(list, expr)", "Every item matches."),
    ("now()", "Current time in ms."),
    ("minutes(n)", "n minutes in ms."),
    ("hours(n)", "n hours in ms."),
    ("days(n)", "n days in ms."),
    ("lower(s)", "Lower case."),
];

/// The model as `data.model` answers it and as `docs/data-model.json` stores it.
pub fn describe() -> Value {
    let entities: serde_json::Map<String, Value> = ENTITIES.iter().map(|e| {
        let fields: Vec<Value> = e.fields.iter().map(|x| {
            let mut v = json!({ "name": x.name, "type": x.ty, "scope": if x.scope.is_empty() { e.scope } else { x.scope }, "doc": x.doc });
            if x.opt { v["optional"] = json!(true); }
            if x.reserved { v["reserved"] = json!(true); }
            if x.volatile { v["volatile"] = json!(true); }
            if let Some(k) = x.rel { v["relation"] = json!(k.name()); }
            if !x.keys.is_empty() { v["keys"] = json!(x.keys); }
            v
        }).collect();
        (e.name.to_string(), json!({ "scope": e.scope, "doc": e.doc, "fields": fields }))
    }).collect();
    json!({
        "version": VERSION,
        "entities": entities,
        "collections": COLLECTIONS.iter().map(|c| { let mut v = json!({ "name": c.name, "entity": c.kind.name(), "doc": c.doc }); if let Some(a) = c.arg { v["arg"] = json!(a); } v }).collect::<Vec<_>>(),
        "types": TYPES.iter().map(|(n, d)| (n.to_string(), json!(d))).collect::<serde_json::Map<String, Value>>(),
        "functions": FUNCTIONS.iter().map(|(n, d)| json!({ "name": n, "doc": d })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_table_is_consistent() {
        for (i, e) in ENTITIES.iter().enumerate() {
            assert_eq!(e.kind.idx(), i, "{} is out of order", e.name);
            assert_eq!(e.fields[0].name, "id", "{}: id comes first", e.name);
            assert!(e.fields.len() <= 64, "{}: change masks hold 64 fields", e.name);
            let mut names: Vec<&str> = e.fields.iter().map(|f| f.name).collect();
            names.sort();
            names.dedup();
            assert_eq!(names.len(), e.fields.len(), "{}: duplicate field", e.name);
            for x in e.fields { assert_eq!(x.rel.is_some(), x.ty.contains("->"), "{}.{}: relation and type disagree", e.name, x.name); }
        }
        for c in COLLECTIONS { assert_eq!(collection(c.name).unwrap().kind, c.kind); }
        assert_eq!(Kind::by_name("space"), Some(Kind::Space));
    }

    /// docs/data-model.json is generated from this table: rerun with WEBOX_WRITE_MODEL=1 after changing the model.
    #[test]
    fn data_model_json_is_current() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/data-model.json");
        let now = serde_json::to_string_pretty(&describe()).unwrap() + "\n";
        if std::env::var_os("WEBOX_WRITE_MODEL").is_some() { std::fs::write(&path, &now).unwrap(); }
        let saved = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
        assert!(saved == now, "docs/data-model.json is out of date: run the tests with WEBOX_WRITE_MODEL=1");
    }
}
