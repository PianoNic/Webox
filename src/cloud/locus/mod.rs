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
//!   `reachability` data (media cluster choice), camera video. Interop with Locus' media servers is only tested against a
//!   second local webrtc-rs peer.
//! * **Screen share**, sending: `share.rs` (capture, ROAP re-OFFER with the `slides` lines, floor request on the locus'
//!   `content` media share). Receiving: the `content` floor GRANTED to the other side (`share_rx::remote_sharer`) makes
//!   sure the `slides` lines exist; their H.264 is decoded in `cloud/share_rx.rs` and shown as the call window's stage.
//! * **Locus state** follows the official client's `SequenceManager`: full loci newer than ours replace it, deltas
//!   (`baseSequence`) are merged onto the sequence they name, a delta we can't place (missed / out-of-order event) makes
//!   us re-read the whole locus (`apply_event`). The call ends as soon as the other side declined or left
//!   (`shouldDisconnectOne2OneCall`), and an incoming call answered on another of my devices stops ringing here.
//!
//! "Verbunden" is shown only when the callee's participant state is JOINED **and** our peer connection is `Connected`;
//! a call whose media doesn't come up (no ROAP answer, no ICE / DTLS, no RTP) is reported to the user (`media_stall`).
//!
//! Safety: `WEBOX_CALL_TEST_ONLY=<address>` makes every outgoing call to any other address fail before any network request,
//! only e-mail style addresses can be dialled (no phone numbers), one call at a time, at most one dial every 10 s.
use super::media::{route_ip, Media, MediaState, Net};
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
                    match ps.iter_mut().find(|q| !st(p, "id").is_empty() && st(q, "id") == st(p, "id")) { Some(q) => *q = p.clone(), None => ps.push(p.clone()) }
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
    /// Start sharing the picked screen / window (`Some`), or stop (`None`); see `share.rs`.
    Share(Option<share::ShareReq>),
    /// "Treffen" / "Beitreten" in a group space (its id): join the space's meeting, or start it (session.rs `join_space`).
    JoinSpace(String, audio::Choice) }

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
    pub mute: SelfMute }

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
    /// The ROAP seq of our OFFER that (re)negotiates the share's lines while its ANSWER is awaited, and since when.
    lines_seq: Option<u32>, lines_at: Option<Instant>,
    /// The far end shares (its name), since when, and whether the lines were negotiated once more to declare the SSRC
    /// its share really arrives from (`share_rx_tick`).
    remote_share: Option<String>, remote_since: Option<Instant>, rx_reoffered: bool, rx_warned: bool,
    /// The mute state last sent to the UI.
    mute_info: Option<MuteInfo>,
    /// A group space's meeting (the space id): stays up while others come and go (session.rs `group_state`).
    space: Option<String>,
}

impl Active {
    fn new(url: String, self_id: String, self_url: String, outgoing: bool, name: String, email: String) -> Active {
        Active { url, self_id, self_url, media_id: String::new(), correlation_id: String::new(), outgoing, name, email, last: None, answered: outgoing,
            media: None, roap_seq: 0, roap_done: false, media_warned: false, view: None, seq: 0, read_at: None, stale: false, stats_at: None,
            locus: Value::Null, joined_at: None, media_up_at: None, offer_at: None, share: None,
            lines_seq: None, lines_at: None, remote_share: None, remote_since: None, rx_reoffered: false, rx_warned: false, mute_info: None, space: None }
    }
}

/// The other side is joined but ICE / DTLS isn't up after this long: the user is told (the call stays, like the
/// official client's no-media timers; the user decides to hang up).
pub const MEDIA_TIMEOUT: Duration = Duration::from_secs(15);
/// Media is up but not one RTP packet came from the server.
pub const RTP_TIMEOUT: Duration = Duration::from_secs(8);
/// Our ROAP OFFER got no ANSWER.
pub const ROAP_TIMEOUT: Duration = Duration::from_secs(10);

/// The notice for a call whose media never came up, or `None` while there is still time. Pure, for the tests.
pub fn media_stall(now: Instant, offer_at: Option<Instant>, joined_at: Option<Instant>, media_up_at: Option<Instant>, rx: u64, media: Option<MediaState>) -> Option<&'static str> {
    let over = |t: Option<Instant>, d: Duration| t.is_some_and(|t| now.saturating_duration_since(t) >= d);
    if over(offer_at, ROAP_TIMEOUT) { return Some("Keine Antwort des Medienservers auf die Verbindungsanfrage (ROAP). Der Anruf hat möglicherweise keinen Ton."); }
    if media == Some(MediaState::Failed) { return Some("Audioverbindung fehlgeschlagen (ICE/DTLS). Der Anruf hat keinen Ton."); }
    if media != Some(MediaState::Connected) && over(joined_at, MEDIA_TIMEOUT) { return Some("Keine Audioverbindung zum Medienserver (ICE/DTLS kommt nicht zustande). Netzwerk oder Firewall prüfen."); }
    if media == Some(MediaState::Connected) && rx == 0 && over(media_up_at, RTP_TIMEOUT) { return Some("Verbunden, aber es kommt kein Ton an (keine Audiodaten vom Medienserver)."); }
    None
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

/// The media connection id of a join answer: `mediaConnections[]` at the top, else under the locus' `self.devices[]` (this
/// device first). Without it no ROAP OK can be sent.
pub fn media_id_of(r: &Value, device_url: &str) -> String {
    let first = |v: &Value| v["mediaConnections"].as_array().into_iter().flatten().map(|c| st(c, "mediaId")).find(|m| !m.is_empty());
    let devs = locus_of(r)["self"]["devices"].as_array().cloned().unwrap_or_default();
    first(r).or_else(|| devs.iter().filter(|d| st(d, "url") == device_url).chain(devs.iter()).find_map(first)).unwrap_or_default()
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

mod session;
pub mod share;
