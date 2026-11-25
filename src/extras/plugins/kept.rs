//! `message.deleted` / `message.edited` and what plugins ask Webox to keep (docs/plugins.md, "Deleted and edited messages").
//!
//! The window hands every new list of a space to `Plugins::list_arrived` together with the list it replaces. `changes` compares
//! the two: a message that turned into a tombstone (the "… deleted …" row with the same id) or vanished inside the range the
//! new list covers was deleted; one whose text changed was edited. The events fire once per id (an edit once per new text).
//! A plugin's answer (`{"keep": true, "style": ...}` / `{"history": [...]}`) or the method `message.keep` puts the message
//! into `Kept`, and `apply` draws it into every later list of that space where it was written (the tombstone row goes).
//! Kept messages live in memory only; persisting them is the plugin's business.
use crate::cloud::{Message, Span};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// `Message::deleted`: how a kept deleted message is drawn.
pub const STYLE_TEXT: u8 = 1;
pub const STYLE_OVERLAY: u8 = 2;
/// Kept, but drawn like any message (no red, no "(deleted)", pictures in colour): a plugin switched its highlight off.
pub const STYLE_PLAIN: u8 = 3;

/// Kinds of the spans in `Message::history` / `Message::current` (plugin-given rich lines).
/// An earlier text given as a plain string: secondary grey, struck through.
pub const SPAN_OLD: u8 = 0;
/// Unchanged text of a diff: secondary grey (in the current text: its own colour).
pub const SPAN_PLAIN: u8 = 1;
/// Added text of a diff: green.
pub const SPAN_ADDED: u8 = 2;
/// Removed text of a diff: red, struck through.
pub const SPAN_REMOVED: u8 = 3;

fn style_of(v: &Value) -> u8 { match v.as_str() { Some("overlay") => STYLE_OVERLAY, Some("plain") => STYLE_PLAIN, _ => STYLE_TEXT } }

/// At most this many spans per line and characters per line (a plugin cannot make a row arbitrarily expensive).
const SPANS_MAX: usize = 400;
const LINE_MAX: usize = 20_000;

/// Spans as plugins give them: `[{"text", "kind"}]` (kind "added" / "removed", anything else plain), `{"spans": [...]}`, or a
/// bare string (one span of kind `whole`). Empty pieces are dropped; neighbours of the same kind are merged.
pub fn spans(v: &Value, whole: u8) -> Vec<Span> {
    let raw: Vec<Span> = match v {
        Value::String(t) => vec![(t.clone(), whole)],
        Value::Array(a) => a.iter().take(SPANS_MAX).filter_map(|x| {
            let t = x.get("text").and_then(Value::as_str)?;
            Some((t.to_string(), match x.get("kind").and_then(Value::as_str) { Some("added") => SPAN_ADDED, Some("removed") => SPAN_REMOVED, _ => SPAN_PLAIN }))
        }).collect(),
        Value::Object(_) => return spans(v.get("spans").unwrap_or(&Value::Null), whole),
        _ => vec![],
    };
    let mut out: Vec<Span> = Vec::new();
    let mut n = 0;
    for (t, k) in raw.into_iter().filter(|s| !s.0.is_empty()) {
        let t: String = t.chars().take(LINE_MAX.saturating_sub(n)).collect();
        if t.is_empty() { break; }
        n += t.chars().count();
        match out.last_mut() { Some(l) if l.1 == k => l.0.push_str(&t), _ => out.push((t, k)) }
    }
    out
}

/// What a plugin attached to an edited message.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Edit {
    /// The earlier texts, oldest first.
    pub history: Vec<Vec<Span>>,
    /// The current text with its added parts (diff view); empty = drawn as usual.
    pub current: Vec<Span>,
    /// false: the earlier texts are not drawn in the message, only "(edited)" (a click opens what the plugin shows).
    pub inline: bool,
}
impl Edit {
    /// From an answer / `message.keep` record: `{"history": [string | spans], "current"?: spans, "inline"?: bool}`. None =
    /// nothing to attach (no earlier text and not `"inline": false`).
    fn of(v: &Value) -> Option<Edit> {
        let history: Vec<Vec<Span>> = v.get("history").and_then(Value::as_array).into_iter().flatten().take(64)
            .map(|h| spans(h, SPAN_OLD)).filter(|l| !l.is_empty()).collect();
        let inline = v.get("inline").and_then(Value::as_bool) != Some(false);
        let current = v.get("current").map(|c| spans(c, SPAN_PLAIN)).unwrap_or_default();
        (!history.is_empty() || !inline).then_some(Edit { history, current, inline })
    }
}

