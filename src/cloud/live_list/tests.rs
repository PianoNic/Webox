//! Synthetic activities only (no real data): ids, names and times are made up.
use super::*;
use std::sync::mpsc::Receiver;

/// A session that never goes over the network (texts are plain, `decrypt` passes them through).
pub(in crate::cloud) fn session() -> (Session, Receiver<Resp>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let (ktx, krx) = std::sync::mpsc::channel();
    let s = Session { hist: Default::default(), space_cover_urls: Default::default(), space_covers_done: Default::default(), full_desc: Default::default(), read_marks: Default::default(), covers_done: Default::default(), oracle_done: false, calls: Default::default(), img_jobs: vec![], images_done: Default::default(), uuids: Default::default(), avatars_done: Default::default(), agent: ureq::agent(), token: String::new(), my_email: "me@example.com".into(), kms_token: String::new(), kms_loop: ktx, svc: Default::default(), device: String::new(),
        user: "me".into(), kms: None, keys: Default::default(), kms_rx: krx, spaces: Default::default(), out: tx, cover: String::new(), keep: Default::default(), live: Default::default(), extra: Default::default(), typing: Default::default(), subs: Default::default() };
    (s, rx)
}

fn space(id: &str, last: &str, last_seen: &str) -> Space {
    Space { id: id.into(), title: "Planung".into(), last: last.into(), last_seen: last_seen.into(), unread: last > last_seen, ..Default::default() }
}
fn ack(space: &str, of: &str, at: &str, actor: &str) -> Value {
    json!({"id": format!("ack-{at}"), "verb": "acknowledge", "published": at, "actor": {"id": actor, "objectType": "person"},
        "object": {"id": of, "objectType": "activity"}, "target": {"id": space, "objectType": "conversation"}})
}
fn post(space: &str, id: &str, at: &str, actor: &str) -> Value {
    json!({"id": id, "verb": "post", "published": at, "actor": {"id": actor}, "object": {"objectType": "comment", "displayName": "Hallo"},
        "target": {"id": space, "objectType": "conversation"}})
}
fn drain(rx: &Receiver<Resp>) -> Vec<ListPatch> { rx.try_iter().filter_map(|r| match r { Resp::List(p) => Some(p), _ => None }).collect() }

#[test]
fn read_rule() {
    assert_eq!(read_after("2026-01-01T10:00:00.000Z", "2026-01-01T10:05:00.000Z", "2026-01-01T10:05:00.000Z"), Some(("2026-01-01T10:05:00.000Z".into(), false)));
    // Something newer than what was acknowledged stays unread.
    assert_eq!(read_after("2026-01-01T10:00:00.000Z", "2026-01-01T10:09:00.000Z", "2026-01-01T10:05:00.000Z"), Some(("2026-01-01T10:05:00.000Z".into(), true)));
    assert_eq!(read_after("2026-01-01T10:05:00.000Z", "2026-01-01T10:09:00.000Z", "2026-01-01T10:04:00.000Z"), None, "older ack: nothing");
    assert_eq!(read_after("", "x", ""), None);
}

#[test]
fn my_ack_from_another_device_clears_unread() {
    let (mut s, rx) = session();
    s.spaces.insert("sp".into(), space("sp", "2026-01-01T10:05:00.000Z", "2026-01-01T10:00:00.000Z"));
    s.merge_acts("sp", vec![post("sp", "m1", "2026-01-01T10:05:00.000Z", "other")]);
    s.incoming(&ack("sp", "m1", "2026-01-01T10:07:00.000Z", "me")).unwrap();
    let p = drain(&rx);
    assert!(matches!(&p[..], [ListPatch::Read { space, last_seen, unread: false }] if space == "sp" && last_seen == "2026-01-01T10:05:00.000Z"), "{p:?}");
    assert!(!s.spaces["sp"].unread);
    // The same ack again (my own receipt's echo): nothing new.
    s.incoming(&ack("sp", "m1", "2026-01-01T10:07:00.000Z", "me")).unwrap();
    assert!(drain(&rx).is_empty());
}

