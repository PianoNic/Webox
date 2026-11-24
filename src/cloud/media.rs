//! The call's media leg: one WebRTC peer connection (webrtc-rs: ICE, DTLS-SRTP, RTP — pure Rust) carrying Opus audio.
//! Locus negotiates it with ROAP v2: our OFFER (SDP with all ICE candidates already gathered, no trickle) goes inside the
//! join request, the server's ANSWER comes back as a mercury `locus.message.roap` event, then we send OK (see `locus.rs`).
//! Audio comes from / goes to `audio::Io` (48 kHz mono, 20 ms Opus frames).
//!
//! Sockets: one UDP socket of our own (`ice_socket.rs`) on the address the system uses to reach Webex (`Net::Route`),
//! no mDNS socket; only when that address is unknown does webrtc-ice bind its own on every interface. Media is
//! outbound-initiated: the Webex media server is ICE lite and answers our checks from the address we sent them to.
//! Only UDP candidates are used (webrtc-rs has no ICE-TCP / TLS); the answer's TCP / TLS ones are dropped (`sdp.rs`).
use super::{dbg_conn, sdp, sdp_compat};
use crate::media::audio::{self, Io, FRAME};
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264, MIME_TYPE_OPUS};
use webrtc::api::setting_engine::SettingEngine;
use webrtc::api::APIBuilder;
use webrtc::ice::mdns::MulticastDnsMode;
use webrtc::ice::network_type::NetworkType;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::media::Sample;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType};
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::rtp_transceiver::RTCRtpTransceiverInit;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_local::TrackLocal;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaState { New, Connecting, Connected, Failed }

/// TURN server handed out by Locus' media service (ROAP TURN discovery); `None` until that is wired.
#[derive(Clone, Debug, Default)]
pub struct Turn { pub url: String, pub username: String, pub password: String }

pub struct Media {
    rt: tokio::runtime::Runtime,
    pc: Arc<RTCPeerConnection>,
    state: Arc<AtomicU8>,
    /// Mic muted: silence is sent instead of the microphone.
    pub muted: Arc<AtomicBool>,
    /// RTP packets received / audio frames sent (diagnostics, and "is audio really flowing").
    pub rx_packets: Arc<std::sync::atomic::AtomicU64>,
    pub tx_frames: Arc<std::sync::atomic::AtomicU64>,
    stop: Arc<AtomicBool>,
    /// ICE runs on IPv6 (host names in the answer then resolve to IPv6 too).
    v6: bool,
    /// The SSRC the server's audio comes from (learned from the first packet; 0 = none yet). A renegotiated answer with
    /// more than one m-line must declare it, or webrtc-rs stops that receiver (`share_answer_sdp`).
    pub rx_ssrc: Arc<AtomicU32>,
    /// Video RTP packets received (the far end's share).
    pub rx_video: Arc<AtomicU64>,
    /// The SSRC H.264 (the slides line's payload type) was last seen arriving from on the ICE socket, whether webrtc-rs
    /// took it or not (0 = none): with several m-lines webrtc-rs drops RTP from undeclared SSRCs.
    pub seen_slides_ssrc: Arc<AtomicU32>,
    /// The screen share's sending side, once `add_share` ran.
    share: std::sync::Mutex<Option<ShareTx>>,
}

/// The content ("slides") video track and the far end's key frame requests (RTCP PLI / FIR).
#[derive(Clone)]
struct ShareTx { track: Arc<TrackLocalStaticSample>, keyframe: Arc<AtomicBool> }

/// Feeds a running screen share into the call's content track (its own thread); dropping or `stop` ends it.
pub struct ShareFeed { stop: Arc<AtomicBool>, thread: Option<std::thread::JoinHandle<()>>, pub sent: Arc<AtomicU64> }
impl ShareFeed {
    pub fn stop(mut self) { self.halt() }
    fn halt(&mut self) { self.stop.store(true, Ordering::SeqCst); if let Some(t) = self.thread.take() { let _ = t.join(); } }
}
impl Drop for ShareFeed { fn drop(&mut self) { self.halt() } }

fn opus_cap() -> RTCRtpCodecCapability {
    RTCRtpCodecCapability { mime_type: MIME_TYPE_OPUS.to_owned(), clock_rate: 48000, channels: 2, sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(), rtcp_feedback: vec![] }
}

/// The H.264 payload type we offer (the share's lines; an answer keeps the offer's numbers).
pub const SLIDES_PT: u8 = 102;

/// What `media::share` sends: H.264 Constrained Baseline, packetization-mode 1 (the content stream of a share).
fn h264_cap() -> RTCRtpCodecCapability {
    use webrtc::rtp_transceiver::RTCPFeedback;
    let fb = |t: &str, p: &str| RTCPFeedback { typ: t.to_owned(), parameter: p.to_owned() };
    RTCRtpCodecCapability { mime_type: MIME_TYPE_H264.to_owned(), clock_rate: 90000, channels: 0,
        sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".to_owned(),
        rtcp_feedback: vec![fb("nack", ""), fb("nack", "pli"), fb("ccm", "fir")] }
}

/// The m-line labels of the official client's layout (one ROAP session, `audio/main`, `video/main`, `audio/slides`,
/// `video/slides`): the first m-line of each kind is `a=content:main`, a later one `a=content:slides`.
/// The label goes in front of the section's first `a=` line (SDP orders a media section `m c b a`).
pub fn label_content(sdp: &str) -> String {
    let (mut audio, mut video, mut out, mut pending) = (0, 0, String::new(), None::<&str>);
    for l in sdp.lines().map(str::trim_end).filter(|l| !l.is_empty() && !l.starts_with("a=content:")) {
        if l.starts_with("m=") { if let Some(p) = pending.take() { out.push_str(p); } }
        if l.starts_with("a=") { if let Some(p) = pending.take() { out.push_str(p); } }
        out.push_str(l);
        out.push_str("\r\n");
        let n = if l.starts_with("m=audio") { audio += 1; audio } else if l.starts_with("m=video") { video += 1; video } else { 0 };
        if n > 0 { pending = Some(if n == 1 { "a=content:main\r\n" } else { "a=content:slides\r\n" }); }
    }
    if let Some(p) = pending { out.push_str(p); }
    out
}

