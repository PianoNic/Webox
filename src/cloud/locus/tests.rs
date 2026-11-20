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
    assert_eq!(v.others, vec![Party { name: "Peer Person".into(), email: "peer@example.com".into(), state: "NOTIFIED".into(), devices: vec![] }]);
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
        .map(|r| (r.to_string(), s["idbroker_base"].as_str().filter(|b| !b.is_empty()).unwrap_or(crate::cloud::auth::US_BROKER).to_string()));
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