#[test]
fn ack_of_an_older_message_keeps_unread_and_uses_ack_time_when_uncached() {
    let (mut s, rx) = session();
    s.spaces.insert("sp".into(), space("sp", "2026-01-01T10:09:00.000Z", "2026-01-01T10:00:00.000Z"));
    // Not cached: the ack's own time.
    s.incoming(&ack("sp", "m-old", "2026-01-01T10:06:00.000Z", "me")).unwrap();
    assert!(matches!(&drain(&rx)[..], [ListPatch::Read { unread: true, last_seen, .. }] if last_seen == "2026-01-01T10:06:00.000Z"));
    // A live message after that, then my ack of it.
    s.incoming(&post("sp", "m2", "2026-01-01T10:12:00.000Z", "other")).unwrap();
    let _ = drain(&rx);
    assert_eq!(s.spaces["sp"].last, "2026-01-01T10:12:00.000Z", "the live message moved the last readable time");
    s.incoming(&ack("sp", "m2", "2026-01-01T10:13:00.000Z", "me")).unwrap();
    assert!(matches!(&drain(&rx)[..], [ListPatch::Read { unread: false, .. }]));
}

#[test]
fn others_acks_stay_read_marks() {
    let (mut s, rx) = session();
    s.spaces.insert("sp".into(), space("sp", "2026-01-01T10:05:00.000Z", "2026-01-01T10:00:00.000Z"));
    s.incoming(&ack("sp", "m1", "2026-01-01T10:07:00.000Z", "other")).unwrap();
    let r: Vec<Resp> = rx.try_iter().collect();
    assert!(r.iter().any(|r| matches!(r, Resp::ReadMarks(..))) && !r.iter().any(|r| matches!(r, Resp::List(_))));
    assert!(s.spaces["sp"].unread);
}

#[test]
fn classify_list_verbs() {
    let conv = |verb: &str, extra: Value| { let mut a = json!({"verb": verb, "object": {"id": "sp", "objectType": "conversation"}}); if let Some(m) = extra.as_object() { for (k, v) in m { a["object"][k] = v.clone(); } } a };
    assert_eq!(classify(&conv("update", json!({"displayName": "eyJ.jwe"})), false), vec![Change::Title("eyJ.jwe".into())]);
    assert_eq!(classify(&conv("update", json!({})), false), vec![], "an update without a title (e.g. a key update)");
    assert_eq!(classify(&conv("hide", json!({})), false), vec![Change::Hidden(true)]);
    assert_eq!(classify(&conv("unhide", json!({})), false), vec![Change::Hidden(false)]);
    assert_eq!(classify(&conv("favorite", json!({})), false), vec![Change::Favorite(true)]);
    assert_eq!(classify(&conv("unfavorite", json!({})), false), vec![Change::Favorite(false)]);
    assert_eq!(classify(&conv("lock", json!({})), false), vec![Change::Locked(true)]);
    assert_eq!(classify(&conv("unlock", json!({})), false), vec![Change::Locked(false)]);
    assert_eq!(classify(&conv("tag", json!({"tags": ["MESSAGE_NOTIFICATIONS_OFF", "MENTION_NOTIFICATIONS_ON"]})), false),
        vec![Change::Tags { add: true, tags: vec!["MESSAGE_NOTIFICATIONS_OFF".into(), "MENTION_NOTIFICATIONS_ON".into()] }]);
    assert_eq!(classify(&conv("untag", json!({"tags": ["FAVORITE"]})), false), vec![Change::Favorite(false)]);
    assert_eq!(classify(&conv("archive", json!({})), false), vec![Change::Reload]);
    // The team's general space gets a conversation added / removed.
    assert_eq!(classify(&json!({"verb": "add", "object": {"objectType": "conversation", "id": "x"}}), false), vec![Change::Reload]);
    let person = |verb: &str| json!({"verb": verb, "object": {"objectType": "person", "id": "p-1", "emailAddress": "ann@example.com"}});
    assert_eq!(classify(&person("assignModerator"), false), vec![Change::Moderator { ids: vec!["p-1".into(), "ann@example.com".into()], on: true }]);
    assert_eq!(classify(&person("unassignModerator"), false), vec![Change::Moderator { ids: vec!["p-1".into(), "ann@example.com".into()], on: false }]);
    assert_eq!(classify(&person("leave"), true), vec![Change::Removed]);
    assert_eq!(classify(&person("leave"), false), vec![], "someone else left: only the message row (fold.rs)");
    assert_eq!(classify(&post("sp", "m", "t", "x"), false), vec![]);
}

