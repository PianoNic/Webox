//! The space list kept live from `conversation.activity` events (no list reload): read state from my other devices,
//! conversations that appear, and changes made elsewhere to a space as a whole.
//!
//! * Read state: my own acknowledgement moves `last_seen` to the acknowledged activity's time; `unread` = something
//!   readable is newer; the UI drops that space's toasts. The time used is the acknowledged activity's `published` when
//!   it is cached, else the ack's own `published` (an ack follows what it acknowledges).
//! * New conversations: one `GET /conversations/<id>?activitiesLimit=0&participantsLimit=10` (the list request's own
//!   parameters) for a conversation not in the list, then the list gets it (`ListPatch::Upsert`).
//! * Changes elsewhere: the verbs UPDATE, HIDE, UNHIDE, FAVORITE, UNFAVORITE, TAG, UNTAG, LOCK, UNLOCK, ASSIGNMODERATOR,
//!   UNASSIGNMODERATOR, LEAVE, ARCHIVE, UNARCHIVE, ADD/REMOVE. `update` + object conversation `displayName` (JWE) and
//!   `leave`/`add` + object person are read as in fold.rs; hide / unhide / favorite / unfavorite / tag / untag /
//!   assignModerator / unassignModerator are read in the shapes Webox itself sends (see spaces_ext). Team / archive changes
//!   (team general-space `add` / `remove` of a conversation, `archive`, `unarchive`) reload the list once, debounced.
use super::*;
use super::spaces_ext::{Notify, SpaceInfo};
use std::collections::HashSet;

/// A change of the space list the UI applies in place (app/live_list.rs).
#[derive(Debug, Clone)]
pub enum ListPatch {
    /// My read state: (space, last seen ISO time, still unread).
    Read { space: String, last_seen: String, unread: bool },
    /// A conversation the list does not have yet (inserted by its last activity).
    Upsert(Space),
    /// Its notification setting / moderators / lock, as the list load sends them for every space.
    Info(SpaceInfo),
    Title { space: String, title: String },
    Favorite(String, bool),
    Hidden(String, bool),
    Locked(String, bool),
    /// Mute tags added (`true`) or removed on a space; the UI applies them to its current setting (`notify_after`).
    NotifyTags { space: String, add: bool, tags: Vec<String> },
    /// (space, the person's ids, assigned).
    Moderator { space: String, ids: Vec<String>, on: bool },
    /// I left / was removed: the space leaves the list.
    Remove(String),
}

/// What one activity means for the list.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Title(String),
    Favorite(bool),
    Hidden(bool),
    Locked(bool),
    Tags { add: bool, tags: Vec<String> },
    Moderator { ids: Vec<String>, on: bool },
    Removed,
    /// Team membership / archive: reload the list once.
    Reload,
}

/// The ids a person object carries (`id`, `entryUUID`, `emailAddress`), as spaces_ext `space_info` collects moderators.
fn person_ids(o: &Value) -> Vec<String> { ["id", "entryUUID", "emailAddress"].iter().map(|k| st(o, k)).filter(|v| !v.is_empty()).collect() }

