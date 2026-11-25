use super::*;

// Fixtures in the shape of locus objects; values invented.
fn locus(self_state: &str, other_state: &str, active: bool) -> Value {
    json!({"url": "https://locus-a.wbx2.com/locus/api/v1/loci/abc", "fullState": {"type": "CALL", "state": if active {"ACTIVE"} else {"INACTIVE"}, "active": active},
        "self": {"id": "self-1", "state": self_state, "person": {"id": "p1", "email": "me@example.com", "name": "Me"}},
        "participants": [
            {"id": "self-1", "type": "USER", "state": self_state, "person": {"id": "p1", "email": "me@example.com", "name": "Me"}},
            {"id": "peer-1", "type": "USER", "state": other_state, "person": {"id": "p2", "email": "peer@example.com", "name": "Peer Person"}}]})
}

#[test]
fn parses_participants_and_self() {
    let v = parse_locus(&locus("JOINED", "NOTIFIED", true), "me@example.com");
    assert_eq!((v.kind.as_str(), v.active, v.self_id.as_str(), v.self_state.as_str()), ("CALL", true, "self-1", "JOINED"));
    assert_eq!(v.others, vec![Party { name: "Peer Person".into(), email: "peer@example.com".into(), state: "NOTIFIED".into(), devices: vec![], muted: false }]);
}
#[test]
fn outgoing_progress() {
    let st = |s, o, a| call_state(&parse_locus(&locus(s, o, a), "me@example.com"), true);
    assert_eq!(st("JOINED", "IDLE", true), Some(CallState::Dialing));
    assert_eq!(st("JOINED", "NOTIFIED", true), Some(CallState::Ringing));
    assert_eq!(st("JOINED", "JOINED", true), Some(CallState::Connected));
    assert_eq!(st("JOINED", "DECLINED", true), Some(CallState::Ended("declined".into())));
    assert_eq!(st("LEFT", "NOTIFIED", true), Some(CallState::Ended("left".into())));
    assert_eq!(st("JOINED", "NOTIFIED", false), Some(CallState::Ended("unanswered".into())));
}
#[test]
fn ringing_from_device_state() {
    let mut l = locus("JOINED", "IDLE", true);
    l["participants"][1]["devices"] = json!([{"state": "NOTIFIED"}]);
    assert_eq!(call_state(&parse_locus(&l, "me@example.com"), true), Some(CallState::Ringing));
}
#[test]
fn incoming_detection() {
    let st = |s, o, a| call_state(&parse_locus(&locus(s, o, a), "me@example.com"), false);
    assert_eq!(st("IDLE", "JOINED", true), Some(CallState::Incoming));
    assert_eq!(st("NOTIFIED", "JOINED", true), Some(CallState::Incoming));
    assert_eq!(st("JOINED", "JOINED", true), Some(CallState::Connected));
    assert_eq!(st("IDLE", "LEFT", true), Some(CallState::Ended("missed".into())));
    assert_eq!(st("DECLINED", "JOINED", true), Some(CallState::Ended("declined".into())));
    assert_eq!(st("IDLE", "JOINED", false), Some(CallState::Ended("missed".into())));
}
#[test]
fn idle_locus_events_read_only_what_can_ring() {
    let ev = |l: Value| json!({"eventType": "locus.participant_alerted", "locus": l});
    assert!(idle_event_needs_read(&ev(locus("NOTIFIED", "JOINED", true)), "me@example.com"), "a call ringing for me");
    assert!(!idle_event_needs_read(&ev(locus("JOINED", "JOINED", true)), "me@example.com"), "a call I'm already in elsewhere");
    assert!(!idle_event_needs_read(&ev(locus("IDLE", "LEFT", false)), "me@example.com"), "an ended call");
    let mut meeting = locus("IDLE", "JOINED", true);
    meeting["fullState"]["type"] = json!("MEETING");
    assert!(!idle_event_needs_read(&ev(meeting), "me@example.com"), "meetings are not read");
    assert!(idle_event_needs_read(&json!({"eventType": "locus.self_changed", "locusUrl": "https://locus-a.wbx2.com/locus/api/v1/loci/x"}), "me@example.com"),
        "no locus body: read, so no incoming call is missed");
}