/// The server's answer to our share re-OFFER, made fit for webrtc-rs: every m-line gets our mid when it has none, the
/// server's own audio `a=ssrc` lines go (it sends audio from others), and the first audio line declares the SSRC the
/// call's audio really comes from: webrtc-rs stops every receiver whose SSRC a renegotiated answer doesn't name. With
/// more than one m-line webrtc-rs only takes RTP from declared SSRCs, so the video lines keep what the server declares,
/// and `slides_ssrc` (the SSRC the far end's share was seen arriving from, see `ice_socket`) replaces that when known.
pub fn share_answer_sdp(sdp: &str, mids: &[String], audio_ssrc: u32, slides_ssrc: Option<u32>) -> String {
    let mut sections: Vec<Vec<String>> = vec![vec![]];
    for l in sdp.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if l.starts_with("m=") { sections.push(vec![]); }
        if let Some(s) = sections.last_mut() { s.push(l.to_string()); }
    }
    let mut out: Vec<String> = sections.remove(0);
    let mut videos = 0;
    for (i, mut m) in sections.into_iter().enumerate() {
        let (audio, video) = (m[0].starts_with("m=audio"), m[0].starts_with("m=video"));
        if video { videos += 1; }
        let slides = video && videos == 2;
        if audio || (slides && slides_ssrc.is_some()) { m.retain(|l| !l.starts_with("a=ssrc:") && !l.starts_with("a=ssrc-group:")); }
        if !m.iter().any(|l| l.starts_with("a=mid:")) { if let Some(id) = mids.get(i).filter(|id| !id.is_empty()) { m.push(format!("a=mid:{id}")); } }
        if i == 0 && audio && audio_ssrc != 0 { m.push(format!("a=ssrc:{audio_ssrc} cname:webex")); }
        if let Some(s) = slides_ssrc.filter(|_| slides) { m.push(format!("a=ssrc:{s} cname:webex-slides")); }
        out.extend(m);
    }
    let mut s = out.join("\r\n");
    s.push_str("\r\n");
    s
}

/// The index of the `n`-th (1-based) m-line of `kind` ("audio" / "video").
pub fn section_of_kind(sdp: &str, kind: &str, n: usize) -> Option<usize> {
    sdp.lines().filter(|l| l.starts_with("m=")).enumerate().filter(|(_, l)| l[2..].starts_with(kind)).nth(n.checked_sub(1)?).map(|(i, _)| i)
}

/// The SDP with the direction of its `idx`-th m-line set to `dir` (sendrecv / sendonly / recvonly / inactive).
pub fn with_direction(sdp: &str, idx: usize, dir: &str) -> String {
    let (mut out, mut i, mut done) = (String::new(), None::<usize>, false);
    let flush_dir = |out: &mut String, done: &mut bool, here: bool| if here && !*done { out.push_str(&format!("a={dir}\r\n")); *done = true; };
    for l in sdp.lines().map(str::trim_end).filter(|l| !l.is_empty()) {
        if l.starts_with("m=") { flush_dir(&mut out, &mut done, i == Some(idx)); i = Some(i.map_or(0, |x| x + 1)); }
        if i == Some(idx) && matches!(l, "a=sendrecv" | "a=sendonly" | "a=recvonly" | "a=inactive") { flush_dir(&mut out, &mut done, true); continue; }
        out.push_str(l);
        out.push_str("\r\n");
    }
    flush_dir(&mut out, &mut done, i == Some(idx));
    out
}

/// The mids of a description's m-lines, in order ("" for a line without one).
pub fn mids_of(sdp: &str) -> Vec<String> {
    let mut v: Vec<String> = vec![];
    for l in sdp.lines().map(str::trim) {
        if l.starts_with("m=") { v.push(String::new()); }
        if let (Some(m), Some(last)) = (l.strip_prefix("a=mid:"), v.last_mut()) { *last = m.to_string(); }
    }
    v
}

/// Where ICE may bind its UDP sockets.
pub enum Net {
    /// The local address the system routes to Webex through (`route_ip`); every interface when unknown.
    Route(Option<IpAddr>),
    /// 127.0.0.1 only (tests: no socket on a real interface, so no firewall prompt).
    #[cfg(test)]
    Loopback,
}

/// The local address of an outbound TCP connection to `host:443`: the interface the system uses to reach Webex.
pub fn route_ip(host: &str) -> Option<IpAddr> {
    use std::net::ToSocketAddrs;
    let to = (host, 443).to_socket_addrs().ok()?.next()?;
    let s = std::net::TcpStream::connect_timeout(&to, Duration::from_secs(3)).ok()?;
    let ip = s.local_addr().ok()?.ip();
    let _ = s.shutdown(std::net::Shutdown::Both);
    Some(ip)
}

/// Needs the tokio runtime (the ICE socket is registered with it).
fn settings(net: Net, seen_slides: Arc<AtomicU32>) -> Result<SettingEngine, String> {
    let mut s = SettingEngine::default();
    s.set_ice_multicast_dns_mode(MulticastDnsMode::Disabled); // the default binds 0.0.0.0:5353; Webex never sends mDNS names
    let keep: Option<IpAddr> = match net {
        Net::Route(ip) => ip,
        #[cfg(test)]
        Net::Loopback => { s.set_include_loopback_candidate(true); Some(IpAddr::from([127, 0, 0, 1])) }
    };
    if let Some(ip) = keep {
        s.set_network_types(vec![if ip.is_ipv4() { NetworkType::Udp4 } else { NetworkType::Udp6 }]);
        s.set_ip_filter(Box::new(move |a| a == ip));
        // our own socket on that address only (reads survive ICMP errors, see ice_socket.rs)
        s.set_udp_network(super::ice_socket::mux(ip, seen_slides, SLIDES_PT).map_err(|e| format!("ICE socket: {e}"))?);
    }
    Ok(s)
}

/// The far end's screen share on the `video/slides` line: RTP -> H.264 access units -> the decoder thread (`share_rx`).
/// Key frames are asked for with RTCP PLI at the start, after a loss or a bad picture, and again while nothing decodes.
async fn receive_share(track: Arc<webrtc::track::track_remote::TrackRemote>, stop: Arc<AtomicBool>, rx_video: Arc<AtomicU64>,
    pc: Arc<std::sync::Mutex<Option<std::sync::Weak<RTCPeerConnection>>>>) {
    use webrtc::media::io::sample_builder::SampleBuilder;
    use webrtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
    use webrtc::rtp::codecs::h264::H264Packet;
    let ssrc = track.ssrc();
    dbg_conn("call: share: video from the far end arrives");
    let dec = super::share_rx::Decode::start();
    let mut sb = SampleBuilder::new(128, H264Packet::default(), 90_000);
    let (mut last_pli, mut last_seq): (Option<Instant>, Option<u16>) = (None, None);
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if super::share_rx::pli_due(dec.keyframe.load(Ordering::SeqCst), dec.decoded.load(Ordering::Relaxed) > 0, last_pli, now) {
            let pc = pc.lock().ok().and_then(|p| p.as_ref().and_then(|w| w.upgrade()));
            if let Some(pc) = pc {
                let _ = pc.write_rtcp(&[Box::new(PictureLossIndication { sender_ssrc: 0, media_ssrc: ssrc })]).await;
            }
            dec.keyframe.store(false, Ordering::SeqCst);
            last_pli = Some(now);
        }
        let pkt = match tokio::time::timeout(Duration::from_millis(250), track.read_rtp()).await {
            Err(_) => continue, // nothing for a while: the PLI above may be due again
            Ok(Err(_)) => break,
            Ok(Ok((pkt, _))) => pkt,
        };
        rx_video.fetch_add(1, Ordering::Relaxed);
        let seq = pkt.header.sequence_number;
        if last_seq.is_some_and(|p| seq != p.wrapping_add(1) && seq.wrapping_sub(p) < 0x8000) { dec.keyframe.store(true, Ordering::SeqCst); }
        last_seq = Some(last_seq.map_or(seq, |p| if seq.wrapping_sub(p) < 0x8000 { seq } else { p }));
        sb.push(pkt);
        while let Some(s) = sb.pop() {
            if s.prev_dropped_packets > 0 { dec.keyframe.store(true, Ordering::SeqCst); }
            dec.push(s.data.to_vec());
        }
    }
    dbg_conn("call: share: video from the far end ended");
}

