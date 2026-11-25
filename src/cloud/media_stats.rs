//! Call statistics ("Anrufstatistik", the official `MediaStatistics` dialog): per stream (audio, video, share) and
//! direction the counters the call window turns into packet loss, jitter, bit rate and frame rate, plus the round trip.
//!
//! Where each figure comes from (webrtc-rs 0.14 `get_stats` has no receive-side loss / jitter, and leaves out a receiver
//! without a mid, which is where a Webex server's audio can arrive):
//! * round trip: the nominated ICE candidate pair's `currentRoundTripTime` (`get_stats`), else an RTCP receiver report's;
//! * sent bytes / packets: `get_stats` outbound-rtp;
//! * sent loss / jitter: the far end's RTCP receiver reports about our streams (`Live::rr`);
//! * received bytes / loss / jitter: counted from the RTP headers as they are read (`Live::rx`, RFC 3550 A.3 / A.8);
//! * share: picture size and pictures sent (`Live::share_sent`); the received share's size comes from `share_rx`.
//!
//! Cost: a few integer operations per packet. `get_stats` runs only while the statistics panel is open (`want`), every
//! `EVERY`, on the call's own thread (the locus tick); nothing is read or published while it is closed.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use webrtc::rtcp::packet::Packet;
use webrtc::rtcp::receiver_report::ReceiverReport;
use webrtc::rtcp::sender_report::SenderReport;
use webrtc::stats::{StatsReport, StatsReportType};

/// Stream slots of a `Snapshot`.
pub const AUDIO: usize = 0;
pub const VIDEO: usize = 1;
pub const SHARE: usize = 2;
/// RTP clock rates: Opus, H.264.
pub const AUDIO_CLOCK: u32 = 48_000;
pub const VIDEO_CLOCK: u32 = 90_000;
/// How often the panel's figures are renewed.
pub const EVERY: Duration = Duration::from_secs(2);

/// One direction of one stream. Counters are cumulative since the stream started; the panel takes differences.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flow {
    pub bytes: u64,
    pub packets: u64,
    /// Received: packets the sequence numbers say were sent, and how many of them never came.
    pub expected: u64,
    pub lost: u64,
    /// Sent: the share of our packets the far end lost since its previous receiver report (0..1).
    pub fraction_lost: Option<f32>,
    /// Interarrival jitter (received: ours; sent: the far end's report).
    pub jitter_ms: Option<f32>,
    /// Picture size (video / share).
    pub size: Option<(u32, u32)>,
    /// Pictures sent (share).
    pub frames: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub at: Option<Instant>,
    pub rtt_ms: Option<f32>,
    pub tx: [Flow; 3],
    pub rx: [Flow; 3],
}

/// Received RTP of one stream: extended sequence numbers and the RFC 3550 jitter estimate.
#[derive(Default)]
struct Rx { ssrc: u32, started: bool, base: u64, max: u64, received: u64, bytes: u64, prior_expected: u64, prior_received: u64, jitter: f64, transit: Option<i32>, t0: Option<Instant> }

impl Rx {
    fn packet(&mut self, ssrc: u32, seq: u16, ts: u32, len: usize, at: Instant, clock: u32) {
        if self.started && ssrc != self.ssrc {
            // a new source (renegotiation): its sequence numbers start over, the totals carry on
            self.prior_expected += self.max - self.base + 1;
            self.prior_received += self.received;
            self.received = 0;
            self.started = false;
            self.transit = None;
        }
        if !self.started {
            self.started = true;
            self.ssrc = ssrc;
            self.base = seq as u64 + (1 << 16); // room below for a few reordered packets
            self.max = self.base;
        } else {
            let d = seq.wrapping_sub(self.max as u16) as i16 as i64;
            let ext = (self.max as i64 + d).max(0) as u64;
            if ext > self.max { self.max = ext; }
            if ext < self.base { self.base = ext; }
        }
        self.received += 1;
        self.bytes += len as u64;
        // transit time in timestamp units; its change between packets feeds the jitter (RFC 3550 A.8)
        let t0 = *self.t0.get_or_insert(at);
        let arrival = (at.saturating_duration_since(t0).as_secs_f64() * clock as f64) as u64 as u32;
        let transit = arrival.wrapping_sub(ts) as i32;
        if let Some(prev) = self.transit { let d = transit.wrapping_sub(prev).unsigned_abs() as f64; self.jitter += (d - self.jitter) / 16.0; }
        self.transit = Some(transit);
    }

    fn flow(&self, clock: u32) -> Flow {
        let expected = self.prior_expected + if self.started { self.max - self.base + 1 } else { 0 };
        let received = self.prior_received + self.received;
        Flow { bytes: self.bytes, packets: received, expected, lost: expected.saturating_sub(received),
            jitter_ms: self.started.then(|| (self.jitter / clock as f64 * 1000.0) as f32), ..Default::default() }
    }
}