/// How a kept deleted message is drawn, from an answer / `message.keep` record: `"style"`, `"collapsed"`, `"attachments"`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeletedLook { pub style: u8, pub collapsed: bool, pub attachments: bool }
impl DeletedLook {
    pub fn of(v: &Value) -> DeletedLook {
        DeletedLook { style: style_of(&v["style"]), collapsed: v["collapsed"] == true, attachments: v["attachments"] != false }
    }
}
impl Default for DeletedLook { fn default() -> Self { DeletedLook { style: STYLE_TEXT, collapsed: false, attachments: true } } }

/// What plugins asked to keep.
#[derive(Default)]
pub struct Kept {
    /// Space id -> deleted messages to keep showing (`deleted` set to their style, `collapsed` as asked), full copies.
    pub(super) deleted: HashMap<String, Vec<Message>>,
    /// Kept deleted messages drawn without their pictures, files and link cards (`"attachments": false`).
    no_files: HashSet<String>,
    /// Message id -> what is attached to it.
    pub(super) edits: HashMap<String, Edit>,
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

/// The space a change happened in, as the events name it: id, title, and its team (id and name; empty outside a team).
#[derive(Clone, Copy, Debug, Default)]
pub struct Where<'a> { pub space: &'a str, pub name: &'a str, pub team: &'a str, pub team_name: &'a str }

/// The event payload of a deleted message. `date` / `time` are UTC; with `published_ms` and the rest they are enough for
/// `message.keep` to draw it again after a restart. 2: `team` / `team_name`.
pub fn deleted_payload(w: Where, m: &Message) -> Value {
    let (date, time) = m.utc.as_ref().map_or((m.date.as_str(), m.time.as_str()), |u| (u.0.as_str(), u.1.as_str()));
    json!({ "space": w.space, "space_name": w.name, "team": w.team, "team_name": w.team_name, "id": m.id, "author": m.author, "email": m.email, "text": m.text,
        "mine": m.mine, "parent": m.parent, "published_ms": m.published_ms, "date": date, "time": time })
}

/// The event payload of an edited message. 2: `email`, `team` / `team_name`, `published_ms`, and `old_edited` (the old text
/// was already an edit, so the original text was never seen).
pub fn edited_payload(w: Where, m: &Message, new_text: &str) -> Value {
    json!({ "space": w.space, "space_name": w.name, "team": w.team, "team_name": w.team_name, "id": m.id, "author": m.author, "email": m.email, "mine": m.mine,
        "published_ms": m.published_ms, "old_edited": m.edited, "old_text": m.text, "new_text": new_text })
}

/// A kept deleted message without its pictures, files ("📎 name" lines) and link cards.
fn without_files(m: &Message) -> Message {
    let mut m = m.clone();
    m.images.clear();
    m.files.clear();
    m.previews.clear();
    m.text = m.text.lines().filter(|l| !l.trim_start().starts_with('📎')).collect::<Vec<_>>().join("\n");
    m
}

impl Kept {
    pub fn is_empty(&self) -> bool { self.deleted.is_empty() && self.edits.is_empty() }

    /// Forgets everything kept; the replaced tombstones stay so `strip` can put them back.
    pub fn clear(&mut self) { self.deleted.clear(); self.edits.clear(); self.no_files.clear(); }

