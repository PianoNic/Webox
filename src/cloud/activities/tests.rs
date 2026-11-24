use serde_json::json;
#[test]
fn friendly_messages() {
    assert_eq!(super::friendly_failure(&json!({"verb": "add", "object": {"objectType": "reaction2"}})), "Reaktion konnte nicht gesendet werden");
    assert_eq!(super::friendly_failure(&json!({"verb": "post", "object": {"objectType": "comment"}})), "Nachricht konnte nicht gesendet werden");
}

/// The posted `object` for `# 😀`, plain text, and mentions; `ENC(..)` stands for the JWE.
#[test]
fn posted_comment_objects() {
    use crate::richtext::{render, MentionRef};
    let mut enc = |t: &str| -> Result<String, String> { Ok(format!("ENC({t})")) };
    // Formatted: displayName = text content, content = HTML, no mentions key.
    let o = super::comment_object(&render("# 😀", true, &[]), "# 😀", &mut enc).unwrap();
    assert_eq!(o, json!({"objectType": "comment", "displayName": "ENC(😀)", "content": "ENC(<h1>😀</h1>)"}));
    // Plain text: displayName only (real plain messages have no `content`).
    let o = super::comment_object(&render("hello\nworld", true, &[]), "hello\nworld", &mut enc).unwrap();
    assert_eq!(o, json!({"objectType": "comment", "displayName": "ENC(hello\nworld)"}));
    // Markdown off: the syntax is literal.
    let o = super::comment_object(&render("# x", false, &[]), "# x", &mut enc).unwrap();
    assert_eq!(o, json!({"objectType": "comment", "displayName": "ENC(# x)"}));
    // Mentions: content + mentions.items[{id uuid, objectType person}], @all as a group.
    let ms = [MentionRef { id: "uuid-1".into(), name: "Mara Lind".into(), all: false }, MentionRef { id: String::new(), name: "all".into(), all: true }];
    let o = super::comment_object(&render("@Mara Lind @all hi", false, &ms), "@Mara Lind @all hi", &mut enc).unwrap();
    assert_eq!(o["displayName"], "ENC(Mara Lind all hi)");
    let c = o["content"].as_str().unwrap();
    assert!(c.contains("data-object-id=\"uuid-1\"") && c.contains(">Mara Lind</spark-mention>"), "{c}");
    assert_eq!(o["mentions"]["items"], json!([{"id": "uuid-1", "objectType": "person"}]));
    assert_eq!(o["groupMentions"], json!({"items": [{"groupType": "all", "objectType": "groupMention"}]}));
    assert!(o["mentions"].get("groups").is_none());
    // @all alone: groupMentions only, no empty `mentions`.
    let o = super::comment_object(&render("@all hi", false, &ms[1..]), "@all hi", &mut enc).unwrap();
    assert!(o.get("mentions").is_none() && o["groupMentions"]["items"][0]["groupType"] == "all", "{o}");
    // A mention whose text was deleted is not announced.
    let o = super::comment_object(&render("no mention", false, &ms), "no mention", &mut enc).unwrap();
    assert!(o.get("mentions").is_none() && o.get("content").is_none());
}

/// A Giphy share in the shape real ones have; `SCR(..)` stands for the JWE of the scr JSON.
#[test]
fn gif_share_has_the_real_shape() {
    // One endpoint only (no /activities attempt, no personRefresh).
    assert_eq!(super::GIF_SHARE_PATH, "/content?transcode=true&async=false");
    use serde_json::Value;
    use crate::jose;
    let mut enc = |t: &str| -> Result<String, String> { Ok(format!("ENC({t})")) };
    let mut scr = |v: &Value| -> Result<String, String> { Ok(format!("SCR{v}")) };
    let b = super::gif_share_body("kms://k/keys/1", "Happy Cat", "https://media3.giphy.com/media/abc/giphy.gif", 480, 270, 123456, "file-uuid", "2026-11-16T10:00:00.000Z", &mut enc, &mut scr).unwrap();
    assert_eq!(b["verb"], "share");
    assert_eq!(b["encryptionKeyUrl"], "kms://k/keys/1");
    let o = &b["object"];
    assert_eq!((o["objectType"].as_str(), o["contentCategory"].as_str()), (Some("content"), Some("images")));
    let f = &o["files"]["items"][0];
    assert_eq!((f["objectType"].as_str(), f["mimeType"].as_str(), f["url"].as_str()), (Some("file"), Some("image/gif"), Some("https://giphy.com")));
    assert_eq!((f["id"].as_str(), f["displayName"].as_str(), f["fileSize"].as_u64(), f["encryptionKeyUrl"].as_str()), (Some("file-uuid"), Some("ENC(Happy Cat)"), Some(123456), Some("kms://k/keys/1")));
    assert_eq!((f["image"]["width"].as_u64(), f["image"]["height"].as_u64(), f["image"]["url"].as_str()), (Some(480), Some(270), Some("https://giphy.com")));
    // Both scrs carry {aad, enc, iv, key, loc} with the media url as loc (the keys of a real giphy scr).
    for s in [&f["scr"], &f["image"]["scr"]] {
        let v: Value = serde_json::from_str(s.as_str().unwrap().strip_prefix("SCR").unwrap()).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect(); keys.sort();
        assert_eq!(keys, ["aad", "enc", "iv", "key", "loc"]);
        assert_eq!(v["loc"], "https://media3.giphy.com/media/abc/giphy.gif");
        assert_eq!(v["enc"], "A256GCM");
        assert_eq!(jose::unb64(v["key"].as_str().unwrap()).unwrap().len(), 32);
        assert_eq!(jose::unb64(v["iv"].as_str().unwrap()).unwrap().len(), 12);
    }
    // No title → a file name; the received side can read what we build.
    let b = super::gif_share_body("k", "  ", "https://media.giphy.com/x.gif", 1, 1, 0, "i", "t", &mut enc, &mut scr).unwrap();
    assert_eq!(b["object"]["files"]["items"][0]["displayName"], "ENC(giphy.gif)");
}

