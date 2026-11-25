//! The data API end to end: the store filled from fixture app state, queries, `data.get`, subscriptions and their diffs,
//! the caps, and that a frame without changes does no work.
use super::build::tests::{message, person, Fixture};
use super::model::Kind;
use super::{Fingerprint, Hub, Marks};
use serde_json::{json, Value};

/// A hub in use by plugin "p", filled from `f`.
fn hub(f: &Fixture) -> Hub {
    let h = Hub::default();
    h.use_by("p");
    let m = h.plan(Marks::default(), Fingerprint::default()).expect("first frame builds everything");
    h.apply(&m, &f.source());
    h
}
/// Applies `f` as a frame that changed the kinds of `m`.
fn frame(h: &Hub, f: &Fixture, m: Marks) {
    if let Some(m) = h.plan(m, Fingerprint::default()) { h.apply(&m, &f.source()); }
}
fn q(h: &Hub, a: Value) -> Value { h.query("p", &a, true) }
fn ids(v: &Value) -> Vec<String> { v["items"].as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap().to_string()).collect() }
/// Lets the subscription rate limit pass.
fn later(h: &Hub) { for x in super::lock(&h.0.st).subs.values_mut().flatten() { x.last_run = None; } }

#[test]
fn queries_filter_sort_limit_search() {
    let f = Fixture::new();
    let h = hub(&f);
    // hidden spaces only when the query names `hidden`; sort descending
    assert_eq!(ids(&q(&h, json!({ "query": "spaces sort -last_activity" }))), ["s-anna", "s-dev"]);
    assert_eq!(ids(&q(&h, json!({ "query": "spaces where hidden" }))), ["s-old"]);
    // a relation: the peer's presence; @me; params
    assert_eq!(ids(&q(&h, json!({ "query": "spaces where peer.presence.state == 'meeting'" }))), ["s-anna"]);
    assert_eq!(ids(&q(&h, json!({ "query": "messages($s) where author == @me", "params": { "s": "s-dev" } }))), ["m1"]);
    assert_eq!(ids(&q(&h, json!({ "query": "messages('s-dev') where kind == 'message' and at > 1500 and at < now() sort -at limit 2" }))), ["m5", "m3"]);
    assert_eq!(ids(&q(&h, json!({ "query": "thread('m2')" }))), ["m2", "m3"]);
    // search: matches only, prefix matches first, accents folded
    assert_eq!(ids(&q(&h, json!({ "query": "contacts search $q in name sort name", "params": { "q": "ber" } }))), ["u-anna"]);
    assert_eq!(ids(&q(&h, json!({ "query": "people search 'a' in name sort name" }))), ["u-me", "u-anna", "u-ben", "email:carl@y.com", "u-dir"],
        "names (or a word of them) starting with a first, then those containing it");
    assert_eq!(ids(&q(&h, json!({ "query": "people where name ~ 'DORÄ'" }))), ["u-dir"]);
    assert_eq!(ids(&q(&h, json!({ "query": "people where name ~ 'dorx'" }))), Vec::<String>::new());
    assert_eq!(ids(&q(&h, json!({ "query": "people where name ~ 'dóra'" }))), ["u-dir"]);
    // functions and lists
    assert_eq!(ids(&q(&h, json!({ "query": "spaces where len(members) == 3 and any(members, presence.state == 'dnd')" }))), ["s-dev"]);
    assert_eq!(ids(&q(&h, json!({ "query": "messages('s-dev') where any(reactions, it.mine)" }))), ["m2"]);
    // contacts leave out me, directory people and strangers
    let mut c = ids(&q(&h, json!({ "query": "contacts" })));
    c.sort();
    assert_eq!(c, ["u-anna", "u-ben"]);
    // limit / cursor paging over the result, total before paging
    let p1 = q(&h, json!({ "query": "people sort name", "limit": 2 }));
    assert_eq!((p1["total"].clone(), p1["cursor"].clone()), (json!(6), json!("2")));
    let p2 = q(&h, json!({ "query": "people sort name", "limit": 2, "cursor": "2" }));
    let p3 = q(&h, json!({ "query": "people sort name", "limit": 2, "cursor": "4" }));
    assert_eq!(ids(&p1).into_iter().chain(ids(&p2)).chain(ids(&p3)).count(), 6);
    assert!(p3.get("cursor").is_none());
    // group by: groups in key order, with their positions
    let g = q(&h, json!({ "query": "messages('s-dev') group by kind sort at" }));
    assert_eq!(g["groups"], json!([{ "key": "deleted", "count": 1, "start": 0 }, { "key": "message", "count": 4, "start": 1 }]));
    // several parts: concatenated, each item says its entity
    let r = q(&h, json!({ "query": "teams sort name; spaces where favorite sort title", "fields": ["name", "title"] }));
    assert_eq!(r["items"], json!([{ "id": "t1", "entity": "team", "name": "Product Team" }, { "id": "s-anna", "entity": "space", "title": "Anna Berg" }]));
    // field selection through relations, nested in the answer; absent fields left out
    let r = q(&h, json!({ "query": "spaces where id == 's-anna'", "fields": ["title", "peer.name", "peer.presence.state", "team.name"] }));
    assert_eq!(r["items"], json!([{ "id": "s-anna", "title": "Anna Berg", "peer": { "name": "Anna Berg", "presence": { "state": "meeting" } } }]));
    // the JSON form
    assert_eq!(ids(&q(&h, json!({ "query": { "from": "spaces", "where": "kind == 'group'", "sort": ["title"] } }))), ["s-dev"]);
    // errors are answers, not panics
    assert_eq!(q(&h, json!({ "query": "peple" }))["error"].as_str().unwrap().split(';').next().unwrap(), "query: column 1: unknown collection 'peple' (did you mean 'people'?)");
    assert!(q(&h, json!({ "query": "people", "fields": ["nope"] }))["error"].as_str().unwrap().starts_with("field \"nope\": "));
    assert!(q(&h, json!({ "query": "people", "limit": 5001 }))["error"].as_str().unwrap().contains("up to 5000"));
    assert!(q(&h, json!({ "query": "people", "cursor": "x" }))["error"].is_string());
    assert!(q(&h, json!({}))["error"].is_string());
}

