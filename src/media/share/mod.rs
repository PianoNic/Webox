//! Screen sharing, the sending side: what can be shared (`sources`, with thumbnails for the picker), and a running share
//! (`start`) that captures the chosen screen or window and encodes it to H.264 for the call's content stream.
//!
//! * Windows (`windows/`): Windows.Graphics.Capture for monitors (HMONITOR) and windows (HWND), DXGI Desktop Duplication
//!   for monitors where WGC is missing; Media Foundation's H.264 encoder MFT (the GPU's when there is one, else Microsoft's
//!   software encoder). Pictures go through the CPU (staging texture -> BGRA -> scaled NV12, `convert.rs`).
//! * Linux (`linux/`): the xdg-desktop-portal ScreenCast (PipeWire node) or, on X11 without the portal, `ximagesrc`; GStreamer
//!   converts, scales and encodes (x264enc / openh264enc / vah264enc / vaapih264enc, whichever is installed).
//!
//! The output is what an RTP packetizer for packetization-mode 1 wants: one access unit per picture in Annex B form
//! (start codes), Constrained Baseline, no B-frames, SPS + PPS in front of every key frame (`h264.rs`). The stream picks a
//! new resolution by itself when the shared window is resized (a key frame with new parameter sets follows).
//!
//! Not built yet: the "you are sharing" border around the shared screen / window and the floating share control bar; the
//! system's own capture border (yellow on Windows) is left on until then.
// The call's content stream is the consumer of most of this API; until it is wired up only the picker and the tests use it.
#![allow(dead_code)]
pub mod convert;
pub mod h264;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use self::windows as backend;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use self::linux as backend;
#[cfg(target_os = "macos")]
mod macos; // not built yet (ScreenCaptureKit + VideoToolbox): nothing to share
#[cfg(target_os = "macos")]
use self::macos as backend;
#[cfg(test)]
mod ts;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What a source is (the picker's two sections, plus the system dialog and the built-in test picture).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A whole monitor ("Bildschirm").
    Screen,
    /// One application window ("Anwendung").
    Window,
    /// Wayland: the desktop portal's own dialog picks screen or window (`handle` 1 = screens, 2 = windows).
    Portal,
    /// A moving test picture made in-process (tests, `--sample`); captures nothing.
    Test,
}

/// One entry of the picker.
#[derive(Clone, Debug, PartialEq)]
pub struct ShareSource {
    pub kind: Kind,
    /// HMONITOR / HWND on Windows, X11 window id (0 = the whole X screen) on Linux.
    pub handle: isize,
    /// Monitor index (0 = primary) for screens; used by the DXGI fallback and for the "Screen 1" names.
    pub index: u32,
    pub name: String,
    /// The program a window belongs to (file name without extension; empty for screens).
    pub app: String,
    /// Size in pixels (0 x 0 = unknown, e.g. the portal).
    pub size: (u32, u32),
    pub primary: bool,
    /// A minimised window: no live picture until it is restored.
    pub minimized: bool,
}

impl ShareSource {
    /// The built-in test picture (`w` x `h`, moving colour bars and a frame counter strip).
    pub fn test_pattern(w: u32, h: u32) -> ShareSource {
        ShareSource { kind: Kind::Test, handle: 0, index: 0, name: "Test".into(), app: String::new(), size: (w, h), primary: false, minimized: false }
    }
}

/// The picker's "optimise for" choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Optimize {
    /// Sharp text: full resolution (up to 1920 x 1080), few pictures per second.
    #[default]
    Text,
    /// Smooth motion: 1280 x 720 at 30 fps.
    Motion,
}

/// Encoder settings. `Opts::preset` gives the picker's two choices; the call code may change bitrate and frame rate while
/// the share runs (`ShareStream::set_bitrate` / `set_fps`) when the far end asks for less.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Opts {
    /// Largest picture sent; bigger sources are scaled down keeping the aspect ratio.
    pub max_size: (u32, u32),
    pub fps: u32,
    /// Bits per second.
    pub bitrate: u32,
    /// Seconds between key frames (0 = only on request).
    pub keyframe_secs: u32,
    /// Include the mouse pointer.
    pub cursor: bool,
}

impl Opts {
    pub fn preset(o: Optimize) -> Opts {
        match o {
            Optimize::Text => Opts { max_size: (1920, 1080), fps: 5, bitrate: 1_500_000, keyframe_secs: 10, cursor: true },
            Optimize::Motion => Opts { max_size: (1280, 720), fps: 30, bitrate: 2_500_000, keyframe_secs: 4, cursor: true },
        }
    }
}

impl Default for Opts { fn default() -> Opts { Opts::preset(Optimize::Text) } }

/// One encoded picture.
#[derive(Clone, Debug)]
pub struct AccessUnit {
    /// Annex B: NAL units, each behind a 00 00 00 01 start code (split with `h264::nals`).
    pub data: Vec<u8>,
    /// Capture time in microseconds since the share started (monotonic; RTP time = pts_us * 90 / 1000).
    pub pts_us: u64,
    /// An IDR picture (SPS + PPS included).
    pub keyframe: bool,
    pub width: u32,
    pub height: u32,
}

/// Where a share is.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Opening the capture (on Wayland: the portal dialog is up).
    Starting,
    /// Pictures are coming; `encoder` names the H.264 encoder (e.g. "Media Foundation (hardware): ..." or "x264enc").
    Running { width: u32, height: u32, encoder: String },
    /// The source went away (window closed, monitor unplugged) or the user cancelled the portal dialog.
    Ended,
    Failed(String),
}