/// Paging boundaries: the older page repeats the boundary activity (dedupe), arrives out of order, and the reaction summary
/// for a message that only exists in the older page lives in the newer page.
#[test]
fn older_pages_merge_and_refold() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (ktx, krx) = std::sync::mpsc::channel();
    let mut s = super::Session { hist: Default::default(), space_cover_urls: Default::default(), space_covers_done: Default::default(), full_desc: Default::default(), read_marks: Default::default(), covers_done: Default::default(), oracle_done: false, calls: Default::default(), img_jobs: vec![], images_done: Default::default(), uuids: Default::default(), avatars_done: Default::default(), agent: ureq::agent(), token: String::new(), my_email: String::new(), kms_token: String::new(), kms_loop: ktx, svc: Default::default(), device: String::new(),
        user: "me".into(), kms: None, keys: Default::default(), kms_rx: krx, spaces: Default::default(), out: tx, cover: String::new(), keep: Default::default(), live: Default::default(), extra: Default::default(), typing: Default::default(), subs: Default::default() };
    let post = |id: &str, t: &str| json!({"id": id, "verb": "post", "object": {"displayName": id, "objectType": "comment"}, "actor": {"id": "x"}, "published": format!("2026-01-01T{t}.000Z")});
    let summary = json!({"id": "s1", "verb": "add", "published": "2026-01-01T10:04:00.000Z", "parent": {"id": "m1", "type": "reactionSummary"},
        "object": {"objectType": "reaction2Summary", "reactions": [{"displayName": "thumbsup", "count": 1, "users": [{"id": "u1"}]}]}, "actor": {"id": "x"}});
    // Newest page first: m3 and the summary of m1 (m1 itself isn't loaded yet).
    assert_eq!(s.merge_acts("sp", vec![summary.clone(), post("m3", "10:03:00")]), 2);
    let ids = |s: &mut super::Session| -> Vec<String> { let a = s.hist.acts["sp"].clone(); s.fold(&a, "").iter().map(|m| m.id.clone()).collect() };
    assert_eq!(ids(&mut s), ["m3"]);
    // The older page, newest first as the service sends it, repeating the boundary activity m3.
    assert_eq!(s.merge_acts("sp", vec![post("m3", "10:03:00"), post("m2", "10:02:00"), post("m1", "10:01:00")]), 2);
    let a = s.hist.acts["sp"].clone();
    let msgs = s.fold(&a, "");
    assert_eq!(msgs.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["m1", "m2", "m3"]); // chronological, no duplicate
    assert_eq!(msgs[0].reactions.len(), 1); // the summary from the newer page attached to the older message
    assert_eq!((msgs[0].reactions[0].name.as_str(), msgs[0].reactions[0].count), ("thumbsup", 1));
    assert_eq!(msgs.iter().position(|m| m.id == "m3"), Some(2)); // = prepended count when m3 was the first message before
    // An empty page adds nothing (the caller then reports reached_start).
    assert_eq!(s.merge_acts("sp", vec![]), 0);
    assert_eq!(s.merge_acts("sp", vec![post("m1", "10:01:00")]), 0);
}

