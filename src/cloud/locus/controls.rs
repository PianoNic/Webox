//! Participant and locus controls of a group call: the roster (everyone in the call with their mute state, raised hand
//! and role), muting another participant, raising / lowering a hand, removing someone, inviting someone, and ending the
//! call for everyone.
//!
//! Official client (spark-windows-desktop-ui.dll 46.11): `CallParticipantListViewModel::{removeParticipantFromMeeting,
//! unmuteParticipant, setMuteAllParticipantAudio, participantListItemAction}`, the log lines "Audio muteParticipant
//! request is sent", "Attempting to toggle raiseHand", "Attempting to lowerHand for participantId", "LowerHand
//! ParticipantContextMenuItem is invoked to lower one participant's hand" / "… to lower self hand", the context-menu ids
//! `MuteParticipantAudio`, `UnmuteParticipantAudio`, `RemoveParticipant`, `LowerHand`, `HostEndForAll`, and
//! `addParticipant` / `addParticipantButtonPressed`. QML: `ParticipantList/ParticipantItemDelegate.qml` (row 48 high,
//! 32 avatar, 28 control buttons), `PhoneWidget/ParticipantsPopup.qml` ("Remove from call"). The DLL builds no locus
//! URL for these itself (they live in the framework), so the **request shapes are webex-js-sdk's** (`plugin-meetings`
//! `MembersRequest` / `MeetingRequest`) and have not been seen on the wire for this account:
//!
//! * mute / unmute someone: `PATCH {locusUrl}/participant/{id}/controls {"audio":{"muted":bool}}` (unmuting someone else
//!   asks them to unmute: their `controls.audio.requestedToUnmute`)
//! * raise / lower a hand: `PATCH {locusUrl}/participant/{id}/controls {"hand":{"raised":bool}}`
//! * remove: `PUT {locusUrl}/participant/{id}/leave {"reason":id}`
//! * invite: `PUT {locusUrl}/participant {"invitees":[{"address":email}],"alertIfActive":true}`
//! * end for all: `POST {locusUrl}/end`
//!
//! The roster is read from the locus Webox already follows (`parse_locus`): `participants[].controls.audio.muted` +
//! `status.audioStatus` (muted, `participant_muted`), `controls.hand.raised` (hand), `controls.role.roles[]` MODERATOR or
//! the locus `host.id` (host).
use super::*;

/// One participant of the roster.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Member {
    /// The locus participant id (what the controls address).
    pub id: String, pub name: String, pub email: String,
    /// Participant state: JOINED, or IDLE / NOTIFIED while it still rings for them.
    pub state: String,
    pub muted: bool, pub hand: bool, pub host: bool,
    /// This is us.
    pub me: bool,
}
impl Member {
    pub fn joined(&self) -> bool { self.state == "JOINED" }
}

/// Everyone in the call as the roster shows them, and what we may do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Roster {
    /// Raised hands first (the official `SortByRaiseHand`), then us, then the others in the call, then the ones still
    /// being called; by name within each.
    pub members: Vec<Member>,
    /// We are host or moderator: we may lower others' hands, remove people and end the call for everyone.
    pub me_host: bool,
    /// More than a 1:1 call (a space call or meeting): raise hand and "end for all" apply.
    pub group: bool,
}
impl Roster {
    pub fn me(&self) -> Option<&Member> { self.members.iter().find(|m| m.me) }
    pub fn get(&self, id: &str) -> Option<&Member> { self.members.iter().find(|m| !id.is_empty() && m.id == id) }
    /// Nobody in the call holds the host role (a space call): Locus lets anyone remove a participant.
    pub fn hostless(&self) -> bool { !self.members.iter().any(|m| m.host) }
    pub fn joined(&self) -> usize { self.members.iter().filter(|m| m.joined()).count() }
    /// We may remove someone: host, or nobody is.
    pub fn may_remove(&self) -> bool { self.me_host || self.hostless() }
}

/// The participant holds the host / moderator role (webex-js-sdk `MemberUtil.isModerator` / `hasRole`).
fn is_host(l: &Value, p: &Value) -> bool {
    let roles = p["controls"]["role"]["roles"].as_array().into_iter().flatten()
        .any(|r| matches!(r["type"].as_str(), Some("MODERATOR" | "COHOST")) && r["hasRole"].as_bool() != Some(false));
    let host_id = st(&l["host"], "id");
    roles || p["moderator"].as_bool() == Some(true) || (!host_id.is_empty() && st(&p["person"], "id") == host_id)
}