/// `call: ice …` log lines with the time since the peer was made.
fn log_state(t0: Instant, what: &str, s: impl std::fmt::Display) { dbg_conn(&format!("call: {what} {s} after {} ms", t0.elapsed().as_millis())); }

impl Media {
    pub fn new(io: Arc<Io>, turn: Option<Turn>, net: Net) -> Result<Media, String> {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().thread_name("webox-media").build().map_err(|e| e.to_string())?;
        let state = Arc::new(AtomicU8::new(0));
        let muted = Arc::new(AtomicBool::new(false));
        let rx_packets = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let tx_frames = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let v6 = matches!(net, Net::Route(Some(IpAddr::V6(_))));
        let rx_ssrc = Arc::new(AtomicU32::new(0));
        let rx_video = Arc::new(AtomicU64::new(0));
        let seen_slides_ssrc = Arc::new(AtomicU32::new(0));
        let seen = seen_slides_ssrc.clone();
        let pc = {
            let (state, io, rxp, stop, rx_ssrc, rx_video) = (state.clone(), io.clone(), rx_packets.clone(), stop.clone(), rx_ssrc.clone(), rx_video.clone());
            rt.block_on(async move {
                let mut m = MediaEngine::default();
                m.register_codec(RTCRtpCodecParameters { capability: opus_cap(), payload_type: 111, ..Default::default() }, RTPCodecType::Audio).map_err(|e| e.to_string())?;
                // offered only once a share adds its video lines (an audio call's offer has no m=video)
                m.register_codec(RTCRtpCodecParameters { capability: h264_cap(), payload_type: SLIDES_PT, ..Default::default() }, RTPCodecType::Video).map_err(|e| e.to_string())?;
                let registry = register_default_interceptors(Registry::new(), &mut m).map_err(|e| e.to_string())?;
                let api = APIBuilder::new().with_media_engine(m).with_interceptor_registry(registry).with_setting_engine(settings(net, seen)?).build();
                let ice_servers = turn.filter(|t| !t.url.is_empty()).map(|t| vec![RTCIceServer { urls: vec![t.url], username: t.username, credential: t.password, ..Default::default() }]).unwrap_or_default();
                let pc = Arc::new(api.new_peer_connection(RTCConfiguration { ice_servers, ..Default::default() }).await.map_err(|e| e.to_string())?);
                let (s2, t0) = (state.clone(), Instant::now());
                // Disconnected is a gap in the checks that may recover; ICE reports Failed when it doesn't
                pc.on_peer_connection_state_change(Box::new(move |s| {
                    log_state(t0, "peer", s);
                    s2.store(match s { RTCPeerConnectionState::Connected => 2, RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed => 3,
                        RTCPeerConnectionState::New => 0, _ => 1 }, Ordering::SeqCst);
                    Box::pin(async {})
                }));
                pc.on_ice_connection_state_change(Box::new(move |s| { log_state(t0, "ice", s); Box::pin(async {}) }));
                pc.sctp().transport().on_state_change(Box::new(move |s| { log_state(t0, "dtls", s); Box::pin(async {}) }));
                // Incoming audio: RTP -> Opus decode -> playback buffer.
                // webrtc-rs awaits each on_track handler while holding the handler's lock, so a handler that reads its
                // track until the call ends blocks every later track (the share's, or audio from a new server SSRC after a
                // renegotiation): the read loops run in tasks of their own and the handler returns at once.
                let pc_slot: Arc<std::sync::Mutex<Option<std::sync::Weak<RTCPeerConnection>>>> = Arc::new(std::sync::Mutex::new(Some(Arc::downgrade(&pc))));
                pc.on_track(Box::new(move |track, _, _| {
                    let (io, rxp, stop, rx_ssrc, rx_video, slot) = (io.clone(), rxp.clone(), stop.clone(), rx_ssrc.clone(), rx_video.clone(), pc_slot.clone());
                    tokio::spawn(async move {
                        if track.kind() == RTPCodecType::Video {
                            receive_share(track, stop, rx_video, slot).await;
                            return;
                        }
                        dbg_conn(&format!("call: audio track from the far end (ssrc {})", if rx_ssrc.load(Ordering::Relaxed) == 0 { "first" } else { "changed" }));
                        rx_ssrc.store(track.ssrc(), Ordering::Relaxed);
                        let Ok(mut dec) = ropus::Decoder::new(audio::RATE, ropus::Channels::Mono) else { return };
                        let mut pcm = vec![0i16; 5760];
                        while !stop.load(Ordering::Relaxed) {
                            let Ok((pkt, _)) = track.read_rtp().await else { break };
                            if rxp.fetch_add(1, Ordering::Relaxed) == 0 { dbg_conn("call: first RTP packet received"); }
                            if pkt.payload.is_empty() { continue; }
                            if let Ok(n) = dec.decode(&pkt.payload, &mut pcm, ropus::DecodeMode::Normal) {
                                io.play(&pcm[..n]);
                                // Keep latency low: if more than 400 ms queued, drop down to 100 ms.
                                if let Ok(mut q) = io.to_play.lock() { if q.len() > 19_200 { let drop = q.len() - 4_800; q.drain(..drop); } }
                            }
                        }
                    });
                    Box::pin(async {})
                }));
                Ok::<_, String>(pc)
            })?
        };
        let me = Media { rt, pc, state, muted, rx_packets, tx_frames, stop, v6, rx_ssrc, rx_video, seen_slides_ssrc, share: std::sync::Mutex::new(None) };
        me.start_sender(io)?;
        Ok(me)
    }