/// GIF files are fetched as the file itself (animated) with the still as fallback; other pictures keep using the thumbnail.
#[test]
fn gifs_use_the_file_not_the_still() {
    let gif = json!({"mimeType": "image/gif", "url": "https://giphy.com", "scr": "FILE", "image": {"url": "https://files/thumb", "scr": "STILL"}});
    assert_eq!(super::image_source(&gif, "x.gif"), ("https://giphy.com".into(), "FILE".into(), Some("STILL".into())));
    // Detected by name or by the thumbnail's own mime type as well.
    let by_name = json!({"mimeType": "application/octet-stream", "url": "u", "scr": "F", "image": {"url": "t", "scr": "S"}});
    assert_eq!(super::image_source(&by_name, "Party.GIF").2, Some("S".into()));
    let png = json!({"mimeType": "image/png", "url": "https://files/f", "scr": "FILE", "image": {"url": "https://files/thumb", "scr": "STILL"}});
    assert_eq!(super::image_source(&png, "a.png"), ("https://files/thumb".into(), "STILL".into(), None));
    let bare = json!({"mimeType": "image/png", "url": "https://files/f", "scr": "FILE"});
    assert_eq!(super::image_source(&bare, "a.png"), ("https://files/f".into(), "FILE".into(), None));
    let gif_no_still = json!({"mimeType": "image/gif", "url": "u", "scr": "FILE"});
    assert_eq!(super::image_source(&gif_no_still, "a.gif").2, None);
}

#[test]
fn team_colours() {
    assert_eq!(super::super::parse_team_color("#FE8216"), Some((0xFE, 0x82, 0x16)));
    assert_eq!(super::super::parse_team_color("#FF2E8BE6"), Some((0x2E, 0x8B, 0xE6)));
    assert_eq!(super::super::parse_team_color("orange"), None);
}

/// Giphy shares all carry url "https://giphy.com": different messages (or scr) must still get different keys.
#[test]
fn attachment_keys_are_unique() {
    let k = |a: &str, i: usize, u: &str, s: &str| super::attachment_key(a, i, u, s);
    assert_ne!(k("m1", 0, "https://giphy.com", "scrA"), k("m2", 0, "https://giphy.com", "scrA")); // other message
    assert_ne!(k("m1", 0, "https://giphy.com", "scrA"), k("m1", 0, "https://giphy.com", "scrB")); // other scr
    assert_ne!(k("m1", 0, "https://giphy.com", "scrA"), k("m1", 1, "https://giphy.com", "scrA")); // other file in the message
    assert_eq!(k("m1", 0, "u", "s"), k("m1", 0, "u", "s")); // stable
}

