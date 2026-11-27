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
//! * **Not done:** media over TCP / TLS 443 (the server's TCP and `xTLS` candidates are dropped, webrtc-rs only does UDP
//!   ICE) and TURN discovery (`TURN_DISCOVERY_REQUEST`), both only needed when UDP to the media cluster is blocked; the
//!   `reachability` data (media cluster choice). Interop with Locus' media servers is only tested against a second local
//!   webrtc-rs peer.
//! * **Camera video** (`video.rs`): all four m-lines are in the first offer (`Media::offer`); our camera on
//!   `video/main` with `videoMuted` following it, the far end's camera decoded into `video_rx::MAIN`.
//! * **Screen share**, sending: `share.rs` (capture, floor request on the locus' `content` media share; the `slides`
//!   lines exist from the first offer). Receiving: the `content` floor GRANTED to the other side (`share_rx::remote_sharer`);
//!   its H.264 is decoded in `cloud/share_rx.rs` and shown as the call window's stage.
//! * **Locus state** follows the official client's `SequenceManager`: full loci newer than ours replace it, deltas
//!   (`baseSequence`) are merged onto the sequence they name, a delta we can't place (missed / out-of-order event) makes
//!   us re-read the whole locus (`apply_event`). The call ends as soon as the other side declined or left
//!   (`shouldDisconnectOne2OneCall`), and an incoming call answered on another of my devices stops ringing here.
//!
//! "Connected" is shown only when the callee's participant state is JOINED **and** our peer connection is `Connected`;
//! a call whose media doesn't come up (no ROAP answer, no ICE / DTLS, no RTP) is reported to the user (`media_stall`).
//! Media that was up and drops is reconnected the official way (`recover_step`, `Session::recovery_tick`): a new media
//! connection with a new ROAP OFFER on the same participant, at most `MEDIA_RECONNECT_MAX` times, then the call ends;
//! paused while offline, and at once when the local network address changes.
//!
//! Safety: `WEBOX_CALL_TEST_ONLY=<address>` makes every outgoing call to any other address fail before any network request,
//! only e-mail style addresses can be dialled (no phone numbers), one call at a time, at most one dial every 10 s.
use super::media::{route_ip, route_probe, route_target, Media, MediaState, Net};
use super::*;
use crate::media::audio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const GUARD_ENV: &str = "WEBOX_CALL_TEST_ONLY";
const REDIAL_COOLDOWN: Duration = Duration::from_secs(10);
/// The call's tick: re-evaluates the known locus (media may have connected); it reads the locus only when `poll_due`.
const TICK: Duration = Duration::from_secs(1);
/// Locus state comes with the Mercury events (each carries the full locus); a GET is only the fallback: every 15 s until
/// connected, every 30 s while connected, and at most every 2 s for an event without the full locus.
const POLL_RINGING: Duration = Duration::from_secs(15);
const POLL_CONNECTED: Duration = Duration::from_secs(30);
const MIN_READ_GAP: Duration = Duration::from_secs(2);
/// Media counters in the debug log, from memory (never a request).
const STATS_EVERY: Duration = Duration::from_secs(10);

/// Whether the tick at `now` should read the locus: never read yet, an event asked for a read (`stale`, at most every
/// `MIN_READ_GAP`), or the fallback poll is due.
pub fn poll_due(read_at: Option<Instant>, now: Instant, connected: bool, stale: bool) -> bool {
    let Some(t) = read_at else { return true };
    let since = now.saturating_duration_since(t);
    (stale && since >= MIN_READ_GAP) || since >= if connected { POLL_CONNECTED } else { POLL_RINGING }
}

/// The full locus a Mercury event carries (`fullState`, `self`, `participants`), if it does.
pub fn full_locus(d: &Value) -> Option<&Value> {
    let l = locus_of(d);
    (!st(&l["fullState"], "type").is_empty() && l["self"].is_object() && l["participants"].is_array()).then_some(l)
}

/// The locus' sequence number (the newest of `sequence.entries` / `rangeEnd`): events can arrive out of order.
pub fn locus_seq(l: &Value) -> u64 { seq_of(&l["sequence"]) }

/// The newest number of a locus sequence object (`{entries:[…], rangeStart, rangeEnd}`; 0 = none).
fn seq_of(s: &Value) -> u64 {
    let last = s["entries"].as_array().and_then(|e| e.iter().filter_map(Value::as_u64).max()).unwrap_or(0);
    last.max(s["rangeEnd"].as_u64().unwrap_or(0))
}

/// What to do with a locus that came with an event, like the official client's `SequenceManager::processEvent`
/// (compareResult `Update` / `Discard` / `Desync`): a full locus newer than ours replaces it, a delta (`baseSequence`) is
/// merged only when its base is exactly the sequence we hold, anything older is dropped, and a delta whose base we don't
/// hold (a missed or out-of-order event, or the first event being a delta) means our copy is stale: re-read the locus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SeqCmp { Update, Discard, Desync }