#[test]
fn mute_tags_apply_to_the_current_setting() {
    let off = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    // Webox's own write order: untag all four, then tag the new pair.
    let n = notify_after(Notify::All, false, &off(&spaces_ext::MUTE_TAGS));
    assert_eq!(n, Notify::Global);
    assert_eq!(notify_after(n, true, &off(&["MESSAGE_NOTIFICATIONS_OFF", "MENTION_NOTIFICATIONS_ON"])), Notify::Mentions);
    assert_eq!(notify_after(Notify::Global, true, &off(&["MESSAGE_NOTIFICATIONS_OFF", "MENTION_NOTIFICATIONS_OFF"])), Notify::Off);
}

/// A made-up 1:1 conversation as the single-conversation GET answers it (keys as the list answer's, see lists.rs).
fn conversation(id: &str) -> Value {
    json!({"id": id, "tags": ["ONE_ON_ONE"], "lastReadableActivityDate": "2026-01-02T08:00:00.000Z", "lastSeenActivityDate": "2026-01-01T08:00:00.000Z",
        "encryptionKeyUrl": "", "participants": {"items": [
            {"id": "me", "entryUUID": "me", "displayName": "Ich", "emailAddress": "me@example.com"},
            {"id": "00000000-0000-4000-8000-0000000000aa", "entryUUID": "00000000-0000-4000-8000-0000000000aa", "displayName": "Erika Beispiel", "emailAddress": "erika@example.com"}]}})
}

#[test]
fn a_new_conversation_joins_the_list() {
    let (mut s, rx) = session();
    assert!(s.add_conversation("new-1", &conversation("new-1")));
    let p = drain(&rx);
    assert!(matches!(&p[0], ListPatch::Upsert(sp) if sp.id == "new-1" && sp.title == "Erika Beispiel" && sp.direct && sp.unread), "{p:?}");
    assert!(matches!(&p[1], ListPatch::Info(i) if i.id == "new-1"));
    assert!(s.spaces.contains_key("new-1"), "known now: later activities of it don't fetch again");
    assert!(!s.add_conversation("other", &conversation("new-1")), "an answer for another id is ignored");
}

#[test]
fn unknown_conversation_fetch_is_not_repeated_and_skipped_for_other_verbs() {
    let (mut s, _rx) = session();
    // No service map in tests: the GET fails without leaving the machine.
    let hide = json!({"verb": "hide", "target": {"id": "x"}, "object": {"objectType": "conversation"}});
    assert!(!s.unknown_space("x", &hide));
    assert!(!s.live.asked.contains_key("x"), "a hide of a space I don't list fetches nothing");
    assert!(!s.unknown_space("x", &post("x", "m", "2026-01-01T10:00:00.000Z", "other")));
    let first = s.live.asked["x"];
    assert!(!s.unknown_space("x", &post("x", "m2", "2026-01-01T10:00:01.000Z", "other")));
    assert_eq!(s.live.asked["x"], first, "within a minute the failed fetch is not repeated");
}

fn on(space: &str, verb: &str, object: Value) -> Value {
    json!({"id": format!("{verb}-1"), "verb": verb, "published": "2026-01-01T11:00:00.000Z", "actor": {"id": "other"}, "object": object,
        "target": {"id": space, "objectType": "conversation"}})
}

#[test]
fn changes_made_elsewhere_patch_the_list() {
    let (mut s, rx) = session();
    s.spaces.insert("sp".into(), space("sp", "2026-01-01T10:00:00.000Z", "2026-01-01T10:00:00.000Z"));
    s.merge_acts("sp", vec![post("sp", "m1", "2026-01-01T10:00:00.000Z", "other")]); // opened here (cached)
    s.incoming(&on("sp", "update", json!({"objectType": "conversation", "displayName": "Neuer Name"}))).unwrap();
    s.incoming(&on("sp", "favorite", json!({"objectType": "conversation", "id": "sp"}))).unwrap();
    s.incoming(&on("sp", "lock", json!({"objectType": "conversation", "id": "sp"}))).unwrap();
    s.incoming(&on("sp", "assignModerator", json!({"objectType": "person", "id": "p-2"}))).unwrap();
    let p = drain(&rx);
    assert!(p.iter().any(|x| matches!(x, ListPatch::Title { space, title } if space == "sp" && title == "Neuer Name")), "{p:?}");
    assert!(p.iter().any(|x| matches!(x, ListPatch::Favorite(sp, true) if sp == "sp")));
    assert!(p.iter().any(|x| matches!(x, ListPatch::Locked(sp, true) if sp == "sp")));
    assert!(p.iter().any(|x| matches!(x, ListPatch::Moderator { on: true, ids, .. } if ids == &vec!["p-2".to_string()])));
    assert_eq!(s.spaces["sp"].title, "Neuer Name");
    assert!(s.spaces["sp"].favorite);
    // Removed by someone else: the space leaves the list.
    s.incoming(&on("sp", "leave", json!({"objectType": "person", "id": "me"}))).unwrap();
    assert!(drain(&rx).iter().any(|x| matches!(x, ListPatch::Remove(sp) if sp == "sp")));
    assert!(!s.spaces.contains_key("sp"));
}