    /// Adds the microphone track and the 20 ms Opus send loop (runs from the start; packets flow once DTLS is up).
    fn start_sender(&self, io: Arc<Io>) -> Result<(), String> {
        let (pc, muted, tx, stop) = (self.pc.clone(), self.muted.clone(), self.tx_frames.clone(), self.stop.clone());
        self.rt.block_on(async move {
            let track = Arc::new(TrackLocalStaticSample::new(opus_cap(), "audio".to_owned(), "webox".to_owned()));
            let sender = pc.add_track(Arc::clone(&track) as Arc<dyn TrackLocal + Send + Sync>).await.map_err(|e| e.to_string())?;
            tokio::spawn(async move { let mut b = vec![0u8; 1500]; while sender.read(&mut b).await.is_ok() {} }); // RTCP
            let mut enc = ropus::Encoder::builder(audio::RATE, ropus::Channels::Mono, ropus::Application::Voip).build().map_err(|e| format!("{e:?}"))?;
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_millis(20));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut out = [0u8; 1275];
                while !stop.load(Ordering::Relaxed) {
                    tick.tick().await;
                    let frame = if muted.load(Ordering::Relaxed) { let _ = io.take_frame(); [0i16; FRAME] } else { io.take_frame().unwrap_or([0i16; FRAME]) };
                    let Ok(n) = enc.encode(&frame, &mut out) else { continue };
                    if track.write_sample(&Sample { data: bytes::Bytes::copy_from_slice(&out[..n]), duration: Duration::from_millis(20), ..Default::default() }).await.is_ok() {
                        tx.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
            Ok(())
        })
    }

    /// Creates the local offer and waits until ICE gathering is complete (ROAP carries all candidates in the SDP).
    pub fn offer(&self) -> Result<String, String> {
        let pc = self.pc.clone();
        self.rt.block_on(async move {
            let offer = pc.create_offer(None).await.map_err(|e| e.to_string())?;
            let mut done = pc.gathering_complete_promise().await;
            pc.set_local_description(offer).await.map_err(|e| e.to_string())?;
            tokio::time::timeout(Duration::from_secs(5), done.recv()).await.map_err(|_| "ICE gathering timed out".to_string())?;
            Ok(pc.local_description().await.ok_or("no local description")?.sdp)
        })
    }

    pub fn apply_answer(&self, sdp: String) -> Result<(), String> { self.apply_answer_with(sdp, &system_lookup) }

    /// `apply_answer` with the resolver for host names in the answer (a stub in tests).
    fn apply_answer_with(&self, sdp: String, lookup: &dyn Fn(&str) -> Vec<IpAddr>) -> Result<(), String> {
        let pc = self.pc.clone();
        let mid = self.rt.block_on(pc.local_description()).map_or(String::new(), |d| sdp_compat::first_mid(&d.sdp));
        let sdp = usable(&sdp, self.v6, lookup, &mid);
        self.state.store(1, Ordering::SeqCst);
        let desc = RTCSessionDescription::answer(sdp.clone()).map_err(|e| e.to_string())?;
        let r = self.rt.block_on(async move { pc.set_remote_description(desc).await.map_err(|e| e.to_string()) });
        r.map_err(|e| with_line(e, &sdp))
    }

    /// For a remote OFFER (the server renegotiating, and the loopback test): returns our ANSWER SDP.
    pub fn answer(&self, offer_sdp: String) -> Result<String, String> {
        let pc = self.pc.clone();
        let offer_sdp = usable(&offer_sdp, self.v6, &system_lookup, "");
        self.rt.block_on(async move {
            pc.set_remote_description(RTCSessionDescription::offer(offer_sdp.clone()).map_err(|e| e.to_string())?).await.map_err(|e| with_line(e.to_string(), &offer_sdp))?;
            let ans = pc.create_answer(None).await.map_err(|e| e.to_string())?;
            let mut done = pc.gathering_complete_promise().await;
            pc.set_local_description(ans).await.map_err(|e| e.to_string())?;
            tokio::time::timeout(Duration::from_secs(5), done.recv()).await.map_err(|_| "ICE gathering timed out".to_string())?;
            Ok(pc.local_description().await.ok_or("no local description")?.sdp)
        })
    }

    /// The share's media lines exist already (a second share in the same call needs no new negotiation).
    pub fn has_share(&self) -> bool { self.share.lock().map(|s| s.is_some()).unwrap_or(false) }

    /// Adds the share's media lines in the official client's layout, `video/main` (inactive), `audio/slides`
    /// (inactive), `video/slides` (sendonly, H.264), and returns the re-OFFER (same ICE / DTLS session, labelled with
    /// `a=content`). webrtc-rs keeps the receiver it made for the server's audio as a transceiver of its own; it becomes the
    /// (inactive) `audio/slides` line instead of a fifth one.
    pub fn add_share(&self) -> Result<String, String> {
        if self.has_share() { return Err("share lines exist already".into()); }
        let pc = self.pc.clone();
        let tx = self.rt.block_on(async move {
            let init = |d| Some(RTCRtpTransceiverInit { direction: d, send_encodings: vec![] });
            // the transceiver webrtc-rs made for audio from an undeclared SSRC (no mid yet, no sender track)
            let mut spare_audio = None;
            for t in pc.get_transceivers().await {
                if t.mid().is_none() && t.kind() == RTPCodecType::Audio { spare_audio = Some(t); }
            }
            // webrtc-rs makes track-less transceivers only as recvonly; they are set inactive right after
            let video_main = pc.add_transceiver_from_kind(RTPCodecType::Video, init(RTCRtpTransceiverDirection::Recvonly)).await.map_err(|e| e.to_string())?;
            video_main.set_direction(RTCRtpTransceiverDirection::Inactive).await;
            // `audio/slides` stays recvonly for webrtc-rs: an inactive line pauses its receiver, and the spare transceiver
            // carries the call's audio. The server is offered (and the answer read as) inactive / sendonly instead.
            match spare_audio {
                Some(t) => t.set_direction(RTCRtpTransceiverDirection::Recvonly).await,
                None => { pc.add_transceiver_from_kind(RTPCodecType::Audio, init(RTCRtpTransceiverDirection::Recvonly)).await.map_err(|e| e.to_string())?; }
            }
            let track = Arc::new(TrackLocalStaticSample::new(h264_cap(), "slides".to_owned(), "webox-share".to_owned()));
            // sendrecv like the official client ("suggestedShareDirection: sendrecv"): the same line carries the far end's share
            let t = pc.add_transceiver_from_track(Arc::clone(&track) as Arc<dyn TrackLocal + Send + Sync>, init(RTCRtpTransceiverDirection::Sendrecv)).await.map_err(|e| e.to_string())?;
            // RTCP from the far end: a picture loss / full intra request asks for a key frame
            let keyframe = Arc::new(AtomicBool::new(false));
            let (k, sender) = (keyframe.clone(), t.sender().await);
            tokio::spawn(async move {
                use webrtc::rtcp::payload_feedbacks::{full_intra_request::FullIntraRequest, picture_loss_indication::PictureLossIndication};
                while let Ok((pkts, _)) = sender.read_rtcp().await {
                    if pkts.iter().any(|p| p.as_any().is::<PictureLossIndication>() || p.as_any().is::<FullIntraRequest>()) { k.store(true, Ordering::SeqCst); }
                }
            });
            Ok::<_, String>(ShareTx { track, keyframe })
        })?;
        if let Ok(mut s) = self.share.lock() { *s = Some(tx); }
        self.reoffer()
    }

    /// A new OFFER of the lines we have (the share's included), for one more ROAP round on the same session.
    pub fn reoffer(&self) -> Result<String, String> {
        let pc = self.pc.clone();
        let sdp = self.rt.block_on(async move {
            let offer = pc.create_offer(None).await.map_err(|e| e.to_string())?;
            let mut done = pc.gathering_complete_promise().await;
            pc.set_local_description(offer).await.map_err(|e| e.to_string())?;
            let _ = tokio::time::timeout(Duration::from_secs(5), done.recv()).await; // gathered for the first offer already
            Ok::<_, String>(pc.local_description().await.ok_or("no local description")?.sdp)
        })?;
        let sdp = label_content(&sdp);
        // the official layout offers `audio/slides` inactive (no computer audio is shared)
        Ok(match section_of_kind(&sdp, "audio", 2) { Some(i) => with_direction(&sdp, i, "inactive"), None => sdp })
    }

    /// The server's ANSWER to the share re-OFFER (`share_answer_sdp` on top of the usual clean-up).
    pub fn apply_share_answer(&self, sdp: String) -> Result<(), String> { self.apply_share_answer_with(sdp, &system_lookup) }

    fn apply_share_answer_with(&self, sdp: String, lookup: &dyn Fn(&str) -> Vec<IpAddr>) -> Result<(), String> {
        let pc = self.pc.clone();
        let local = self.rt.block_on(pc.local_description()).map(|d| d.sdp).unwrap_or_default();
        let mids = mids_of(&local);
        // the far end's share seen on the wire but not taken by webrtc-rs (it isn't the declared SSRC): declare that one
        let seen = self.seen_slides_ssrc.load(Ordering::Relaxed);
        let slides = (seen != 0 && self.rx_video.load(Ordering::Relaxed) == 0).then_some(seen);
        if slides.is_some() { dbg_conn("call: share: declaring the SSRC the far end's share arrives from"); }
        let sdp = share_answer_sdp(&usable(&sdp, self.v6, lookup, mids.first().map_or("", |s| s.as_str())), &mids, self.rx_ssrc.load(Ordering::Relaxed), slides);
        // our `audio/slides` is recvonly in webrtc-rs (see add_share): its answer must read sendonly, whatever the
        // server said, or webrtc-rs pauses the receiver the call's audio arrives on
        let sdp = match section_of_kind(&local, "audio", 2) { Some(i) => with_direction(&sdp, i, "sendonly"), None => sdp };
        let desc = RTCSessionDescription::answer(sdp.clone()).map_err(|e| e.to_string())?;
        let r = self.rt.block_on(async move { pc.set_remote_description(desc).await.map_err(|e| e.to_string()) });
        r.map_err(|e| with_line(e, &sdp))
    }

    /// Sends the share's pictures on the content line until the feed is stopped or the stream ends. The far end's
    /// key frame requests reach the encoder.
    pub fn feed_share(&self, stream: Arc<std::sync::Mutex<crate::media::share::ShareStream>>) -> Result<ShareFeed, String> {
        let tx = self.share.lock().ok().and_then(|s| s.clone()).ok_or("Die Freigabe-Medienleitung fehlt.")?;
        let (stop, sent) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU64::new(0)));
        let (s2, n, rt) = (stop.clone(), sent.clone(), self.rt.handle().clone());
        let thread = std::thread::Builder::new().name("webox-share-feed".into()).spawn(move || {
            let mut last_pts: Option<u64> = None;
            while !s2.load(Ordering::SeqCst) {
                // the lock is held only to take what is queued (the call thread reads the stream's status meanwhile)
                let aus = match stream.lock() {
                    Ok(s) => { if tx.keyframe.swap(false, Ordering::SeqCst) { s.request_keyframe(); } s.drain() }
                    Err(_) => break,
                };
                if aus.is_empty() { std::thread::sleep(Duration::from_millis(5)); continue; }
                for au in aus {
                    let dur = last_pts.map_or(Duration::from_millis(33), |p| Duration::from_micros(au.pts_us.saturating_sub(p).clamp(1_000, 1_000_000)));
                    last_pts = Some(au.pts_us);
                    let sample = Sample { data: bytes::Bytes::from(au.data), duration: dur, ..Default::default() };
                    if rt.block_on(tx.track.write_sample(&sample)).is_ok() && n.fetch_add(1, Ordering::Relaxed) == 0 { dbg_conn("call: share: first picture sent"); }
                }
            }
        }).map_err(|e| e.to_string())?;
        Ok(ShareFeed { stop, thread: Some(thread), sent })
    }

    pub fn state(&self) -> MediaState {
        match self.state.load(Ordering::SeqCst) { 0 => MediaState::New, 2 => MediaState::Connected, 3 => MediaState::Failed, _ => MediaState::Connecting }
    }

    pub fn close(self) {
        self.stop.store(true, Ordering::SeqCst);
        let pc = self.pc.clone();
        let _ = self.rt.block_on(async move { tokio::time::timeout(Duration::from_secs(3), pc.close()).await });
        self.rt.shutdown_background();
    }
}