/// Anonymized shapes of the non-message activities in a real listing.
#[test]
fn notices() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (ktx, krx) = std::sync::mpsc::channel();
    let mut s = super::Session { hist: Default::default(), space_cover_urls: Default::default(), space_covers_done: Default::default(), full_desc: Default::default(), read_marks: Default::default(), covers_done: Default::default(), oracle_done: false, calls: Default::default(), img_jobs: vec![], images_done: Default::default(), uuids: Default::default(), avatars_done: Default::default(), agent: ureq::agent(), token: String::new(), my_email: "me@example.com".into(), kms_token: String::new(), kms_loop: ktx, svc: Default::default(), device: String::new(),
        user: "me-uuid".into(), kms: None, keys: Default::default(), kms_rx: krx, spaces: Default::default(), out: tx, cover: String::new(), keep: Default::default(), live: Default::default(), extra: Default::default(), typing: Default::default(), subs: Default::default() };
    let call = |id: &str, t: &str, secs: u64, init_me: bool| json!({"id": id, "verb": "update", "published": format!("2026-01-01T{t}.000Z"),
        "object": {"objectType": "locusSessionSummary", "duration": secs, "participants": {"items": [
            {"email": "me@example.com", "id": "me-uuid", "isInitiator": init_me, "state": "LEFT"},
            {"email": "a@example.com", "id": "a-uuid", "isInitiator": !init_me, "state": "LEFT", "person": {"displayName": "Anna Beispiel"}}]}},
        "target": {"tags": ["ONE_ON_ONE"]}});
    let acts = vec![
        call("c1", "16:11:00", 0, true), call("c2", "16:20:00", 0, false), call("c3", "16:30:00", 125, true),
        json!({"id": "l", "verb": "leave", "published": "2026-01-01T17:00:00.000Z", "actor": {"id": "t", "entryUUID": "t"}, "object": {"objectType": "person", "id": "t", "displayName": "Tom Muster"}, "target": {"tags": []}}),
        json!({"id": "cr", "verb": "create", "published": "2026-01-01T09:00:00.000Z", "actor": {"displayName": "Zoe"}, "object": {"objectType": "conversation"}}),
        json!({"id": "ad1", "verb": "add", "published": "2026-01-01T09:00:01.000Z", "actor": {"displayName": "Zoe", "id": "z"}, "object": {"objectType": "person", "id": "x", "displayName": "X"}, "target": {"tags": []}}),
        json!({"id": "ad2", "verb": "add", "published": "2026-01-01T18:00:00.000Z", "actor": {"displayName": "Zoe", "id": "z"}, "object": {"objectType": "person", "id": "y", "displayName": "Yan"}, "target": {"tags": []}}),
        json!({"id": "ad3", "verb": "add", "published": "2026-01-01T18:00:00.000Z", "actor": {"displayName": "Zoe", "id": "z"}, "object": {"objectType": "person", "id": "y", "displayName": "Yan"}, "target": {"tags": ["ONE_ON_ONE"]}}),
    ];
    // The active language pack decides the words: the expectations go through the same QObject strings (in German e.g.
    // "Anna Beispiel war nicht erreichbar.", "Sie haben einen Anruf verpasst.", "Sie hatten einen Anruf (2 Min.)").
    let mut d = super::super::Space { id: "dm".into(), direct: true, ..Default::default() };
    s.spaces.insert("dm".into(), d.clone());
    let calls: Vec<serde_json::Value> = acts[..3].iter().map(|a| { let mut a = a.clone(); a.as_object_mut().unwrap().remove("target"); a }).collect();
    let m = s.fold_in("dm", &calls, "");
    let t: Vec<(String, &str)> = m.iter().map(|x| (x.notice.clone(), x.time.as_str())).collect();
    assert_eq!(t, [(crate::trq!("{{0}} was unavailable.", "Anna Beispiel"), "16:11"), (crate::trq!("You missed a call.").into_owned(), "16:20"),
        (crate::trq!("You had a call ({{0}})", crate::trq!("{{0}} min", 2)), "16:30")]);
    // A group space: leave / create / add (the add within 5 s of the creation is not announced).
    d.direct = false; d.id = "g".into(); d.title = "Projekt".into();
    s.spaces.insert("g".into(), d);
    let m = s.fold_in("g", &acts[3..], "");
    let t: Vec<&str> = m.iter().map(|x| x.notice.as_str()).collect();
    assert_eq!(t, [crate::trq!("{{0}} left the space.", "Tom Muster"), crate::trq!("This starts the \"{{0}}\" space, created by {{1}}.", "Projekt", "Zoe"),
        crate::trq!("{{0}} added {{1}} to this space.", "Zoe", "Yan")]);
    assert!(m.iter().all(|x| x.date == "2026-01-01"), "info rows carry their day, so the list puts a date separator above them");
}

/// Repeated `create` rows in a 1:1 ("Jemand hat den Bereich erstellt."): the service invents a `create` with a new id, a
/// nameless actor and empty `object.tags` on every `maxDate` request for an old 1:1; only one may show.
#[test]
fn synthetic_creates_of_a_direct_chat() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (ktx, krx) = std::sync::mpsc::channel();
    let mut s = super::Session { hist: Default::default(), space_cover_urls: Default::default(), space_covers_done: Default::default(), full_desc: Default::default(), read_marks: Default::default(), covers_done: Default::default(), oracle_done: false, calls: Default::default(), img_jobs: vec![], images_done: Default::default(), uuids: Default::default(), avatars_done: Default::default(), agent: ureq::agent(), token: String::new(), my_email: String::new(), kms_token: String::new(), kms_loop: ktx, svc: Default::default(), device: String::new(),
        user: "me".into(), kms: None, keys: Default::default(), kms_rx: krx, spaces: Default::default(), out: tx, cover: String::new(), keep: Default::default(), live: Default::default(), extra: Default::default(), typing: Default::default(), subs: Default::default() };
    s.spaces.insert("dm".into(), super::super::Space { id: "dm".into(), direct: true, ..Default::default() });
    let made_up = |id: &str| json!({"id": id, "verb": "create", "objectType": "activity", "published": "2025-03-04T08:20:00.000Z",
        "actor": {"id": "u-1", "entryUUID": "u-1", "objectType": "person"}, "object": {"objectType": "conversation", "tags": []}});
    let post = json!({"id": "p1", "verb": "post", "published": "2026-08-24T12:32:00.000Z", "object": {"objectType": "comment", "displayName": "x"}, "actor": {"id": "me"}});
    assert_eq!(s.merge_acts("dm", vec![post]), 1);
    assert_eq!(s.merge_acts("dm", vec![made_up("c1")]), 1);
    for i in 2..10 { assert_eq!(s.merge_acts("dm", vec![made_up(&format!("c{i}"))]), 0, "a second create is never kept"); }
    let a = s.hist.acts["dm"].clone();
    let m = s.fold_in("dm", &a, "");
    assert_eq!(m.iter().filter(|x| !x.notice.is_empty()).count(), 0, "a 1:1 has no start sentence");
    assert_eq!(m.len(), 1);
    // A group space's start with a nameless creator: the sentence without a name, never a made-up "Jemand".
    s.spaces.insert("g".into(), super::super::Space { id: "g".into(), ..Default::default() });
    let m = s.fold_in("g", &[made_up("c1")], "");
    assert_eq!(m[0].notice, crate::trq!("This starts the space."));
    assert!(!m[0].notice.contains("Jemand"));
    // The creator is found among the space's participants by uuid.
    s.uuids.insert("ann@example.com".into(), "u-1".into());
    s.spaces.get_mut("g").unwrap().participants.push(super::super::Person { id: "ann@example.com".into(), name: "Ann Bell".into(), ..Default::default() });
    assert_eq!(s.fold_in("g", &[made_up("c1")], "")[0].notice, crate::trq!("This starts the space, created by {{0}}.", "Ann Bell"));
    // A create tagged ONE_ON_ONE is a 1:1 even without the space.
    let tagged = json!({"id": "c", "verb": "create", "published": "2025-03-04T08:20:00.000Z", "actor": {"displayName": "Ann"}, "object": {"objectType": "conversation", "tags": ["ONE_ON_ONE"]}});
    assert!(s.fold_in("", &[tagged], "").is_empty());
}