#[test]
fn get_and_model() {
    let f = Fixture::new();
    let h = hub(&f);
    let r = h.get("p", &json!({ "entity": "person", "id": "u-anna", "fields": ["name", "presence.state", "presence.until", "direct.title"] }), true);
    assert_eq!(r["item"], json!({ "id": "u-anna", "name": "Anna Berg", "presence": { "state": "meeting", "until": 9999 }, "direct": { "title": "Anna Berg" } }));
    assert!(r["rev"].as_u64().unwrap() >= 1);
    // every stored field by default (reserved and volatile ones left out)
    let me = h.get("p", &json!({ "entity": "me" }), true)["item"].clone();
    assert_eq!((me["person"].clone(), me["dnd"].clone(), me.get("cameras").is_none()), (json!("u-me"), json!(false), true));
    let call = h.get("p", &json!({ "entity": "call", "id": "call:1", "fields": ["state", "me.muted", "stats"] }), true);
    assert_eq!(call["item"], json!({ "id": "call:1", "state": "connected", "me": { "muted": true } }), "no statistics while none are taken");
    assert_eq!(h.get("p", &json!({ "entity": "person", "id": "nobody" }), true)["error"], json!("no person with id \"nobody\""));
    assert!(h.get("p", &json!({ "entity": "persn", "id": "x" }), true)["error"].as_str().unwrap().starts_with("unknown entity \"persn\""));
    let m = super::api::model();
    assert_eq!(m["version"], json!("webox-data/1"));
    assert!(m["entities"]["space"]["fields"].as_array().unwrap().iter().any(|f| f["name"] == "peer" && f["relation"] == "person"));
    // reads are recorded by scope
    assert!(h.scopes_used("p").contains(&"people.presence"));
}

#[test]
fn store_records_changed_entities_and_fields() {
    let mut f = Fixture::new();
    let h = hub(&f);
    let rev = q(&h, json!({ "query": "me" }))["rev"].as_u64().unwrap();
    // nothing changed in the app: a rebuild changes nothing, the revision stays
    frame(&h, &f, Marks::all());
    assert_eq!(q(&h, json!({ "query": "me" }))["rev"].as_u64().unwrap(), rev);
    // one space renamed: that row alone gets the new revision
    f.spaces[1].title = "Dev".into();
    let s = super::lock(&h.0.st);
    drop(s);
    frame(&h, &f, Marks::of(&[Kind::Space]));
    let st = super::lock(&h.0.st);
    assert_eq!(st.store.rev, rev + 1);
    assert_eq!(st.store.get(Kind::Space, "s-dev").unwrap().rev, rev + 1);
    assert!(st.store.get(Kind::Space, "s-anna").unwrap().rev <= rev);
}