pub fn compare_seq(current: u64, l: &Value) -> SeqCmp {
    let incoming = locus_seq(l);
    if current != 0 && incoming != 0 && incoming <= current { return SeqCmp::Discard; }
    if !is_delta(l) { return SeqCmp::Update; }
    if current != 0 && seq_of(&l["baseSequence"]) == current { SeqCmp::Update } else { SeqCmp::Desync }
}

/// A delta locus (only what changed since `baseSequence`).
pub fn is_delta(l: &Value) -> bool { l["baseSequence"].is_object() }

/// Applies a delta to the full locus we hold: top-level objects are replaced, participants are replaced or added by id.
pub fn merge_delta(full: &Value, delta: &Value) -> Value {
    let mut out = full.clone();
    let (Some(o), Some(d)) = (out.as_object_mut(), delta.as_object()) else { return delta.clone() };
    for (k, v) in d {
        match k.as_str() {
            "baseSequence" => {}
            "participants" => {
                let mut ps = o.get("participants").and_then(Value::as_array).cloned().unwrap_or_default();
                for p in v.as_array().into_iter().flatten() {
                    let id = p["id"].as_str().unwrap_or("");
                    match ps.iter_mut().find(|q| !id.is_empty() && q["id"].as_str().unwrap_or("") == id) { Some(q) => *q = p.clone(), None => ps.push(p.clone()) }
                }
                o.insert(k.clone(), Value::Array(ps));
            }
            _ => { o.insert(k.clone(), v.clone()); }
        }
    }
    out
}

/// What a Mercury `locus.*` event does to the locus we hold (`held`, sequence `held_seq`).
#[derive(Debug, PartialEq)]
pub enum Apply {
    /// The locus to apply now (the event's full locus, or the delta merged into ours).
    Use(Value),
    /// Older than (or the same as) what we hold.
    Discard,
    /// The event isn't enough (no full locus, or a delta whose base we don't hold): re-read the locus with a GET.
    Read,
}

pub fn apply_event(held: &Value, held_seq: u64, d: &Value) -> Apply {
    let l = locus_of(d);
    if l["sequence"].is_object() {
        match compare_seq(held_seq, l) {
            SeqCmp::Discard => return Apply::Discard,
            SeqCmp::Desync => return Apply::Read,
            SeqCmp::Update if is_delta(l) => return if held.is_object() { Apply::Use(merge_delta(held, l)) } else { Apply::Read },
            SeqCmp::Update => {}
        }
    } else if is_delta(l) { return Apply::Read }
    full_locus(d).map_or(Apply::Read, |l| Apply::Use(l.clone()))
}

/// Two locus urls name the same locus: equal, or the same `/loci/{id}` on another Locus host (events may carry the url
/// of the host that sent them).
pub fn same_locus(a: &str, b: &str) -> bool {
    let id = |u: &str| u.split("/loci/").nth(1).map(|r| r.split(['/', '?']).next().unwrap_or("").to_string()).filter(|s| !s.is_empty());
    a == b || matches!((id(a), id(b)), (Some(x), Some(y)) if x == y)
}

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
    Dtmf(String),
    /// A roster control (mute / remove someone, raise hand, invite, end for all); see `controls.rs`.
    Control(controls::Ctl),
    /// Start sharing the picked screen / window (`Some`), or stop (`None`); see `share.rs`.
    Share(Option<share::ShareReq>),
    /// "Meet" / "Join" in a group space (its id): join the space's meeting, or start it (session.rs `join_space`).
    JoinSpace(String, audio::Choice),
    /// Our camera in the call: on with this camera (`Some`; again with another device = camera switch), or off (`None`).
    /// May be sent before `Dial` / `Answer`: the camera then starts once the call's media is up. See `video.rs`.
    /// (The call window's video button sends it; until that lands only the tests do.)
    #[allow(dead_code)]
    Video(Option<crate::media::camera::PreviewKey>) }

/// `POST {locusUrl}/sendDtmf` body.
pub fn dtmf_body(device_url: &str, tones: &str, correlation_id: &str) -> Value {
    json!({"deviceUrl": device_url, "dtmf": {"correlationId": correlation_id, "tones": tones}})
}
/// Tones a keypad may send (0-9, *, #; one or more).
pub fn dtmf_ok(t: &str) -> bool { !t.is_empty() && t.chars().all(|c| c.is_ascii_digit() || c == '*' || c == '#') }

