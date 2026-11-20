use super::*;

#[test]
fn notify_round_trips_through_tags() {
    for n in [Notify::Global, Notify::All, Notify::Mentions, Notify::Off] { assert_eq!(Notify::from_tags(n.tags()), n); }
    assert_eq!(Notify::from_tags(&["MUTED"]), Notify::Off);
    assert_eq!(Notify::from_tags(&["FAVORITE"]), Notify::Global);
    assert_eq!(MUTE_TAGS.len(), 4);
}

#[test]
fn bodies_follow_the_sdk() {
    let b = tag_body("untag", "c1", &MUTE_TAGS);
    assert_eq!(b["verb"], "untag"); assert_eq!(b["object"]["objectType"], "conversation"); assert_eq!(b["object"]["tags"].as_array().unwrap().len(), 4);
    assert_eq!(simple_body("hide", "c1")["object"]["id"], "c1");
    assert_eq!(unack_body("a9")["verb"], "unacknowledge");
    assert_eq!(unack_body("a9")["object"]["objectType"], "activity");
    assert_eq!(moderator_body(true, "u")["verb"], "assignModerator");
    assert_eq!(moderator_body(false, "u")["verb"], "unassignModerator");
    let r = remove_body("u2", "JWE");
    assert_eq!(r["verb"], "leave"); assert_eq!(r["object"]["id"], "u2"); assert_eq!(r["kmsMessage"], "JWE");
    assert_eq!(kms_delete_auth("kms://x/resources/1", "u2")["uri"], "kms://x/resources/1/authorizations?authId=u2");
    assert_eq!(kms_team_add("kms://r/s", "kms://r/g")["userIds"][0], "kms://r/g");
    assert_eq!(team_add_space_body("s", "K")["object"]["objectType"], "conversation");
    assert_eq!(team_remove_space_body("s", "K")["verb"], "remove");
    let a = archive_body("t1", "team");
    assert_eq!(a["verb"], "archive"); assert_eq!(a["target"]["objectType"], "team");
}

#[test]
fn creation_body_adds_everyone_once() {
    let b = creation_body("me", &["u2".into(), "me".into(), "u3".into()], "kms://k", "T", Some("S"), "M", true);
    assert_eq!(b["objectType"], "team");
    assert_eq!(b["summary"], "S");
    let acts = b["activities"]["items"].as_array().unwrap();
    assert_eq!(acts[0]["verb"], "create");
    assert_eq!(acts.iter().filter(|a| a["verb"] == "add").count(), 3); // me, u2, u3
    assert_eq!(b["participants"]["items"].as_array().unwrap().len(), 3);
    let c = creation_body("me", &[], "k", "T", None, "M", false);
    assert_eq!(c["objectType"], "conversation");
    assert!(c.get("summary").is_none());
}

#[test]
fn list_entries_yield_settings_moderators_and_teams() {
    let c = json!({"id": "c1", "tags": ["MESSAGE_NOTIFICATIONS_OFF", "MENTION_NOTIFICATIONS_ON", "LOCKED"],
        "participants": {"items": [{"id": "a@x", "entryUUID": "u-a", "roomProperties": {"isModerator": true}}, {"id": "b@x"}]},
        "team": {"id": "t1", "generalConversationUuid": "g1", "archived": false}});
    let i = space_info(&c);
    assert_eq!(i.notify, Notify::Mentions);
    assert!(i.locked);
    assert!(i.moderators.contains(&"a@x".to_string()) && i.moderators.contains(&"u-a".to_string()));
    assert!(!i.moderators.contains(&"b@x".to_string()));
    let t = team_of(&c).unwrap();
    assert_eq!((t.id.as_str(), t.general.as_str()), ("t1", "g1"));
    assert!(team_of(&json!({"id": "c2"})).is_none());
}