#[test]
fn subscriptions_send_only_what_changed() {
    let mut f = Fixture::new();
    let h = hub(&f);
    // a plugin must listen to data.changed
    assert!(h.subscribe("p", &json!({ "id": "s", "query": "spaces" }), true, false)["error"].as_str().unwrap().contains("data.changed"));
    let first = h.subscribe("p", &json!({ "id": "online", "query": "contacts where presence.online sort name", "fields": ["name", "presence.state"] }), true, true);
    assert_eq!(first["sub"], json!("online"));
    assert_eq!(first["items"], json!([{ "id": "u-anna", "name": "Anna Berg", "presence": { "state": "meeting" } }]));

    // a field nobody subscribed to changes: no event, the subscription does not even run
    let work = h.work();
    f.last_active.insert("u-anna".into(), 777);
    frame(&h, &f, Marks::of(&[Kind::Presence]));
    later(&h);
    assert!(h.take_events().is_empty());
    assert_eq!(h.work(), work + 1, "the store was rebuilt, the subscription not run");

    // update: only the changed field
    f.people[0].status = "active".into();
    frame(&h, &f, Marks::of(&[Kind::Person, Kind::Presence]));
    let ev = h.take_events();
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].0, "p");
    let e: Value = serde_json::from_str(&ev[0].1).unwrap();
    assert_eq!(e, json!({ "sub": "online", "rev": 1, "ops": [{ "op": "update", "id": "u-anna", "set": { "presence.state": "active" } }] }));

    // insert: Ben comes online (sorted after Anna)
    later(&h);
    f.people[1].status = "active".into();
    frame(&h, &f, Marks::of(&[Kind::Person, Kind::Presence]));
    let e: Value = serde_json::from_str(&h.take_events()[0].1).unwrap();
    assert_eq!(e["rev"], json!(2));
    assert_eq!(e["ops"], json!([{ "op": "insert", "at": 1, "item": { "id": "u-ben", "name": "Ben Carter", "presence": { "state": "active" } } }]));

    // move: Anna renamed to Zoe sorts last
    later(&h);
    f.people[0].name = "Zoe Berg".into();
    for s in &mut f.spaces { for p in &mut s.participants { if p.id == "u-anna" { p.name = "Zoe Berg".into(); } } }
    frame(&h, &f, Marks::of(&[Kind::Person, Kind::Presence]));
    let e: Value = serde_json::from_str(&h.take_events()[0].1).unwrap();
    assert_eq!(e["ops"], json!([{ "op": "move", "id": "u-ben", "to": 0 }, { "op": "update", "id": "u-anna", "set": { "name": "Zoe Berg" } }]));

    // remove: Ben goes offline
    later(&h);
    f.people[1].status = "inactive".into();
    frame(&h, &f, Marks::of(&[Kind::Person, Kind::Presence]));
    let e: Value = serde_json::from_str(&h.take_events()[0].1).unwrap();
    assert_eq!(e["ops"], json!([{ "op": "remove", "id": "u-ben" }]));

    // rate limit: a second change within 100 ms waits; it is sent once its turn comes
    f.people[1].status = "active".into();
    frame(&h, &f, Marks::of(&[Kind::Person, Kind::Presence]));
    assert!(h.take_events().is_empty(), "held back by the rate limit");
    later(&h);
    frame(&h, &f, Marks::default());
    assert_eq!(serde_json::from_str::<Value>(&h.take_events()[0].1).unwrap()["ops"][0]["op"], json!("insert"));

    // unsubscribe: nothing more
    assert_eq!(h.unsubscribe("p", &json!({ "id": "online" })), json!({}));
    assert!(h.unsubscribe("p", &json!({ "id": "online" }))["error"].is_string());
    f.people[1].status = "inactive".into();
    later(&h);
    frame(&h, &f, Marks::of(&[Kind::Person, Kind::Presence]));
    assert!(h.take_events().is_empty());
}

#[test]
fn messages_subscription_and_rekey() {
    let mut f = Fixture::new();
    let h = hub(&f);
    h.subscribe("p", &json!({ "id": "chat", "query": "messages('s-dev') where kind == 'message' sort at", "fields": ["text", "author"] }), true, true);
    // a new message in another space: not this subscription's business
    f.messages.get_mut("s-anna").unwrap().push(message("d2", "anna@x.com", "Anna Berg", "x", 9));
    frame(&h, &f, Marks::default().space("s-anna"));
    later(&h);
    assert!(h.take_events().is_empty());
    // a message by someone only known by e-mail arrives, then that person's real id
    f.messages.get_mut("s-dev").unwrap().push(message("m6", "dan@y.com", "Dan", "new", 5000));
    frame(&h, &f, Marks::of(&[Kind::Person, Kind::Presence]).space("s-dev"));
    let e: Value = serde_json::from_str(&h.take_events()[0].1).unwrap();
    assert_eq!(e["ops"], json!([{ "op": "insert", "at": 4, "item": { "id": "m6", "text": "new", "author": "email:dan@y.com" } }]));
    later(&h);
    h.subscribe("p", &json!({ "id": "people", "query": "people where name == 'Dan'", "fields": ["name"] }), true, true);
    f.people.push(person("u-dan", "Dan", "dan@y.com", "active"));
    frame(&h, &f, Marks::of(&[Kind::Person, Kind::Presence]).space("s-dev"));
    let mut evs: Vec<Value> = h.take_events().iter().map(|e| serde_json::from_str(&e.1).unwrap()).collect();
    evs.sort_by_key(|e| e["sub"].as_str().unwrap().to_string());
    assert_eq!(evs[0], json!({ "sub": "chat", "rev": 2, "ops": [{ "op": "update", "id": "m6", "set": { "author": "u-dan" } }] }));
    assert_eq!(evs[1], json!({ "sub": "people", "rev": 1, "ops": [{ "op": "rekey", "from": "email:dan@y.com", "to": "u-dan" }] }));
}