    /// Keeps `m` of `space` as deleted, drawn as `look` says (a later keep of the same id replaces it).
    pub fn keep_deleted(&mut self, space: &str, mut m: Message, look: DeletedLook) {
        m.deleted = look.style.max(STYLE_TEXT);
        m.collapsed = look.collapsed;
        m.rev = 0;
        if look.attachments { self.no_files.remove(&m.id); } else { self.no_files.insert(m.id.clone()); }
        let l = self.deleted.entry(space.to_string()).or_default();
        l.retain(|x| x.id != m.id);
        l.push(m);
    }

    /// `message.keep`: `{"deleted": [payload + "style" / "collapsed" / "attachments"], "edited": [{"id", "history", "current"?,
    /// "inline"?}], "replace": bool}`.
    pub fn keep(&mut self, a: &Value) -> Value {
        // What was kept so far: a message given again keeps its full copy (pictures, files, rich text), not just the record.
        let before = if a.get("replace").and_then(Value::as_bool) == Some(true) {
            self.edits.clear();
            self.no_files.clear();
            std::mem::take(&mut self.deleted)
        } else { self.deleted.clone() };
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        for d in a.get("deleted").and_then(Value::as_array).into_iter().flatten() {
            let (space, id) = (s(d, "space"), s(d, "id"));
            if space.is_empty() || id.is_empty() { continue; }
            let known = before.get(&space).and_then(|l| l.iter().find(|m| m.id == id)).cloned();
            let m = known.unwrap_or_else(|| Message { id, author: s(d, "author"), email: s(d, "email"), text: s(d, "text"), mine: d.get("mine").and_then(Value::as_bool).unwrap_or(false),
                parent: s(d, "parent"), published_ms: d.get("published_ms").and_then(Value::as_i64).unwrap_or(0), date: s(d, "date"), time: s(d, "time"), ..Default::default() });
            self.keep_deleted(&space, m, DeletedLook::of(d));
        }
        for e in a.get("edited").and_then(Value::as_array).into_iter().flatten() {
            let id = s(e, "id");
            if !id.is_empty() { self.set_edit(&id, e); }
        }
        json!({})
    }

    /// What to attach to message `id`: an answer to `message.edited` or a `message.keep` record (`Edit::of`); nothing to attach
    /// forgets it.
    pub fn set_edit(&mut self, id: &str, v: &Value) {
        match Edit::of(v) { Some(e) => { self.edits.insert(id.to_string(), e); } None => { self.edits.remove(id); } }
    }

    /// Takes out what an earlier `apply` added to `list` (kept rows go, their tombstones come back, histories are cleared).
    pub fn strip(&self, list: &mut Vec<Message>) {
        list.retain_mut(|m| {
            if m.deleted == 0 {
                if !m.history.is_empty() || !m.current.is_empty() || m.edit_tag { m.history.clear(); m.current.clear(); m.edit_tag = false; m.rev = 0; }
                return true;
            }
            match self.tombs.get(&m.id) { Some(t) => { *m = t.clone(); true } None => false }
        });
    }

    /// The copy of kept message `k` that is drawn.
    fn drawn(&self, k: &Message) -> Message { if self.no_files.contains(&k.id) { without_files(k) } else { k.clone() } }

