//! The decoded media between the decoder thread and its consumers: bounded queues of video frames and audio samples, and the
//! clock that keeps the picture in step with the sound.
//!
//! Memory: at most `MAX_FRAMES` frames and `MAX_FRAME_BYTES` bytes of pictures (frames are decoded at most 1920 x 1080, see
//! `MAX_SIZE`), and `AUDIO_SECS` seconds of sound; the decoder waits when either is full.
//! Clock: while there is sound, the clock is the presentation time of the next sample the speaker takes (`audio_pts`), so the
//! picture follows the audio device's own pace; without sound (no track, no device, or the sound has ended before the picture)
//! a wall clock carries on from the last audio time.
use std::collections::VecDeque;
use std::time::Instant;

/// Largest decoded picture: bigger videos are scaled down by the framework while decoding (the message list and the viewer never
/// need more; a 4K frame is 33 MB).
pub const MAX_SIZE: (u32, u32) = (1920, 1080);
pub const MAX_FRAMES: usize = 24;
pub const MAX_FRAME_BYTES: usize = 64 << 20;
pub const AUDIO_SECS: f64 = 2.0;
/// Below this much buffered sound (while playing) the audio clock is about to stall: a full picture queue then gives way
/// (drops its oldest frame) instead of waiting, so a badly interleaved file cannot block both queues.
pub const AUDIO_LOW_SECS: f64 = 0.2;

/// One decoded picture, RGBA, tightly packed (`w * 4` bytes per row).
#[derive(Clone)]
pub struct Frame { pub pts: f64, pub w: usize, pub h: usize, pub rgba: Vec<u8> }

#[derive(Default)]
pub struct Queues {
    pub frames: VecDeque<Frame>,
    pub frame_bytes: usize,
    /// Interleaved f32 samples at `rate` / `ch`; `audio_pts` is the time of the front sample.
    pub audio: VecDeque<f32>,
    pub audio_pts: f64,
    pub rate: u32,
    pub ch: u16,
    /// The decoder has read the whole file.
    pub eof: bool,
}

impl Queues {
    pub fn audio_secs(&self) -> f64 { if self.rate == 0 || self.ch == 0 { 0.0 } else { self.audio.len() as f64 / (self.rate as f64 * self.ch as f64) } }
    pub fn has_video_room(&self, bytes: usize) -> bool { self.frames.len() < MAX_FRAMES && (self.frames.is_empty() || self.frame_bytes + bytes <= MAX_FRAME_BYTES) }
    pub fn has_audio_room(&self) -> bool { self.audio_secs() < AUDIO_SECS }
    pub fn push_frame(&mut self, f: Frame) { self.frame_bytes += f.rgba.len(); self.frames.push_back(f); }
    pub fn pop_frame(&mut self) -> Option<Frame> { let f = self.frames.pop_front()?; self.frame_bytes -= f.rgba.len(); Some(f) }
    /// Appends a block of sound; an empty queue starts at the block's own time, otherwise the block is taken as continuing.
    pub fn push_audio(&mut self, pts: f64, rate: u32, ch: u16, data: &[f32]) {
        if self.audio.is_empty() || self.rate != rate || self.ch != ch { self.audio.clear(); self.audio_pts = pts; self.rate = rate; self.ch = ch; }
        self.audio.extend(data.iter().copied());
    }
    /// Takes one sample frame (all channels) into `out`; false when the queue is empty. Advances the audio clock.
    pub fn pop_audio_frame(&mut self, out: &mut [f32]) -> bool {
        let ch = self.ch as usize;
        if ch == 0 || self.audio.len() < ch { return false; }
        for (i, s) in self.audio.drain(..ch).enumerate() { if i < out.len() { out[i] = s; } }
        self.audio_pts += 1.0 / self.rate as f64;
        true
    }
    /// After a seek: everything queued is from before it.
    pub fn flush(&mut self, at: f64) { self.frames.clear(); self.frame_bytes = 0; self.audio.clear(); self.audio_pts = at; self.eof = false; }
}

/// The picture to show at time `t`: frames that are already due except the newest of them are skipped (late), the newest due one is
/// returned. Returns (frame to show now, seconds until the next queued frame is due).
pub fn pick_frame(q: &mut Queues, t: f64) -> (Option<Frame>, Option<f64>) {
    const EARLY: f64 = 0.004; // show a frame up to 4 ms before its time (the repaint lands a little late otherwise)
    while q.frames.len() >= 2 && q.frames[1].pts <= t + EARLY { q.pop_frame(); }
    let now = if q.frames.front().is_some_and(|f| f.pts <= t + EARLY) { q.pop_frame() } else { None };
    (now, q.frames.front().map(|f| (f.pts - t).max(0.0)))
}