#[test]
fn caps_hold() {
    let mut f = Fixture::new();
    let h = hub(&f);
    for i in 0..super::SUBS_MAX { assert!(h.subscribe("p", &json!({ "id": format!("s{i}"), "query": "spaces" }), true, true).get("error").is_none()); }
    assert_eq!(h.subscribe("p", &json!({ "id": "one-more", "query": "spaces" }), true, true)["error"], json!("at most 32 subscriptions per plugin"));
    assert!(h.subscribe("p", &json!({ "id": "s0", "query": "teams" }), true, true).get("error").is_none(), "replacing one is fine");
    assert!(h.subscribe("p", &json!({ "id": "x".repeat(65), "query": "spaces" }), true, true)["error"].is_string());
    for i in 0..super::SUBS_MAX { h.unsubscribe("p", &json!({ "id": format!("s{i}") })); }
    // more than 1000 operations in one event: a reset instead
    f.people = (0..1100).map(|i| person(&format!("u{i}"), &format!("P{i}"), &format!("p{i}@x.com"), "active")).collect();
    frame(&h, &f, Marks::of(&[Kind::Person, Kind::Presence]));
    h.subscribe("p", &json!({ "id": "all", "query": "contacts limit 5000", "fields": ["name"], "limit": 5000 }), true, true);
    f.people.clear();
    f.spaces.clear();
    frame(&h, &f, Marks::of(&[Kind::Person, Kind::Presence, Kind::Space]));
    let e: Value = serde_json::from_str(&h.take_events()[0].1).unwrap();
    assert_eq!(e["ops"], json!([{ "op": "reset" }]));
    // a query never produces more than 5000 items
    assert!(q(&h, json!({ "query": "people limit 6000" }))["error"].as_str().unwrap().contains("maximum of 5000"));
}

#[test]
fn an_idle_frame_does_no_store_work() {
    let mut f = Fixture::new();
    let h = hub(&f);
    h.subscribe("p", &json!({ "id": "s", "query": "spaces" }), true, true);
    let fp = Fingerprint { app: 1, spaces: 2, ..Default::default() };
    let _ = h.plan(Marks::default(), fp).map(|m| h.apply(&m, &f.source()));
    let work = h.work();
    // frames with no answer and the same window state: nothing is planned, built or run
    for _ in 0..100 { assert!(h.plan(Marks::default(), fp).is_none()); }
    assert_eq!(h.work(), work);
    // the window changed something (a space opened): only then
    f.app.selected = Some("s-anna".into());
    let m = h.plan(Marks::default(), Fingerprint { app: 9, ..fp }).expect("the window's change is noticed");
    assert_eq!(m, Marks::of(&[Kind::App]));
    h.apply(&m, &f.source());
    assert_eq!(h.work(), work + 1, "one rebuild; the space subscription does not read the app state");
}

#[test]
fn unused_store_is_off_and_empty() {
    let f = Fixture::new();
    let h = Hub::default();
    assert!(!h.active());
    let h2 = hub(&f);
    assert!(h2.active());
    h2.subscribe("p", &json!({ "id": "s", "query": "spaces" }), true, true);
    h2.forget("p");
    assert!(!h2.active());
    let st = super::lock(&h2.0.st);
    assert!(st.subs.is_empty() && !st.store.ready && st.store.table(Kind::Space).items.is_empty(), "memory is given back");
    drop(st);
    // a call from a plugin thread switches it on again and waits for the first fill, at most a short while
    let t = std::time::Instant::now();
    assert_eq!(h.query("p", &json!({ "query": "spaces" }), false)["ready"], json!(false));
    assert!(t.elapsed() >= std::time::Duration::from_secs(1) && h.active());
}
