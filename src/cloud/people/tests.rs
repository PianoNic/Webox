use super::*;

/// `statusList[]` entries: DND carries the real end in `expires`/`expiresTime`; for "active" those are only the record's
/// time-to-live and must not become an "until".
#[test]
fn status_detail_from_apheleia_records() {
    let dnd = json!({"status": "dnd", "category": "busy", "expires": "2026-11-23T07:55:00.000Z", "expiresTime": "2026-11-23T07:55:00.000Z", "expiresTTL": 600776, "lastActive": "2026-11-13T10:00:00.000Z"});
    assert_eq!(status_detail("DoNotDisturb", &dnd), (Some(iso_ms("2026-11-23T07:55:00.000Z")), String::new()));
    let active = json!({"status": "active", "expires": "2026-11-16T10:03:00.000Z", "expiresTime": "2026-11-16T10:03:00.000Z", "expiresTTL": 178});
    assert_eq!(status_detail("active", &active), (None, String::new()));
    let idle = json!({"status": "inactive", "expiresTTL": -1});
    assert_eq!(status_detail("inactive", &idle), (None, String::new()));
    // Live numeric form (Unix ms) and a custom text.
    let live = json!({"expiresTime": 1_794_577_037_527i64, "statusText": "Bis Montag weg"});
    assert_eq!(status_detail("OutOfOffice", &live), (Some(1_794_577_037_527), "Bis Montag weg".into()));
    assert_eq!(status_detail("DoNotDisturb", &json!({"expiresTime": 0})), (None, String::new()));
}

const U: &str = "00000000-0000-4000-8000-000000000001";

/// The presence model's numeric states (Unknown, Pending, Active, Inactive, Dnd, Quiet, Busy, OutOfOffice, Call, Meeting,
/// Presenting, CalendarItem, Away).
#[test]
fn numeric_states_follow_the_model_enum() {
    let names: Vec<String> = (0..=13).map(state_status).collect();
    assert_eq!(names, ["", "", "active", "inactive", "DoNotDisturb", "quiet", "busy", "OutOfOffice", "call", "meeting", "presenting", "calendar", "away", ""]);
}

#[test]
fn apheleia_names() {
    for (raw, want) in [("active", "active"), ("inactive", "inactive"), ("dnd", "DoNotDisturb"), ("busy", "busy"), ("ooo", "OutOfOffice"), ("call", "call"),
        ("meeting", "meeting"), ("presenting", "presenting"), ("quiet", "quiet"), ("calendarItem", "calendar"), ("unknown", ""), ("pending", "")] {
        assert_eq!(normalize_status(raw), want, "{raw}");
    }
}

/// apheleia.subscription_update: top-level subject/status/statusTime/expiresTime.
#[test]
fn parses_apheleia_subscription_update() {
    let ev = json!({"eventType": "apheleia.subscription_update", "subject": U, "status": "call", "category": "busy",
        "statusTime": "2026-11-17T10:00:00.000Z", "expiresTime": "2026-11-17T10:10:00.000Z", "expiresTTL": 600, "lastActive": "2026-11-17T09:59:00.000Z"});
    let u = &parse_presence(&ev)[0];
    assert_eq!((u.uuid.as_str(), u.status.as_deref()), (U, Some("call")));
    assert_eq!(u.until, Some(iso_ms("2026-11-17T10:10:00.000Z")));
    assert_eq!(u.order, Order::StatusTime(iso_ms("2026-11-17T10:00:00.000Z")));
    assert_eq!(u.last, iso_ms("2026-11-17T09:59:00.000Z"));
}

/// usersub.userstate with an availability composition, and the common one that only moves timestamps.
#[test]
fn parses_usersub_userstate() {
    let full = json!({"eventType": "usersub.userstate", "user": U, "sequence": 17, "ttl": 600, "compositions": [
        {"type": "workStatus"}, {"type": "ci_record_last_modified_timestamp", "composition": "2026-11-17T10:00:00.000Z"},
        {"type": "lastActivity", "composition": {"lastActiveTime": "2026-11-17T10:05:00.000Z"}}, {"type": "availability", "composition": {"type": "presenting"}}]});
    let u = &parse_presence(&full)[0];
    assert_eq!((u.status.as_deref(), u.order, u.last), (Some("presenting"), Order::Sequence(17), iso_ms("2026-11-17T10:05:00.000Z")));
    let stamps = json!({"eventType": "usersub.userstate", "user": U, "sequence": "18", "ttl": 123, "compositions": [
        {"type": "workStatus"}, {"type": "ci_record_last_modified_timestamp", "composition": "2026-11-17T10:00:00.000Z"}, {"type": "cross-client-state"}]});
    assert!(parse_presence(&stamps).is_empty(), "nothing the UI shows");
}

/// The parsed record form, also as a `userStates` list.
#[test]
fn parses_user_states_records() {
    let ev = json!({"userStates": [
        {"contactId": U, "presenceState": 8, "lastActiveTime": 1_794_900_000_000i64, "statusTime": 0, "expiresTime": 1_794_900_600_000i64, "sequence": 5},
        {"contactId": "00000000-0000-4000-8000-000000000002", "presenceState": 2, "lastActiveTime": 1_794_900_000_000i64, "expiresTime": 0, "sequence": 6}]});
    let v = parse_presence(&ev);
    assert_eq!(v.len(), 2);
    assert_eq!((v[0].status.as_deref(), v[0].until, v[0].order), (Some("call"), Some(1_794_900_600_000), Order::Sequence(5)));
    assert_eq!((v[1].status.as_deref(), v[1].until), (Some("active"), None));
}