/// The roster of a locus (`self_id`: our participant id; `my_email` the fallback to know us).
pub fn roster(l: &Value, self_id: &str, my_email: &str) -> Roster {
    let mut members: Vec<Member> = l["participants"].as_array().into_iter().flatten()
        .filter(|p| p["type"].as_str().is_none_or(|t| t == "USER"))
        .filter(|p| matches!(p["state"].as_str(), Some("JOINED" | "IDLE" | "NOTIFIED")))
        .map(|p| {
            let email = st(&p["person"], "email");
            let me = (!self_id.is_empty() && st(p, "id") == self_id) || (!my_email.is_empty() && email.eq_ignore_ascii_case(my_email));
            let name = [st(&p["person"], "name"), st(&p["person"], "displayName"), email.clone()].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
            Member { id: st(p, "id"), name, email, state: st(p, "state"), muted: participant_muted(p),
                hand: p["controls"]["hand"]["raised"].as_bool() == Some(true), host: is_host(l, p), me }
        }).collect();
    members.sort_by_key(|m| (!m.hand, !m.me, !m.joined(), m.name.to_lowercase()));
    let me_host = members.iter().any(|m| m.me && m.host);
    let group = st(&l["fullState"], "type") != "CALL" || members.iter().filter(|m| !m.me).count() > 1;
    Roster { members, me_host, group }
}

/// A control the user asked for from the roster.
#[derive(Clone, Debug, PartialEq)]
pub enum Ctl {
    /// Mute (true) / unmute (false) another participant.
    Mute(String, bool),
    /// Raise / lower a hand (ours, or lower another's as host).
    Hand(String, bool),
    /// Remove a participant from the call.
    Remove(String),
    /// Invite an address into the call.
    Invite(String),
    /// End the call for everyone.
    EndAll,
}

/// Method, URL and body of a control *(webex-js-sdk shapes, see the module doc)*.
pub fn request(locus_url: &str, c: &Ctl) -> (&'static str, String, Value) {
    let p = |id: &str, tail: &str| format!("{locus_url}/participant/{id}/{tail}");
    match c {
        Ctl::Mute(id, m) => ("PATCH", p(id, "controls"), json!({"audio": {"muted": m}})),
        Ctl::Hand(id, r) => ("PATCH", p(id, "controls"), json!({"hand": {"raised": r}})),
        Ctl::Remove(id) => ("PUT", p(id, "leave"), json!({"reason": id})),
        Ctl::Invite(a) => ("PUT", format!("{locus_url}/participant"), json!({"invitees": [{"address": a.trim().to_lowercase()}], "alertIfActive": true})),
        Ctl::EndAll => ("POST", format!("{locus_url}/end"), json!({})),
    }
}

/// Whether the control may be sent, checked against the roster before any request: only participants of this call
/// (never us where it makes no sense), only what our role allows, invites only to plain Webex addresses (and with the
/// test guard set only the allowed one).
pub fn check(r: &Roster, c: &Ctl, guard: Option<&str>) -> Result<(), String> {
    let other = |id: &str| r.get(id).filter(|m| !m.me).ok_or_else(|| crate::trq!("This person is no longer in the call.").into_owned());
    match c {
        Ctl::Mute(id, _) => { if !other(id)?.joined() { return Err(crate::trq!("This person hasn't joined the call yet.").into_owned()); } Ok(()) }
        Ctl::Hand(id, raise) => match r.get(id) {
            Some(m) if m.me => if r.group { Ok(()) } else { Err(crate::trq!("Raising a hand is only available in group calls.").into_owned()) },
            Some(_) if !raise && r.me_host => Ok(()),
            Some(_) => Err(crate::trq!("Only hosts can lower other people's hands.").into_owned()),
            None => Err(crate::trq!("This person is no longer in the call.").into_owned()),
        },
        Ctl::Remove(id) => { other(id)?; if r.may_remove() { Ok(()) } else { Err(crate::trq!("Only hosts can remove people.").into_owned()) } }
        Ctl::Invite(a) => {
            call_allowed(a, guard)?;
            if r.members.iter().any(|m| m.email.eq_ignore_ascii_case(a.trim())) { return Err(crate::trq!("This person is already in the call.").into_owned()); }
            Ok(())
        }
        Ctl::EndAll => if r.me_host && r.group { Ok(()) } else { Err(crate::trq!("Only hosts can end the call for everyone.").into_owned()) },
    }
}

/// Log name of a control (no ids, no addresses).
fn what(c: &Ctl) -> &'static str {
    match c { Ctl::Mute(_, true) => "mute other", Ctl::Mute(_, false) => "unmute other", Ctl::Hand(_, true) => "raise hand", Ctl::Hand(_, false) => "lower hand",
        Ctl::Remove(_) => "remove", Ctl::Invite(_) => "invite", Ctl::EndAll => "end for all" }
}

