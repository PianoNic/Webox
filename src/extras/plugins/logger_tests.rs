//! What a message logger needs beyond `message.deleted` / `message.edited`: `space.menu` / `space.menu_action`,
//! `message.edit_tag_clicked` with `ui.dialog`, and the richer answers (diff spans, the current text, `"inline": false`, a
//! folded deleted message without its attachments).
use super::kept::{Where, SPAN_ADDED, SPAN_OLD, SPAN_PLAIN, SPAN_REMOVED, STYLE_PLAIN};
use super::tests::load_in_process;
use collabhost_plugin_api::{Host, Plugin};
use serde_json::{json, Value};
use std::sync::Mutex;

static SEEN: Mutex<Vec<(String, Value)>> = Mutex::new(vec![]);

/// Offers "Protokoll leeren" in the menu of space "s1" only, opens a dialog when an edit tag is clicked, keeps deleted
/// messages folded without attachments and answers edits with a diff.
#[derive(Default)]
struct Logger { host: Option<Host> }
impl Plugin for Logger {
    const NAME: &'static str = "LogUi"; const AUTHOR: &'static str = ""; const VERSION: &'static str = ""; const DESCRIPTION: &'static str = "";
    const EVENTS: &'static str = "space.menu space.menu_action message.edit_tag_clicked message.deleted message.edited";
    fn start(&mut self, host: Host) { self.host = Some(host); }
    fn on_event(&mut self, name: &str, payload: &str) -> Option<String> {
        let v: Value = serde_json::from_str(payload).unwrap();
        SEEN.lock().unwrap().push((name.into(), v.clone()));
        match name {
            "space.menu" => (v["space"] == "s1").then(|| json!({ "items": [{ "id": "clear", "label": "Protokoll leeren", "danger": true }] }).to_string()),
            "message.edit_tag_clicked" => {
                let r = self.host?.call("ui.dialog", &json!({ "title": "Bearbeitungsverlauf", "switch": { "label": "Unterschiede anzeigen" },
                    "tabs": [{ "time_ms": 1000, "text": "alt", "alt": [{ "text": "alt", "kind": "removed" }, { "text": "neu", "kind": "added" }] }, { "label": "Aktuell", "text": v["text"] }] }).to_string());
                assert_eq!(r.as_deref(), Some("{}"));
                None
            }
            "message.deleted" => Some(json!({ "keep": true, "style": "plain", "collapsed": true, "attachments": false }).to_string()),
            "message.edited" => Some(json!({ "history": [[{ "text": "Hallo " }, { "text": "Welt", "kind": "removed" }]],
                "current": [{ "text": "Hallo " }, { "text": "Leute", "kind": "added" }] }).to_string()),
            _ => None,
        }
    }
}

