//! `message.deleted` / `message.edited` and what plugins ask Webox to keep (docs/plugins.md, "Deleted and edited messages").
//!
//! The window hands every new list of a space to `Plugins::list_arrived` together with the list it replaces. `changes` compares
//! the two: a message that turned into a tombstone (the "… deleted …" row with the same id) or vanished inside the range the
//! new list covers was deleted; one whose text changed was edited. The events fire once per id (an edit once per new text).
//! A plugin's answer (`{"keep": true, "style": ...}` / `{"history": [...]}`) or the method `message.keep` puts the message
//! into `Kept`, and `apply` draws it into every later list of that space where it was written (the tombstone row goes).
//! Kept messages live in memory only; persisting them is the plugin's business.
use crate::cloud::Message;
use serde_json::{json, Value};
use std::collections::HashMap;

/// `Message::deleted`: how a kept deleted message is drawn.
pub const STYLE_TEXT: u8 = 1;
pub const STYLE_OVERLAY: u8 = 2;

fn style_of(v: &Value) -> u8 { if v.as_str() == Some("overlay") { STYLE_OVERLAY } else { STYLE_TEXT } }

/// What plugins asked to keep.
#[derive(Default)]
pub struct Kept {
    /// Space id -> deleted messages to keep showing (`deleted` set to their style).
    pub(super) deleted: HashMap<String, Vec<Message>>,
    /// Message id -> its earlier texts, oldest first.
    pub(super) edits: HashMap<String, Vec<String>>,
    /// Message id -> the tombstone row a kept message replaced (put back when the message is no longer kept).
    tombs: HashMap<String, Message>,
}

/// A change between two lists of a space.
#[derive(Clone, Copy, Debug)]
pub enum Change<'a> { Deleted(&'a Message), Edited(&'a Message, &'a str) }

/// The deletes and edits between `old` and `new` (lists of one space). Notices, local copies and rows that are already a kept
/// deleted message are not compared. A message missing from `new` counts as deleted only when `new` reaches back to it (an
/// older page that is simply not part of `new` is not a delete).
pub fn changes<'a>(old: &'a [Message], new: &'a [Message]) -> Vec<Change<'a>> {
    let by_id: HashMap<&str, &Message> = new.iter().map(|m| (m.id.as_str(), m)).collect();
    old.iter().filter(|m| m.notice.is_empty() && m.deleted == 0 && !m.id.is_empty() && !m.id.contains("-local")).filter_map(|m| match by_id.get(m.id.as_str()) {
        Some(n) if !n.notice.is_empty() => Some(Change::Deleted(m)),
        Some(n) if n.deleted == 0 && n.text != m.text => Some(Change::Edited(m, n.text.as_str())),
        Some(_) => None,
        None => (m.published_ms > 0 && new.iter().any(|n| n.published_ms > 0 && n.published_ms <= m.published_ms)).then_some(Change::Deleted(m)),
    }).collect()
}

/// The event payload of a deleted message. `date` / `time` are UTC; with `published_ms` and the rest they are enough for
/// `message.keep` to draw it again after a restart.
pub fn deleted_payload(space: &str, space_name: &str, m: &Message) -> Value {
    let (date, time) = m.utc.as_ref().map_or((m.date.as_str(), m.time.as_str()), |u| (u.0.as_str(), u.1.as_str()));
    json!({ "space": space, "space_name": space_name, "id": m.id, "author": m.author, "email": m.email, "text": m.text, "mine": m.mine,
        "parent": m.parent, "published_ms": m.published_ms, "date": date, "time": time })
}

/// The event payload of an edited message.
pub fn edited_payload(space: &str, space_name: &str, m: &Message, new_text: &str) -> Value {
    json!({ "space": space, "space_name": space_name, "id": m.id, "author": m.author, "mine": m.mine, "old_text": m.text, "new_text": new_text })
}

impl Kept {
    pub fn is_empty(&self) -> bool { self.deleted.is_empty() && self.edits.is_empty() }

    /// Forgets everything kept; the replaced tombstones stay so `strip` can put them back.
    pub fn clear(&mut self) { self.deleted.clear(); self.edits.clear(); }