/// The user-facing failure text (English source, translated where it is used).
fn failed(c: &Ctl) -> &'static str {
    match c { Ctl::Mute(_, true) => "Mute failed", Ctl::Mute(_, false) => "Unmute failed",
        Ctl::Hand(_, true) => "Raising the hand failed", Ctl::Hand(_, false) => "Lowering the hand failed",
        Ctl::Remove(_) => "Removing failed", Ctl::Invite(_) => "Inviting failed", Ctl::EndAll => "Ending for everyone failed" }
}

impl Session {
    /// WRITE (only from the roster): sends one control for the current, answered call.
    pub(super) fn control(&mut self, c: Ctl) -> Result<(), String> {
        let Some(a) = self.calls.cur.as_ref().filter(|a| a.answered) else { return Ok(()) };
        let url = a.url.clone();
        if !webex_host(&url) { return Err(crate::trq!("Invalid call address.").into_owned()); }
        let r = a.view.as_ref().map(|v| v.roster.clone()).unwrap_or_default();
        check(&r, &c, std::env::var(GUARD_ENV).ok().as_deref())?; // before any network request
        let (method, u, body) = request(&url, &c);
        let res = self.http(method, &u, Some(body));
        dbg_conn(&format!("call: {} -> {}", what(&c), if res.is_ok() { "ok" } else { "failed" }));
        if c == Ctl::EndAll && res.is_ok() { self.end_call("left"); }
        res.map(|_| ()).map_err(|e| format!("{}: {}", crate::trq!(failed(&c)), scrub(&e)))
    }

