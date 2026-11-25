//! Receive-side jitter buffer of a call's Opus audio: RTP packets are held by (extended) sequence number and come out one
//! 20 ms slot at a time, so late and reordered packets still play in order and a lost one is concealed (Opus FEC from the
//! next packet when it is here, else PLC; at most 100 ms in a row). The depth follows the observed jitter (40..200 ms);
//! a level that stays off it (clock drift, a changed target) is corrected by dropping or repeating one 10 ms block
//! (`shrink` / `stretch`), never by a large cut. Pure: no decoder, no clock of its own (arrival times are passed in).
use std::collections::BTreeMap;
use std::time::Instant;

/// One packet's slot in ms (the Opus frame the call path uses).
const SLOT_MS: f64 = 20.0;
/// Bounds of the target depth, in slots: 40..200 ms.
const MIN_TARGET: u64 = 2;
const MAX_TARGET: u64 = 10;
/// Lost slots concealed in a row (100 ms); after that silence, or a fresh start once nothing is buffered.
const MAX_CONCEAL: u32 = 5;
/// Slots the level must stay off target before one 10 ms block is dropped / repeated; then again every `REPEAT` slots.
const HOLD: u32 = 10;
const REPEAT: u32 = 2;
/// More than 1 s buffered means the output stopped taking audio: cut back to the target.
const MAX_LEVEL: u64 = 50;
/// A sequence jump this far (2 s) from the playout point is a restarted stream: start over.
const RESET_JUMP: u64 = 100;

/// What the next slot plays.
#[derive(Debug, PartialEq)]
pub enum Play {
    /// Not (or no longer) enough buffered: play nothing, the output plays silence.
    Wait,
    /// Decode this packet.
    Packet(Vec<u8>),
    /// This slot's packet is lost, the next one is here: decode that in FEC mode for this slot (it still plays in its own).
    Fec(Vec<u8>),
    /// Lost and nothing after it yet: Opus PLC.
    Plc,
    /// Lost longer than concealment covers: a slot of silence.
    Silence,
}

/// Drift correction applied to the slot's PCM.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Adjust { None, Shrink, Stretch }

#[derive(Default)]
pub struct Jitter {
    pkts: BTreeMap<u64, Vec<u8>>,
    /// Highest extended sequence number seen.
    top: Option<u64>,
    /// The slot that plays next (None before the first packet and after a stall).
    next: Option<u64>,
    started: bool,
    lost_row: u32,
    /// The last slot that played a real packet.
    played: Option<u64>,
    /// Interarrival jitter in ms (RFC 3550 style), the last packet's transit and the first arrival.
    jitter: f64,
    transit: Option<f64>,
    t0: Option<Instant>,
    high: u32,
    low: u32,
}

impl Jitter {
    /// The extended sequence number of `seq`: the one closest to the highest seen (16-bit wraparound).
    fn extend(&self, seq: u16) -> u64 {
        let Some(top) = self.top else { return (1 << 16) | seq as u64 };
        let c = (top & !0xFFFF) | seq as u64;
        [c.saturating_sub(1 << 16), c, c + (1 << 16)].into_iter().min_by_key(|x| x.abs_diff(top)).unwrap_or(c)
    }

    /// Slots from the playout point to the highest packet (missing ones included).
    pub fn level(&self) -> u64 { match (self.next, self.top) { (Some(n), Some(t)) => (t + 1).saturating_sub(n), _ => 0 } }

    /// The depth to hold, in slots: one slot plus three times the jitter, within 40..200 ms.
    pub fn target(&self) -> u64 { (((SLOT_MS + 3.0 * self.jitter) / SLOT_MS).ceil() as u64).clamp(MIN_TARGET, MAX_TARGET) }