/// One participant of a locus as far as the call UI cares.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Party { pub name: String, pub email: String, pub state: String, pub devices: Vec<String>, pub muted: bool }
impl Party {
    /// Left the call: participant state LEFT, or every one of its devices LEFT (the official client's
    /// `otherPairedDevice->left`).
    pub fn left(&self) -> bool { self.state == "LEFT" || (!self.devices.is_empty() && self.devices.iter().all(|d| d == "LEFT")) }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocusView { pub url: String, pub kind: String, pub active: bool, pub self_id: String, pub self_state: String, pub self_url: String, pub self_devices: Vec<(String, String)>, pub others: Vec<Party>,
    /// Our audio as the locus has it: muted by someone else (host / moderator), whether we may unmute ourselves, and
    /// whether we were asked to unmute (`self.controls.audio.{muted, disallowUnmute, requestedToUnmute}`).
    pub mute: SelfMute,
    /// Everyone in the call for the participants panel (`controls::roster`).
    pub roster: controls::Roster }

/// Our own mute state from the locus (`self.controls.audio`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SelfMute { pub remote: bool, pub unmute_allowed: bool, pub unmute_requested: bool }

/// A participant's audio is muted: muted by someone else (`controls.audio.muted`), or muted by themselves, which the
/// locus shows as `status.audioStatus` RECVONLY (they still hear, send nothing). The rule of webex-js-sdk's
/// `MemberUtil.isMuted`; the official client's roster tracks the same pair (`isRemoteMuted` / `isLocalAudioMuted`).
pub fn participant_muted(p: &Value) -> bool {
    let remote = p["controls"]["audio"]["muted"].as_bool();
    if remote == Some(true) { return true; }
    match p["status"]["audioStatus"].as_str() { Some("RECVONLY" | "INACTIVE") => true, Some("SENDRECV" | "SENDONLY") => false, _ => remote.unwrap_or(false) }
}

/// Our own mute controls from the locus' `self`.
pub fn self_mute(s: &Value) -> SelfMute {
    let a = &s["controls"]["audio"];
    SelfMute { remote: a["muted"].as_bool() == Some(true), unmute_allowed: a["disallowUnmute"].as_bool() != Some(true),
        unmute_requested: a["requestedToUnmute"].as_bool() == Some(true) }
}

/// What the call window shows about mute (sent when it changes): the other side's microphone, ours as the locus has it
/// (muted by others / unmute not allowed / asked to unmute), and whether our microphone is muted here (`local`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MuteInfo { pub peer_muted: bool, pub me: SelfMute, pub local: bool }

/// The mute state to show for a locus, and whether our microphone must go mute now: someone else just muted us (the
/// official client: "muteAudio() when RemoteMuted and LocalAudioUnmuted"; it never unmutes us by itself).
pub fn mute_update(prev: Option<&MuteInfo>, v: &LocusView, local: bool) -> (MuteInfo, bool) {
    let newly_remote = v.mute.remote && !prev.is_some_and(|p| p.me.remote);
    let local = local || v.mute.remote;
    (MuteInfo { peer_muted: v.others.iter().any(|p| p.muted && !p.left()), me: v.mute, local }, newly_remote)
}

/// `PUT {self.url}/media` without a ROAP message: our audio / video mute state for the other participants (webex-js-sdk
/// `remoteAudioVideoToggle`; the official client logs "<update_media>: normal flow" next to the ROAP one). Locus then
/// shows our participant's `status.audioStatus` RECVONLY (muted) or SENDRECV.
pub fn local_mute_body(device: Value, correlation_id: &str, media_id: &str, audio_muted: bool, video_muted: bool) -> Value {
    json!({"device": device, "usingResource": null, "correlationId": correlation_id, "respOnlySdp": true,
        "clientMediaPreferences": {"preferTranscoding": true},
        "localMedias": [{"localSdp": json!({"audioMuted": audio_muted, "videoMuted": video_muted}).to_string(), "mediaId": media_id}]})
}

/// `PATCH {self.url}/controls` body that unmutes us after someone else muted us (allowed unless `disallowUnmute`).
pub fn unmute_controls_body() -> Value { json!({"audio": {"muted": false}}) }

fn self_id_of(l: &Value) -> String { st(&l["self"], "id") }

/// Reads a locus object (the `locus` of a join answer, a full GET, or a mercury `locus.difference` payload).
pub fn parse_locus(l: &Value, my_email: &str) -> LocusView {
    let full = &l["fullState"];
    let self_id = st(&l["self"], "id");
    let others = l["participants"].as_array().into_iter().flatten().filter(|p| {
        let email = st(&p["person"], "email");
        let me = (!self_id.is_empty() && st(p, "id") == self_id) || (!my_email.is_empty() && email.eq_ignore_ascii_case(my_email));
        !me && p["type"].as_str().is_none_or(|t| t == "USER")
    }).map(|p| Party { name: st(&p["person"], "name"), email: st(&p["person"], "email"), state: st(p, "state"),
        devices: p["devices"].as_array().into_iter().flatten().map(|d| st(d, "state")).collect(), muted: participant_muted(p) }).collect();
    LocusView {
        url: [st(l, "url"), st(l, "locusUrl")].into_iter().find(|u| !u.is_empty()).unwrap_or_default(),
        kind: st(full, "type"),
        // `fullState.state` INACTIVE / TERMINATING ends the call even while `active` still says true
        active: full["active"].as_bool().unwrap_or_else(|| full["state"].as_str() == Some("ACTIVE")) && !matches!(full["state"].as_str(), Some("INACTIVE" | "TERMINATING")),
        self_id, self_state: st(&l["self"], "state"), self_url: st(&l["self"], "url"), others,
        self_devices: l["self"]["devices"].as_array().into_iter().flatten().map(|d| (st(d, "url"), st(d, "state"))).collect(),
        mute: self_mute(&l["self"]),
        roster: controls::roster(l, &self_id_of(l), my_email),
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
///
/// Like the official client's `shouldDisconnectOne2OneCall` (`other->declinedOrLeft`, `otherPairedDevice->left`,
/// `self->declinedOrLeft`): the call ends as soon as the other side declined or left, whatever `fullState` says yet.
pub fn call_state(v: &LocusView, outgoing: bool) -> Option<CallState> {
    let joined = |p: &Party| p.state == "JOINED" && !p.left();
    let any = |s: &str| v.others.iter().any(|p| p.state == s);
    let all = |f: &dyn Fn(&Party) -> bool| !v.others.is_empty() && v.others.iter().all(f);
    let ended = |why: &str| Some(CallState::Ended(why.into()));
    if v.self_state == "DECLINED" { return ended("declined"); }
    if v.self_state == "LEFT" { return ended("left"); }
    if !v.active && !v.kind.is_empty() {
        let was_up = v.self_state == "JOINED" && v.others.iter().any(|p| p.left());
        return ended(if any("DECLINED") { "declined" } else if was_up && outgoing { "left" } else if outgoing { "unanswered" } else { "missed" });
    }
    if v.self_state == "JOINED" {
        if v.others.iter().any(joined) { return Some(CallState::Connected); }
        if all(&|p: &Party| p.state == "DECLINED") { return ended("declined"); }
        if all(&|p: &Party| p.left() || p.state == "DECLINED") { return ended("left"); }
        let alerted = any("NOTIFIED") || v.others.iter().any(|p| p.devices.iter().any(|d| d == "NOTIFIED" || d == "ALERTING"));
        return Some(if alerted { CallState::Ringing } else { CallState::Dialing });
    }
    if !outgoing && matches!(v.self_state.as_str(), "IDLE" | "NOTIFIED" | "") {
        if v.others.iter().any(joined) { return Some(CallState::Incoming); }
        if all(&|p: &Party| p.left() || p.state == "DECLINED") { return ended("missed"); }
    }
    None
}

/// Only plain Webex addresses (e-mail style) may be dialled, and with the test guard set only the one allowed address.
pub fn call_allowed(addr: &str, guard: Option<&str>) -> Result<(), String> {
    let a = addr.trim();
    if a.is_empty() || a.contains(char::is_whitespace) || a.contains(':') || a.matches('@').count() != 1 || a.starts_with('@') || a.ends_with('@') {
        return Err(crate::trq!("Only Webex addresses (email) can be called.").into_owned());
    }
    match guard.map(str::trim).filter(|g| !g.is_empty()) {
        Some(g) if !g.eq_ignore_ascii_case(a) => Err(crate::trq!("Test guard active: calls only to the allowed address.").into_owned()),
        _ => Ok(()),
    }
}

/// Log line with states only (enum-like values), never names or addresses.
fn describe(v: &LocusView) -> String {
    let others: Vec<String> = v.others.iter().map(|p| format!("{}[{}]", p.state, p.devices.join("/"))).collect();
    format!("locus kind={} active={} self={} others={}", v.kind, v.active, v.self_state, others.join(","))
}

/// Why the locus is in its state, for the log: `fullState.state`, our own `self.reason` and the other participants'
/// `reason`s (enum values such as MEDIA_INACTIVITY, FORCED, MOVED, ENDED; never names). Tells a server-side drop apart
/// from the other side leaving.
pub fn reasons(l: &Value) -> String {
    let clean = |v: &Value| v.as_str().unwrap_or("").chars().take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_').take(32).collect::<String>();
    let self_id = st(&l["self"], "id");
    let others: Vec<String> = l["participants"].as_array().into_iter().flatten()
        .filter(|p| st(p, "id") != self_id).map(|p| clean(&p["reason"])).filter(|r| !r.is_empty()).collect();
    format!("state={} self_reason={} reasons={}", clean(&l["fullState"]["state"]), clean(&l["self"]["reason"]), others.join(","))
}

/// Every reason and state field of our own participant (`self`, its devices, their intents and media connections),
/// as `path=VALUE`, for the call log when a call ends: which side ended it and why (e.g. `reason=MEDIA_INACTIVITY`,
/// `devices.0.state=LEFT`). Only enum-like values (letters, digits, `_`, `-`, `.`), never urls, ids or names.
pub fn self_reason_fields(l: &Value) -> String {
    fn walk(v: &Value, path: &str, depth: usize, out: &mut Vec<String>) {
        let enum_like = |s: &str| !s.is_empty() && s.len() <= 48 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
        match v {
            Value::Object(m) => for (k, x) in m {
                if out.len() >= 40 { return; }
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                let kl = k.to_ascii_lowercase();
                let wanted = kl.contains("reason") || kl.ends_with("state") || kl == "type" && path.contains("intent") || kl.contains("status");
                match x {
                    Value::String(s) if wanted && enum_like(s) => out.push(format!("{p}={s}")),
                    Value::Bool(b) if wanted => out.push(format!("{p}={b}")),
                    Value::Object(_) | Value::Array(_) if depth < 4 && !matches!(kl.as_str(), "person" | "identity" | "controls") => walk(x, &p, depth + 1, out),
                    _ => {}
                }
            },
            Value::Array(a) => for (i, x) in a.iter().enumerate().take(8) { walk(x, &format!("{path}.{i}"), depth + 1, out) },
            _ => {}
        }
    }
    let mut out = vec![];
    walk(&l["self"], "self", 0, &mut out);
    out.join(" ")
}

/// Why the call ended from Locus' side, for the toast (English), or None when the other side simply left / declined
/// (the call window says that itself). `why`: the end state's reason (`call_state`).
pub fn end_notice(l: &Value, why: &str) -> Option<String> {
    let self_state = st(&l["self"], "state");
    let reason = |v: &Value| v.as_str().unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(40).collect::<String>();
    let mut r = reason(&l["self"]["reason"]);
    if r.is_empty() { r = l["self"]["devices"].as_array().into_iter().flatten().map(|d| reason(&d["reason"])).find(|x| !x.is_empty()).unwrap_or_default(); }
    let others_gone = l["participants"].as_array().into_iter().flatten()
        .any(|p| st(p, "id") != st(&l["self"], "id") && matches!(st(p, "state").as_str(), "LEFT" | "DECLINED"));
    if self_state == "LEFT" {
        return Some(if r.is_empty() { "Webex removed this device from the call (no reason given).".into() } else { format!("Webex ended the call for this device (reason: {r}).") });
    }
    let fs = reason(&l["fullState"]["state"]);
    if matches!(fs.as_str(), "INACTIVE" | "TERMINATING") && !others_gone && why != "declined" && why != "missed" && why != "unanswered" {
        return Some(if r.is_empty() { format!("Webex ended the call (call state {fs}).") } else { format!("Webex ended the call (reason: {r}).") });
    }
    None
}

/// The other side looks gone only because every one of their devices shows LEFT, while they are still JOINED. That
/// happens in passing (the call moves to another of their devices, their client rejoins after a network blip, a delta
/// lists the old device first), so the call ends only when it is still so after `DEVICES_GONE_GRACE`.
pub fn peer_devices_gone(v: &LocusView) -> bool {
    !v.others.is_empty() && v.others.iter().all(|p| p.state == "JOINED" && p.left())
}
pub const DEVICES_GONE_GRACE: Duration = Duration::from_secs(8);

/// Error text without anything that looks like an address (logs and toasts never carry personal values).
fn scrub(e: &str) -> String {
    e.split_whitespace().map(|w| if w.contains('@') { "<addr>" } else { w }).collect::<Vec<_>>().join(" ").chars().take(180).collect()
}

struct Active {
    url: String, self_id: String, self_url: String, media_id: String, correlation_id: String,
    outgoing: bool, name: String, email: String, last: Option<CallState>, answered: bool,
    media: Option<Media>, roap_seq: u32, roap_done: bool,
    /// The seq of the last ANSWER applied (a repeated one gets OK again; one for an offer given up on does not).
    roap_applied: u32,
    /// Media trouble was already reported for this call (`media_trouble`).
    media_warned: bool,
    /// Event-driven state: the last locus applied, its sequence, when it was last read by GET, whether an event asked
    /// for a read, and when the media counters were last logged.
    view: Option<LocusView>, seq: u64, read_at: Option<Instant>, stale: bool, stats_at: Option<Instant>,
    /// The full locus the deltas are merged into (`Null` until one was seen).
    locus: Value,
    /// Since when the other side is joined but our media isn't up (`MEDIA_TIMEOUT`), and since when media is up without a
    /// single RTP packet from the server (`RTP_TIMEOUT`).
    joined_at: Option<Instant>, media_up_at: Option<Instant>,
    /// When our OFFER went out and no ANSWER has come yet (`ROAP_TIMEOUT`).
    offer_at: Option<Instant>,
    /// Screen share of this call (`share.rs`), and whether the other side's share was already announced.
    share: Option<share::Share>,
    /// The far end shares (its name), since when, and whether "no picture" was said already (`share_rx_tick`).
    remote_share: Option<String>, remote_since: Option<Instant>, rx_warned: bool,
    /// The mute state last sent to the UI.
    mute_info: Option<MuteInfo>,
    /// A group space's meeting (the space id): stays up while others come and go (session.rs `group_state`).
    space: Option<String>,
    /// The roster last sent to the UI.
    roster_sent: Option<controls::Roster>,
    /// Camera video of this call, both ways (`video.rs`).
    video: video::CallVideo,
    /// Since when the other side shows only LEFT devices while still JOINED (`peer_devices_gone`).
    gone_since: Option<Instant>,
    /// When a locus was last applied (an event or a successful read): the grace above ends only on a fresh one.
    fresh_at: Option<Instant>,
    /// Since when the media is not connected after having been up (`recovery_tick`; "Reconnecting…" after
    /// `RECONNECTING_AFTER`).
    media_down_at: Option<Instant>,
    /// The media connection was up once in this call: a later drop is the media reconnect's business (`recovery_tick`).
    was_up: bool,
    /// The official media reconnect of this call (`recovery_tick`).
    recovery: Recovery,
    /// "Reconnecting…" was sent to the call window (`Resp::CallReconnecting`).
    reconnecting: bool,
    /// The last ROAP ERROR the server sent for this call (its cause fields, for the end reason in the call log).
    roap_error: String,
    /// The Locus media keep-alive (`keepAliveUrl`, every `keepAliveSecs`; `keepalive_tick`) and when it last went out.
    keep_alive: Option<KeepAlive>, keep_alive_at: Option<Instant>,
}

impl Active {
    fn new(url: String, self_id: String, self_url: String, outgoing: bool, name: String, email: String) -> Active {
        Active { url, self_id, self_url, media_id: String::new(), correlation_id: String::new(), outgoing, name, email, last: None, answered: outgoing,
            media: None, roap_seq: 0, roap_done: false, roap_applied: 0, media_warned: false, view: None, seq: 0, read_at: None, stale: false, stats_at: None,
            locus: Value::Null, joined_at: None, media_up_at: None, offer_at: None, share: None,
            remote_share: None, remote_since: None, rx_warned: false, mute_info: None, space: None, roster_sent: None,
            video: Default::default(), gone_since: None, fresh_at: None, media_down_at: None, was_up: false, recovery: Recovery::default(), reconnecting: false, roap_error: String::new(), keep_alive: None, keep_alive_at: None }
    }
}

/// The other side is joined but ICE / DTLS isn't up after this long: the user is told (the call stays, like the
/// official client's no-media timers; the user decides to hang up).
pub const MEDIA_TIMEOUT: Duration = Duration::from_secs(15);
/// Media is up but not one RTP packet came from the server.
pub const RTP_TIMEOUT: Duration = Duration::from_secs(8);
/// The official media reconnect (`spark-windows-desktop-ui.dll` 46.11.0.36194, FUN_184ef4ff4 at RVA 0x4ef4ff4, the
/// handler of the media connection's `ConnectionState::Terminated` / `IceFailure`):
///
/// * on hold: nothing ("ConnectionState::Terminated: Call held. do not attempt to reconnect media");
/// * offline: a flag is set and nothing else ("Network is offline, wait for network restored, before triggering media
///   reconnect."); the network handler (FUN_184ebbfc2, on "network/restored" and "network/ip-address-changed") sees the
///   flag and reconnects ("Network restored, and wme session was terminated, reconnect media");
/// * else ("Network is available, triggering media reconnect"), while `reconnects < 5`
///   (`if (*(ulong*)(call+0x2818) < 5)`): the counter goes up, "The <n> time to reconnect media for ICE Failure during
///   meeting", "remove media connection [<id>]" and `MediaManager::reconnectMedia`: a NEW media connection (new peer
///   connection, ICE, DTLS) and its offer to Locus;
/// * at 5: the counter is cleared, "Media fail to reconnect after Terminated for 5 times, end meeting/call and report
///   media drop events", the call ends (end reason `MediaReconnectMaxRetry`).
///
/// Webox's media stack (webrtc-rs) has no WME session that terminates on its own, so its stand-in for `Terminated` is
/// the peer connection Failed, or not connected for `MEDIA_RECONNECT_AFTER` (WME's own timer is not in the decompile).
pub const MEDIA_RECONNECT_MAX: u32 = 5;
pub const MEDIA_RECONNECT_AFTER: Duration = Duration::from_secs(10);
/// A reconnect whose new connection is not up by then failed (the official client waits for the new connection's
/// next `Terminated`): the ROAP answer's `ROAP_TIMEOUT` plus the join's `MEDIA_TIMEOUT`.
pub const MEDIA_RECONNECT_TIMEOUT: Duration = Duration::from_secs(25);

/// The state of a call's media reconnect.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recovery {
    /// Reconnects started since the media was last up (the official counter, `call+0x2818`).
    pub attempts: u32,
    /// The running reconnect's start; None between them.
    pub attempt_at: Option<Instant>,
    /// The running reconnect failed already (its offer was not sent / refused, its answer not applied).
    pub attempt_failed: bool,
    /// Offline while the media was down: reconnect once the network is back (the official flag).
    pub offline_wait: bool,
}

/// What the network looks like this tick (`route_probe` against the address the media connection was made from).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Route { Same, Changed, Offline }

/// Why a reconnect starts (the call log).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Trigger { IceFailed, DownTooLong, NetworkChanged, NetworkRestored, AttemptFailed }

impl Trigger {
    pub fn text(self) -> &'static str {
        match self {
            Trigger::IceFailed => "ICE failed",
            Trigger::DownTooLong => "media down too long",
            Trigger::NetworkChanged => "local network address changed",
            Trigger::NetworkRestored => "network restored",
            Trigger::AttemptFailed => "the last attempt failed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Recover {
    Nothing,
    /// The media connection is up again: the counter is cleared.
    Recovered,
    /// Offline: wait for the network (the first tick of it sets `offline_wait`).
    WaitOffline,
    /// Tear the media connection down and build a new one.
    Rebuild(Trigger),
    /// `MEDIA_RECONNECT_MAX` reconnects failed: end the call.
    End,
}

/// One tick of the media reconnect. Pure, for the tests. `was_up`: the media was up once in this call (the join phase
/// has its own counter in the official client and stays with `media_stall` here); `connected` / `failed`: the current
/// connection's state; `down_since`: since when it is not connected.
pub fn recover_step(r: &Recovery, now: Instant, was_up: bool, connected: bool, failed: bool, down_since: Option<Instant>, route: Route, held: bool) -> Recover {
    if !was_up || held { return Recover::Nothing; }
    let rebuild = |t| if r.attempts >= MEDIA_RECONNECT_MAX { Recover::End } else { Recover::Rebuild(t) };
    if route == Route::Offline { return if connected { Recover::Nothing } else { Recover::WaitOffline }; }
    if route == Route::Changed { return rebuild(Trigger::NetworkChanged); }
    if connected { return if r.attempts > 0 || r.attempt_at.is_some() { Recover::Recovered } else { Recover::Nothing }; }
    if r.offline_wait { return rebuild(Trigger::NetworkRestored); }
    if let Some(t) = r.attempt_at {
        let over = r.attempt_failed || failed || now.saturating_duration_since(t) >= MEDIA_RECONNECT_TIMEOUT;
        return if over { rebuild(Trigger::AttemptFailed) } else { Recover::Nothing };
    }
    if failed { return rebuild(Trigger::IceFailed); }
    if down_since.is_some_and(|d| now.saturating_duration_since(d) >= MEDIA_RECONNECT_AFTER) { return rebuild(Trigger::DownTooLong); }
    Recover::Nothing
}

/// The call window says "Reconnecting…" once media that was up is down this long.
pub const RECONNECTING_AFTER: Duration = Duration::from_secs(5);
/// Our ROAP OFFER got no ANSWER.
pub const ROAP_TIMEOUT: Duration = Duration::from_secs(10);

/// The notice (English source, translated where shown) for a call whose media never came up, or `None` while there is
/// still time. Pure, for the tests.
pub fn media_stall(now: Instant, offer_at: Option<Instant>, joined_at: Option<Instant>, media_up_at: Option<Instant>, rx: u64, media: Option<MediaState>) -> Option<&'static str> {
    let over = |t: Option<Instant>, d: Duration| t.is_some_and(|t| now.saturating_duration_since(t) >= d);
    if over(offer_at, ROAP_TIMEOUT) { return Some("No answer from the media server to the connection request (ROAP). The call may have no sound."); }
    if media == Some(MediaState::Failed) { return Some("Audio connection failed (ICE/DTLS). The call has no sound."); }
    // only before the media was ever up: a later drop is the media reconnect's (`recovery_tick`), not "no connection"
    if media != Some(MediaState::Connected) && media_up_at.is_none() && over(joined_at, MEDIA_TIMEOUT) { return Some("No audio connection to the media server (ICE/DTLS doesn't connect). Check the network or firewall."); }
    if media == Some(MediaState::Connected) && rx == 0 && over(media_up_at, RTP_TIMEOUT) { return Some("Connected, but no sound arrives (no audio data from the media server)."); }
    None
}

#[derive(Default)]
pub struct Calls {
    pub(super) device_type: String, pub(super) jobs: Option<Sender<Job>>,
    cur: Option<Active>, last_dial: Option<Instant>, live: Arc<AtomicBool>, engine: Option<audio::Engine>, muted: bool,
    /// The camera the user wants on in the call (`CallReq::Video`); kept from before the call is placed / answered, and
    /// for the whole call. None = camera off.
    video_want: Option<crate::media::camera::PreviewKey>,
    /// A tick is waiting in the job queue (the ticker sends no second one).
    tick_queued: Arc<AtomicBool>,
    /// Where the system routes to Webex (`route_target`) and from which local address when the current media connection
    /// was made (`route_probe`): the network-change check of `recovery_tick`.
    route: (Option<std::net::SocketAddr>, Option<std::net::IpAddr>),
}

/// The ROAP v2 message envelope Locus expects in `localMedias[].localSdp` (JSON string), camera off.
pub fn roap_local_sdp(message_type: &str, sdp: Option<&str>, seq: u32, muted: bool) -> String { roap_local_sdp_av(message_type, sdp, seq, muted, true) }

/// `roap_local_sdp` with the camera's state (`videoMuted` false while our camera is sent).
pub fn roap_local_sdp_av(message_type: &str, sdp: Option<&str>, seq: u32, muted: bool, video_muted: bool) -> String {
    let mut m = json!({"messageType": message_type, "version": "2", "seq": seq});
    if let Some(s) = sdp { m["sdps"] = json!([s]); }
    if message_type == "OFFER" { m["tieBreaker"] = json!(4294967294u64); }
    // our answer to a server OFFER failed (webex-js-sdk sends the same): the server may offer again
    if message_type == "ERROR" { m["errorType"] = json!("FAILED"); }
    json!({"roapMessage": m, "audioMuted": muted, "videoMuted": video_muted}).to_string()
}

/// A ROAP message out of a mercury `locus.message.roap` event: (messageType, seq, first SDP).
pub fn roap_in(d: &Value) -> Option<(String, u32, Option<String>)> {
    if d["eventType"].as_str() != Some("locus.message.roap") { return None; }
    let m = &d["message"];
    Some((st(m, "messageType"), m["seq"].as_u64().unwrap_or(0) as u32, m["sdps"].as_array().and_then(|a| a.first()).and_then(Value::as_str).map(String::from)))
}

/// The media connection id of a join answer: `mediaConnections[]` at the top, else under the locus' `self.devices[]` (this
/// device first). Without it no ROAP OK can be sent.
pub fn media_id_of(r: &Value, device_url: &str) -> String {
    let first = |v: &Value| v["mediaConnections"].as_array().into_iter().flatten().map(|c| st(c, "mediaId")).find(|m| !m.is_empty());
    let devs = locus_of(r)["self"]["devices"].as_array().cloned().unwrap_or_default();
    first(r).or_else(|| devs.iter().filter(|d| st(d, "url") == device_url).chain(devs.iter()).find_map(first)).unwrap_or_default()
}

/// The Locus keep-alive of a joined device: `mediaConnections[{keepAliveUrl, keepAliveSecs}]` of the join answer (at the
/// top, else under this device in `self.devices[]`).
#[derive(Clone, Debug, PartialEq)]
pub struct KeepAlive { pub url: String, pub secs: u64 }

/// The keep-alive request: `GET keepAliveUrl`, no body (official client, `spark-windows-desktop-ui.dll` RVA 0x5757bcd: the
/// Locus request helper with method code 0 = GET, tag "keep_alive", "Cannot perform locus request. keepAliveUrl is
/// empty.", errors "Locus keepAlive error. httpStatus = ").
pub const KEEPALIVE_VERB: &str = "GET";

/// How often the keep-alive goes out: `(keepAliveSecs - 1) * 330 ms`, 6.27 s for the usual 20, like the official
/// `LocusManager::createKeepAliveTimer` (`(secs - 1) * 33 * 10` ms, a repeating timer; it refuses keepAliveSecs < 2).
pub fn keep_alive_every(secs: u64) -> Duration {
    Duration::from_millis(secs.saturating_sub(1).max(1) * 330)
}

pub fn keep_alive_of(r: &Value, device_url: &str) -> Option<KeepAlive> {
    let of = |v: &Value| v["mediaConnections"].as_array().into_iter().flatten().find_map(|c| {
        let url = st(c, "keepAliveUrl");
        let secs = c["keepAliveSecs"].as_u64().or_else(|| c["keepAliveSecs"].as_str().and_then(|s| s.parse().ok())).unwrap_or(0);
        (!url.is_empty() && secs > 0).then_some(KeepAlive { url, secs })
    });
    let devs = locus_of(r)["self"]["devices"].as_array().cloned().unwrap_or_default();
    of(r).or_else(|| devs.iter().filter(|d| st(d, "url") == device_url).chain(devs.iter()).find_map(of))
}

/// The cause fields of a ROAP ERROR (short enum-like values only) for the debug log.
pub fn roap_error(m: &Value) -> String {
    let f = |k: &str| scrub(&st(m, k)).chars().filter(|c| c.is_ascii_graphic() || *c == ' ').take(80).collect::<String>();
    format!("seq={} errorType={} errorCause={}", m["seq"], f("errorType"), f("errorCause"))
}

/// The answer a join / media response may already carry in `mediaConnections[].remoteSdp` (a JSON string holding a roapMessage).
pub fn remote_roap(r: &Value) -> Option<(String, u32, Option<String>)> {
    let rs = r["mediaConnections"].as_array()?.iter().find_map(|c| c["remoteSdp"].as_str())?;
    let v: Value = serde_json::from_str(rs).ok()?;
    let m = if v["roapMessage"].is_object() { &v["roapMessage"] } else { &v };
    if st(m, "messageType").is_empty() { return None; } // a remoteSdp without a ROAP message (the answer comes by mercury)
    Some((st(m, "messageType"), m["seq"].as_u64().unwrap_or(0) as u32, m["sdps"].as_array().and_then(|a| a.first()).and_then(Value::as_str).map(String::from)))
}

#[cfg(test)]
mod tests;

pub mod controls;
mod session;
pub mod share;
pub mod video;