#[test]
fn a_direct_chat_keeps_the_other_persons_name() {
    let (mut s, rx) = session();
    s.spaces.insert("d".into(), Space { direct: true, ..space("d", "2026-01-01T10:00:00.000Z", "2026-01-01T10:00:00.000Z") });
    s.list_activity("d", &on("d", "update", json!({"objectType": "conversation", "displayName": "Egal"})));
    assert!(drain(&rx).is_empty());
}

#[test]
fn team_changes_reload_once_debounced() {
    let (mut s, rx) = session();
    s.spaces.insert("g".into(), space("g", "2026-01-01T10:00:00.000Z", "2026-01-01T10:00:00.000Z"));
    s.list_activity("g", &on("g", "add", json!({"objectType": "conversation", "id": "c-9"})));
    s.list_activity("g", &on("g", "remove", json!({"objectType": "conversation", "id": "c-8"})));
    assert!(drain(&rx).is_empty(), "no patch, a reload instead");
    assert!(s.live.reload_at.is_some());
    s.list_reload_due();
    assert!(s.live.reload_at.is_some(), "not before the debounce");
}

#[test]
fn events_in_an_unopened_space_load_nothing() {
    let (mut s, rx) = session();
    s.spaces.insert("sp".into(), space("sp", "2026-01-01T10:00:00.000Z", "2026-01-01T10:00:00.000Z"));
    // A reaction, an edit and a hide in a space never opened here: no history load (its request would fail in tests and come
    // back as Err), only the list patch for the hide.
    let reaction = json!({"id": "r1", "verb": "add", "published": "2026-01-01T10:01:00.000Z", "actor": {"id": "other"}, "parent": {"id": "m1", "type": "reaction"},
        "object": {"objectType": "reaction2", "displayName": "heart"}, "target": {"id": "sp"}});
    let edit = json!({"id": "e1", "verb": "post", "published": "2026-01-01T10:02:00.000Z", "actor": {"id": "other"}, "parent": {"id": "m1", "type": "edit"},
        "object": {"objectType": "comment", "displayName": "neu"}, "target": {"id": "sp"}});
    assert!(s.incoming(&reaction).is_ok());
    assert!(s.incoming(&edit).is_ok());
    assert!(s.incoming(&on("sp", "hide", json!({"objectType": "conversation", "id": "sp"}))).is_ok());
    assert!(!s.hist.acts.contains_key("sp"), "nothing loaded");
    let r: Vec<Resp> = rx.try_iter().collect();
    assert!(!r.iter().any(|r| matches!(r, Resp::Messages(..) | Resp::Error(_))));
    assert!(r.iter().any(|r| matches!(r, Resp::List(ListPatch::Hidden(_, true)))));
    // A thread reply still arrives as a new message.
    let reply = json!({"id": "re1", "verb": "post", "published": "2026-01-01T10:03:00.000Z", "actor": {"id": "other"}, "parent": {"id": "m1", "type": "reply"},
        "object": {"objectType": "comment", "displayName": "Antwort"}, "target": {"id": "sp"}});
    s.incoming(&reply).unwrap();
    assert!(rx.try_iter().any(|r| matches!(r, Resp::Incoming(sp, m) if sp == "sp" && m.parent == "m1")));
}

#[test]
fn opens_unknown_verbs() {
    for v in ["post", "share", "add", "create"] { assert!(opens_unknown(v)); }
    for v in ["acknowledge", "hide", "leave", "update", "delete"] { assert!(!opens_unknown(v)); }
}