/// The counters a call's media fills as packets come and go (one per `Media`).
#[derive(Default)]
pub struct Live {
    rx: Mutex<[Rx; 3]>,
    /// The far end's last report about each of our streams: (fraction lost, jitter ms).
    rr: Mutex<[Option<(f32, f32)>; 3]>,
    rr_rtt: Mutex<Option<f32>>,
    share_size: AtomicU64,
    share_frames: AtomicU64,
}

impl Live {
    /// An RTP packet of stream `i` was read (`len`: its payload bytes).
    pub fn rx(&self, i: usize, h: &webrtc::rtp::header::Header, len: usize, clock: u32) {
        if let Ok(mut r) = self.rx.lock() { r[i].packet(h.ssrc, h.sequence_number, h.timestamp, len, Instant::now(), clock); }
    }

    /// RTCP that reached our sender of stream `i` (SSRC `own`, when known): the far end's reception report about it.
    pub fn rr(&self, i: usize, own: Option<u32>, pkts: &[Box<dyn Packet + Send + Sync>], clock: u32) {
        for p in pkts {
            let reports = if let Some(r) = p.as_any().downcast_ref::<ReceiverReport>() { &r.reports } else if let Some(s) = p.as_any().downcast_ref::<SenderReport>() { &s.reports } else { continue };
            let Some(b) = reports.iter().find(|b| own.is_none_or(|o| o == b.ssrc)) else { continue };
            if let Ok(mut r) = self.rr.lock() { r[i] = Some((b.fraction_lost as f32 / 256.0, b.jitter as f32 / clock as f32 * 1000.0)); }
            // round trip from the report (RFC 3550 6.4.1): now - LSR - DLSR, all in 1/65536 s of the NTP clock
            if b.last_sender_report != 0 {
                let now = ntp_mid(std::time::SystemTime::now());
                let rtt = now.wrapping_sub(b.last_sender_report).wrapping_sub(b.delay);
                if rtt < 65536 * 10 { if let Ok(mut x) = self.rr_rtt.lock() { *x = Some(rtt as f32 / 65.536); } }
            }
        }
    }

    /// The share's current picture size and the number of pictures sent so far.
    pub fn share_sent(&self, w: u32, h: u32, frames: u64) {
        self.share_size.store(((w as u64) << 32) | h as u64, Ordering::Relaxed);
        self.share_frames.store(frames, Ordering::Relaxed);
    }

    /// The counters filled in from the packets, with `get_stats`' figures on top.
    pub fn snapshot(&self, report: &StatsReport) -> Snapshot {
        let mut s = Snapshot { at: Some(Instant::now()), ..Default::default() };
        if let Ok(r) = self.rx.lock() { for (i, rx) in r.iter().enumerate() { s.rx[i] = rx.flow(if i == AUDIO { AUDIO_CLOCK } else { VIDEO_CLOCK }); } }
        if let Ok(r) = self.rr.lock() { for (i, x) in r.iter().enumerate() { if let Some((f, j)) = x { s.tx[i].fraction_lost = Some(*f); s.tx[i].jitter_ms = Some(*j); } } }
        let sz = self.share_size.load(Ordering::Relaxed);
        if sz != 0 { s.tx[SHARE].size = Some(((sz >> 32) as u32, sz as u32)); }
        s.tx[SHARE].frames = self.share_frames.load(Ordering::Relaxed);
        for r in report.reports.values() {
            match r {
                StatsReportType::CandidatePair(p) if p.nominated && p.current_round_trip_time > 0.0 => s.rtt_ms = Some((p.current_round_trip_time * 1000.0) as f32),
                StatsReportType::OutboundRTP(o) => {
                    let i = if o.kind == "audio" { AUDIO } else if o.track_identifier == SHARE_TRACK { SHARE } else { VIDEO };
                    s.tx[i].bytes += o.bytes_sent;
                    s.tx[i].packets += o.packets_sent;
                }
                _ => {}
            }
        }
        if s.rtt_ms.is_none() { s.rtt_ms = self.rr_rtt.lock().ok().and_then(|x| *x); }
        s
    }
}

/// The id of the share's sending track (`Media::add_lines`).
pub const SHARE_TRACK: &str = "slides";

/// The middle 32 bits of the NTP time (what a receiver report's LSR echoes).
fn ntp_mid(t: std::time::SystemTime) -> u32 {
    let d = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() + 2_208_988_800; // 1900 -> 1970
    let frac = ((d.subsec_nanos() as u64) << 32) / 1_000_000_000;
    (((secs << 32) | frac) >> 16) as u32
}

// The statistics panel and the call's thread meet here: the panel says it is open, the call's tick publishes.
static WANT: AtomicBool = AtomicBool::new(false);
static LATEST: Mutex<Option<Snapshot>> = Mutex::new(None);