/// A remote description with only the candidates webrtc-rs can use, host names resolved (logged without addresses).
fn usable(sdp: &str, v6: bool, lookup: &dyn Fn(&str) -> Vec<IpAddr>, mid: &str) -> String {
    dbg_conn(&format!("call: remote sdp {}", sdp::summary(sdp)));
    let (s, dropped) = sdp::udp_only(sdp);
    if !dropped.is_empty() { dbg_conn(&format!("call: dropped remote candidates {dropped:?} (no ICE-TCP / TLS in webrtc-rs)")); }
    let (s, r) = sdp::resolve_names(&s, v6, lookup);
    if r.names > 0 { dbg_conn(&format!("call: resolved {} fqdn candidates into {} ({} unresolved, dropped)", r.names, r.candidates, r.unresolved)); }
    let (s, fixes) = sdp_compat::webrtc_compat(&s, mid);
    if !fixes.is_empty() { dbg_conn(&format!("call: remote sdp adapted for webrtc-rs: {}", fixes.join(", "))); }
    s
}

/// The system resolver, at most 3 s per name (runs on the call's thread, never the UI's).
fn system_lookup(name: &str) -> Vec<IpAddr> {
    use std::net::ToSocketAddrs;
    let (tx, rx) = std::sync::mpsc::channel();
    let n = name.to_string();
    std::thread::spawn(move || { let _ = tx.send((n.as_str(), 0).to_socket_addrs().map(|a| a.map(|s| s.ip()).collect::<Vec<_>>()).unwrap_or_default()); });
    rx.recv_timeout(Duration::from_secs(3)).unwrap_or_default()
}

/// A failed remote description with the first candidate line webrtc-rs can't read.
fn with_line(e: String, sdp: &str) -> String { match sdp::first_bad_candidate(sdp) { Some(l) => format!("{e} [{l}]"), None => e } }

#[cfg(test)]
mod tests {
    use super::*;