#[test]
fn roap_messages_match_the_sdk_shapes() {
    // Outgoing: what goes into localMedias[].localSdp.
    let o: Value = serde_json::from_str(&roap_local_sdp("OFFER", Some("v=0\r\n"), 1, false)).unwrap();
    assert_eq!(o["roapMessage"]["messageType"], "OFFER");
    assert_eq!((o["roapMessage"]["version"].as_str(), o["roapMessage"]["seq"].as_u64(), o["roapMessage"]["tieBreaker"].as_u64()), (Some("2"), Some(1), Some(4294967294)));
    assert_eq!(o["roapMessage"]["sdps"][0], "v=0\r\n");
    assert_eq!((o["audioMuted"].as_bool(), o["videoMuted"].as_bool()), (Some(false), Some(true)));
    let ok: Value = serde_json::from_str(&roap_local_sdp("OK", None, 1, true)).unwrap();
    assert!(ok["roapMessage"]["sdps"].is_null() && ok["roapMessage"]["tieBreaker"].is_null() && ok["audioMuted"] == true);
    // Incoming: mercury locus.message.roap.
    let ev = json!({"eventType": "locus.message.roap", "correlationId": "c1", "message": {"messageType": "ANSWER", "seq": 1, "sdps": ["v=0\r\nm=audio"], "version": "2"}});
    assert_eq!(roap_in(&ev), Some(("ANSWER".into(), 1, Some("v=0\r\nm=audio".into()))));
    assert!(roap_in(&json!({"eventType": "locus.difference"})).is_none());
    // Inline answer in a join response.
    let resp = json!({"mediaConnections": [{"mediaId": "m1", "remoteSdp": json!({"roapMessage": {"messageType": "ANSWER", "seq": 1, "sdps": ["x"]}}).to_string()}]});
    assert_eq!(remote_roap(&resp), Some(("ANSWER".into(), 1, Some("x".into()))));
    assert!(remote_roap(&json!({"mediaConnections": [{"mediaId": "m1"}]})).is_none());
    assert!(remote_roap(&json!({"mediaConnections": [{"mediaId": "m1", "remoteSdp": "{}"}]})).is_none());
}
/// Reads per minute of 1 s ticks (fake clock), with the reads `poll_due` asks for; `stale_at` = ticks where an event
/// without the full locus arrived.
fn reads_per_minute(connected: bool, stale_at: &[u64]) -> usize {
    let t0 = Instant::now();
    let (mut read_at, mut stale, mut reads) = (Some(t0), false, 0);
    for s in 1..=60 {
        let now = t0 + Duration::from_secs(s);
        if stale_at.contains(&s) { stale = true; }
        if poll_due(read_at, now, connected, stale) { reads += 1; read_at = Some(now); stale = false; }
    }
    reads
}
#[test]
fn locus_poll_cadence() {
    assert_eq!(reads_per_minute(true, &[]), 2); // every 30 s while connected
    assert_eq!(reads_per_minute(false, &[]), 4); // every 15 s before
    // a burst of events without the full locus: at most one read per 2 s
    assert!(reads_per_minute(true, &(10..20).collect::<Vec<_>>()) <= 2 + 5);
    assert!(poll_due(None, Instant::now(), true, false), "first read");
}
#[test]
fn events_carry_the_full_locus() {
    let full = json!({"eventType": "locus.self_changed", "locus": locus("JOINED", "JOINED", true)});
    let l = full_locus(&full).expect("full locus");
    assert_eq!(parse_locus(l, "").others[0].state, "JOINED");
    let partial = json!({"eventType": "locus.difference", "locusUrl": "https://locus-a.wbx2.com/locus/api/v1/loci/x", "locus": {"baseSequence": {}, "sequence": {"entries": [5]}}});
    assert!(full_locus(&partial).is_none());
    assert_eq!(locus_seq(&partial["locus"]), 5);
    assert_eq!(locus_seq(&json!({})), 0);
}
#[test]
fn media_id_from_the_top_or_from_this_device() {
    assert_eq!(media_id_of(&json!({"mediaConnections": [{"mediaId": "m1"}], "locus": {}}), "d"), "m1");
    let devs = json!({"locus": {"self": {"devices": [{"url": "other", "mediaConnections": [{"mediaId": "m2"}]}, {"url": "d", "mediaConnections": [{"type": "SDP", "mediaId": "m3"}]}]}}});
    assert_eq!(media_id_of(&devs, "d"), "m3");
    assert_eq!(media_id_of(&devs, "unknown"), "m2");
    assert_eq!(media_id_of(&json!({"mediaConnections": [{"mediaId": ""}]}), "d"), "");
}
#[test]
fn roap_error_logs_only_its_cause() {
    let m = json!({"messageType": "ERROR", "seq": 2, "errorType": "DOUBLECONFLICT", "errorCause": "glare by someone@example.com\n"});
    assert_eq!(roap_error(&m), "seq=2 errorType=DOUBLECONFLICT errorCause=glare by <addr>");
}
#[test]
fn mercury_event_shapes() {
    let d = json!({"eventType": "locus.difference", "locusUrl": "https://locus-a.wbx2.com/locus/api/v1/loci/abc", "locus": {"baseSequence": {}, "sequence": {}}});
    assert!(locus_url_of(&d).ends_with("/loci/abc"));
    let d2 = json!({"eventType": "locus.difference", "locus": {"url": "https://locus-a.wbx2.com/locus/api/v1/loci/xyz"}});
    assert!(locus_url_of(&d2).ends_with("/loci/xyz"));
    let wrapped = json!({"locus": locus("IDLE", "JOINED", true)});
    assert_eq!(parse_locus(locus_of(&wrapped), "").kind, "CALL");
}
#[test]
fn guard_blocks_everyone_else() {
    let g = Some("jonas.weber@example.com");
    assert!(call_allowed("jonas.weber@example.com", g).is_ok());
    assert!(call_allowed("Jonas.Weber@example.com", g).is_ok());
    assert!(call_allowed("someone.else@example.com", g).is_err());
    assert!(call_allowed("jonas.weber@example.com.evil.example", g).is_err());
    assert!(call_allowed("+12025550142", None).is_err());
    assert!(call_allowed("tel:+12025550142", None).is_err());
    assert!(call_allowed("sip:a@b.c", None).is_err());
    assert!(call_allowed("a b@c.d", None).is_err());
    assert!(call_allowed("someone@x.y", None).is_ok());
    assert!(call_allowed("someone@x.y", Some("")).is_ok());
}
#[test]
fn dtmf_body_matches_sdk() {
    assert_eq!(dtmf_body("https://wdm-a.wbx2.com/d/1", "5#", "c1"), json!({"deviceUrl": "https://wdm-a.wbx2.com/d/1", "dtmf": {"correlationId": "c1", "tones": "5#"}}));
    assert!(dtmf_ok("0123456789*#") && !dtmf_ok("") && !dtmf_ok("1a") && !dtmf_ok("+"));
}
/// A full locus with a sequence number, in the shape Mercury events carry it.
fn seq_locus(self_state: &str, other_state: &str, active: bool, seq: u64) -> Value {
    let mut l = locus(self_state, other_state, active);
    l["sequence"] = json!({"entries": [seq], "rangeStart": 0, "rangeEnd": 0});
    l
}
fn ev(kind: &str, l: Value) -> Value { json!({"eventType": kind, "locusUrl": "https://locus-a.wbx2.com/locus/api/v1/loci/abc", "locus": l}) }
fn applied(a: Apply) -> Value { match a { Apply::Use(l) => l, other => panic!("expected Use, got {other:?}") } }