/// Read receipts: only when unseen, never for an info row, never twice.
#[test]
fn read_receipt_rules() {
    assert!(super::ack_due("2026-01-01T10:00:00.000Z", "2026-01-01T10:00:00.001Z"));
    assert!(!super::ack_due("2026-01-01T10:00:00.000Z", "2026-01-01T10:00:00.000Z"));
    assert!(!super::ack_due("", ""));
    assert!(super::ack_due("", "2026-01-01T10:00:00.000Z"));
    assert!(super::super::device_gone("PUT https://wdm-a.wbx2.com/wdm/api/v1/devices/x → 404 {\"message\":\"not found\"}"));
    assert!(!super::super::device_gone("PUT https://wdm-a.wbx2.com/wdm/api/v1/devices/x → 401 {}"));
    assert!(!super::super::device_gone("timed out"));
    // Nothing unseen: no request at all (the session has no network here, so a request would fail the call).
    let (tx, _rx) = std::sync::mpsc::channel();
    let (ktx, krx) = std::sync::mpsc::channel();
    let mut s = super::Session { hist: Default::default(), space_cover_urls: Default::default(), space_covers_done: Default::default(), full_desc: Default::default(), read_marks: Default::default(), covers_done: Default::default(), oracle_done: false, calls: Default::default(), img_jobs: vec![], images_done: Default::default(), uuids: Default::default(), avatars_done: Default::default(), agent: ureq::agent(), token: String::new(), my_email: String::new(), kms_token: String::new(), kms_loop: ktx, svc: Default::default(), device: String::new(),
        user: "me".into(), kms: None, keys: Default::default(), kms_rx: krx, spaces: Default::default(), out: tx, cover: String::new(), keep: Default::default(), live: Default::default(), extra: Default::default(), typing: Default::default(), subs: Default::default() };
    s.spaces.insert("sp".into(), super::super::Space { id: "sp".into(), last_seen: "2026-01-01T10:05:00.000Z".into(), ..Default::default() });
    let post = |id: &str, t: &str, who: &str| json!({"id": id, "verb": "post", "published": format!("2026-01-01T{t}.000Z"), "actor": {"id": who}, "object": {"objectType": "comment"}});
    s.merge_acts("sp", vec![post("a", "10:01:00", "x"), json!({"id": "n", "verb": "update", "published": "2026-01-01T10:09:00.000Z", "object": {"objectType": "locusSessionSummary"}})]);
    assert!(s.ack("sp", "n").is_ok(), "the info row is newer, but the newest readable message was seen already");
    // My own newer message: seen without a receipt, last_seen moves on.
    s.merge_acts("sp", vec![post("m", "10:10:00", "me")]);
    assert!(s.ack("sp", "m").is_ok());
    assert_eq!(s.spaces["sp"].last_seen, "2026-01-01T10:10:00.000Z");
}

