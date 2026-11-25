//! The host methods plugin threads use to read the window's data (`spaces.list`, `messages.search`, `people.search`,
//! `space.history` 2), `ui.confirm`, `settings.set` and `clipboard.write`.
use super::tests::load_in_process;
use crate::cloud::{Message, Person, Space};
use collabhost_plugin_api::{Host, Plugin};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Subscribes to `message.received` (that is what makes the window copy its data for plugins).
#[derive(Default)]
struct Reader;
impl Plugin for Reader {
    const NAME: &'static str = "Reader"; const AUTHOR: &'static str = ""; const VERSION: &'static str = ""; const DESCRIPTION: &'static str = "";
    const EVENTS: &'static str = "message.received";
}

/// The host context of `p`'s last plugin, as a number so a test thread can take it along.
fn ctx(p: &super::Plugins) -> usize { p.list.last().unwrap().loaded.as_ref().unwrap().host.ctx as usize }
/// Calls a host method through `HostApi::call` with context `c`, from the calling thread.
fn call_in(c: usize, m: &str, j: Value) -> Value {
    let (m, j) = (collabhost_plugin_api::cs(m), collabhost_plugin_api::cs(&j.to_string()));
    let r = super::h_call(c as super::Ptr, m.as_ptr(), j.as_ptr());
    assert!(!r.is_null(), "known method");
    let t = unsafe { std::ffi::CStr::from_ptr(r) }.to_string_lossy().into_owned();
    super::h_free(r);
    serde_json::from_str(&t).unwrap()
}
fn call(p: &super::Plugins, m: &str, j: Value) -> Value { call_in(ctx(p), m, j) }

fn person(name: &str, email: &str, status: &str) -> Person { Person { id: email.into(), name: name.into(), email: email.into(), status: status.into() } }
fn space(id: &str, title: &str, direct: bool) -> Space { Space { id: id.into(), title: title.into(), direct, last: "17:05".into(), ..Default::default() } }
fn msg(id: &str, author: &str, text: &str, ms: i64) -> Message {
    Message { id: id.into(), author: author.into(), email: format!("{}@example.com", author.to_lowercase()), text: text.into(), date: "2026-11-24".into(),
        time: "10:00".into(), utc: Some(("2026-11-24".into(), "09:00".into())), published_ms: ms, ..Default::default() }
}

/// What the window holds: three spaces (one hidden, one read-only), contacts and messages.
fn window() -> (Vec<Space>, Vec<Person>, HashMap<String, Vec<Message>>) {
    let mut team = space("s1", "Project Alpha", false);
    team.participants = vec![person("Anna Berg", "anna@example.com", "active"), person("Ben Kurz", "ben@example.com", "dnd")];
    team.unread = true;
    let closed = Space { closed: true, ..space("s2", "Anna Berg", true) };
    let hidden = Space { hidden: true, ..space("s3", "Secret", false) };
    let people = vec![person("Anna Berg", "anna@example.com", "inactive"), person("Cleo Roth", "cleo@example.com", "meeting")];
    let mut m = HashMap::new();
    m.insert("s1".into(), vec![msg("m1", "Anna", "Release on Friday?", 1000), Message { notice: "Ben joined".into(), ..Default::default() },
        msg("m2", "Ben", "Yes, the release is on", 3000)]);
    m.insert("s2".into(), vec![msg("m3", "Anna", "release notes?", 2000)]);
    (vec![team, closed, hidden], people, m)
}