    /// Takes a packet that arrived at `at`; false when it is dropped (a duplicate, or its slot already played).
    pub fn push(&mut self, seq: u16, payload: Vec<u8>, at: Instant) -> bool {
        let mut e = self.extend(seq);
        if let (Some(n), Some(t)) = (self.next, self.top) {
            if e + RESET_JUMP < n || e > t.max(n) + RESET_JUMP { *self = Jitter::default(); e = self.extend(seq); }
        }
        let t0 = *self.t0.get_or_insert(at);
        let transit = at.saturating_duration_since(t0).as_secs_f64() * 1000.0 - e as f64 * SLOT_MS;
        if let Some(p) = self.transit.replace(transit) { self.jitter += ((transit - p).abs() - self.jitter) / 16.0; }
        let n = *self.next.get_or_insert(e);
        if e < n && !self.started { self.next = Some(e); }
        // concealing with nothing buffered (a silent / DTX gap: the sender skipped packets, not numbers): the packet after
        // the last one played resumes the stream here instead of counting as late
        else if e < n && self.lost_row > 0 && self.pkts.is_empty() && self.played.is_none_or(|p| e > p) { self.next = Some(e); }
        else if e < n { return false; }
        if self.pkts.contains_key(&e) { return false; }
        self.pkts.insert(e, payload);
        self.top = Some(self.top.map_or(e, |t| t.max(e)));
        if self.level() > MAX_LEVEL {
            let keep = self.top.unwrap_or(e) + 1 - self.target();
            self.pkts = self.pkts.split_off(&keep);
            self.next = Some(keep);
        }
        true
    }

    /// The next 20 ms slot, with the drift correction to apply to its PCM.
    pub fn pop(&mut self) -> (Play, Adjust) {
        let Some(n) = self.next else { return (Play::Wait, Adjust::None) };
        let (level, target) = (self.level(), self.target());
        if !self.started {
            if level < target { return (Play::Wait, Adjust::None); }
            (self.started, self.high, self.low, self.lost_row) = (true, 0, 0, 0);
        }
        let play = match self.pkts.remove(&n) {
            Some(p) => { (self.lost_row, self.played) = (0, Some(n)); Play::Packet(p) }
            None => {
                self.lost_row += 1;
                if let Some(p) = self.pkts.get(&(n + 1)) { Play::Fec(p.clone()) }
                else if self.lost_row <= MAX_CONCEAL { Play::Plc }
                // a stall: start over with the next packet that comes, at that packet
                else if level <= 1 { (self.started, self.next) = (false, None); return (Play::Wait, Adjust::None); }
                else { Play::Silence }
            }
        };
        self.next = Some(n + 1);
        (play, self.drift(level, target))
    }

    /// One 10 ms correction once the level stayed above / below the target for `HOLD` slots, then every `REPEAT`.
    fn drift(&mut self, level: u64, target: u64) -> Adjust {
        let (on, off, adj) = if level > target + 1 { (&mut self.high, &mut self.low, Adjust::Shrink) }
            else if level < target { (&mut self.low, &mut self.high, Adjust::Stretch) }
            else { (self.high, self.low) = (0, 0); return Adjust::None };
        *off = 0;
        *on += 1;
        if *on < HOLD { return Adjust::None; }
        *on = HOLD - REPEAT;
        adj
    }
}

/// Crossfade of `a` into `b` (same length).
fn fade<'a>(a: &'a [i16], b: &'a [i16]) -> impl Iterator<Item = i16> + 'a {
    let n = a.len() as f32;
    a.iter().zip(b).enumerate().map(move |(i, (&x, &y))| { let w = i as f32 / n; (x as f32 * (1.0 - w) + y as f32 * w) as i16 })
}

/// The frame half as long: its first half crossfaded into its second (starts and ends where the frame does).
pub fn shrink(pcm: &[i16]) -> Vec<i16> {
    let h = pcm.len() / 2;
    if h == 0 { return pcm.to_vec(); }
    fade(&pcm[..h], &pcm[h..2 * h]).collect()
}