    /// Two peers in this process negotiate with each other exactly the way ROAP does (offer with all candidates ->
    /// answer -> connect), then audio goes mic buffer -> Opus -> RTP/SRTP -> Opus -> playback buffer. Sockets on
    /// 127.0.0.1 only (no firewall prompt), no audio device, nobody is called. The answer also lists TCP and TLS
    /// candidates like a Webex media server's, which must not spoil it.
    #[test]
    fn two_local_peers_exchange_audio() {
        let (io1, io2) = (Arc::new(Io::default()), Arc::new(Io::default()));
        let a = Media::new(io1.clone(), None, Net::Loopback).unwrap();
        let b = Media::new(io2.clone(), None, Net::Loopback).unwrap();
        let offer = a.offer().unwrap();
        assert!(offer.contains("opus/48000/2") && offer.contains("a=candidate"), "offer carries the codec and candidates");
        let cands: Vec<&str> = offer.lines().filter(|l| l.starts_with("a=candidate:")).collect();
        assert!(cands.iter().all(|l| l.contains(" 127.0.0.1 ")), "loopback only: {cands:?}");
        let answer = b.answer(offer).unwrap();
        let extra = "a=candidate:9 1 TCP 1962934271 127.0.0.1 5004 typ host tcptype passive\r\na=candidate:10 1 xTLS 1962933759 127.0.0.1 443 typ host tcptype passive\r\n";
        let answer = answer.replacen("a=candidate:", &format!("{extra}a=candidate:"), 1);
        assert!(answer.contains("xTLS"));
        a.apply_answer(answer).unwrap();
        let tone = audio::tone(440.0, 3000, 0.5);
        let t0 = std::time::Instant::now();
        let mut fed = 0;
        while t0.elapsed() < Duration::from_secs(8) {
            // Act as the microphone of peer A: keep ~100 ms queued.
            if io1.captured.lock().unwrap().len() < 4800 && fed + FRAME <= tone.len() { io1.captured.lock().unwrap().extend(tone[fed..fed + FRAME].iter()); fed += FRAME; }
            if a.state() == MediaState::Connected && b.state() == MediaState::Connected && b.rx_packets.load(Ordering::Relaxed) > 40 { break; }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!((a.state(), b.state()), (MediaState::Connected, MediaState::Connected), "ICE + DTLS connected");
        assert!(b.rx_packets.load(Ordering::Relaxed) > 40, "B received RTP: {}", b.rx_packets.load(Ordering::Relaxed));
        let heard: Vec<i16> = io2.to_play.lock().unwrap().iter().copied().collect();
        assert!(audio::rms(&heard) > 0.05, "B plays audible audio, rms {}", audio::rms(&heard));
        a.close();
        b.close();
    }

    /// The shape of a Webex media server's ROAP answer (example.com / documentation values), with peer B's real ICE
    /// credentials, fingerprint, DTLS role and port filled in so it can really connect.
    const WEBEX_ANSWER: &str = "v=0\r\no=linus 18446744073709551616 1 IN IP4 media.example.com\r\ns=-\r\nt=0 0\r\nc=IN IP4 media.example.com\r\nb=TIAS:64000\r\n\
        a=ice-lite\r\na=msid-semantic: WMS *\r\na=group:BUNDLE 0\r\na=fingerprint:SHA-256 {FP}\r\n\
        m=audio 5004 UDP/TLS/RTP/SAVPF 111 101 13\r\nb=RS:0\r\nc=IN IP4 media.example.com\r\nb=TIAS:64000\r\n\
        a=content:main\r\na=sendrecv\r\na=rtpmap:111 opus/48000/2\r\na=fmtp:111 maxplaybackrate=48000;maxaveragebitrate=64000;stereo=1;sprop-stereo=0;useinbandfec=1\r\n\
        a=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-15\r\na=rtcp-fb:* nack\r\na=rtcp-fb:* ccm cisco-scr\r\na=rtcp-fb:111 transport-cc\r\n\
        a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level\r\na=extmap:2 http://protocols.example.com/virtualid\r\na=extmap:255 http://protocols.example.com/timestamp#100us\r\n\
        a=rtcp-xr:rcvr-rtt=all voip-metrics\r\na=rtcp-mux\r\na=ice-ufrag:{UFRAG}\r\na=ice-pwd:{PWD}\r\na=ice-options:cisco+tls+fqdn\r\na=setup:{SETUP}\r\na=label:0\r\n\
        a=sprop-source:0 count=1;policies=rs:1\r\na=sprop-simul:0 0 *\r\na=x-ssrc-range:100-200\r\na=cisco-mari:v1\r\na=cisco-mari-rate\r\na=ssrc:4242 cname:linus\r\n\
        a=candidate:1 1 UDP 2130706431 media.example.com {PORT} typ host\r\na=candidate:2 1 UDP 2130706175 media.example.com 9000 typ host\r\n\
        a=candidate:3 1 TCP 1962934271 media.example.com 5004 typ host tcptype passive\r\na=candidate:4 1 xTLS 1962933759 media.example.com 443 typ host tcptype passive\r\n\
        a=end-of-candidates\r\n";

    /// `WEBEX_ANSWER` with peer B's real values (from B's own answer `real`). The placeholders are `{NAME}`: none of the
    /// values (letters, hex with colons, digits) can contain a brace, so one filled-in value is never rewritten by the next
    /// replacement. The bare-word placeholders used before (UFRAG, PWD, FP, SETUP, PORT) were: a random ICE ufrag / pwd that
    /// contains "FP" or "PWD" (about 1.7 % of runs) got the fingerprint pasted into it, the credentials no longer matched and
    /// ICE failed on both sides (CI: `(Failed, Failed)`, before the 30 s deadline stuck in `Connecting`).
    fn webex_answer(real: &str) -> String {
        let get = |k: &str| real.lines().find_map(|l| l.trim().strip_prefix(k)).unwrap_or_else(|| panic!("{k} in {real}")).trim().to_string();
        let port = real.lines().find(|l| l.starts_with("a=candidate:")).and_then(|l| l.split_whitespace().nth(5)).unwrap().to_string();
        fill(WEBEX_ANSWER, &[("UFRAG", get("a=ice-ufrag:")), ("PWD", get("a=ice-pwd:")), ("FP", get("a=fingerprint:sha-256 ")),
            ("SETUP", get("a=setup:").to_uppercase()), ("PORT", port)])
    }

    fn fill(template: &str, values: &[(&str, String)]) -> String {
        values.iter().fold(template.to_string(), |s, (k, v)| s.replace(&format!("{{{k}}}"), v))
    }

    #[test]
    fn filled_answer_keeps_credentials_that_look_like_placeholders() {
        // the values that broke the old construction: a ufrag holding "FP", a pwd holding "PWD" and "PORT"
        let real = "a=ice-ufrag:abFPcdPWDefSETUPx\r\na=ice-pwd:xxPWDyyPORTzzFPqqSETUPwwwwwwwwwww\r\na=fingerprint:sha-256 AB:CD:EF\r\na=setup:active\r\na=candidate:1 1 udp 1 127.0.0.1 50000 typ host\r\n";
        let a = webex_answer(real);
        assert!(a.contains("a=ice-ufrag:abFPcdPWDefSETUPx\r\n"), "{a}");
        assert!(a.contains("a=ice-pwd:xxPWDyyPORTzzFPqqSETUPwwwwwwwwwww\r\n"), "{a}");
        assert!(a.contains("a=fingerprint:SHA-256 AB:CD:EF\r\n") && a.contains("a=setup:ACTIVE\r\n"), "{a}");
        assert!(a.contains("media.example.com 50000 typ host"), "{a}");
        assert!(!a.contains('{'), "every placeholder filled: {a}");
    }

    /// A share in a running call, as with a Webex media server: A's audio answer came without a usable SSRC (webrtc-rs
    /// then made a receiver of its own), A adds the share lines and re-OFFERs on the same session, B answers. Audio keeps
    /// flowing to A after the renegotiation, and B receives the H.264 pictures A sends on the slides line. 127.0.0.1 only.
    #[test]
    fn share_renegotiation_keeps_audio_and_sends_pictures() {
        let (io1, io2) = (Arc::new(Io::default()), Arc::new(Io::default()));
        let a = Media::new(io1.clone(), None, Net::Loopback).unwrap();
        let b = Media::new(io2.clone(), None, Net::Loopback).unwrap();
        let answer = webex_answer(&b.answer(a.offer().unwrap()).unwrap());
        let stub = |n: &str| if n == "media.example.com" { vec![IpAddr::from([127, 0, 0, 1])] } else { vec![] };
        a.apply_answer_with(answer, &stub).unwrap();
        let tone = audio::tone(440.0, 20000, 0.5);
        let mut fed = 0;
        let feed_tone = |fed: &mut usize| for io in [&io1, &io2] { if io.captured.lock().unwrap().len() < 4800 && *fed + FRAME <= tone.len() { io.captured.lock().unwrap().extend(tone[*fed..*fed + FRAME].iter()); } };
        let rx = |m: &Media| m.rx_packets.load(Ordering::Relaxed);
        let t0 = std::time::Instant::now();
        while t0.elapsed() < Duration::from_secs(30) && rx(&a) <= 40 { feed_tone(&mut fed); fed += FRAME; std::thread::sleep(Duration::from_millis(10)); }
        assert!(rx(&a) > 40, "audio before the share: {}", rx(&a));
        assert_ne!(a.rx_ssrc.load(Ordering::Relaxed), 0, "the server's audio SSRC is known");

        let offer = a.add_share().unwrap();
        let lines: Vec<(&str, &str)> = offer.split("m=").skip(1).map(|m| (m.split_whitespace().next().unwrap(), if m.contains("a=content:slides") { "slides" } else { "main" })).collect();
        assert_eq!(lines.iter().filter(|l| l.0 == "audio").count(), 2, "{offer}");
        assert_eq!(lines.iter().filter(|l| l.0 == "video").count(), 2, "{offer}");
        assert_eq!(lines.iter().filter(|l| l.1 == "slides").count(), 2, "{lines:?}");
        assert!(offer.contains("H264/90000") && offer.split("m=").last().is_some_and(|m| m.contains("a=sendrecv")), "slides line sendrecv: {offer}");
        let b_answer = b.answer(offer).unwrap();
        a.apply_share_answer_with(b_answer, &stub).unwrap();
        assert!(a.has_share());

        // pictures (an access unit of made-up NAL units: the packetizer only splits on start codes)
        let tx = a.share.lock().unwrap().clone().unwrap();
        let before = rx(&a);
        let t1 = std::time::Instant::now();
        let mut n = 0u32;
        while t1.elapsed() < Duration::from_secs(30) && (b.rx_video.load(Ordering::Relaxed) < 20 || rx(&a) < before + 40) {
            feed_tone(&mut fed); fed += FRAME;
            if fed + FRAME > tone.len() { fed = 0; }
            let mut au = vec![0, 0, 0, 1, 0x67, 0x42, 0xe0, 0x1f, 0, 0, 0, 1, 0x68, 0xce, 0x3c, 0x80, 0, 0, 0, 1, 0x65];
            au.extend(vec![(n % 200) as u8 + 1; 3000]);
            let sample = Sample { data: bytes::Bytes::from(au), duration: Duration::from_millis(33), ..Default::default() };
            a.rt.block_on(tx.track.write_sample(&sample)).unwrap();
            n += 1;
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(b.rx_video.load(Ordering::Relaxed) >= 20, "B received the share: {} video packets", b.rx_video.load(Ordering::Relaxed));
        assert!(rx(&a) >= before + 40, "audio still reaches A after the renegotiation: {} -> {}", before, rx(&a));
        a.close();
        b.close();
    }

    /// Receiving a share: A shares real H.264 (OpenH264's encoder, a moving picture) on the slides line, B depacketizes it,
    /// asks for a key frame (RTCP PLI, which reaches A's encoder side) and decodes pictures of the share's size for the
    /// call window. 127.0.0.1 only.
    #[test]
    fn received_share_is_decoded() {
        let (io1, io2) = (Arc::new(Io::default()), Arc::new(Io::default()));
        let a = Media::new(io1.clone(), None, Net::Loopback).unwrap();
        let b = Media::new(io2.clone(), None, Net::Loopback).unwrap();
        b.apply_answer(a.answer(b.offer().unwrap()).unwrap()).unwrap();
        let t0 = std::time::Instant::now();
        while t0.elapsed() < Duration::from_secs(30) && (a.state() != MediaState::Connected || b.state() != MediaState::Connected) { std::thread::sleep(Duration::from_millis(20)); }
        assert_eq!((a.state(), b.state()), (MediaState::Connected, MediaState::Connected));
        // A adds the share lines (sendrecv) and offers them; B answers
        let offer = a.add_share().unwrap();
        assert!(offer.split("m=").nth(4).is_some_and(|m| m.contains("a=sendrecv") && m.contains("a=content:slides")), "slides line sendrecv: {offer}");
        a.apply_share_answer(b.answer(offer).unwrap()).unwrap();
        let tx = a.share.lock().unwrap().clone().unwrap();
        let (w, h) = (320usize, 240usize);
        let mut enc = openh264::encoder::Encoder::new().unwrap();
        let mut yuv = openh264::formats::YUVBuffer::new(w, h);
        let mut rgb = vec![0u8; w * h * 3];
        crate::cloud::share_rx::clear();
        let (t1, mut n) = (std::time::Instant::now(), 0usize);
        let mut asked = false;
        while t1.elapsed() < Duration::from_secs(30) {
            if tx.keyframe.swap(false, Ordering::SeqCst) { asked = true; enc.force_intra_frame(); }
            for (i, px) in rgb.chunks_mut(3).enumerate() { let (x, y) = (i % w, i / w); px.copy_from_slice(&[((x + n * 4) % 256) as u8, (y % 256) as u8, 128]); }
            yuv.read_rgb(openh264::formats::RgbSliceU8::new(&rgb, (w, h)));
            let au = enc.encode(&yuv).unwrap().to_vec();
            if !au.is_empty() { a.rt.block_on(tx.track.write_sample(&Sample { data: bytes::Bytes::from(au), duration: Duration::from_millis(66), ..Default::default() })).unwrap(); }
            n += 1;
            if crate::cloud::share_rx::latest().is_some_and(|p| p.seq > 0) && asked && n > 10 { break; }
            std::thread::sleep(Duration::from_millis(66));
        }
        let p = crate::cloud::share_rx::latest().expect("B decoded a picture of the share");
        assert_eq!(p.image.size, [w, h], "decoded at the share's size");
        assert!(asked, "B asked for a key frame (PLI) and A's sender saw it");
        assert!(b.rx_video.load(Ordering::Relaxed) > 0);
        assert_ne!(b.seen_slides_ssrc.load(Ordering::Relaxed), 0, "the ICE socket noted the share's SSRC");
        crate::cloud::share_rx::clear();
        assert!(crate::cloud::share_rx::latest().is_none(), "released: no picture left");
        a.close();
        b.close();
    }

    #[test]
    fn share_sdp_helpers() {
        let offer = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:1\r\nm=video 9 UDP/TLS/RTP/SAVPF 102\r\na=mid:2\r\nm=video 9 UDP/TLS/RTP/SAVPF 102\r\na=mid:3\r\n";
        let l = label_content(offer);
        assert_eq!(l.matches("a=content:main").count(), 2);
        assert_eq!(l.matches("a=content:slides").count(), 2);
        assert_eq!(label_content(&l), l, "labelling twice changes nothing");
        // after c= / b=, before the first a= line (webrtc-rs and strict parsers want `m c b a`)
        let l = label_content("v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 102\r\nc=IN IP4 0.0.0.0\r\nb=AS:500\r\na=mid:3\r\n");
        assert!(l.contains("b=AS:500\r\na=content:main\r\na=mid:3"), "{l}");
        assert!(webrtc::sdp::SessionDescription::unmarshal(&mut std::io::Cursor::new(format!("{l}").replace("v=0\r\n", "v=0\r\no=- 1 1 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\n").as_bytes())).is_ok(), "{l}");
        assert_eq!(mids_of(offer), vec!["0", "1", "2", "3"]);
        // a server answer without mids and with its own SSRCs
        let ans = "v=0\r\nm=audio 5004 UDP/TLS/RTP/SAVPF 111\r\na=ssrc:4242 cname:linus\r\na=sendrecv\r\nm=audio 0 UDP/TLS/RTP/SAVPF 111\r\na=inactive\r\nm=video 0 UDP/TLS/RTP/SAVPF 102\r\na=inactive\r\nm=video 5004 UDP/TLS/RTP/SAVPF 102\r\na=recvonly\r\na=ssrc:7 cname:linus\r\n";
        let s = share_answer_sdp(ans, &mids_of(offer), 99, None);
        assert!(!s.contains("4242"), "the server's audio SSRC goes: {s}");
        assert!(s.contains("a=ssrc:7 cname:linus"), "the video lines keep what the server declares: {s}");
        assert!(s.contains("a=ssrc:99 cname:webex"), "{s}");
        // the share's SSRC seen arriving replaces the declared one on the slides line
        let s2 = share_answer_sdp(ans, &mids_of(offer), 99, Some(1234));
        assert!(!s2.contains("ssrc:7 ") && s2.split("m=").nth(4).unwrap().contains("a=ssrc:1234 cname:webex-slides"), "{s2}");
        assert_eq!(mids_of(&s), vec!["0", "1", "2", "3"]);
        // directions per m-line
        assert_eq!(section_of_kind(offer, "audio", 2), Some(1));
        assert_eq!(section_of_kind(offer, "video", 2), Some(3));
        assert_eq!(section_of_kind(offer, "video", 3), None);
        let d = with_direction(&s, 1, "sendonly");
        assert!(d.contains("a=mid:1") && d.split("m=").nth(2).unwrap().contains("a=sendonly") && !d.split("m=").nth(2).unwrap().contains("a=inactive"), "{d}");
        assert!(d.split("m=").nth(1).unwrap().contains("a=sendrecv"), "other lines untouched: {d}");
        assert_eq!(with_direction("v=0\r\nm=audio 9 X 111\r\na=mid:0\r\n", 0, "inactive"), "v=0\r\nm=audio 9 X 111\r\na=mid:0\r\na=inactive\r\n");
    }

    /// The complete answer path (TCP / TLS filter, host names via a stub resolver, webrtc-rs rewrite, webrtc-rs
    /// set_remote_description) on a Webex-shaped answer: it is taken and audio flows both ways. 127.0.0.1 only.
    #[test]
    fn webex_shaped_answer_connects_both_ways() {
        let (io1, io2) = (Arc::new(Io::default()), Arc::new(Io::default()));
        let a = Media::new(io1.clone(), None, Net::Loopback).unwrap();
        let b = Media::new(io2.clone(), None, Net::Loopback).unwrap();
        let answer = webex_answer(&b.answer(a.offer().unwrap()).unwrap());
        assert!(webrtc::sdp::SessionDescription::unmarshal(&mut std::io::Cursor::new(answer.as_bytes())).is_err(), "raw shape is refused by webrtc-rs");
        let stub = |n: &str| if n == "media.example.com" { vec![IpAddr::from([127, 0, 0, 1])] } else { vec![] };
        a.apply_answer_with(answer, &stub).unwrap();
        let tone = audio::tone(440.0, 4000, 0.5);
        let (t0, mut fed) = (std::time::Instant::now(), 0);
        let rx = |m: &Media| m.rx_packets.load(Ordering::Relaxed);
        // Leaves as soon as audio flows both ways; the long deadline only matters on an overloaded CI runner (ICE + DTLS on
        // loopback once needed more than 10 s there).
        while t0.elapsed() < Duration::from_secs(30) {
            for io in [&io1, &io2] { if io.captured.lock().unwrap().len() < 4800 && fed + FRAME <= tone.len() { io.captured.lock().unwrap().extend(tone[fed..fed + FRAME].iter()); } }
            fed += FRAME;
            if rx(&a) > 40 && rx(&b) > 40 { break; }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!((a.state(), b.state()), (MediaState::Connected, MediaState::Connected), "ICE + DTLS connected");
        assert!(rx(&a) > 40 && rx(&b) > 40, "RTP both ways: A {} B {}", rx(&a), rx(&b));
        let heard: Vec<i16> = io1.to_play.lock().unwrap().iter().copied().collect();
        assert!(audio::rms(&heard) > 0.05, "A hears B (server SSRC ignored), rms {}", audio::rms(&heard));
        a.close();
        b.close();
    }
}