    /// Tells the UI the roster when it changed (`Resp::CallRoster`).
    pub(super) fn roster_sync(&mut self, v: &LocusView) {
        let Some(c) = self.calls.cur.as_mut() else { return };
        if c.roster_sent.as_ref() == Some(&v.roster) { return; }
        c.roster_sent = Some(v.roster.clone());
        let _ = self.out.send(Resp::CallRoster(v.roster.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const L: &str = "https://locus-a.wbx2.com/locus/api/v1/loci/abc";

    /// A space call's locus in the shape Locus sends (values invented): we are host, one muted by others, one muted by
    /// themselves with the hand up, one still being called, one who left, and a room device.
    fn group() -> Value {
        json!({"url": L, "fullState": {"type": "MEETING", "state": "ACTIVE", "active": true},
            "host": {"id": "person-me", "name": "Me"},
            "self": {"id": "self-1", "state": "JOINED", "url": format!("{L}/participant/self-1"), "person": {"id": "person-me", "email": "me@example.com", "name": "Me"}},
            "participants": [
                {"id": "self-1", "type": "USER", "state": "JOINED", "person": {"id": "person-me", "email": "me@example.com", "name": "Me"},
                    "status": {"audioStatus": "SENDRECV"}, "controls": {"role": {"roles": [{"type": "MODERATOR", "hasRole": true}]}}},
                {"id": "pa", "type": "USER", "state": "JOINED", "person": {"id": "person-a", "email": "anna@example.com", "name": "Anna"},
                    "status": {"audioStatus": "SENDRECV"}, "controls": {"audio": {"muted": true}, "hand": {"raised": false}}},
                {"id": "pb", "type": "USER", "state": "JOINED", "person": {"id": "person-b", "email": "bob@example.com", "name": "Bob"},
                    "status": {"audioStatus": "RECVONLY"}, "controls": {"hand": {"raised": true}}},
                {"id": "pc", "type": "USER", "state": "NOTIFIED", "person": {"id": "person-c", "email": "carl@example.com", "name": "Carl"}},
                {"id": "pd", "type": "USER", "state": "LEFT", "person": {"id": "person-d", "email": "dora@example.com", "name": "Dora"}},
                {"id": "pr", "type": "RESOURCE_ROOM", "state": "JOINED", "person": {"id": "room", "name": "Room"}}]})
    }

    #[test]
    fn roster_lists_everyone_in_the_call_with_mute_hand_and_role() {
        let v = parse_locus(&group(), "me@example.com");
        let r = &v.roster;
        let names: Vec<_> = r.members.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["Bob", "Me", "Anna", "Carl"], "hand first, then me, then joined, then ringing; left and rooms dropped");
        assert!(r.me_host && r.group);
        let anna = r.get("pa").unwrap();
        assert!(anna.muted && !anna.hand && !anna.host, "muted by others");
        let bob = r.get("pb").unwrap();
        assert!(bob.muted && bob.hand, "muted himself (RECVONLY), hand up");
        assert!(r.me().unwrap().host && !r.me().unwrap().muted);
        assert!(!r.get("pc").unwrap().joined());
        assert_eq!(r.joined(), 3);
    }

    #[test]
    fn a_one_to_one_call_is_no_group_and_has_no_host() {
        let mut l = group();
        l["fullState"]["type"] = json!("CALL");
        l["host"] = Value::Null;
        l["participants"] = json!([l["participants"][0].clone(), l["participants"][1].clone()]);
        l["participants"][0]["controls"] = json!({});
        let r = parse_locus(&l, "me@example.com").roster;
        assert!(!r.group && !r.me_host && r.hostless());
        assert_eq!(check(&r, &Ctl::Hand("self-1".into(), true), None), Err(crate::trq!("Raising a hand is only available in group calls.").into_owned()));
        assert!(check(&r, &Ctl::EndAll, None).is_err());
        assert_eq!(check(&r, &Ctl::Remove("pa".into()), None), Ok(()), "hostless: Locus lets anyone remove");
    }

    #[test]
    fn control_requests_have_the_sdk_shapes() {
        assert_eq!(request(L, &Ctl::Mute("pa".into(), true)), ("PATCH", format!("{L}/participant/pa/controls"), json!({"audio": {"muted": true}})));
        assert_eq!(request(L, &Ctl::Hand("self-1".into(), false)), ("PATCH", format!("{L}/participant/self-1/controls"), json!({"hand": {"raised": false}})));
        assert_eq!(request(L, &Ctl::Remove("pb".into())), ("PUT", format!("{L}/participant/pb/leave"), json!({"reason": "pb"})));
        assert_eq!(request(L, &Ctl::Invite(" Zoe@Example.com ".into())), ("PUT", format!("{L}/participant"), json!({"invitees": [{"address": "zoe@example.com"}], "alertIfActive": true})));
        assert_eq!(request(L, &Ctl::EndAll), ("POST", format!("{L}/end"), json!({})));
    }

    #[test]
    fn controls_are_checked_against_the_roster_and_role() {
        let r = parse_locus(&group(), "me@example.com").roster;
        assert_eq!(check(&r, &Ctl::Mute("pa".into(), false), None), Ok(()));
        assert!(check(&r, &Ctl::Mute("self-1".into(), true), None).is_err(), "own mute goes through CallReq::Mute");
        assert!(check(&r, &Ctl::Mute("pc".into(), true), None).is_err(), "not joined yet");
        assert!(check(&r, &Ctl::Mute("pd".into(), true), None).is_err(), "left: not in the roster");
        assert!(check(&r, &Ctl::Mute("nobody".into(), true), None).is_err());
        assert_eq!(check(&r, &Ctl::Hand("self-1".into(), true), None), Ok(()));
        assert_eq!(check(&r, &Ctl::Hand("pb".into(), false), None), Ok(()), "host lowers Bob's hand");
        assert!(check(&r, &Ctl::Hand("pb".into(), true), None).is_err(), "nobody raises another's hand");
        assert_eq!(check(&r, &Ctl::Remove("pa".into()), None), Ok(()));
        assert_eq!(check(&r, &Ctl::EndAll, None), Ok(()));
        // not host while a host exists
        let mut l = group();
        l["participants"][0]["controls"] = json!({});
        l["host"] = json!({"id": "person-a"});
        let r2 = parse_locus(&l, "me@example.com").roster;
        assert!(!r2.me_host && r2.get("pa").unwrap().host);
        assert!(check(&r2, &Ctl::Remove("pb".into()), None).is_err());
        assert!(check(&r2, &Ctl::Hand("pb".into(), false), None).is_err());
        assert!(check(&r2, &Ctl::EndAll, None).is_err());
        // invites: plain addresses only, the test guard, nobody twice
        assert_eq!(check(&r, &Ctl::Invite("zoe@example.com".into()), None), Ok(()));
        assert!(check(&r, &Ctl::Invite("+41441234567".into()), None).is_err());
        assert!(check(&r, &Ctl::Invite("zoe@example.com".into()), Some("test@example.com")).is_err(), "guard: only the allowed address");
        assert!(check(&r, &Ctl::Invite("ANNA@example.com".into()), None).is_err(), "already in the call");
    }

    #[test]
    fn a_hand_raised_in_a_delta_reaches_the_roster() {
        let full = group();
        let delta = json!({"baseSequence": {}, "participants": [{"id": "pa", "type": "USER", "state": "JOINED", "person": {"id": "person-a", "email": "anna@example.com", "name": "Anna"},
            "status": {"audioStatus": "SENDRECV"}, "controls": {"audio": {"muted": false}, "hand": {"raised": true}}}]});
        let r = parse_locus(&merge_delta(&full, &delta), "me@example.com").roster;
        let anna = r.get("pa").unwrap();
        assert!(anna.hand && !anna.muted);
        assert_eq!(r.members[0].name, "Anna", "raised hands sort first (Anna before Bob by name)");
    }
}
