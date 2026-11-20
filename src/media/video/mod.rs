//! Video playback with the platform's own media framework: Media Foundation on Windows (`mf.rs`: IMFSourceReader, DXVA through
//! a D3D11 device manager where the GPU decodes, software otherwise), GStreamer on Linux (`gst.rs`: playbin into app sinks).
//! No FFmpeg is bundled; what plays is what the system can decode (see `format.rs`).
//!
//! A `Player` owns one decoder thread. The thread reads the file, converts pictures to RGBA (at most `sync::MAX_SIZE`) and sound
//! to f32, and fills the bounded queues of `sync.rs`; the sound goes to the speaker chosen in Settings > Audio (`out.rs`, cpal),
//! and the speaker's progress is the clock the UI shows pictures by (`Player::frame`). Seeking flushes the queues and decodes from
//! the key frame before the target up to the target (exact position). Dropping the `Player` stops the thread and the sound.
pub mod format;
pub mod sync;
pub mod synth;
mod out;
#[cfg(windows)]
mod mf;
#[cfg(windows)]
use mf as backend;
#[cfg(target_os = "linux")]
mod gst;
#[cfg(target_os = "linux")]
use gst as backend;
#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
pub use sync::Frame;
use sync::{Clock, Queues};

/// What the decoder reports about the file (size, hardware and readiness: for the tests and diagnostics; the UI uses the rest).
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct Info {
    /// Seconds; 0 = unknown.
    pub duration: f64,
    /// The decoded picture size (after scaling to `MAX_SIZE`).
    pub width: u32,
    pub height: u32,
    pub has_video: bool,
    pub has_audio: bool,
    /// The GPU decodes (DXVA on Windows; on Linux whether a hardware decoder element was picked is not reported: false).
    pub hardware: bool,
    /// The file is open and its streams are known.
    pub ready: bool,
    /// Why it cannot play (open or decode error).
    pub error: Option<String>,
}

/// One unit from a backend.
pub(crate) enum Packet { Video(Frame), Audio { pts: f64, rate: u32, ch: u16, data: Vec<f32> } }

/// A backend's open file.
pub(crate) trait Source {
    /// The next picture or block of sound; None at the end of the file.
    fn read(&mut self) -> Result<Option<Packet>, String>;
    fn seek(&mut self, secs: f64) -> Result<(), String>;
}

struct Shared {
    q: Mutex<Queues>,
    cv: Condvar,
    info: Mutex<Info>,
    clock: Mutex<Clock>,
    seek: Mutex<Option<f64>>,
    stop: AtomicBool,
    /// No speaker could be opened (or none is wanted): the sound is thrown away and the wall clock runs the picture.
    audio_off: AtomicBool,
    audio_started: AtomicBool,
    /// f32 bits.
    volume: AtomicU32,
    muted: AtomicBool,
    repaint: Mutex<Option<eframe::egui::Context>>,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Queues> { self.q.lock().unwrap_or_else(|e| e.into_inner()) }
    fn gain(&self) -> f32 { if self.muted.load(Ordering::Relaxed) { 0.0 } else { f32::from_bits(self.volume.load(Ordering::Relaxed)) } }
    fn playing(&self) -> bool { self.clock.lock().map(|c| c.playing).unwrap_or(false) }
    fn repaint(&self) { if let Some(c) = self.repaint.lock().ok().and_then(|c| c.clone()) { c.request_repaint(); } }
    /// The audio time while the speaker drives the clock.
    fn audio_clock(&self, q: &Queues) -> Option<f64> {
        let info_audio = self.info.lock().map(|i| i.has_audio).unwrap_or(false);
        let sound = info_audio && !self.audio_off.load(Ordering::Relaxed);
        // at the end of the file the sound may run out before the picture: the wall clock takes over from there
        (sound && !(q.eof && q.audio.is_empty())).then_some(q.audio_pts)
    }
}

/// A playing (or paused) video.
pub struct Player { sh: Arc<Shared> }

impl Player {
    /// Opens `path` on a new decoder thread, paused at the start. `speaker`: the output device's name (Settings > Audio,
    /// "" = the system default; None = no sound at all, e.g. tests and poster frames).
    pub fn open(path: &Path, speaker: Option<&str>, ctx: Option<eframe::egui::Context>) -> Player {
        let sh = Arc::new(Shared {
            q: Mutex::new(Queues::default()), cv: Condvar::new(), info: Mutex::new(Info::default()), clock: Mutex::new(Clock::default()),
            seek: Mutex::new(None), stop: AtomicBool::new(false), audio_off: AtomicBool::new(speaker.is_none()), audio_started: AtomicBool::new(false),
            volume: AtomicU32::new(1.0f32.to_bits()), muted: AtomicBool::new(false), repaint: Mutex::new(ctx),
        });
        let (sh2, p, spk) = (sh.clone(), path.to_path_buf(), speaker.unwrap_or("").to_string());
        let _ = std::thread::Builder::new().name("webox-video".into()).spawn(move || decode_thread(sh2, &p, spk));
        Player { sh }
    }

