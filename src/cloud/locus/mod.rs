//! Calling over the Webex Locus service: signalling + ROAP media negotiation.
//!
//! # What a 1:1 call needs, and what is implemented
//! * **Signalling.** `POST {locus}/loci/call?alternateRedirect=true` (body: `device`, `usingResource`,
//!   `moveMediaToResource`, `correlationId`, `respOnlySdp`, `allowMultiDevice`, `ensureConversation`, `supportsNativeLobby`,
//!   `clientMediaPreferences`, `invitee:{address}`, `localMedias`) creates the call and alerts the callee.
//!   The answer holds `locus` (url, `self.id`, `participants[].state`, `fullState.{type,active}`). Changes arrive over the
//!   mercury socket as `locus.difference` / `locus.state_message` events (the socket we already run). Cancel / hang up =
//!   `PUT {locusUrl}/participant/{selfId}/leave`; decline an incoming call = `PUT {locusUrl}/participant/decline`;
//!   "I am ringing" = `PUT {locusUrl}/participant/alert`; answering = `POST {locusUrl}/participant` (join) with media.
//!   Needs only the bearer token and the device url from the registration we already have (no extra device capability).
//! * **Media negotiation (ROAP v2).** The create / join body also carries `localMedias:[{localSdp}]`, where `localSdp`
//!   is a JSON string `{roapMessage:{messageType:"OFFER",version:"2",seq:1,tieBreaker:4294967294,sdps:[SDP]},audioMuted,videoMuted}`
//!   and the SDP is a normal WebRTC offer with all ICE candidates (no trickle). The server's ANSWER arrives as mercury
//!   `locus.message.roap` (`message.sdps[0]`, matched by `correlationId`) or inline in `mediaConnections[].remoteSdp`; we set it and
//!   send `OK` with the same `seq` via `PUT {self.url}/media` (`device`, `correlationId`, `localMedias:[{localSdp,mediaId}]`).
//!   Server OFFERs are answered with an ANSWER. A call created without the offer leaves the callee stuck on "connecting",
//!   so the offer is always built first (no offer, no request).
//! * **Media transport** (`media.rs`): webrtc-rs peer connection (ICE, DTLS-SRTP, RTP), Opus 48 kHz via `ropus`, devices via `cpal` (`audio.rs`).
//! * **Not done:** TURN discovery (`TURN_DISCOVERY_REQUEST`, only needed when UDP to the media cluster is blocked), the
//!   `reachability` data (media cluster choice), video / screen share, noise removal, echo cancellation. Interop with
//!   Locus' media servers is only tested against a second local webrtc-rs peer.
//!
//! "Verbunden" is shown only when the callee's participant state is JOINED **and** our peer connection is `Connected`.
//!
//! Safety: `WEBOX_CALL_TEST_ONLY=<address>` makes every outgoing call to any other address fail before any network request,
//! only e-mail style addresses can be dialled (no phone numbers), one call at a time, at most one dial every 10 s.
use super::media::{Media, MediaState};
use super::*;
use crate::media::audio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const GUARD_ENV: &str = "WEBOX_CALL_TEST_ONLY";
const REDIAL_COOLDOWN: Duration = Duration::from_secs(10);
const TICK: Duration = Duration::from_secs(1);

/// `Dialing` = request sent / invitee not alerted yet; `Ringing` = invitee's device is alerting; `Connecting` = the invitee
/// joined but our media leg (ICE/DTLS) isn't up yet; `Connected` only when both are true.
#[derive(Clone, Debug, PartialEq)]
pub enum CallState { Dialing, Ringing, Incoming, Connecting, Connected, Ended(String) }
#[derive(Clone, Debug)]
pub struct CallEvent { pub state: CallState, pub name: String, pub email: String, pub incoming: bool }
/// `HangUp` also declines a call that is still ringing for us. `Audit` reports remaining active calls, `Cleanup` leaves
/// active calls that this very device (and nobody else) is joined to.
pub enum CallReq { Dial(String, audio::Choice), Answer(audio::Choice), HangUp, Mute(bool), Devices(audio::Choice), SmartMic(audio::SmartMic), #[allow(dead_code)] Audit, Cleanup,
    // keypad tones during a connected call
    Dtmf(String) }