/// An update whose sequence is not newer is ignored; status times may repeat but not go back.
#[test]
fn older_updates_are_ignored() {
    let mut b = Book::default();
    let up = |status: &str, order| Update { uuid: U.into(), status: Some(status.into()), order, ..Default::default() };
    assert!(b.accept(&up("call", Order::Sequence(10)), 0));
    assert!(!b.accept(&up("call", Order::Sequence(10)), 0));
    assert!(!b.accept(&up("meeting", Order::Sequence(9)), 0));
    assert!(b.accept(&up("active", Order::Sequence(11)), 0));
    assert!(b.accept(&up("call", Order::StatusTime(1000)), 0));
    assert!(b.accept(&up("call", Order::StatusTime(1000)), 0));
    assert!(!b.accept(&up("active", Order::StatusTime(999)), 0));
}

/// Snapshot and live events have separate orders; one whose last activity is older than an applied one is dropped (the
/// desktop client's "PresenceIgnored"), so a late snapshot can't bring back yesterday's "inactive". No activity time: kept.
#[test]
fn an_older_last_activity_is_ignored_across_orders() {
    let mut b = Book::default();
    let live = Update { uuid: U.into(), status: Some("active".into()), last: 2_000, order: Order::Sequence(7), ..Default::default() };
    assert!(b.accept(&live, 0));
    let snapshot = Update { uuid: U.into(), status: Some("inactive".into()), last: 1_000, order: Order::StatusTime(5), ..Default::default() };
    assert!(!b.accept(&snapshot, 0));
    let no_time = Update { uuid: U.into(), status: Some("call".into()), last: 0, order: Order::Sequence(8), ..Default::default() };
    assert!(b.accept(&no_time, 0));
    let newer = Update { uuid: U.into(), status: Some("inactive".into()), last: 3_000, order: Order::StatusTime(6), ..Default::default() };
    assert!(b.accept(&newer, 0));
}

/// "In einem Anruf" with an end; the call ends without an event → once the end has passed the person is due for a
/// snapshot again (then at most once a minute); an "active" update clears it.
#[test]
fn a_call_that_ends_without_an_event_is_checked_again() {
    let mut b = Book::default();
    let call = Update { uuid: U.into(), status: Some("call".into()), until: Some(10_000), order: Order::StatusTime(1), ..Default::default() };
    assert!(b.accept(&call, 5_000));
    assert!(b.due(9_999).is_empty());
    assert_eq!(b.due(10_000), [U.to_string()]);
    assert!(b.due(10_001).is_empty(), "not again before a minute");
    assert_eq!(b.due(10_000 + PRESENCE_RECHECK_MS), [U.to_string()]);
    // The snapshot (or a live event) says active: nothing pending any more.
    let active = Update { uuid: U.into(), status: Some("active".into()), until: None, order: Order::StatusTime(2), ..Default::default() };
    assert!(b.accept(&active, 80_000));
    assert!(b.due(1_000_000).is_empty());
    // A snapshot still reporting an end in the past does not make it due every second.
    let stale = Update { uuid: U.into(), status: Some("call".into()), until: Some(50_000), order: Order::StatusTime(3), ..Default::default() };
    assert!(b.accept(&stale, 100_000));
    assert!(b.due(100_001).is_empty());
    assert_eq!(b.due(100_000 + PRESENCE_RECHECK_MS), [U.to_string()]);
    // A status-less update (activity only) keeps the pending end.
    let stamps = Update { uuid: U.into(), status: None, last: 1, order: Order::StatusTime(4), ..Default::default() };
    assert!(b.accept(&stamps, 200_000));
    assert_eq!(b.due(200_000 + PRESENCE_RECHECK_MS), [U.to_string()]);
}

/// Active / inactive carry only the record's time-to-live as `expiresTime`: never due.
#[test]
fn active_and_inactive_never_expire() {
    let mut b = Book::default();
    for s in ["active", "inactive"] {
        let ev = json!({"subject": U, "status": s, "expiresTime": "2026-11-17T10:10:00.000Z", "statusTime": "2026-11-17T10:00:00.000Z"});
        let u = parse_presence(&ev).remove(0);
        assert!(b.accept(&u, 0));
    }
    assert!(b.due(i64::MAX).is_empty());
}

#[test]
fn iso() {
    assert_eq!(super::iso_ms("1970-01-02T00:00:01.500Z"), 86_401_500);
    assert_eq!(super::iso_ms("2026-11-13T13:37:17.527Z"), 1_794_577_037_527);
    assert_eq!(super::iso_ms("garbage"), 0);
}

/// The avatar / usersub calls take their URLs from the service catalog, but go through the same host gate as `http()`.
#[test]
fn people_requests_send_the_bearer_only_to_webex() {
    let agent = ureq::agent();
    for u in ["https://avatar-prod-us-east-2.webexcontent.com/profiles/urls", "https://usersub-a.wbx2.com/usersub/api/v1/subscribe"] {
        assert_eq!(with_bearer(agent.get(u), u, "T").header("Authorization"), Some("Bearer T"), "{u}");
    }
    for u in ["https://evil.example/profiles/urls", "https://evil.example?.webex.com/x", "http://avatar-a.wbx2.com/x", "https://a.webex.com@evil.example/", "/profiles/urls"] {
        assert_eq!(with_bearer(agent.get(u), u, "T").header("Authorization"), None, "{u}");
    }
}