    /// Keeps `m` of `space` as deleted, drawn in `style` (a later keep of the same id replaces it).
    pub fn keep_deleted(&mut self, space: &str, mut m: Message, style: u8) {
        m.deleted = style.max(STYLE_TEXT);
        m.rev = 0;
        let l = self.deleted.entry(space.to_string()).or_default();
        l.retain(|x| x.id != m.id);
        l.push(m);
    }

    /// `message.keep`: `{"deleted": [payload + "style"], "edited": [{"id", "history"}], "replace": bool}`.
    pub fn keep(&mut self, a: &Value) -> Value {
        // What was kept so far: a message given again keeps its full copy (pictures, files, rich text), not just the record.
        let before = if a.get("replace").and_then(Value::as_bool) == Some(true) { self.edits.clear(); std::mem::take(&mut self.deleted) } else { self.deleted.clone() };
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        for d in a.get("deleted").and_then(Value::as_array).into_iter().flatten() {
            let (space, id) = (s(d, "space"), s(d, "id"));
            if space.is_empty() || id.is_empty() { continue; }
            let known = before.get(&space).and_then(|l| l.iter().find(|m| m.id == id)).cloned();
            let m = known.unwrap_or_else(|| Message { id, author: s(d, "author"), email: s(d, "email"), text: s(d, "text"), mine: d.get("mine").and_then(Value::as_bool).unwrap_or(false),
                parent: s(d, "parent"), published_ms: d.get("published_ms").and_then(Value::as_i64).unwrap_or(0), date: s(d, "date"), time: s(d, "time"), ..Default::default() });
            self.keep_deleted(&space, m, style_of(&d["style"]));
        }
        for e in a.get("edited").and_then(Value::as_array).into_iter().flatten() {
            let id = s(e, "id");
            if !id.is_empty() { self.set_history(&id, &e["history"]); }
        }
        json!({})
    }

    /// Earlier texts of message `id` (a JSON array of strings; empty or anything else forgets them).
    pub fn set_history(&mut self, id: &str, h: &Value) {
        let h: Vec<String> = h.as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect();
        if h.is_empty() { self.edits.remove(id); } else { self.edits.insert(id.to_string(), h); }
    }

    /// Takes out what an earlier `apply` added to `list` (kept rows go, their tombstones come back, histories are cleared).
    pub fn strip(&self, list: &mut Vec<Message>) {
        list.retain_mut(|m| {
            if m.deleted == 0 { if !m.history.is_empty() { m.history.clear(); m.rev = 0; } return true; }
            match self.tombs.get(&m.id) { Some(t) => { *m = t.clone(); true } None => false }
        });
    }

    /// Draws what is kept into `list` (a list of `space`): a kept deleted message replaces its tombstone, or goes where it
    /// stood by `published_ms` when the list reaches back that far; earlier texts attach to their message.
    pub fn apply(&mut self, space: &str, list: &mut Vec<Message>) {
        if let Some(kept) = self.deleted.get(space) {
            for k in kept {
                match list.iter().position(|m| m.id == k.id) {
                    // The tombstone stands at the deletion time: the message goes back where it was written.
                    Some(i) if !list[i].notice.is_empty() => {
                        self.tombs.insert(k.id.clone(), list.remove(i));
                        let at = if k.published_ms > 0 { list.iter().position(|m| m.published_ms > k.published_ms).unwrap_or(list.len()) } else { i };
                        list.insert(at, k.clone());
                    }
                    Some(_) => {} // there (again) as a normal message, or already drawn
                    None => {
                        let reaches = k.published_ms > 0 && list.iter().any(|m| m.published_ms > 0 && m.published_ms <= k.published_ms);
                        if reaches {
                            let at = list.iter().position(|m| m.published_ms > k.published_ms).unwrap_or(list.len());
                            list.insert(at, k.clone());
                        }
                    }
                }
            }
        }
        if !self.edits.is_empty() {
            for m in list.iter_mut().filter(|m| m.deleted == 0 && m.notice.is_empty()) {
                if let Some(h) = self.edits.get(&m.id) { if m.history != *h { m.history = h.clone(); m.rev = 0; } }
            }
        }
    }
}