/// Tombstones (same id as the deleted message; actor = who deleted, object.actor = who wrote it; the `delete` that made it
/// points at the tombstone) and the team sentences of a team's own space.
#[test]
fn deleted_messages_and_team_rows() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (ktx, krx) = std::sync::mpsc::channel();
    let mut s = super::Session { hist: Default::default(), space_cover_urls: Default::default(), space_covers_done: Default::default(), full_desc: Default::default(), read_marks: Default::default(), covers_done: Default::default(), oracle_done: false, calls: Default::default(), img_jobs: vec![], images_done: Default::default(), uuids: Default::default(), avatars_done: Default::default(), agent: ureq::agent(), token: String::new(), my_email: String::new(), kms_token: String::new(), kms_loop: ktx, svc: Default::default(), device: String::new(),
        user: "me-uuid".into(), kms: None, keys: Default::default(), kms_rx: krx, spaces: Default::default(), out: tx, cover: String::new(), keep: Default::default(), live: Default::default(), extra: Default::default(), typing: Default::default(), subs: Default::default() };
    s.spaces.insert("g".into(), super::super::Space { id: "g".into(), title: "Sandbox Room".into(), ..Default::default() });
    let me = json!({"id": "me-uuid", "entryUUID": "me-uuid", "displayName": "Ich"});
    let ann = json!({"id": "a-uuid", "entryUUID": "a-uuid", "displayName": "Ann"});
    let tomb = |id: &str, by: &serde_json::Value, of: &serde_json::Value, parent: serde_json::Value| json!({"id": id, "verb": "tombstone", "published": "2025-12-15T14:08:00.000Z",
        "actor": by, "parent": parent, "object": {"id": id, "objectType": "activity", "verb": "post", "actor": of, "object": {"objectType": "comment"}}});
    let acts = vec![
        tomb("t1", &me, &me, serde_json::Value::Null),
        json!({"id": "d1", "verb": "delete", "published": "2025-12-15T14:08:01.000Z", "actor": me, "object": {"id": "t1", "objectType": "activity", "verb": "tombstone"}}),
        tomb("t2", &ann, &me, serde_json::Value::Null),
        tomb("t3", &ann, &ann, json!({"id": "root", "type": "reply"})),
        tomb("t4", &me, &me, json!({"id": "x", "type": "edit"})),
    ];
    let m = s.fold_in("g", &acts, "");
    let t: Vec<&str> = m.iter().map(|x| x.notice.as_str()).collect();
    assert_eq!(t, [crate::trq!("You deleted your message.").into_owned(), crate::trq!("{{0}} deleted {{1}}'s message.", "Ann", "Ich"), crate::trq!("{{0}} deleted their own reply", "Ann")]);
    assert_eq!((m[0].date.as_str(), m[0].time.as_str()), ("2025-12-15", "14:08"));
    // A team's own space (id == team id): the "team" sentences.
    s.spaces.insert("T".into(), super::super::Space { id: "T".into(), team: "T".into(), title: "Product Team".into(), ..Default::default() });
    let acts = vec![
        json!({"id": "c", "verb": "create", "published": "2025-01-01T08:00:00.000Z", "actor": ann, "object": {"objectType": "conversation", "tags": ["TEAM"]}}),
        json!({"id": "l", "verb": "leave", "published": "2025-02-01T08:00:00.000Z", "actor": ann, "object": {"objectType": "person", "id": "a-uuid", "entryUUID": "a-uuid", "displayName": "Ann"}}),
        json!({"id": "a", "verb": "add", "published": "2025-03-01T08:00:00.000Z", "actor": me, "object": {"objectType": "person", "id": "a-uuid", "displayName": "Ann"}}),
    ];
    let t: Vec<String> = s.fold_in("T", &acts, "").into_iter().map(|x| x.notice).collect();
    assert_eq!(t, [crate::trq!("This starts the \"{{0}}\" team, created by {{1}}.", "Product Team", "Ann"), crate::trq!("{{0}} left the team.", "Ann"), crate::trq!("You added {{0}} to this team.", "Ann")]);
    // A team's other spaces keep the "space" sentences.
    s.spaces.insert("S".into(), super::super::Space { id: "S".into(), team: "T".into(), ..Default::default() });
    assert_eq!(s.fold_in("S", &acts[1..2], "")[0].notice, crate::trq!("{{0}} left the space.", "Ann"));
    assert_eq!(super::super::http_status("GET https://x → 401 {\"message\":\"\"}"), Some(401));
    assert_eq!(super::super::http_status("timed out"), None);
}