    /// Draws what is kept into `list` (a list of `space`): a kept deleted message replaces its tombstone, or goes where it
    /// stood by `published_ms` when the list reaches back that far; earlier texts attach to their message.
    pub fn apply(&mut self, space: &str, list: &mut Vec<Message>) {
        if let Some(kept) = self.deleted.get(space) {
            let mut tombs = Vec::new();
            for k in kept {
                match list.iter().position(|m| m.id == k.id) {
                    // The tombstone stands at the deletion time: the message goes back where it was written.
                    Some(i) if !list[i].notice.is_empty() => {
                        tombs.push((k.id.clone(), list.remove(i)));
                        let at = if k.published_ms > 0 { list.iter().position(|m| m.published_ms > k.published_ms).unwrap_or(list.len()) } else { i };
                        list.insert(at, self.drawn(k));
                    }
                    Some(_) => {} // there (again) as a normal message, or already drawn
                    None => {
                        let reaches = k.published_ms > 0 && list.iter().any(|m| m.published_ms > 0 && m.published_ms <= k.published_ms);
                        if reaches {
                            let at = list.iter().position(|m| m.published_ms > k.published_ms).unwrap_or(list.len());
                            list.insert(at, self.drawn(k));
                        }
                    }
                }
            }
            self.tombs.extend(tombs);
        }
        if !self.edits.is_empty() {
            for m in list.iter_mut().filter(|m| m.deleted == 0 && m.notice.is_empty()) {
                if let Some(e) = self.edits.get(&m.id) {
                    let lines: &[Vec<Span>] = if e.inline { &e.history } else { &[] };
                    if m.history != lines || m.current != e.current || !m.edit_tag { m.history = lines.to_vec(); m.current = e.current.clone(); m.edit_tag = true; m.rev = 0; }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_parse_strings_lists_and_objects() {
        assert_eq!(spans(&json!("old"), SPAN_OLD), [("old".to_string(), SPAN_OLD)]);
        let v = json!([{ "text": "Hello " }, { "text": "world", "kind": "removed" }, { "text": "everyone", "kind": "added" }, { "text": "" }, { "text": "!", "kind": "x" }, { "kind": "added" }]);
        assert_eq!(spans(&v, SPAN_OLD), [("Hello ".into(), SPAN_PLAIN), ("world".into(), SPAN_REMOVED), ("everyone".into(), SPAN_ADDED), ("!".into(), SPAN_PLAIN)]);
        assert_eq!(spans(&json!({ "spans": [{ "text": "a" }, { "text": "b" }] }), SPAN_OLD), [("ab".to_string(), SPAN_PLAIN)], "neighbours of one kind merge");
        assert!(spans(&json!(3), SPAN_OLD).is_empty());
    }

    #[test]
    fn an_edit_with_inline_off_keeps_only_the_tag() {
        let mut k = Kept::default();
        k.set_edit("m1", &json!({ "history": [], "inline": false }));
        k.set_edit("m2", &json!({ "history": ["old"], "current": [{ "text": "new", "kind": "added" }] }));
        k.set_edit("m3", &json!({ "history": [] }));
        let mut l = vec![Message { id: "m1".into(), text: "a".into(), ..Default::default() }, Message { id: "m2".into(), text: "new".into(), ..Default::default() },
            Message { id: "m3".into(), text: "c".into(), ..Default::default() }];
        k.apply("s", &mut l);
        assert!(l[0].history.is_empty() && l[0].edit_tag);
        assert_eq!((l[1].history.clone(), l[1].current.clone(), l[1].edit_tag), (vec![vec![("old".to_string(), SPAN_OLD)]], vec![("new".to_string(), SPAN_ADDED)], true));
        assert!(!l[2].edit_tag, "nothing to attach");
        k.strip(&mut l);
        assert!(l.iter().all(|m| m.history.is_empty() && m.current.is_empty() && !m.edit_tag));
    }

    #[test]
    fn a_deleted_message_is_drawn_as_its_look_says() {
        let mut k = Kept::default();
        let m = Message { id: "m1".into(), text: "Picture\n📎 a.pdf".into(), published_ms: 10, images: vec![Default::default()], files: vec![Default::default()], ..Default::default() };
        k.keep_deleted("s", m.clone(), DeletedLook::of(&json!({ "style": "plain", "collapsed": true, "attachments": false })));
        let mut l = vec![Message { id: "m0".into(), published_ms: 5, ..Default::default() }];
        k.apply("s", &mut l);
        assert_eq!((l[1].deleted, l[1].collapsed, l[1].text.as_str(), l[1].images.len(), l[1].files.len()), (STYLE_PLAIN, true, "Picture", 0, 0));
        // Kept again with its attachments: the full copy comes back.
        k.keep_deleted("s", m, DeletedLook::default());
        let mut l = vec![Message { id: "m0".into(), published_ms: 5, ..Default::default() }];
        k.apply("s", &mut l);
        assert_eq!((l[1].deleted, l[1].collapsed, l[1].images.len(), l[1].files.len()), (STYLE_TEXT, false, 1, 1));
    }
}