#[test]
fn remote_leave_while_connected_ends_the_call() {
    // connected, then the peer's participant goes LEFT (locus still ACTIVE in the same event)
    let held = seq_locus("JOINED", "JOINED", true, 10);
    let l = applied(apply_event(&held, 10, &ev("locus.participant_left", seq_locus("JOINED", "LEFT", true, 11))));
    assert_eq!(call_state(&parse_locus(&l, "me@example.com"), true), Some(CallState::Ended("left".into())));
    // the peer's devices all LEFT while the participant still reads JOINED (official: otherPairedDevice->left)
    let mut l = seq_locus("JOINED", "JOINED", true, 12);
    l["participants"][1]["devices"] = json!([{"state": "LEFT", "url": "d1"}]);
    assert_eq!(call_state(&parse_locus(&l, "me@example.com"), true), Some(CallState::Ended("left".into())));
    // TERMINATING with `active` still true: over
    let mut l = seq_locus("JOINED", "JOINED", true, 13);
    l["fullState"]["state"] = json!("TERMINATING");
    assert!(!parse_locus(&l, "").active);
    assert!(matches!(call_state(&parse_locus(&l, "me@example.com"), true), Some(CallState::Ended(_))));
    // an answered incoming call: the caller hangs up
    assert_eq!(call_state(&parse_locus(&seq_locus("JOINED", "LEFT", false, 14), "me@example.com"), true), Some(CallState::Ended("left".into())));
}
#[test]
fn remote_cancel_while_ringing_ends_the_incoming_call() {
    let held = seq_locus("NOTIFIED", "JOINED", true, 5);
    assert_eq!(call_state(&parse_locus(&held, "me@example.com"), false), Some(CallState::Incoming));
    // the caller cancels: LEFT and the locus INACTIVE
    let l = applied(apply_event(&held, 5, &ev("locus.participant_left", seq_locus("NOTIFIED", "LEFT", false, 6))));
    assert_eq!(call_state(&parse_locus(&l, "me@example.com"), false), Some(CallState::Ended("missed".into())));
    // ... or only LEFT, the locus not yet inactive
    let l = applied(apply_event(&held, 5, &ev("locus.participant_left", seq_locus("NOTIFIED", "LEFT", true, 6))));
    assert_eq!(call_state(&parse_locus(&l, "me@example.com"), false), Some(CallState::Ended("missed".into())));
}
#[test]
fn remote_decline_ends_the_outgoing_call() {
    let held = seq_locus("JOINED", "NOTIFIED", true, 3);
    let l = applied(apply_event(&held, 3, &ev("locus.participant_declined", seq_locus("JOINED", "DECLINED", true, 4))));
    assert_eq!(call_state(&parse_locus(&l, "me@example.com"), true), Some(CallState::Ended("declined".into())));
    let l = applied(apply_event(&held, 3, &ev("locus.participant_declined", seq_locus("JOINED", "DECLINED", false, 4))));
    assert_eq!(call_state(&parse_locus(&l, "me@example.com"), true), Some(CallState::Ended("declined".into())));
}
#[test]
fn deltas_merge_only_onto_their_base() {
    let held = seq_locus("JOINED", "JOINED", true, 20);
    // a delta naming only the peer: merged into what we hold, self stays as it was
    let delta = json!({"baseSequence": {"entries": [20]}, "sequence": {"entries": [21]},
        "participants": [{"id": "peer-1", "type": "USER", "state": "LEFT", "person": {"email": "peer@example.com", "name": "Peer Person"}}]});
    let l = applied(apply_event(&held, 20, &ev("locus.difference", delta.clone())));
    assert_eq!(locus_seq(&l), 21);
    let v = parse_locus(&l, "me@example.com");
    assert_eq!((v.self_state.as_str(), v.others.len(), v.others[0].state.as_str()), ("JOINED", 1, "LEFT"));
    assert!(l.get("baseSequence").is_none());
    assert_eq!(call_state(&v, true), Some(CallState::Ended("left".into())));
    // a delta that only lists self must not make the peer vanish (the old code took it as the whole locus)
    let self_only = json!({"baseSequence": {"entries": [20]}, "sequence": {"entries": [21]}, "fullState": {"type": "CALL", "state": "ACTIVE", "active": true},
        "self": {"id": "self-1", "state": "JOINED"}, "participants": [{"id": "self-1", "type": "USER", "state": "JOINED", "person": {"email": "me@example.com"}}]});
    let l = applied(apply_event(&held, 20, &ev("locus.self_changed", self_only)));
    assert_eq!(call_state(&parse_locus(&l, "me@example.com"), true), Some(CallState::Connected));
}
#[test]
fn missed_or_out_of_order_delta_resyncs() {
    let held = seq_locus("JOINED", "JOINED", true, 20);
    let delta = |base: u64, seq: u64| ev("locus.difference", json!({"baseSequence": {"entries": [base]}, "sequence": {"entries": [seq]}, "participants": []}));
    // 21 was missed: the delta on top of 21 can't be applied -> read the whole locus
    assert_eq!(apply_event(&held, 20, &delta(21, 22)), Apply::Read);
    assert_eq!(compare_seq(20, &delta(21, 22)["locus"]), SeqCmp::Desync);
    // an older / repeated event is dropped
    assert_eq!(apply_event(&held, 20, &delta(19, 20)), Apply::Discard);
    assert_eq!(apply_event(&held, 20, &ev("locus.participant_joined", seq_locus("JOINED", "NOTIFIED", true, 18))), Apply::Discard);
    // the first event being a delta (nothing held) -> read
    assert_eq!(apply_event(&Value::Null, 0, &delta(5, 6)), Apply::Read);
    // a newer full locus always replaces ours, whatever we missed
    assert!(matches!(apply_event(&held, 20, &ev("locus.participant_left", seq_locus("JOINED", "LEFT", true, 30))), Apply::Use(_)));
    // an event without the locus -> read
    assert_eq!(apply_event(&held, 20, &json!({"eventType": "locus.self_changed", "locusUrl": "x"})), Apply::Read);
    // rangeEnd counts as the newest number too
    assert_eq!(locus_seq(&json!({"sequence": {"entries": [], "rangeStart": 3, "rangeEnd": 9}})), 9);
}
#[test]
fn locus_urls_match_across_hosts() {
    assert!(same_locus("https://locus-a.wbx2.com/locus/api/v1/loci/abc", "https://locus-us-east-1.wbx2.com/locus/api/v1/loci/abc"));
    assert!(same_locus("https://locus-a.wbx2.com/locus/api/v1/loci/abc", "https://locus-a.wbx2.com/locus/api/v1/loci/abc?x=1"));
    assert!(!same_locus("https://locus-a.wbx2.com/locus/api/v1/loci/abc", "https://locus-a.wbx2.com/locus/api/v1/loci/abd"));
    assert!(!same_locus("https://a/x", "https://b/x"));
}
#[test]
fn stalled_media_is_reported() {
    let t0 = Instant::now();
    let now = |s: u64| t0 + Duration::from_secs(s);
    // no ROAP answer
    assert!(media_stall(now(5), Some(t0), None, None, 0, Some(MediaState::New)).is_none());
    assert!(media_stall(now(11), Some(t0), None, None, 0, Some(MediaState::New)).unwrap().contains("ROAP"));
    // the other side joined, ICE never comes up
    assert!(media_stall(now(10), None, Some(t0), None, 0, Some(MediaState::Connecting)).is_none());
    assert!(media_stall(now(16), None, Some(t0), None, 0, Some(MediaState::Connecting)).unwrap().contains("ICE"));
    // "connected" but not one packet: that is no connection
    assert!(media_stall(now(9), None, Some(t0), Some(t0), 0, Some(MediaState::Connected)).unwrap().contains("no sound"));
    assert!(media_stall(now(9), None, Some(t0), Some(t0), 50, Some(MediaState::Connected)).is_none());
    assert!(media_stall(now(1), None, None, None, 0, Some(MediaState::Failed)).is_some());
}
#[test]
fn share_floor_and_refusals() {
    let mut l = locus("JOINED", "JOINED", true);
    assert!(share::content_share_url(&l).is_none());
    l["mediaShares"] = json!([{"name": "whiteboard", "url": "https://locus-a.wbx2.com/locus/api/v1/loci/abc/mediashares/w"},
        {"name": "content", "url": "https://locus-a.wbx2.com/locus/api/v1/loci/abc/mediashares/c", "floor": {"disposition": "GRANTED", "beneficiary": {"url": "https://self"}}}]);
    assert_eq!(share::content_share_url(&l).as_deref(), Some("https://locus-a.wbx2.com/locus/api/v1/loci/abc/mediashares/c"));
    assert_eq!(share::content_floor(&l), ("GRANTED".to_string(), "https://self".to_string()));
    let b = share::floor_body("GRANTED", "https://self", "https://dev", "DESKTOP");
    assert_eq!(b["floor"]["disposition"], "GRANTED");
    assert_eq!(b["floor"]["beneficiary"]["devices"][0], json!({"deviceType": "DESKTOP", "url": "https://dev"}));
    assert_eq!(b["floor"]["requester"]["url"], "https://self");
    // every refusal has a reason for the toast
    let c = Some(&CallState::Connected);
    assert!(share::share_blocked(Some(&CallState::Ringing), true, true, false, Some("u")).unwrap().contains("connected"));
    assert!(share::share_blocked(c, true, true, false, None).unwrap().contains("content"));
    assert!(share::share_blocked(c, true, true, true, Some("u")).unwrap().contains("negotiated"));
    assert!(share::share_blocked(c, true, true, false, Some("u")).is_none());
    assert!(share::floor_error("PUT https://x → 409 {}") == crate::trq!("Can't share: someone else is sharing right now."));
    assert!(share::floor_error("PUT https://x → 403 {}") == crate::trq!("Sharing not allowed: your organization's policy doesn't allow sharing in this call."));
    assert!(share::floor_error("PUT https://x → 500 {}").ends_with("PUT https://x → 500 {}"));
}