/// The statistics panel is open (`true`, each frame it is drawn) or closed (`false`: the figures go).
pub fn want(on: bool) {
    if WANT.swap(on, Ordering::Relaxed) && !on { if let Ok(mut g) = LATEST.lock() { *g = None; } }
}
pub fn wanted() -> bool { WANT.load(Ordering::Relaxed) }

/// The newest figures for the panel (None before the first reading).
pub fn latest() -> Option<Snapshot> { LATEST.lock().ok().and_then(|g| g.clone()) }

/// Called from the call's tick: a new reading when the panel is open and the last one is `EVERY` old.
pub fn due(now: Instant) -> bool {
    wanted() && LATEST.lock().ok().is_none_or(|g| g.as_ref().and_then(|s| s.at).is_none_or(|t| now.saturating_duration_since(t) + Duration::from_millis(200) >= EVERY))
}

pub fn publish(s: Snapshot) {
    if !wanted() { return; }
    if let Ok(mut g) = LATEST.lock() { *g = Some(s); }
    crate::ui::wake();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn received_loss_and_jitter_from_rtp_headers() {
        let mut r = Rx::default();
        let t0 = Instant::now();
        // 100 packets every 20 ms, timestamps 960 apart, numbers 5, 6 and 50 missing; one arrives 10 ms late
        for n in 0u16..100 {
            if matches!(n, 5 | 6 | 50) { continue; }
            let late = if n == 30 { 10 } else { 0 };
            r.packet(7, 65_500u16.wrapping_add(n), n as u32 * 960, 100, t0 + Duration::from_millis(n as u64 * 20 + late), AUDIO_CLOCK);
        }
        let f = r.flow(AUDIO_CLOCK);
        assert_eq!((f.expected, f.packets, f.lost), (100, 97, 3), "sequence numbers wrap at 65535: {f:?}");
        assert_eq!(f.bytes, 9700);
        let j = f.jitter_ms.unwrap();
        assert!(j > 0.0 && j < 2.0, "one late packet: a little jitter, {j}");
        // a steady stream has none
        let mut s = Rx::default();
        for n in 0u16..50 { s.packet(1, n, n as u32 * 960, 10, t0 + Duration::from_millis(n as u64 * 20), AUDIO_CLOCK); }
        assert!(s.flow(AUDIO_CLOCK).jitter_ms.unwrap() < 0.1);
        // a new SSRC: counting starts over, the totals carry on
        for n in 0u16..10 { s.packet(2, 9000 + n, 0, 10, t0, AUDIO_CLOCK); }
        let f = s.flow(AUDIO_CLOCK);
        assert_eq!((f.expected, f.packets, f.lost), (60, 60, 0));
    }

    #[test]
    fn reordered_packets_are_not_lost() {
        let mut r = Rx::default();
        let t = Instant::now();
        for n in [10u16, 12, 11, 13, 9] { r.packet(1, n, 0, 1, t, VIDEO_CLOCK); }
        let f = r.flow(VIDEO_CLOCK);
        assert_eq!((f.expected, f.packets, f.lost), (5, 5, 0), "{f:?}");
    }

    #[test]
    fn receiver_report_gives_sent_loss_and_jitter() {
        use webrtc::rtcp::reception_report::ReceptionReport;
        let l = Live::default();
        let rr = ReceiverReport { ssrc: 1, reports: vec![
            ReceptionReport { ssrc: 99, fraction_lost: 200, jitter: 9000, ..Default::default() },
            ReceptionReport { ssrc: 42, fraction_lost: 64, jitter: 480, ..Default::default() }], ..Default::default() };
        let pkts: Vec<Box<dyn Packet + Send + Sync>> = vec![Box::new(rr)];
        l.rr(AUDIO, Some(42), &pkts, AUDIO_CLOCK);
        l.share_sent(1920, 1080, 33);
        let s = l.snapshot(&StatsReport { reports: Default::default() });
        assert_eq!(s.tx[AUDIO].fraction_lost, Some(0.25), "our SSRC's block, not the other one");
        assert_eq!(s.tx[AUDIO].jitter_ms, Some(10.0));
        assert_eq!((s.tx[SHARE].size, s.tx[SHARE].frames), (Some((1920, 1080)), 33));
        assert_eq!(s.rx[AUDIO].jitter_ms, None, "nothing received yet");
    }

    #[test]
    fn published_only_while_wanted() {
        want(false);
        publish(Snapshot::default());
        assert!(latest().is_none());
        want(true);
        assert!(due(Instant::now()));
        publish(Snapshot { at: Some(Instant::now()), ..Default::default() });
        assert!(latest().is_some() && !due(Instant::now()));
        want(false);
        assert!(latest().is_none(), "closing the panel drops the figures");
    }
}