/// Pure: the list-level meaning of an activity. `object_is_me` says whether its object (a person) is me.
pub fn classify(a: &Value, object_is_me: bool) -> Vec<Change> {
    let o = &a["object"];
    let ot = o["objectType"].as_str().unwrap_or("");
    let verb = a["verb"].as_str().unwrap_or("");
    let tags = || -> Vec<String> { o["tags"].as_array().map(|t| t.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default() };
    match (verb, ot) {
        ("update", "conversation") if !st(o, "displayName").is_empty() => vec![Change::Title(st(o, "displayName"))],
        ("hide", _) => vec![Change::Hidden(true)],
        ("unhide", _) => vec![Change::Hidden(false)],
        ("favorite", _) => vec![Change::Favorite(true)],
        ("unfavorite", _) => vec![Change::Favorite(false)],
        ("lock", _) => vec![Change::Locked(true)],
        ("unlock", _) => vec![Change::Locked(false)],
        ("tag" | "untag", _) => {
            let (add, t) = (verb == "tag", tags());
            let mut out = vec![];
            if t.iter().any(|x| x == "FAVORITE") { out.push(Change::Favorite(add)); }
            if t.iter().any(|x| x == "HIDDEN") { out.push(Change::Hidden(add)); }
            if t.iter().any(|x| x == "LOCKED") { out.push(Change::Locked(add)); }
            let mute: Vec<String> = t.into_iter().filter(|x| x == "MUTED" || spaces_ext::MUTE_TAGS.contains(&x.as_str())).collect();
            if !mute.is_empty() { out.push(Change::Tags { add, tags: mute }); }
            out
        }
        ("assignModerator" | "unassignModerator", "person") => vec![Change::Moderator { ids: person_ids(o), on: verb == "assignModerator" }],
        ("leave", "person") if object_is_me => vec![Change::Removed],
        ("add" | "remove", "conversation") | ("archive" | "unarchive", _) => vec![Change::Reload],
        _ => vec![],
    }
}

/// A space's notification setting after mute tags were added / removed (the same tag model as `Notify::from_tags`).
pub fn notify_after(current: Notify, add: bool, tags: &[String]) -> Notify {
    let mut set: HashSet<String> = current.tags().iter().map(|s| s.to_string()).collect();
    for t in tags { if add { set.insert(t.clone()); } else { set.remove(t); } }
    let v: Vec<&str> = set.iter().map(String::as_str).collect();
    Notify::from_tags(&v)
}

/// The read-state rule: the acknowledged time and whether the space stays unread. `None` = nothing new (already seen).
pub fn read_after(last_seen: &str, last_readable: &str, acked_at: &str) -> Option<(String, bool)> {
    (!acked_at.is_empty() && acked_at > last_seen).then(|| (acked_at.to_string(), last_readable > acked_at))
}

/// Verbs that make a conversation appear in my list when it is not there yet (a message, being added, a new space).
pub fn opens_unknown(verb: &str) -> bool { matches!(verb, "post" | "share" | "add" | "create") }

/// How long a failed single-conversation fetch is not repeated.
const UNKNOWN_RETRY: Duration = Duration::from_secs(60);
/// Debounce of the fallback list reload (several team activities arrive together).
const RELOAD_DEBOUNCE: Duration = Duration::from_secs(2);

/// Session state of this module.
#[derive(Default)]
pub struct Live {
    asked: HashMap<String, Instant>,
    reload_at: Option<Instant>,
}

impl Session {
    /// My own acknowledgement (from this or another device of mine).
    pub(super) fn self_read(&mut self, space: &str, a: &Value) {
        let acked = st(&a["object"], "id");
        let cached = self.hist.acts.get(space).and_then(|l| l.iter().find(|x| st(x, "id") == acked)).map(|x| st(x, "published"));
        let at = cached.or_else(|| Some(st(&a["object"], "published"))).filter(|p| !p.is_empty()).unwrap_or_else(|| st(a, "published"));
        let readable = self.newest_readable(space);
        let Some(sp) = self.spaces.get_mut(space) else { return };
        let last = if readable > sp.last { readable } else { sp.last.clone() };
        let Some((seen, unread)) = read_after(&sp.last_seen, &last, &at) else { return };
        sp.last_seen = seen.clone();
        sp.unread = unread;
        let _ = self.out.send(Resp::List(ListPatch::Read { space: space.to_string(), last_seen: seen, unread }));
    }

    /// The newest readable (post / share, not an edit) activity time in the cached history.
    fn newest_readable(&self, space: &str) -> String {
        self.hist.acts.get(space).into_iter().flatten().filter(|a| (a["verb"] == "post" || a["verb"] == "share") && a["parent"]["type"] != "edit")
            .map(|a| st(a, "published")).max().unwrap_or_default()
    }

    /// Keeps `last` (lastReadableActivityDate) current for live messages, so the read-state rule knows whether an ack read everything.
    pub(super) fn note_readable(&mut self, space: &str, a: &Value) {
        if !((a["verb"] == "post" || a["verb"] == "share") && a["parent"]["type"] != "edit") { return }
        let p = st(a, "published");
        if let Some(sp) = self.spaces.get_mut(space) { if p > sp.last { sp.last = p; } }
    }

    /// An activity for a conversation the list does not have. Fetches it once (one GET) and adds it; `true` = known now.
    pub(super) fn unknown_space(&mut self, id: &str, a: &Value) -> bool {
        if id.is_empty() || !opens_unknown(a["verb"].as_str().unwrap_or("")) { return false }
        if self.live.asked.get(id).is_some_and(|t| t.elapsed() < UNKNOWN_RETRY) { return false }
        self.live.asked.insert(id.to_string(), Instant::now());
        let url = self.url("conversation", &format!("/conversations/{id}?activitiesLimit=0&participantsLimit=10"));
        let c = match self.http("GET", &url, None) { Ok(c) => c, Err(e) => { dbg_conn(&format!("new conversation: GET failed ({})", http_status(&e).map_or("transport".into(), |c| c.to_string()))); return false } };
        self.add_conversation(id, &c)
    }

    /// The fetched conversation joins the list (and the UI's), its people are followed.
    pub(super) fn add_conversation(&mut self, id: &str, c: &Value) -> bool {
        if st(c, "id") != id { return false }
        let c = c.clone();
        let (mut people, mut teams) = (HashMap::new(), HashMap::new());
        let mut one = vec![self.space_of(&c, &mut people, &mut teams)];
        // A team's general space is titled "Allgemein" like in the list load (lists.rs `name_team_generals`).
        super::lists::name_team_generals(&mut one, &mut teams, &crate::tr!("QObject", "General"));
        let sp = one.remove(0);
        self.spaces.insert(sp.id.clone(), sp.clone());
        dbg_conn("new conversation: added to the list");
        let _ = self.out.send(Resp::List(ListPatch::Upsert(sp)));
        let _ = self.out.send(Resp::List(ListPatch::Info(spaces_ext::space_info(&c))));
        // A team the list doesn't know yet: its name / colour come with the list (fallback reload).
        if teams.keys().any(|t| !spaces_ext::team_known(t)) { self.live.reload_at = Some(Instant::now() + RELOAD_DEBOUNCE); }
        self.fetch_images();
        let new: Vec<Person> = people.into_values().collect();
        self.load_avatars(&new);
        self.subscribe_new(&new, Some(id));
        true
    }

    /// What an activity changes in the list, applied here and sent to the UI. Any space I know (opened or not).
    pub(super) fn list_activity(&mut self, id: &str, a: &Value) {
        let me = self.actor_is_me(&a["object"]);
        for ch in classify(a, me) {
            let patch = match ch {
                Change::Title(jwe) => {
                    let Some(sp) = self.spaces.get(id) else { continue };
                    if sp.direct { continue } // a 1:1 is titled by the other person
                    let key = Some(st(a, "encryptionKeyUrl")).filter(|k| !k.is_empty()).unwrap_or_else(|| sp.key_url.clone());
                    let title = self.decrypt(&jwe, &key);
                    if title.is_empty() || title.starts_with("eyJ") { continue } // not decryptable: keep the old title
                    if let Some(sp) = self.spaces.get_mut(id) { sp.title = title.clone(); }
                    ListPatch::Title { space: id.into(), title }
                }
                Change::Favorite(on) => { if let Some(sp) = self.spaces.get_mut(id) { sp.favorite = on; } ListPatch::Favorite(id.into(), on) }
                Change::Hidden(on) => { if let Some(sp) = self.spaces.get_mut(id) { sp.hidden = on; } ListPatch::Hidden(id.into(), on) }
                Change::Locked(on) => ListPatch::Locked(id.into(), on),
                Change::Tags { add, tags } => ListPatch::NotifyTags { space: id.into(), add, tags },
                Change::Moderator { ids, on } => ListPatch::Moderator { space: id.into(), ids, on },
                Change::Removed => { self.spaces.remove(id); ListPatch::Remove(id.into()) }
                Change::Reload => { self.live.reload_at = Some(Instant::now() + RELOAD_DEBOUNCE); continue }
            };
            let _ = self.out.send(Resp::List(patch));
        }
    }

    /// The debounced fallback reload (job loop tick).
    pub(super) fn list_reload_due(&mut self) {
        if self.live.reload_at.is_some_and(|t| Instant::now() >= t) {
            self.live.reload_at = None;
            if let Err(e) = self.spaces() { dbg_conn(&format!("list reload failed: {}", http_status(&e).map_or("transport".into(), |c| c.to_string()))); }
        }
    }
}

#[cfg(test)]
pub(super) mod tests;