/// Audio time while sound drives the clock, else a wall clock carried on from the last audio / seek time.
pub struct Clock { pub playing: bool, base: f64, at: Instant }

impl Default for Clock { fn default() -> Self { Clock { playing: false, base: 0.0, at: Instant::now() } } }

impl Clock {
    /// `audio`: Some(audio_pts) while the speaker drives the time.
    pub fn now(&mut self, audio: Option<f64>) -> f64 {
        match audio {
            Some(a) => { self.base = a; self.at = Instant::now(); a }
            None if self.playing => self.base + self.at.elapsed().as_secs_f64(),
            None => self.base,
        }
    }
    pub fn set_playing(&mut self, on: bool, audio: Option<f64>) { let t = self.now(audio); self.base = t; self.at = Instant::now(); self.playing = on; }
    pub fn jump(&mut self, t: f64) { self.base = t; self.at = Instant::now(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn f(pts: f64) -> Frame { Frame { pts, w: 1, h: 1, rgba: vec![0; 4] } }

    #[test]
    fn frames_follow_the_clock() {
        let mut q = Queues::default();
        for i in 0..5 { q.push_frame(f(i as f64 / 10.0)); }
        assert_eq!(q.frame_bytes, 20);
        let (now, due) = pick_frame(&mut q, 0.0);
        assert_eq!(now.map(|f| f.pts), Some(0.0));
        assert!((due.unwrap() - 0.1).abs() < 1e-9);
        assert!(pick_frame(&mut q, 0.05).0.is_none()); // the next one is not due yet: keep showing the current one
        // the clock jumped ahead (a stall): late frames are skipped, the newest due one is shown
        let (now, due) = pick_frame(&mut q, 0.31);
        assert_eq!(now.map(|f| f.pts), Some(0.3));
        assert!((due.unwrap() - 0.09).abs() < 1e-9);
        assert_eq!(q.frames.len(), 1);
        assert_eq!(q.frame_bytes, 4);
        assert_eq!(pick_frame(&mut q, 1.0).0.map(|f| f.pts), Some(0.4));
        assert_eq!(pick_frame(&mut q, 1.0), (None, None));
    }

    impl PartialEq for Frame { fn eq(&self, o: &Self) -> bool { self.pts == o.pts } }
    impl std::fmt::Debug for Frame { fn fmt(&self, w: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(w, "Frame@{}", self.pts) } }

    #[test]
    fn audio_drives_the_clock() {
        let mut q = Queues::default();
        q.push_audio(1.0, 100, 2, &[0.5; 40]); // 0.2 s of stereo at 100 Hz
        assert!((q.audio_secs() - 0.2).abs() < 1e-9);
        q.push_audio(1.2, 100, 2, &[0.25; 20]); // continues
        let mut out = [0.0f32; 2];
        for _ in 0..20 { assert!(q.pop_audio_frame(&mut out)); }
        assert_eq!(out, [0.5, 0.5]);
        assert!((q.audio_pts - 1.2).abs() < 1e-9);
        assert!(q.pop_audio_frame(&mut out) && out == [0.25, 0.25]);
        q.flush(7.0);
        assert!(!q.pop_audio_frame(&mut out));
        assert_eq!(q.audio_pts, 7.0);
        assert!(q.has_audio_room() && q.has_video_room(1 << 30)); // an empty queue always takes one frame
    }

    #[test]
    fn wall_clock_carries_on_from_the_audio_time() {
        let mut c = Clock::default();
        assert_eq!(c.now(None), 0.0); // paused
        c.set_playing(true, Some(3.0));
        assert_eq!(c.now(Some(3.5)), 3.5);
        std::thread::sleep(std::time::Duration::from_millis(30));
        let t = c.now(None); // the sound ended: the wall clock goes on from 3.5
        assert!((3.52..3.7).contains(&t), "{t}");
        c.set_playing(false, None);
        let p = c.now(None);
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(c.now(None), p); // paused: stands still
        c.jump(10.0);
        assert_eq!(c.now(None), 10.0);
    }
}
