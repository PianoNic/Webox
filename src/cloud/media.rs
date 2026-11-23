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
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_OPUS};
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
}

fn opus_cap() -> RTCRtpCodecCapability {
    RTCRtpCodecCapability { mime_type: MIME_TYPE_OPUS.to_owned(), clock_rate: 48000, channels: 2, sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(), rtcp_feedback: vec![] }
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
fn settings(net: Net) -> Result<SettingEngine, String> {
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
        s.set_udp_network(super::ice_socket::mux(ip).map_err(|e| format!("ICE socket: {e}"))?);
    }
    Ok(s)
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
        let pc = {
            let (state, io, rxp, stop) = (state.clone(), io.clone(), rx_packets.clone(), stop.clone());
            rt.block_on(async move {
                let mut m = MediaEngine::default();
                m.register_codec(RTCRtpCodecParameters { capability: opus_cap(), payload_type: 111, ..Default::default() }, RTPCodecType::Audio).map_err(|e| e.to_string())?;
                let registry = register_default_interceptors(Registry::new(), &mut m).map_err(|e| e.to_string())?;
                let api = APIBuilder::new().with_media_engine(m).with_interceptor_registry(registry).with_setting_engine(settings(net)?).build();
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
                pc.on_track(Box::new(move |track, _, _| {
                    let (io, rxp, stop) = (io.clone(), rxp.clone(), stop.clone());
                    Box::pin(async move {
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
                    })
                }));
                Ok::<_, String>(pc)
            })?
        };
        let me = Media { rt, pc, state, muted, rx_packets, tx_frames, stop, v6 };
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
        a=ice-lite\r\na=msid-semantic: WMS *\r\na=group:BUNDLE 0\r\na=fingerprint:SHA-256 FP\r\n\
        m=audio 5004 UDP/TLS/RTP/SAVPF 111 101 13\r\nb=RS:0\r\nc=IN IP4 media.example.com\r\nb=TIAS:64000\r\n\
        a=content:main\r\na=sendrecv\r\na=rtpmap:111 opus/48000/2\r\na=fmtp:111 maxplaybackrate=48000;maxaveragebitrate=64000;stereo=1;sprop-stereo=0;useinbandfec=1\r\n\
        a=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-15\r\na=rtcp-fb:* nack\r\na=rtcp-fb:* ccm cisco-scr\r\na=rtcp-fb:111 transport-cc\r\n\
        a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level\r\na=extmap:2 http://protocols.example.com/virtualid\r\na=extmap:255 http://protocols.example.com/timestamp#100us\r\n\
        a=rtcp-xr:rcvr-rtt=all voip-metrics\r\na=rtcp-mux\r\na=ice-ufrag:UFRAG\r\na=ice-pwd:PWD\r\na=ice-options:cisco+tls+fqdn\r\na=setup:SETUP\r\na=label:0\r\n\
        a=sprop-source:0 count=1;policies=rs:1\r\na=sprop-simul:0 0 *\r\na=x-ssrc-range:100-200\r\na=cisco-mari:v1\r\na=cisco-mari-rate\r\na=ssrc:4242 cname:linus\r\n\
        a=candidate:1 1 UDP 2130706431 media.example.com PORT typ host\r\na=candidate:2 1 UDP 2130706175 media.example.com 9000 typ host\r\n\
        a=candidate:3 1 TCP 1962934271 media.example.com 5004 typ host tcptype passive\r\na=candidate:4 1 xTLS 1962933759 media.example.com 443 typ host tcptype passive\r\n\
        a=end-of-candidates\r\n";

    /// The complete answer path (TCP / TLS filter, host names via a stub resolver, webrtc-rs rewrite, webrtc-rs
    /// set_remote_description) on a Webex-shaped answer: it is taken and audio flows both ways. 127.0.0.1 only.
    #[test]
    fn webex_shaped_answer_connects_both_ways() {
        let (io1, io2) = (Arc::new(Io::default()), Arc::new(Io::default()));
        let a = Media::new(io1.clone(), None, Net::Loopback).unwrap();
        let b = Media::new(io2.clone(), None, Net::Loopback).unwrap();
        let real = b.answer(a.offer().unwrap()).unwrap();
        let get = |k: &str| real.lines().find_map(|l| l.trim().strip_prefix(k)).unwrap_or_else(|| panic!("{k} in {real}")).trim().to_string();
        let port = real.lines().find(|l| l.starts_with("a=candidate:")).and_then(|l| l.split_whitespace().nth(5)).unwrap().to_string();
        let answer = WEBEX_ANSWER.replace("UFRAG", &get("a=ice-ufrag:")).replace("PWD", &get("a=ice-pwd:")).replace("FP", &get("a=fingerprint:sha-256 "))
            .replace("SETUP", &get("a=setup:").to_uppercase()).replace("PORT", &port);
        assert!(webrtc::sdp::SessionDescription::unmarshal(&mut std::io::Cursor::new(answer.as_bytes())).is_err(), "raw shape is refused by webrtc-rs");
        let stub = |n: &str| if n == "media.example.com" { vec![IpAddr::from([127, 0, 0, 1])] } else { vec![] };
        a.apply_answer_with(answer, &stub).unwrap();
        let tone = audio::tone(440.0, 4000, 0.5);
        let (t0, mut fed) = (std::time::Instant::now(), 0);
        let rx = |m: &Media| m.rx_packets.load(Ordering::Relaxed);
        while t0.elapsed() < Duration::from_secs(10) {
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