#[test]
fn feed_copies_only_while_a_plugin_listens_and_drops_it_after() {
    let (spaces, people, messages) = window();
    let (mut p, dir) = load_in_process::<Reader>(6);
    let i = p.list.len() - 1;
    p.set_enabled(i, false);
    p.feed(&spaces, &people, &messages, true);
    assert!(super::lock(&p.view).spaces.is_empty() && super::lock(&p.recent).is_empty(), "nobody listens: nothing copied");
    p.set_enabled(i, true);
    // The first frame with a listener copies everything, also lists loaded before.
    p.feed(&spaces, &people, &messages, false);
    assert_eq!(call(&p, "spaces.list", json!({}))["spaces"].as_array().unwrap().len(), 2);
    assert_eq!(call(&p, "space.history", json!({ "space": "s2" }))["messages"][0]["id"], "m3");
    // Later frames copy spaces and people again only when something changed.
    p.feed(&spaces[..1], &people, &messages, false);
    assert_eq!(call(&p, "spaces.list", json!({}))["spaces"].as_array().unwrap().len(), 2);
    p.feed(&spaces[..1], &people, &messages, true);
    assert_eq!(call(&p, "spaces.list", json!({}))["spaces"].as_array().unwrap().len(), 1);
    // Switched off: the copy is dropped.
    p.set_enabled(i, false);
    p.feed(&spaces, &people, &messages, true);
    assert!(super::lock(&p.view).spaces.is_empty() && super::lock(&p.recent).is_empty());
    drop(p);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn spaces_people_history_and_search() {
    let (spaces, people, messages) = window();
    let (mut p, dir) = load_in_process::<Reader>(6);
    p.feed(&spaces, &people, &messages, true);
    // spaces.list: hidden ones left out, in the list's order, filtered by title, limited.
    assert_eq!(call(&p, "spaces.list", json!({})), json!({ "spaces": [
        { "id": "s1", "title": "Project Alpha", "direct": false, "unread": true, "last": "17:05", "writable": true },
        { "id": "s2", "title": "Anna Berg", "direct": true, "unread": false, "last": "17:05", "writable": false }] }));
    assert_eq!(call(&p, "spaces.list", json!({ "query": "alpha" }))["spaces"].as_array().unwrap().len(), 1);
    assert_eq!(call(&p, "spaces.list", json!({ "limit": 1 }))["spaces"].as_array().unwrap().len(), 1);
    assert_eq!(call(&p, "spaces.list", json!({ "query": "secret" }))["spaces"], json!([]));
    // space.history 2: id, e-mail and the UTC date / time; system lines left out.
    assert_eq!(call(&p, "space.history", json!({ "space": "s1", "limit": 1 })), json!({ "messages": [
        { "id": "m2", "author": "Ben", "email": "ben@example.com", "mine": false, "text": "Yes, the release is on", "date": "2026-11-24", "time": "09:00", "published_ms": 3000 }] }));
    // messages.search: case-insensitive, newest first, with the space; optionally in one space; limited.
    let r = call(&p, "messages.search", json!({ "query": "RELEASE" }));
    let ids: Vec<&str> = r["messages"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["m2", "m3", "m1"]);
    assert_eq!((r["messages"][1]["space"].as_str(), r["messages"][1]["space_title"].as_str()), (Some("s2"), Some("Anna Berg")));
    assert_eq!(call(&p, "messages.search", json!({ "query": "release", "space": "s1" }))["messages"].as_array().unwrap().len(), 2);
    assert_eq!(call(&p, "messages.search", json!({ "query": "release", "limit": 1 }))["messages"][0]["id"], "m2");
    assert!(call(&p, "messages.search", json!({ "query": "  " })).get("error").is_some());
    // people.search: contacts first, then the people of the spaces, each once, by name or e-mail, with presence.
    assert_eq!(call(&p, "people.search", json!({ "query": "anna" })), json!({ "people": [{ "id": "anna@example.com", "name": "Anna Berg", "email": "anna@example.com", "status": "inactive" }] }));
    assert_eq!(call(&p, "people.search", json!({ "query": "BEN@" }))["people"][0]["status"], "dnd");
    assert_eq!(call(&p, "people.search", json!({ "query": "example.com" }))["people"].as_array().unwrap().len(), 3);
    assert!(call(&p, "people.search", json!({})).get("error").is_some());
    drop(p);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn settings_set_and_clipboard_go_to_the_outbox() {
    let (p, dir) = load_in_process::<Reader>(6);
    assert_eq!(call(&p, "settings.set", json!({ "id": "token", "value": "abc" })), json!({}));
    assert!(call(&p, "settings.set", json!({ "value": 1 })).get("error").is_some());
    assert_eq!(call(&p, "clipboard.write", json!({ "text": "snippet" })), json!({}));
    assert!(call(&p, "clipboard.write", json!({})).get("error").is_some());
    let out = std::mem::take(&mut *p.out.lock().unwrap());
    // (the test plugin's key depends on the in-process loader; what matters is the setting and its value)
    assert_eq!(out.settings_set.iter().map(|(_, id, v)| (id.as_str(), v.clone())).collect::<Vec<_>>(), [("token", json!("abc"))]);
    assert_eq!(out.clipboard.as_deref(), Some("snippet"));
    drop(p);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ui_confirm_waits_for_the_answer_and_never_blocks_the_ui_thread() {
    let (p, dir) = load_in_process::<Reader>(6);
    // On the window's thread (here: the thread that made `Plugins`) it would wait for itself: refused at once.
    assert!(call(&p, "ui.confirm", json!({ "text": "?" })).get("error").is_some());
    assert!(p.confirm_shown().is_none());
    let c = ctx(&p);
    let ask = |j: Value| std::thread::spawn(move || call_in(c, "ui.confirm", j));
    let shown = |p: &super::Plugins| { let t0 = Instant::now(); loop { if let Some(q) = p.confirm_shown() { return q; } assert!(t0.elapsed() < Duration::from_secs(5)); std::thread::yield_now(); } };
    // Confirmed in the dialog.
    let t = ask(json!({ "title": "Send?", "text": "Claude wants to send to Project Alpha: Hello", "ok": "Send", "cancel": "Decline" }));
    let (id, title, text, ok, cancel) = shown(&p);
    assert_eq!((title.as_str(), text.as_str(), ok.as_str(), cancel.as_str()), ("Send?", "Reader: Claude wants to send to Project Alpha: Hello", "Send", "Decline"));
    p.confirm_answer(id, true);
    assert_eq!(t.join().unwrap(), json!({ "ok": true }));
    // Refused; defaults for the labels.
    let t = ask(json!({ "text": "x" }));
    let (id, title, _, ok, cancel) = shown(&p);
    assert_eq!((title.as_str(), ok.as_str(), cancel.as_str()), ("Reader", "OK", &*crate::trq!("Cancel")));
    p.confirm_answer(id, false);
    assert_eq!(t.join().unwrap(), json!({ "ok": false }));
    // Nobody answers: no after the timeout, and the dialog goes.
    assert_eq!(ask(json!({ "text": "x", "timeout": 1 })).join().unwrap(), json!({ "ok": false, "timeout": true }));
    assert!(p.confirm_shown().is_none());
    assert!(call(&p, "ui.confirm", json!({})).get("error").is_some());
    drop(p);
    let _ = std::fs::remove_dir_all(dir);
}

/// Asks in a thread of its own on start and joins it in `stop`, like a plugin serving an AI client.
static ASKED: std::sync::Mutex<Option<Value>> = std::sync::Mutex::new(None);
#[derive(Default)]
struct Asker { t: Option<std::thread::JoinHandle<()>> }
impl Plugin for Asker {
    const NAME: &'static str = "Asker"; const AUTHOR: &'static str = ""; const VERSION: &'static str = ""; const DESCRIPTION: &'static str = "";
    const EVENTS: &'static str = "settings.changed";
    fn start(&mut self, h: Host) {
        self.t = Some(std::thread::spawn(move || {
            let r = h.call("ui.confirm", r#"{"text":"?","timeout":60}"#).unwrap();
            *ASKED.lock().unwrap() = serde_json::from_str(&r).ok();
        }));
    }
    fn stop(&mut self) { if let Some(t) = self.t.take() { let _ = t.join(); } }
}

#[test]
fn switching_a_plugin_off_answers_its_open_question() {
    let (mut p, dir) = load_in_process::<Asker>(6);
    let t0 = Instant::now();
    while p.confirm_shown().is_none() { assert!(t0.elapsed() < Duration::from_secs(5)); std::thread::yield_now(); }
    // `stop` joins the asking thread: it must not wait for the dialog (60 s) but get "no" at once.
    let i = p.list.len() - 1;
    p.set_enabled(i, false);
    assert!(t0.elapsed() < Duration::from_secs(10));
    assert_eq!(*ASKED.lock().unwrap(), Some(json!({ "ok": false })));
    assert!(p.confirm_shown().is_none());
    drop(p);
    let _ = std::fs::remove_dir_all(dir);
}