/// Join/leave collapsing pair rules: same actor adding several → one row; people joining themselves → one row;
/// another row in between, another day or a single row → no collapse.
#[test]
fn join_leave_collapsing() {
    use std::collections::HashMap;
    let jl = |add: bool, own: bool, actor: &str, me: bool, who: &str| super::JoinLeave { add, own, actor: actor.into(), actor_name: "Ann".into(), actor_me: me,
        person: super::super::Person { id: who.into(), name: who.into(), ..Default::default() } };
    let row = |id: &str, date: &str| super::super::Message { id: id.into(), notice: format!("row {id}"), date: date.into(), ..Default::default() };
    let mut j: HashMap<String, super::JoinLeave> = HashMap::new();
    j.insert("a1".into(), jl(true, false, "ann", false, "X")); j.insert("a2".into(), jl(true, false, "ann", false, "Y")); j.insert("a3".into(), jl(true, false, "ann", false, "Z"));
    j.insert("b1".into(), jl(true, false, "bob", false, "Q"));
    j.insert("s1".into(), jl(false, true, "p", false, "P")); j.insert("s2".into(), jl(false, true, "r", false, "R"));
    j.insert("d1".into(), jl(true, true, "k", false, "K")); j.insert("d2".into(), jl(true, true, "l", false, "L"));
    let list = vec![row("a1", "2026-01-01"), row("a2", "2026-01-01"), row("a3", "2026-01-01"), row("b1", "2026-01-01"),
        row("s1", "2026-01-01"), row("s2", "2026-01-01"), row("m", "2026-01-01"), row("d1", "2026-01-01"), row("d2", "2026-01-02")];
    let out = super::collapse_join_leave(list, &j);
    let ids: Vec<&str> = out.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["a1", "b1", "s1", "m", "d1", "d2"]);
    assert_eq!(out[0].notice, crate::trq!("{{0}} added {{1}} people to this space.", "Ann", 3));
    assert_eq!(out[0].group.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["X", "Y", "Z"]);
    assert!(out[1].group.is_empty(), "another actor starts a new run; alone it stays a plain row");
    assert_eq!(out[2].notice, crate::trq!("{{0}} people left this space.", 2));
    assert!(out[4].group.is_empty() && out[5].group.is_empty(), "different days never merge");
    // personRefresh is sent while the account doesn't set the key.
    assert!(super::activity_flags().starts_with("&personRefresh=true"));
}

#[test]
fn older_page_query_is_the_sdks() {
    assert_eq!(super::older_query("c", "1970-01-01T00:00:01.000Z"), format!("/conversations/c/activities?limit={}&maxDate=999&personRefresh=true&uuidEntryFormat=true&includeChildren=true", super::HISTORY_PAGE));
    assert_eq!(super::catchup_query(1794821536396), "/conversations/catchup?sinceDate=1794821536396");
    assert_eq!(super::newest_query("c", 1000), format!("/conversations/c/activities?limit={}&maxDate=604801000&personRefresh=true&uuidEntryFormat=true&includeChildren=true", super::super::ACTIVITY_PAGE));
    assert!(super::older_query("c", "garbage").contains("&maxDate=garbage&"));
    assert_eq!(super::call_duration(5), crate::trq!("{{0}} sec", 5));
    assert_eq!(super::call_duration(125), crate::trq!("{{0}} min", 2));
    assert_eq!(super::call_duration(3720), crate::trq!("{{0}} hr {{1}} min", 1, 2));
    assert_eq!(crate::i18n::args("{{0}} hr {{1}} min", &[&1, &2]), "1 hr 2 min");
}

/// Anonymized shape of a real listing: reactions only appear as cumulative reaction2Summary activities.
#[test]
fn reaction_summaries() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (ktx, krx) = std::sync::mpsc::channel();
    let mut s = super::Session { hist: Default::default(), space_cover_urls: Default::default(), space_covers_done: Default::default(), full_desc: Default::default(), read_marks: Default::default(), covers_done: Default::default(), oracle_done: false, calls: Default::default(), img_jobs: vec![], images_done: Default::default(), uuids: Default::default(), avatars_done: Default::default(), agent: ureq::agent(), token: String::new(), my_email: String::new(), kms_token: String::new(), kms_loop: ktx, svc: Default::default(), device: String::new(),
        user: "me".into(), kms: None, keys: Default::default(), kms_rx: krx, spaces: Default::default(), out: tx, cover: String::new(), keep: Default::default(), live: Default::default(), extra: Default::default(), typing: Default::default(), subs: Default::default() };
    
    let post = json!({"id": "m1", "verb": "post", "object": {"displayName": "hello", "objectType": "comment"}, "actor": {"id": "x"}, "published": "2026-01-01T10:00:00.000Z"});
    let summary = |t: &str, reactions: serde_json::Value| json!({"id": format!("s{t}"), "verb": "add", "published": format!("2026-01-01T{t}.000Z"),
        "parent": {"id": "m1", "type": "reactionSummary", "actorId": "x", "published": "2026-01-01T10:00:00.000Z"},
        "object": {"objectType": "reaction2Summary", "reactions": reactions}, "actor": {"id": "x"}});
    let acts = vec![
        post,
        summary("10:05:00", json!([{"displayName": "thumbsup", "count": 1, "hmac": "h", "containsLegacyReaction": true, "users": [{"id": "u1", "published": "p"}]}])),
        summary("10:09:00", json!([
            {"displayName": "thumbsup", "count": 2, "hmac": "h", "containsLegacyReaction": true, "users": [{"id": "u1", "published": "p"}, {"id": "me", "published": "p"}]},
            {"displayName": "👍", "count": 1, "hmac": "h", "containsLegacyReaction": false, "users": [{"id": "u2", "published": "p"}],
             "modifiers": [{"displayName": "🏽", "hmac": "h", "modifiedUsersIndex": [0]}]},
            {"displayName": "😀", "count": 1, "hmac": "h", "containsLegacyReaction": false, "users": [{"id": "u3", "published": "p"}],
             "modifiers": [{"displayName": "yellow", "hmac": "h", "modifiedUsersIndex": [0]}]}])),
    ];
    let m = s.fold(&acts, "");
    assert_eq!(m.len(), 1);
    let r = &m[0].reactions;
    assert_eq!(r.len(), 3); // the older snapshot is ignored
    assert_eq!((r[0].name.as_str(), r[0].count, r[0].mine.as_deref()), ("thumbsup", 2, Some("m1|thumbsup")));
    assert_eq!((r[1].name.as_str(), r[1].count, r[1].mine.as_deref()), ("👍🏽", 1, None));
    assert_eq!((r[2].name.as_str(), r[2].count), ("😀", 1)); // the \"yellow\" modifier is not part of the name
    let ids: Vec<&str> = r[0].users.iter().map(|u| u.id.as_str()).collect();
    assert_eq!(ids, ["u1", "me"]); // the reactors, in the summary's order
    assert_eq!(r[0].users[0].published, "p");
}