    pub fn info(&self) -> Info { self.sh.info.lock().map(|i| i.clone()).unwrap_or_default() }
    pub fn playing(&self) -> bool { self.sh.playing() }
    /// The current time in seconds.
    pub fn position(&self) -> f64 {
        let a = { let q = self.sh.lock(); self.sh.audio_clock(&q) };
        let mut c = self.sh.clock.lock().unwrap_or_else(|e| e.into_inner());
        let d = self.info().duration;
        let t = c.now(a);
        if d > 0.0 { t.min(d) } else { t }
    }
    pub fn play(&self) {
        if self.ended() { self.seek(0.0); }
        let a = { let q = self.sh.lock(); self.sh.audio_clock(&q) };
        self.sh.clock.lock().unwrap_or_else(|e| e.into_inner()).set_playing(true, a);
        self.sh.cv.notify_all();
    }
    pub fn pause(&self) {
        let a = { let q = self.sh.lock(); self.sh.audio_clock(&q) };
        self.sh.clock.lock().unwrap_or_else(|e| e.into_inner()).set_playing(false, a);
    }
    pub fn toggle(&self) { if self.playing() { self.pause() } else { self.play() } }
    /// Jumps to `secs` (clamped to the file); the picture there shows once it is decoded.
    pub fn seek(&self, secs: f64) {
        let d = self.info().duration;
        let t = if d > 0.0 { secs.clamp(0.0, d) } else { secs.max(0.0) };
        { let mut q = self.sh.lock(); q.flush(t); }
        self.sh.clock.lock().unwrap_or_else(|e| e.into_inner()).jump(t);
        *self.sh.seek.lock().unwrap_or_else(|e| e.into_inner()) = Some(t);
        self.sh.cv.notify_all();
    }
    pub fn volume(&self) -> f32 { f32::from_bits(self.sh.volume.load(Ordering::Relaxed)) }
    pub fn set_volume(&self, v: f32) { self.sh.volume.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed); }
    pub fn muted(&self) -> bool { self.sh.muted.load(Ordering::Relaxed) }
    pub fn set_muted(&self, m: bool) { self.sh.muted.store(m, Ordering::Relaxed); }
    /// The sound reaches a speaker (false: no audio track, no device, or opened without sound).
    pub fn audible(&self) -> bool { self.info().has_audio && !self.sh.audio_off.load(Ordering::Relaxed) }
    /// The whole file has been played.
    pub fn ended(&self) -> bool {
        let (t, d) = (self.position(), self.info().duration);
        let q = self.sh.lock();
        // every queued picture is due (shown or not: nobody may be looking) and the sound is out
        let pictures_done = q.frames.back().map_or(true, |f| f.pts <= t + 0.01);
        q.eof && pictures_done && (q.audio.is_empty() || self.sh.audio_off.load(Ordering::Relaxed)) && (d <= 0.0 || t >= d - 0.25 || (q.frames.is_empty() && !self.sh.playing()))
    }
    /// The picture for now (None: keep the last one) and when the next one is due (for the repaint).
    pub fn frame(&self) -> (Option<Frame>, Option<Duration>) {
        let t = self.position();
        let mut q = self.sh.lock();
        let (f, due) = sync::pick_frame(&mut q, t);
        drop(q);
        if f.is_some() { self.sh.cv.notify_all(); }
        let playing = self.playing();
        // paused: a frame is still taken when one arrives (after a seek), but no repaints are scheduled for the following ones
        let due = if playing { Some(due.map_or(Duration::from_millis(10), Duration::from_secs_f64)) } else if f.is_none() && self.sh.lock().frames.is_empty() { Some(Duration::from_millis(30)) } else { None };
        (f, due)
    }
}

impl Drop for Player {
    fn drop(&mut self) { self.sh.stop.store(true, Ordering::SeqCst); self.sh.cv.notify_all(); }
}