/// `POST {locusUrl}/sendDtmf` body.
pub fn dtmf_body(device_url: &str, tones: &str, correlation_id: &str) -> Value {
    json!({"deviceUrl": device_url, "dtmf": {"correlationId": correlation_id, "tones": tones}})
}
/// Tones a keypad may send (0-9, *, #; one or more).
pub fn dtmf_ok(t: &str) -> bool { !t.is_empty() && t.chars().all(|c| c.is_ascii_digit() || c == '*' || c == '#') }

/// One participant of a locus as far as the call UI cares.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Party { pub name: String, pub email: String, pub state: String, pub devices: Vec<String> }
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocusView { pub url: String, pub kind: String, pub active: bool, pub self_id: String, pub self_state: String, pub self_url: String, pub self_devices: Vec<(String, String)>, pub others: Vec<Party> }

/// Reads a locus object (the `locus` of a join answer, a full GET, or a mercury `locus.difference` payload).
pub fn parse_locus(l: &Value, my_email: &str) -> LocusView {
    let full = &l["fullState"];
    let self_id = st(&l["self"], "id");
    let others = l["participants"].as_array().into_iter().flatten().filter(|p| {
        let email = st(&p["person"], "email");
        let me = (!self_id.is_empty() && st(p, "id") == self_id) || (!my_email.is_empty() && email.eq_ignore_ascii_case(my_email));
        !me && p["type"].as_str().is_none_or(|t| t == "USER")
    }).map(|p| Party { name: st(&p["person"], "name"), email: st(&p["person"], "email"), state: st(p, "state"),
        devices: p["devices"].as_array().into_iter().flatten().map(|d| st(d, "state")).collect() }).collect();
    LocusView {
        url: [st(l, "url"), st(l, "locusUrl")].into_iter().find(|u| !u.is_empty()).unwrap_or_default(),
        kind: st(full, "type"),
        active: full["active"].as_bool().unwrap_or_else(|| full["state"].as_str() == Some("ACTIVE")),
        self_id, self_state: st(&l["self"], "state"), self_url: st(&l["self"], "url"), others,
        self_devices: l["self"]["devices"].as_array().into_iter().flatten().map(|d| (st(d, "url"), st(d, "state"))).collect(),
    }
}

/// Which locus an event is about: mercury sends `{eventType, locusUrl?, locus:{url,…}}`; a full answer may wrap `{locus:{…}}`.
pub fn locus_of(d: &Value) -> &Value { if d["locus"].is_object() { &d["locus"] } else { d } }
pub fn locus_url_of(d: &Value) -> String {
    [st(d, "locusUrl"), st(&d["locus"], "url"), st(&d["locus"], "locusUrl"), st(d, "url")].into_iter().find(|u| !u.is_empty()).unwrap_or_default()
}

/// A `locus.*` event while no call of ours exists. The event's own `locus` body decides when it says enough: a meeting /
/// other kind, an inactive locus or one not ringing for me is not read. Only an event without that information (no
/// `fullState.type`) is read with a full GET, so an incoming call is never missed.
pub fn idle_event_needs_read(d: &Value, my_email: &str) -> bool {
    let l = locus_of(d);
    if st(&l["fullState"], "type").is_empty() { return true }
    let v = parse_locus(l, my_email);
    v.kind == "CALL" && v.active && call_state(&v, false) == Some(CallState::Incoming)
}

/// The call's state from our side. `outgoing`: we placed it (we joined on creation).
pub fn call_state(v: &LocusView, outgoing: bool) -> Option<CallState> {
    let any = |s: &str| v.others.iter().any(|p| p.state == s);
    let ended = |why: &str| Some(CallState::Ended(why.into()));
    if v.self_state == "DECLINED" { return ended("declined"); }
    if v.self_state == "LEFT" { return ended("left"); }
    if !v.active && !v.kind.is_empty() {
        return ended(if any("DECLINED") { "declined" } else if outgoing { "unanswered" } else { "missed" });
    }
    if v.self_state == "JOINED" {
        if any("JOINED") { return Some(CallState::Connected); }
        if !v.others.is_empty() && v.others.iter().all(|p| p.state == "DECLINED") { return ended("declined"); }
        if !v.others.is_empty() && v.others.iter().all(|p| p.state == "LEFT") { return ended("left"); }
        let alerted = any("NOTIFIED") || v.others.iter().any(|p| p.devices.iter().any(|d| d == "NOTIFIED" || d == "ALERTING"));
        return Some(if alerted { CallState::Ringing } else { CallState::Dialing });
    }
    if !outgoing && matches!(v.self_state.as_str(), "IDLE" | "NOTIFIED" | "") {
        if any("JOINED") { return Some(CallState::Incoming); }
        if !v.others.is_empty() && v.others.iter().all(|p| p.state == "LEFT") { return ended("missed"); }
    }
    None
}