#[test]
fn summary_users_are_parsed_in_order_once() {
    let e = json!({"displayName": "heart", "count": 3, "users": [{"id": "u2", "published": "2026-11-20T09:00:00.000Z"}, {"id": "u1"}, {"id": "u2"}, {"id": ""}]});
    let r = super::fold::reactors(&e);
    let got: Vec<(&str, &str)> = r.iter().map(|u| (u.id.as_str(), u.published.as_str())).collect();
    assert_eq!(got, [("u2", "2026-11-20T09:00:00.000Z"), ("u1", "")]);
    assert!(super::fold::reactors(&json!({"displayName": "heart"})).is_empty());
}
    #[test]
fn folding() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (ktx, krx) = std::sync::mpsc::channel();
    let mut s = super::Session { hist: Default::default(), space_cover_urls: Default::default(), space_covers_done: Default::default(), full_desc: Default::default(), read_marks: Default::default(), covers_done: Default::default(), oracle_done: false, calls: Default::default(), img_jobs: vec![], images_done: Default::default(), uuids: Default::default(), avatars_done: Default::default(), agent: ureq::agent(), token: String::new(), my_email: String::new(), kms_token: String::new(), kms_loop: ktx, svc: Default::default(), device: String::new(),
        user: "me".into(), kms: None, keys: Default::default(), kms_rx: krx, spaces: Default::default(), out: tx, cover: String::new(), keep: Default::default(), live: Default::default(), extra: Default::default(), typing: Default::default(), subs: Default::default() };
    let p = |id: &str, t: &str, parent: serde_json::Value| json!({"id": id, "verb": "post", "object": {"displayName": t}, "actor": {"id": "x"}, "parent": parent});
    let acts = vec![
        p("a", "hello", json!(null)), p("b", "typo", json!(null)), p("e", "fixed", json!({"id": "b", "type": "edit"})),
        p("c", "gone", json!(null)), json!({"id": "d", "verb": "delete", "object": {"id": "c"}}),
        p("r", "in thread", json!({"id": "a", "type": "reply"})),
        json!({"id": "x1", "verb": "add", "object": {"objectType": "reaction2", "displayName": "heart"}, "parent": {"id": "a"}, "actor": {"id": "me"}}),
        json!({"id": "x2", "verb": "add", "object": {"objectType": "reaction2", "displayName": "heart"}, "parent": {"id": "a"}, "actor": {"id": "y"}}),
    ];
    let m = s.fold(&acts, "");
    let ids: Vec<&str> = m.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["a", "b", "r"]);
    assert_eq!((m[1].text.as_str(), m[1].edited), ("fixed", true));
    assert_eq!(m[2].parent, "a");
    assert_eq!((m[0].reactions[0].count, m[0].reactions[0].mine.as_deref()), (2, Some("x1")));
    let who: Vec<&str> = m[0].reactions[0].users.iter().map(|u| u.id.as_str()).collect();
    assert_eq!(who, ["me", "y"]); // live reactions name their actor
}