/// Decodes the first picture of a file (the poster when a share has no thumbnail, the composer's preview). Blocking: call it from a
/// worker thread.
pub fn first_frame(path: &Path) -> Result<(Frame, Info), String> {
    let (mut src, info) = backend::open(path, sync::MAX_SIZE, false)?;
    for _ in 0..2000 {
        match src.read()? {
            Some(Packet::Video(f)) => return Ok((f, info)),
            Some(_) => continue,
            None => break,
        }
    }
    Err("no picture".into())
}

fn decode_thread(sh: Arc<Shared>, path: &Path, speaker: String) {
    let (mut src, info) = match backend::open(path, sync::MAX_SIZE, true) {
        Ok(x) => x,
        Err(e) => { fail(&sh, path, e); return; }
    };
    if let Ok(mut i) = sh.info.lock() { *i = Info { ready: true, ..info }; }
    sh.repaint();
    let mut target: Option<f64> = None;
    let stopped = |sh: &Shared| sh.stop.load(Ordering::SeqCst);
    'outer: while !stopped(&sh) {
        if let Some(t) = sh.seek.lock().unwrap_or_else(|e| e.into_inner()).take() {
            if let Err(e) = src.seek(t) { fail(&sh, path, e); break; }
            target = Some(t);
        }
        if sh.lock().eof {
            let q = sh.lock();
            let _ = sh.cv.wait_timeout(q, Duration::from_millis(50));
            continue;
        }
        let pkt = match src.read() {
            Ok(Some(p)) => p,
            Ok(None) => { sh.lock().eof = true; sh.repaint(); continue; }
            Err(e) => { fail(&sh, path, e); break; }
        };
        match pkt {
            Packet::Video(f) => {
                if target.is_some_and(|t| f.pts + 0.02 < t) { continue; } // decoding up to the seek target
                let mut q = sh.lock();
                loop {
                    if stopped(&sh) || sh.seek.lock().map(|s| s.is_some()).unwrap_or(false) { continue 'outer; }
                    if q.has_video_room(f.rgba.len()) { break; }
                    let starving = sh.playing() && !sh.audio_off.load(Ordering::Relaxed) && info.has_audio && q.audio_secs() < sync::AUDIO_LOW_SECS && !q.eof;
                    if starving { q.pop_frame(); continue; }
                    q = sh.cv.wait_timeout(q, Duration::from_millis(20)).unwrap_or_else(|e| e.into_inner()).0;
                }
                let first = q.frames.is_empty();
                q.push_frame(f);
                drop(q);
                if first { sh.repaint(); }
            }
            Packet::Audio { pts, rate, ch, mut data } => {
                if sh.audio_off.load(Ordering::Relaxed) || ch == 0 || rate == 0 { continue; }
                if let Some(t) = target {
                    let frames = data.len() / ch as usize;
                    let end = pts + frames as f64 / rate as f64;
                    if end <= t { continue; }
                    let skip = (((t - pts) * rate as f64).max(0.0) as usize).min(frames);
                    data.drain(..skip * ch as usize);
                    if skip > 0 { return_audio(&sh, pts + skip as f64 / rate as f64, rate, ch, data, &speaker); continue; }
                }
                return_audio(&sh, pts, rate, ch, data, &speaker);
            }
        }
    }
    sh.stop.store(true, Ordering::SeqCst); // ends the speaker thread too
}

/// Records why the file cannot play (the card shows it). `WEBOX_VIDEO_LOG=<file>` also appends it there with the file name,
/// for reports about files that do not play (the framework's message only; nothing about the account).
fn fail(sh: &Shared, path: &Path, e: String) {
    if let Some(log) = std::env::var_os("WEBOX_VIDEO_LOG") {
        use std::io::Write;
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) { let _ = writeln!(f, "{name}: {e}"); }
    }
    if let Ok(mut i) = sh.info.lock() { i.error = Some(e); }
    sh.repaint();
}

/// Queues a block of sound (waiting for room) and starts the speaker with the first one.
fn return_audio(sh: &Arc<Shared>, pts: f64, rate: u32, ch: u16, data: Vec<f32>, speaker: &str) {
    if !sh.audio_started.swap(true, Ordering::SeqCst) {
        if !out::start(sh.clone(), speaker, rate, ch) { sh.audio_off.store(true, Ordering::SeqCst); return; }
    }
    let mut q = sh.lock();
    loop {
        if sh.stop.load(Ordering::SeqCst) || sh.seek.lock().map(|s| s.is_some()).unwrap_or(false) { return; }
        if q.has_audio_room() { break; }
        q = sh.cv.wait_timeout(q, Duration::from_millis(20)).unwrap_or_else(|e| e.into_inner()).0;
    }
    q.push_audio(pts, rate, ch, &data);
}