/// Live controls shared between a `ShareStream` and its worker thread.
pub(crate) struct Ctl {
    pub stop: AtomicBool,
    pub keyframe: AtomicBool,
    pub bitrate: AtomicU32,
    pub fps: AtomicU32,
    pub status: Mutex<Status>,
    /// Access units the receiver did not take in time (the queue holds 8); a key frame is forced after a drop.
    pub dropped: AtomicU32,
}

impl Ctl {
    pub fn stopped(&self) -> bool { self.stop.load(Ordering::SeqCst) }
    pub fn set_status(&self, s: Status) { *self.status.lock().unwrap_or_else(|e| e.into_inner()) = s; }
    pub fn take_keyframe(&self) -> bool { self.keyframe.swap(false, Ordering::SeqCst) }
    pub fn fps(&self) -> u32 { self.fps.load(Ordering::Relaxed).clamp(1, 60) }
    pub fn bitrate(&self) -> u32 { self.bitrate.load(Ordering::Relaxed).max(50_000) }
}

/// Hands access units to the receiver without ever blocking the capture thread.
pub(crate) struct Out { pub tx: SyncSender<AccessUnit>, pub ctl: Arc<Ctl> }

impl Out {
    /// False once the receiver is gone (the worker then ends).
    pub fn send(&self, au: AccessUnit) -> bool {
        match self.tx.try_send(au) {
            Ok(()) => true,
            Err(std::sync::mpsc::TrySendError::Full(_)) => { self.ctl.dropped.fetch_add(1, Ordering::Relaxed); self.ctl.keyframe.store(true, Ordering::SeqCst); true }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => false,
        }
    }
}

/// A running share. Dropping it stops the capture.
pub struct ShareStream { rx: Receiver<AccessUnit>, ctl: Arc<Ctl>, thread: Option<std::thread::JoinHandle<()>> }

impl ShareStream {
    /// The next access unit (None after `timeout`, or when the share has ended; see `status`).
    pub fn recv(&self, timeout: Duration) -> Option<AccessUnit> {
        match self.rx.recv_timeout(timeout) { Ok(au) => Some(au), Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => None }
    }
    /// Everything queued right now, without waiting.
    pub fn drain(&self) -> Vec<AccessUnit> { self.rx.try_iter().collect() }
    /// The next picture is encoded as a key frame (the far end's PLI / FIR).
    pub fn request_keyframe(&self) { self.ctl.keyframe.store(true, Ordering::SeqCst); }
    pub fn set_bitrate(&self, bps: u32) { self.ctl.bitrate.store(bps, Ordering::Relaxed); }
    pub fn set_fps(&self, fps: u32) { self.ctl.fps.store(fps.clamp(1, 60), Ordering::Relaxed); }
    pub fn status(&self) -> Status { self.ctl.status.lock().map(|s| s.clone()).unwrap_or(Status::Ended) }
    /// Access units dropped because nobody took them.
    pub fn dropped(&self) -> u32 { self.ctl.dropped.load(Ordering::Relaxed) }
    /// Stops capturing and waits for the worker.
    pub fn stop(mut self) { self.halt(); }
    fn halt(&mut self) {
        self.ctl.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() { let _ = t.join(); }
    }
}

impl Drop for ShareStream { fn drop(&mut self) { self.halt(); } }

/// Screens first (primary first), then windows in Z order; Webox's own windows are never listed.
pub fn sources() -> Vec<ShareSource> { backend::sources() }

/// Why nothing can be shared (German, for the picker), or None. Linux: Wayland without a ScreenCast portal.
pub fn unavailable() -> Option<String> {
    #[cfg(target_os = "linux")] { backend::unavailable() }
    #[cfg(not(target_os = "linux"))] { None }
}

/// A small picture of the source (RGBA, fitted into `max`), for the picker; None when there is none (minimised, Wayland).
/// Blocking: call it from a worker thread.
pub fn thumbnail(s: &ShareSource, max: (u32, u32)) -> Option<(u32, u32, Vec<u8>)> {
    if s.kind == Kind::Test { let (w, h) = convert::fit(s.size.0, s.size.1, max); let mut px = vec![0; (w * h * 4) as usize]; convert::test_picture(0, w, h, &mut px); return Some((w, h, px)); }
    backend::thumbnail(s, max)
}

/// Starts sharing `source`. Returns at once; the capture opens on its own thread (watch `status`).
pub fn start(source: &ShareSource, opts: Opts) -> ShareStream {
    let ctl = Arc::new(Ctl {
        stop: AtomicBool::new(false), keyframe: AtomicBool::new(false), bitrate: AtomicU32::new(opts.bitrate), fps: AtomicU32::new(opts.fps),
        status: Mutex::new(Status::Starting), dropped: AtomicU32::new(0),
    });
    let (tx, rx) = std::sync::mpsc::sync_channel(8);
    let (src, out) = (source.clone(), Out { tx, ctl: ctl.clone() });
    let thread = std::thread::Builder::new().name("webox-share".into()).spawn(move || {
        let ctl = out.ctl.clone();
        match backend::run(&src, opts, &out) {
            Ok(()) => { if !matches!(*ctl.status.lock().unwrap_or_else(|e| e.into_inner()), Status::Failed(_)) { ctl.set_status(Status::Ended); } }
            Err(e) => ctl.set_status(Status::Failed(e)),
        }
    }).ok();
    ShareStream { rx, ctl, thread }
}