/// The frame half as long again: first half, second half crossfaded back into the first, then the second half.
pub fn stretch(pcm: &[i16]) -> Vec<i16> {
    let h = pcm.len() / 2;
    if h == 0 { return pcm.to_vec(); }
    let (a, b) = (&pcm[..h], &pcm[h..2 * h]);
    a.iter().copied().chain(fade(b, a)).chain(pcm[h..].iter().copied()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A buffer fed packets `seqs` on time (one per 20 ms from `t0`), payload = the low byte of the sequence number.
    fn fed(seqs: &[u16]) -> (Jitter, Instant) {
        let (mut j, t0) = (Jitter::default(), Instant::now());
        for &s in seqs { j.push(s, vec![s as u8], t0 + Duration::from_millis(20 * s.wrapping_sub(seqs[0]) as u64)); }
        (j, t0)
    }
    fn played(j: &mut Jitter, n: usize) -> Vec<Play> { (0..n).map(|_| j.pop().0).collect() }

    #[test]
    fn waits_for_the_target_then_plays_in_order() {
        let (mut j, _) = fed(&[10]);
        assert_eq!(j.pop().0, Play::Wait, "40 ms not buffered yet");
        let (mut j, _) = fed(&[10, 11, 12]);
        assert_eq!(j.target(), MIN_TARGET);
        assert_eq!(played(&mut j, 3), [Play::Packet(vec![10]), Play::Packet(vec![11]), Play::Packet(vec![12])]);
    }

    #[test]
    fn reordered_packets_play_in_sequence() {
        let (mut j, t0) = (Jitter::default(), Instant::now());
        for (i, s) in [1u16, 3, 2, 4].into_iter().enumerate() { assert!(j.push(s, vec![s as u8], t0 + Duration::from_millis(20 * i as u64))); }
        assert_eq!(played(&mut j, 4), [1, 2, 3, 4].map(|s| Play::Packet(vec![s])));
        // an earlier packet before the start moves the start back
        let (mut j, t0) = (Jitter::default(), Instant::now());
        j.push(6, vec![6], t0);
        j.push(5, vec![5], t0);
        assert_eq!(j.pop().0, Play::Packet(vec![5]));
    }

    #[test]
    fn a_lost_packet_is_concealed_with_fec_or_plc() {
        let (mut j, _) = fed(&[1, 2, 4, 5]);
        assert_eq!(played(&mut j, 2), [Play::Packet(vec![1]), Play::Packet(vec![2])]);
        assert_eq!(j.pop().0, Play::Fec(vec![4]), "the next packet carries the lost one's FEC");
        assert_eq!(played(&mut j, 2), [Play::Packet(vec![4]), Play::Packet(vec![5])]);
        assert_eq!(j.pop().0, Play::Plc, "nothing after it: PLC");
    }

    #[test]
    fn speech_after_a_short_silent_gap_is_not_dropped() {
        // the sender skipped packets in silence (DTX): sequence numbers go on where they stopped
        let (mut j, t0) = fed(&[1, 2]);
        played(&mut j, 2);
        assert_eq!(played(&mut j, 2), [Play::Plc, Play::Plc]);
        assert!(j.push(3, vec![3], t0 + Duration::from_millis(100)), "the next packet resumes the stream");
        assert!(j.push(4, vec![4], t0 + Duration::from_millis(120)));
        assert_eq!(played(&mut j, 2), [Play::Packet(vec![3]), Play::Packet(vec![4])]);
        // a packet older than what played stays late
        assert!(!j.push(2, vec![2], t0 + Duration::from_millis(140)));
    }

    #[test]
    fn concealment_stops_after_100_ms() {
        let (mut j, t0) = fed(&[1, 2]);
        played(&mut j, 2);
        assert_eq!(played(&mut j, 5), (0..5).map(|_| Play::Plc).collect::<Vec<_>>());
        assert_eq!(j.pop().0, Play::Wait, "a stall: rebuffer");
        // audio resumes later: it starts at the first new packet, not 100 ms of silence for the gap
        j.push(20, vec![20], t0 + Duration::from_millis(400));
        j.push(21, vec![21], t0 + Duration::from_millis(420));
        assert_eq!(j.pop().0, Play::Packet(vec![20]));
        // a long hole with packets after it: silence past the concealment, then the packets
        let (mut j, _) = fed(&[1, 2, 3, 12, 13]);
        let p = played(&mut j, 12);
        assert_eq!(&p[3..8], (0..5).map(|_| Play::Plc).collect::<Vec<_>>());
        assert_eq!(&p[8..], [Play::Silence, Play::Silence, Play::Fec(vec![12]), Play::Packet(vec![12])]);
    }

    #[test]
    fn duplicates_and_late_packets_are_dropped() {
        let (mut j, t0) = fed(&[1, 2, 3]);
        assert!(!j.push(2, vec![99], t0), "duplicate");
        played(&mut j, 2);
        assert!(!j.push(1, vec![1], t0), "its slot already played");
        assert_eq!(j.pop().0, Play::Packet(vec![3]));
    }

    #[test]
    fn sequence_numbers_wrap() {
        let (mut j, _) = fed(&[65534, 65535, 0, 1]);
        assert_eq!(played(&mut j, 4), [254, 255, 0, 1].map(|s| Play::Packet(vec![s])));
        // reordered across the wrap
        let (mut j, t0) = (Jitter::default(), Instant::now());
        for s in [65535u16, 1, 0] { j.push(s, vec![s as u8], t0); }
        assert_eq!(played(&mut j, 3), [255, 0, 1].map(|s| Play::Packet(vec![s])));
    }

    #[test]
    fn a_far_jump_restarts_the_stream() {
        let (mut j, t0) = fed(&[100, 101, 102]);
        played(&mut j, 3);
        j.push(5000, vec![1], t0);
        j.push(5001, vec![2], t0);
        assert_eq!(j.pop().0, Play::Packet(vec![1]));
    }

    #[test]
    fn drift_drops_or_repeats_10_ms_blocks() {
        // the far end runs 10 % fast; the output takes 20 ms per round, a shrunk slot gives it 10 ms, a stretched one 30
        let (mut j, t0) = (Jitter::default(), Instant::now());
        let (mut seq, mut adj, mut owed) = (0u16, vec![], 0i32);
        for i in 0..300u64 {
            for _ in 0..if i % 10 == 0 { 2 } else { 1 } { j.push(seq, vec![0], t0 + Duration::from_millis(20 * seq as u64)); seq += 1; }
            owed += 20;
            while owed > 0 {
                let (p, a) = j.pop();
                if p == Play::Wait { break; }
                owed -= match a { Adjust::Shrink => 10, Adjust::Stretch => 30, Adjust::None => 20 };
                adj.push(a);
            }
        }
        assert!(adj.contains(&Adjust::Shrink) && !adj.contains(&Adjust::Stretch), "{adj:?}");
        assert!(j.level() <= j.target() + 3, "kept near the target: {}", j.level());
        // the buffer ran down to one slot below the target (2): blocks are repeated
        let (mut j, t0) = (Jitter::default(), Instant::now());
        let mut adj = vec![];
        for s in 0..40u16 { j.push(s, vec![0], t0 + Duration::from_millis(20 * s as u64)); j.started = true; adj.push(j.pop().1); }
        assert!(adj.contains(&Adjust::Stretch) && !adj.contains(&Adjust::Shrink), "{adj:?}");
        // the PCM side: 10 ms out or in, continuous at both ends
        let f: Vec<i16> = (0..960).map(|i| i as i16).collect();
        let (s, l) = (shrink(&f), stretch(&f));
        assert_eq!((s.len(), l.len()), (480, 1440));
        assert_eq!((s[0], l[0], l[1439]), (0, 0, 959));
        assert!((s[479] - 959).abs() < 3 && (l[959] - 479).abs() < 3);
    }

    #[test]
    fn the_target_follows_the_jitter_within_bounds() {
        let (mut j, t0) = (Jitter::default(), Instant::now());
        for s in 0..200u16 { j.push(s, vec![0], t0 + Duration::from_millis(20 * s as u64 + if s % 2 == 0 { 0 } else { 30 })); }
        assert!(j.target() > MIN_TARGET && j.target() <= MAX_TARGET, "{}", j.target());
        let mut k = Jitter { jitter: 1000.0, ..Default::default() };
        assert_eq!(k.target(), MAX_TARGET);
        k.jitter = 0.0;
        assert_eq!(k.target(), MIN_TARGET);
    }
}