/// Only plain Webex addresses (e-mail style) may be dialled, and with the test guard set only the one allowed address.
pub fn call_allowed(addr: &str, guard: Option<&str>) -> Result<(), String> {
    let a = addr.trim();
    if a.is_empty() || a.contains(char::is_whitespace) || a.contains(':') || a.matches('@').count() != 1 || a.starts_with('@') || a.ends_with('@') {
        return Err("Angerufen werden können nur Webex-Adressen (E-Mail).".into());
    }
    match guard.map(str::trim).filter(|g| !g.is_empty()) {
        Some(g) if !g.eq_ignore_ascii_case(a) => Err("Testschutz aktiv: Anrufe nur an die freigegebene Adresse.".into()),
        _ => Ok(()),
    }
}

/// Log line with states only (enum-like values), never names or addresses.
fn describe(v: &LocusView) -> String {
    let others: Vec<String> = v.others.iter().map(|p| format!("{}[{}]", p.state, p.devices.join("/"))).collect();
    format!("locus kind={} active={} self={} others={}", v.kind, v.active, v.self_state, others.join(","))
}

/// Error text without anything that looks like an address (logs and toasts never carry personal values).
fn scrub(e: &str) -> String {
    e.split_whitespace().map(|w| if w.contains('@') { "<addr>" } else { w }).collect::<Vec<_>>().join(" ").chars().take(180).collect()
}

struct Active {
    url: String, self_id: String, self_url: String, media_id: String, correlation_id: String,
    outgoing: bool, name: String, email: String, last: Option<CallState>, answered: bool,
    media: Option<Media>, roap_seq: u32, roap_done: bool,
}

#[derive(Default)]
pub struct Calls {
    pub(super) device_type: String, pub(super) jobs: Option<Sender<Job>>,
    cur: Option<Active>, last_dial: Option<Instant>, live: Arc<AtomicBool>, engine: Option<audio::Engine>, muted: bool,
}

/// The ROAP v2 message envelope Locus expects in `localMedias[].localSdp` (JSON string).
pub fn roap_local_sdp(message_type: &str, sdp: Option<&str>, seq: u32, muted: bool) -> String {
    let mut m = json!({"messageType": message_type, "version": "2", "seq": seq});
    if let Some(s) = sdp { m["sdps"] = json!([s]); }
    if message_type == "OFFER" { m["tieBreaker"] = json!(4294967294u64); }
    json!({"roapMessage": m, "audioMuted": muted, "videoMuted": true}).to_string()
}

/// A ROAP message out of a mercury `locus.message.roap` event: (messageType, seq, first SDP).
pub fn roap_in(d: &Value) -> Option<(String, u32, Option<String>)> {
    if d["eventType"].as_str() != Some("locus.message.roap") { return None; }
    let m = &d["message"];
    Some((st(m, "messageType"), m["seq"].as_u64().unwrap_or(0) as u32, m["sdps"].as_array().and_then(|a| a.first()).and_then(Value::as_str).map(String::from)))
}

/// The answer a join / media response may already carry in `mediaConnections[].remoteSdp` (a JSON string holding a roapMessage).
pub fn remote_roap(r: &Value) -> Option<(String, u32, Option<String>)> {
    let rs = r["mediaConnections"].as_array()?.iter().find_map(|c| c["remoteSdp"].as_str())?;
    let v: Value = serde_json::from_str(rs).ok()?;
    let m = if v["roapMessage"].is_object() { &v["roapMessage"] } else { &v };
    Some((st(m, "messageType"), m["seq"].as_u64().unwrap_or(0) as u32, m["sdps"].as_array().and_then(|a| a.first()).and_then(Value::as_str).map(String::from)))
}

#[cfg(test)]
mod tests;

mod session;