#[test]
fn local_mute_goes_to_locus_without_roap() {
    let b = local_mute_body(json!({"url": "https://wdm/d/1", "deviceType": "DESKTOP"}), "corr-1", "media-1", true, true);
    let m = &b["localMedias"][0];
    assert_eq!(m["mediaId"], "media-1");
    let sdp: Value = serde_json::from_str(m["localSdp"].as_str().unwrap()).unwrap();
    assert_eq!(sdp, json!({"audioMuted": true, "videoMuted": true}), "mute state only, no roapMessage");
    assert_eq!((b["device"]["deviceType"].as_str(), b["correlationId"].as_str(), b["respOnlySdp"].as_bool()), (Some("DESKTOP"), Some("corr-1"), Some(true)));
    let u: Value = serde_json::from_str(local_mute_body(json!({}), "c", "m", false, true)["localMedias"][0]["localSdp"].as_str().unwrap()).unwrap();
    assert_eq!(u["audioMuted"], false);
    assert_eq!(unmute_controls_body(), json!({"audio": {"muted": false}}));
}
#[test]
fn the_other_sides_mute_shows() {
    let mut l = locus("JOINED", "JOINED", true);
    assert!(!parse_locus(&l, "me@example.com").others[0].muted);
    l["participants"][1]["status"] = json!({"audioStatus": "RECVONLY"});
    assert!(parse_locus(&l, "me@example.com").others[0].muted, "muted by themselves: RECVONLY");
    l["participants"][1]["status"] = json!({"audioStatus": "SENDRECV"});
    assert!(!parse_locus(&l, "me@example.com").others[0].muted, "unmuted again");
    l["participants"][1]["controls"] = json!({"audio": {"muted": true}});
    assert!(parse_locus(&l, "me@example.com").others[0].muted, "muted by a host wins over SENDRECV");
    let (info, _) = mute_update(None, &parse_locus(&l, "me@example.com"), false);
    assert!(info.peer_muted && !info.local);
}
#[test]
fn being_muted_by_others_mutes_us_once() {
    let mut l = locus("JOINED", "JOINED", true);
    let v0 = parse_locus(&l, "me@example.com");
    assert_eq!(v0.mute, SelfMute { remote: false, unmute_allowed: true, unmute_requested: false });
    let (i0, newly) = mute_update(None, &v0, false);
    assert!(!newly && !i0.local);
    // a host mutes us: our microphone goes mute
    l["self"]["controls"] = json!({"audio": {"muted": true, "disallowUnmute": true}});
    let v1 = parse_locus(&l, "me@example.com");
    assert_eq!(v1.mute, SelfMute { remote: true, unmute_allowed: false, unmute_requested: false });
    let (i1, newly) = mute_update(Some(&i0), &v1, false);
    assert!(newly && i1.local && i1.me.remote, "remote mute mutes us locally");
    let (_, again) = mute_update(Some(&i1), &v1, true);
    assert!(!again, "only once");
    // the host asks us to unmute: we stay muted until we do it ourselves
    l["self"]["controls"] = json!({"audio": {"muted": false, "requestedToUnmute": true}});
    let v2 = parse_locus(&l, "me@example.com");
    let (i2, newly) = mute_update(Some(&i1), &v2, true);
    assert!(!newly && i2.local && i2.me.unmute_requested && !i2.me.remote);
}
#[test]
fn scrub_hides_addresses() {
    assert!(!scrub("user a@b.example not found").contains('@'));
}