#[test]
fn space_menu_entries_round_trip_and_nothing_without_a_plugin() {
    let (p, dir) = load_in_process::<Logger>(6);
    let key = p.list.last().unwrap().key.clone();
    assert!(p.space_menu(&json!({ "space": "s2", "title": "Leer", "direct": false, "team": "" })).is_empty(), "nothing logged there");
    let items = p.space_menu(&json!({ "space": "s1", "title": "Team", "direct": false, "team": "" }));
    assert_eq!(items, [super::MenuItem { plugin: key.clone(), id: "clear".into(), label: "Protokoll leeren".into(), icon: String::new(), danger: true }]);
    p.space_menu_action(&key, "s1", "clear");
    p.space_menu_action("someone-else", "s1", "clear");
    {
        let seen = SEEN.lock().unwrap();
        let acts: Vec<&Value> = seen.iter().filter(|s| s.0 == "space.menu_action").map(|s| &s.1).collect();
        assert_eq!(acts, [&json!({ "space": "s1", "item": "clear" })], "only to the plugin that offered it");
    }
    drop(p);
    let _ = std::fs::remove_dir_all(&dir);
    // Without a subscribed plugin the space menu gets nothing and no plugin is asked.
    let p = super::Plugins::with_root(&dir, dir.join("installed"));
    assert!(p.space_menu(&json!({ "space": "s1" })).is_empty());
    p.space_menu_action("x", "s1", "clear");
    drop(p);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_edit_tag_click_opens_the_plugins_dialog_until_it_is_switched_off() {
    let (mut p, dir) = load_in_process::<Logger>(6);
    assert!(p.subscribed("message.edit_tag_clicked"));
    p.emit("message.edit_tag_clicked", &json!({ "space": "s1", "id": "m1", "text": "neu", "mine": false, "published_ms": 1000 }).to_string());
    let d = p.out.lock().unwrap().dialog.take().expect("queued for the window");
    p.dialog = Some(d);
    let d = p.dialog_shown().unwrap();
    assert_eq!((d.title.as_str(), d.tabs.len(), d.selected, d.switch.clone()), ("Bearbeitungsverlauf", 2, 1, Some(("Unterschiede anzeigen".to_string(), false))));
    assert_eq!(d.tabs[1].text, [("neu".to_string(), SPAN_PLAIN)]);
    assert_eq!(d.tabs[0].alt, [("alt".to_string(), SPAN_REMOVED), ("neu".to_string(), SPAN_ADDED)]);
    let i = p.list.len() - 1;
    p.set_enabled(i, false);
    assert!(p.dialog_shown().is_none(), "closed with its plugin");
    drop(p);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn richer_answers_fold_strip_and_colour() {
    let (mut p, dir) = load_in_process::<Logger>(6);
    let m = |id: &str, text: &str, ms: i64| crate::cloud::Message { id: id.into(), author: "Anna".into(), text: text.into(), published_ms: ms, ..Default::default() };
    let mut pic = m("m1", "Foto\n📎 plan.pdf", 1000);
    pic.images = vec![Default::default()];
    pic.edited = true;
    let old = vec![pic, m("m2", "Hallo Welt", 2000)];
    let mut new = vec![m("m2", "Hallo Leute", 2000), crate::cloud::Message { id: "m1".into(), notice: "Anna hat ihre Nachricht gelöscht.".into(), ..Default::default() }];
    p.list_arrived(Where { space: "s1", name: "Team", team: "t1", team_name: "Projekt" }, Some(&old), &mut new);
    let d = new.iter().find(|x| x.id == "m1").unwrap();
    assert_eq!((d.deleted, d.collapsed, d.text.as_str(), d.images.len()), (STYLE_PLAIN, true, "Foto", 0));
    let e = new.iter().find(|x| x.id == "m2").unwrap();
    assert_eq!(e.history, [vec![("Hallo ".to_string(), SPAN_PLAIN), ("Welt".to_string(), SPAN_REMOVED)]]);
    assert_eq!(e.current, [("Hallo ".to_string(), SPAN_PLAIN), ("Leute".to_string(), SPAN_ADDED)]);
    assert!(e.edit_tag);
    {
        let seen = SEEN.lock().unwrap();
        let del = seen.iter().rev().find(|s| s.0 == "message.deleted" && s.1["id"] == "m1").unwrap();
        assert_eq!((del.1["team"].as_str(), del.1["team_name"].as_str()), (Some("t1"), Some("Projekt")));
        let ed = seen.iter().rev().find(|s| s.0 == "message.edited" && s.1["id"] == "m2").unwrap();
        assert_eq!((ed.1["published_ms"].as_i64(), ed.1["old_edited"].as_bool(), ed.1["team"].as_str()), (Some(2000), Some(false), Some("t1")));
    }
    // `message.keep` with "inline": false: only the tag stays; a plain string is the struck-through old text.
    p.kept.lock().unwrap().keep(&json!({ "replace": true, "edited": [{ "id": "m2", "history": ["Hallo Welt"], "inline": false }] }));
    let mut l = new.clone();
    p.reapply_kept("s1", &mut l);
    let e = l.iter().find(|x| x.id == "m2").unwrap();
    assert!(e.history.is_empty() && e.current.is_empty() && e.edit_tag);
    p.kept.lock().unwrap().keep(&json!({ "replace": true, "edited": [{ "id": "m2", "history": ["Hallo Welt"] }] }));
    p.reapply_kept("s1", &mut l);
    assert_eq!(l.iter().find(|x| x.id == "m2").unwrap().history, [vec![("Hallo Welt".to_string(), SPAN_OLD)]]);
    let t = l.iter().find(|x| x.id == "m1").unwrap();
    assert!(t.deleted == 0 && !t.notice.is_empty(), "no longer kept: the tombstone is back");
    drop(p);
    let _ = std::fs::remove_dir_all(dir);
}
