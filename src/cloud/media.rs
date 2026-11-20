//! The call's media leg: one WebRTC peer connection (webrtc-rs: ICE, DTLS-SRTP, RTP — pure Rust) carrying Opus audio.
//! Locus negotiates it with ROAP v2: our OFFER (SDP with all ICE candidates already gathered, no trickle) goes inside the
//! join request, the server's ANSWER comes back as a mercury `locus.message.roap` event, then we send OK (see `locus.rs`).
//! Audio comes from / goes to `audio::Io` (48 kHz mono, 20 ms Opus frames).
use crate::media::audio::{self, Io, FRAME};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_OPUS};
use webrtc::api::APIBuilder;
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
}

fn opus_cap() -> RTCRtpCodecCapability {
    RTCRtpCodecCapability { mime_type: MIME_TYPE_OPUS.to_owned(), clock_rate: 48000, channels: 2, sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(), rtcp_feedback: vec![] }
}

impl Media {
    pub fn new(io: Arc<Io>, turn: Option<Turn>) -> Result<Media, String> {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().thread_name("webox-media").build().map_err(|e| e.to_string())?;
        let state = Arc::new(AtomicU8::new(0));
        let muted = Arc::new(AtomicBool::new(false));
        let rx_packets = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let tx_frames = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let pc = {
            let (state, io, rxp, stop) = (state.clone(), io.clone(), rx_packets.clone(), stop.clone());
            rt.block_on(async move {
                let mut m = MediaEngine::default();
                m.register_codec(RTCRtpCodecParameters { capability: opus_cap(), payload_type: 111, ..Default::default() }, RTPCodecType::Audio).map_err(|e| e.to_string())?;
                let registry = register_default_interceptors(Registry::new(), &mut m).map_err(|e| e.to_string())?;
                let api = APIBuilder::new().with_media_engine(m).with_interceptor_registry(registry).build();
                let ice_servers = turn.filter(|t| !t.url.is_empty()).map(|t| vec![RTCIceServer { urls: vec![t.url], username: t.username, credential: t.password, ..Default::default() }]).unwrap_or_default();
                let pc = Arc::new(api.new_peer_connection(RTCConfiguration { ice_servers, ..Default::default() }).await.map_err(|e| e.to_string())?);
                let s2 = state.clone();
                pc.on_peer_connection_state_change(Box::new(move |s| {
                    s2.store(match s { RTCPeerConnectionState::Connected => 2, RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed | RTCPeerConnectionState::Disconnected => 3,
                        RTCPeerConnectionState::New => 0, _ => 1 }, Ordering::SeqCst);
                    Box::pin(async {})
                }));
                // Incoming audio: RTP -> Opus decode -> playback buffer.
                pc.on_track(Box::new(move |track, _, _| {
                    let (io, rxp, stop) = (io.clone(), rxp.clone(), stop.clone());
                    Box::pin(async move {
                        let Ok(mut dec) = ropus::Decoder::new(audio::RATE, ropus::Channels::Mono) else { return };
                        let mut pcm = vec![0i16; 5760];
                        while !stop.load(Ordering::Relaxed) {
                            let Ok((pkt, _)) = track.read_rtp().await else { break };
                            rxp.fetch_add(1, Ordering::Relaxed);
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
        let me = Media { rt, pc, state, muted, rx_packets, tx_frames, stop };
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

    pub fn apply_answer(&self, sdp: String) -> Result<(), String> {
        let pc = self.pc.clone();
        self.state.store(1, Ordering::SeqCst);
        self.rt.block_on(async move { pc.set_remote_description(RTCSessionDescription::answer(sdp).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string()) })
    }

    /// For a remote OFFER (the server renegotiating, and the loopback test): returns our ANSWER SDP.
    pub fn answer(&self, offer_sdp: String) -> Result<String, String> {
        let pc = self.pc.clone();
        self.rt.block_on(async move {
            pc.set_remote_description(RTCSessionDescription::offer(offer_sdp).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Two peers in this process negotiate with each other exactly the way ROAP does (offer with all candidates ->
    /// answer -> connect), then audio goes mic buffer -> Opus -> RTP/SRTP -> Opus -> playback buffer. No network
    /// outside this machine, no audio device, nobody is called.
    #[test]
    fn two_local_peers_exchange_audio() {
        let (io1, io2) = (Arc::new(Io::default()), Arc::new(Io::default()));
        let a = Media::new(io1.clone(), None).unwrap();
        let b = Media::new(io2.clone(), None).unwrap();
        let offer = a.offer().unwrap();
        assert!(offer.contains("opus/48000/2") && offer.contains("a=candidate"), "offer carries the codec and candidates");
        let answer = b.answer(offer).unwrap();
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
}