/// Manual, live, ONE short test call (rings ≈ 4 s, then hangs up) to the single authorised address (someone who agreed
/// to it; the address here is a placeholder). Run with
/// `WEBOX_CALL_TEST_ONLY=jonas.weber@example.com cargo test live_call -- --ignored --nocapture` on weekdays 08–18.
#[test]
#[ignore]
fn live_call() {
    let guard = std::env::var(GUARD_ENV).unwrap_or_default();
    assert!(guard.contains('@'), "set {GUARD_ENV}=<the one address that agreed to a test call>");
    // This rings a real person, so it never runs by accident.
    assert_eq!(std::env::var("WEBOX_LIVE_CALL_OK").as_deref(), Ok("yes-the-user-asked-for-it"), "live calls need explicit permission");
    let dir = std::path::Path::new(&std::env::var("APPDATA").unwrap()).join("WebOx");
    let s = crate::system::load_settings(&dir);
    let token = s["token"].as_str().unwrap_or("").to_string();
    let kms = s["refresh_token"].as_str().filter(|r| !r.is_empty())
        .map(|r| (r.to_string(), crate::cloud::auth::saved_broker(s["idbroker_base"].as_str())));
    let c = crate::cloud::start(token, s["device"].as_str().map(String::from), kms);
    let end = Instant::now() + Duration::from_secs(60);
    let (mut dialled, mut rang_at, mut hung, mut audited) = (false, None::<Instant>, false, false);
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        if let Some(t) = rang_at { if !hung && t.elapsed() >= Duration::from_secs(5) { println!("hanging up"); c.req(Req::Call(CallReq::HangUp)); hung = true; } }
        match c.rx.recv_timeout(left.min(Duration::from_millis(500))) {
            Ok(Resp::Status(m)) if m == "Connected" && !dialled => { dialled = true; println!("dialling"); c.req(Req::Call(CallReq::Dial(guard.clone(), audio::Choice::default()))); }
            Ok(Resp::Status(m)) => println!("status: {}", if m.starts_with("calls:") { m } else { m.chars().take(40).collect() }),
            Ok(Resp::Call(e)) => {
                println!("call event: {:?}", e.state);
                if matches!(e.state, CallState::Dialing | CallState::Ringing) && rang_at.is_none() { rang_at = Some(Instant::now()); }
                if matches!(e.state, CallState::Ended(_)) && !audited { audited = true; c.req(Req::Call(CallReq::Audit)); }
            }
            Ok(Resp::Error(e)) => { println!("error: {}", scrub(&e)); if !hung && rang_at.is_none() && dialled { break; } }
            Ok(Resp::AuthFailed(_)) => { println!("auth failed"); return }
            Ok(_) => {}
            Err(_) => {}
        }
        if audited && hung { std::thread::sleep(Duration::from_secs(2)); // let the audit answer arrive
            while let Ok(r) = c.rx.try_recv() { if let Resp::Status(m) = r { println!("status: {m}"); } } break; }
    }
    if !hung && rang_at.is_some() { c.req(Req::Call(CallReq::HangUp)); std::thread::sleep(Duration::from_secs(2)); }
}

