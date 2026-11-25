//! What plugin threads may read of the window's data: the space list, the people and (in `Recents`, see mod.rs) the newest
//! messages per space. The window copies them in (`Plugins::feed`) only while a plugin subscribes to `message.received`, so
//! nothing is copied for anyone else, and the copy is dropped when the last such plugin goes. The host methods `spaces.list`,
//! `messages.search` and `people.search` read the copy; none of them makes a network request.
use super::{lock, Ctx, Plugins, Recent, RECENT_MAX};
use crate::cloud::{Message, Person, Space};
use serde_json::{json, Value};
use std::collections::HashMap;

/// One space as `spaces.list` hands it out.
pub(super) struct SpaceRow { id: String, title: String, direct: bool, unread: bool, last: String, writable: bool }
/// One person as `people.search` hands it out.
pub(super) struct PersonRow { id: String, name: String, email: String, status: String }

#[derive(Default)]
pub struct View { pub(super) spaces: Vec<SpaceRow>, people: Vec<PersonRow> }

/// The argument's `limit` (default `def`), at most `max`.
fn limit(a: &Value, def: usize, max: usize) -> usize { a.get("limit").and_then(Value::as_u64).map_or(def, |n| n as usize).min(max) }
/// The argument's `query`, trimmed and lower case ("" = none).
fn query(a: &Value) -> String { a.get("query").and_then(Value::as_str).unwrap_or("").trim().to_lowercase() }
fn has(hay: &str, q: &str) -> bool { q.is_empty() || hay.to_lowercase().contains(q) }

/// A message as `space.history` and `messages.search` hand it out (`date` / `time` UTC, as in `message.deleted`).
pub(super) fn message_json(m: &Recent) -> Value {
    json!({ "id": m.id, "author": m.author, "email": m.email, "mine": m.mine, "text": m.text, "date": m.date, "time": m.time, "published_ms": m.published_ms })
}
pub(super) fn recent_of(m: &Message) -> Recent {
    let (date, time) = m.utc.clone().unwrap_or_else(|| (m.date.clone(), m.time.clone()));
    Recent { id: m.id.clone(), author: m.author.clone(), email: m.email.clone(), mine: m.mine, text: m.text.clone(), date, time, published_ms: m.published_ms }
}

/// `spaces.list {limit?, query?}`: visible spaces in the list's order (latest activity first), filtered by title.
pub(super) fn spaces_list(c: &Ctx, a: &Value) -> Value {
    let q = query(a);
    let v = lock(&c.view);
    let l: Vec<Value> = v.spaces.iter().filter(|s| has(&s.title, &q)).take(limit(a, 50, 500))
        .map(|s| json!({ "id": s.id, "title": s.title, "direct": s.direct, "unread": s.unread, "last": s.last, "writable": s.writable })).collect();
    json!({ "spaces": l })
}

/// `messages.search {query, space?, limit?}`: loaded messages whose text contains `query` (case-insensitive, like the global
/// search), newest first, each with its space.
pub(super) fn messages_search(c: &Ctx, a: &Value) -> Value {
    let q = query(a);
    if q.is_empty() { return json!({ "error": "query missing" }); }
    let only = a.get("space").and_then(Value::as_str);
    let titles: HashMap<String, String> = lock(&c.view).spaces.iter().map(|s| (s.id.clone(), s.title.clone())).collect();
    let r = lock(&c.recent);
    let mut hits: Vec<(&String, &Recent)> = r.iter().filter(|(id, _)| only.is_none_or(|o| o == id.as_str()))
        .flat_map(|(id, l)| l.iter().filter(|m| m.text.to_lowercase().contains(&q)).map(move |m| (id, m))).collect();
    hits.sort_by(|x, y| (y.1.published_ms, &y.1.date, &y.1.time).cmp(&(x.1.published_ms, &x.1.date, &x.1.time)));
    let l: Vec<Value> = hits.into_iter().take(limit(a, 20, RECENT_MAX)).map(|(id, m)| {
        let mut v = message_json(m);
        v["space"] = json!(id);
        v["space_title"] = json!(titles.get(id).cloned().unwrap_or_default());
        v
    }).collect();
    json!({ "messages": l })
}

/// `people.search {query, limit?}`: people Webox knows (contacts and people of loaded spaces) by name or e-mail, with their
/// presence (`status`, as the service names it, e.g. "active", "inactive", "dnd"; "" = unknown).
pub(super) fn people_search(c: &Ctx, a: &Value) -> Value {
    let q = query(a);
    if q.is_empty() { return json!({ "error": "query missing" }); }
    let v = lock(&c.view);
    let l: Vec<Value> = v.people.iter().filter(|p| has(&p.name, &q) || has(&p.email, &q)).take(limit(a, 20, 200))
        .map(|p| json!({ "id": p.id, "name": p.name, "email": p.email, "status": p.status })).collect();
    json!({ "people": l })
}

impl Plugins {
    /// Copies what plugin threads may read: everything once when a plugin subscribes to `message.received` (also the lists
    /// loaded before), then the spaces and people again whenever the window `changed` something; dropped again when no plugin
    /// subscribes any more. The window calls it once per frame; without such a plugin it only checks the subscription.
    pub fn feed(&mut self, spaces: &[Space], people: &[Person], messages: &HashMap<String, Vec<Message>>, changed: bool) {
        let on = self.subscribed("message.received");
        if on && (changed || !self.fed) { self.note_view(spaces, people); }
        if on && !self.fed { for (id, l) in messages { self.note_space(id, l) } }
        if !on && self.fed { *lock(&self.view) = View::default(); lock(&self.recent).clear(); }
        self.fed = on;
    }

    /// The spaces (hidden ones left out) and the people: contacts first, then the people of the spaces, each once.
    pub fn note_view(&self, spaces: &[Space], people: &[Person]) {
        let sp = spaces.iter().filter(|s| !s.hidden).map(|s| SpaceRow { id: s.id.clone(), title: s.title.clone(), direct: s.direct, unread: s.unread,
            last: s.last.clone(), writable: !s.closed && !s.announcement }).collect();
        let mut seen = std::collections::HashSet::new();
        let pp = people.iter().chain(spaces.iter().flat_map(|s| s.participants.iter())).filter(|p| seen.insert(p.id.clone()))
            .map(|p| PersonRow { id: p.id.clone(), name: p.name.clone(), email: p.email.clone(), status: p.status.clone() }).collect();
        *lock(&self.view) = View { spaces: sp, people: pp };
    }
}