#[test]
fn peer_devices_gone_only_while_still_joined() {
    // every device LEFT but the participant JOINED: the grace case (the call waits before it ends)
    let mut l = locus("JOINED", "JOINED", true);
    l["participants"][1]["devices"] = json!([{"state": "LEFT", "url": "d1"}]);
    assert!(peer_devices_gone(&parse_locus(&l, "me@example.com")));
    // one device still JOINED (moved to another device): not gone, and the call stays connected
    l["participants"][1]["devices"] = json!([{"state": "LEFT", "url": "d1"}, {"state": "JOINED", "url": "d2"}]);
    let v = parse_locus(&l, "me@example.com");
    assert!(!peer_devices_gone(&v));
    assert_eq!(call_state(&v, true), Some(CallState::Connected));
    // the participant itself LEFT: a real hang-up, ended at once
    let v = parse_locus(&locus("JOINED", "LEFT", true), "me@example.com");
    assert!(!peer_devices_gone(&v));
    assert_eq!(call_state(&v, true), Some(CallState::Ended("left".into())));
}

#[test]
fn reasons_are_enum_values_only() {
    let mut l = locus("LEFT", "JOINED", true);
    l["self"]["reason"] = json!("MEDIA_INACTIVITY");
    l["participants"][1]["reason"] = json!("MOVED peer@example.com");
    let r = reasons(&l);
    assert!(r.contains("state=ACTIVE") && r.contains("self_reason=MEDIA_INACTIVITY"), "{r}");
    assert!(!r.contains('@') && !r.contains("example"), "{r}");
}

#[test]
fn roap_error_carries_its_type() {
    let v: Value = serde_json::from_str(&roap_local_sdp("ERROR", None, 3, false)).unwrap();
    assert_eq!(v["roapMessage"]["messageType"], "ERROR");
    assert_eq!(v["roapMessage"]["errorType"], "FAILED");
    assert_eq!(v["roapMessage"]["seq"], 3);
    assert!(v["roapMessage"]["sdps"].is_null() && v["roapMessage"]["tieBreaker"].is_null());
}

#[test]
fn media_drop_after_it_was_up_is_no_stall() {
    let t0 = Instant::now();
    // media was up once, ICE now recovering: no "no connection" notice (`ice_tick` restarts ICE instead)
    assert!(media_stall(t0 + Duration::from_secs(60), None, Some(t0), Some(t0), 500, Some(MediaState::Connecting)).is_none());
    // but a failed peer connection is still reported
    assert!(media_stall(t0 + Duration::from_secs(60), None, Some(t0), Some(t0), 500, Some(MediaState::Failed)).is_some());
}
