//! Windows share pipeline: capture (`wgc.rs`, `dupl.rs` for monitors without WGC) -> BGRA -> scaled NV12 (`convert.rs`) ->
//! Media Foundation H.264 (`encoder.rs`). The share thread paces itself to the frame rate; a picture is encoded when the
//! capture delivered a new one, on a key frame request, and at least once a second while the screen stands still (so a
//! receiver that joins late, or lost packets, gets a picture). Window resizes start a new encoder at the new size.
pub(super) mod dupl;
pub(super) mod encoder;
mod sources;
mod wgc;

use super::{convert, h264, AccessUnit, Kind, Opts, Out, ShareSource, Status};
use std::time::{Duration, Instant};

pub(super) enum Grab {
    /// A new picture in the buffer: `w` x `h`, 4 bytes per pixel, rows packed.
    Frame { w: u32, h: u32, bgra: bool },
    /// Nothing new.
    None,
    /// The source is gone.
    Closed,
}

pub(super) trait Capture {
    /// The newest picture since the last call (waits at most `wait` for one where the API can wait).
    fn grab(&mut self, buf: &mut Vec<u8>, wait: Duration) -> Result<Grab, String>;
}

/// The moving test picture.
struct Test { i: u32, w: u32, h: u32 }
impl Capture for Test {
    fn grab(&mut self, buf: &mut Vec<u8>, _: Duration) -> Result<Grab, String> {
        buf.resize((self.w * self.h * 4) as usize, 0);
        convert::test_picture(self.i, self.w, self.h, buf);
        self.i += 1;
        Ok(Grab::Frame { w: self.w, h: self.h, bgra: false })
    }
}

pub(super) fn sources() -> Vec<ShareSource> {
    let mut v = sources::monitors();
    v.extend(sources::windows());
    v
}

pub(super) fn thumbnail(s: &ShareSource, max: (u32, u32)) -> Option<(u32, u32, Vec<u8>)> { sources::thumbnail(s, max) }

/// Opens the capture for `s`; the name says which API runs (for the logs and tests).
fn open(s: &ShareSource, cursor: bool) -> Result<(Box<dyn Capture>, &'static str), String> {
    match s.kind {
        Kind::Test => Ok((Box::new(Test { i: 0, w: s.size.0.max(16), h: s.size.1.max(16) }), "test picture")),
        Kind::Screen => {
            let wgc = if wgc::supported() && std::env::var_os("WEBOX_SHARE_DUPLICATION").is_none() { wgc::Wgc::monitor(s.handle, cursor) } else { Err("Windows.Graphics.Capture is not available".into()) };
            match wgc {
                Ok(c) => Ok((Box::new(c), "Windows.Graphics.Capture")),
                Err(e) => dupl::Dupl::open(s.handle).map(|c| (Box::new(c) as Box<dyn Capture>, "DXGI Desktop Duplication")).map_err(|d| format!("{e}; {d}")),
            }
        }
        Kind::Window => {
            if !wgc::supported() { return Err("sharing a window needs Windows 10 version 1903 or later".into()); }
            sources::restore(s.handle);
            Ok((Box::new(wgc::Wgc::window(s.handle, cursor)?), "Windows.Graphics.Capture"))
        }
        Kind::Portal => Err("not a Windows source".into()),
    }
}

/// The share thread's body.
pub(super) fn run(src: &ShareSource, opts: Opts, out: &Out) -> Result<(), String> {
    use windows::Win32::Media::MediaFoundation::{MFShutdown, MFStartup, MFSTARTUP_FULL, MF_VERSION};
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        // windows-rs caches WinRT activation factories for the whole process: the MTA must outlive this thread, or the next
        // share would use a factory torn down with it
        crate::media::keep_com_loaded();
        MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| format!("Media Foundation: {}", e.message()))?;
    }
    // 1 ms timer resolution while sharing: the default 15.6 ms makes the 30 fps pacing (33 ms) and the encoder waits coarse
    unsafe { windows::Win32::Media::timeBeginPeriod(1); }
    let r = pump(src, opts, out);
    unsafe { windows::Win32::Media::timeEndPeriod(1); let _ = MFShutdown(); }
    r
}

fn pump(src: &ShareSource, opts: Opts, out: &Out) -> Result<(), String> {
    let ctl = &out.ctl;
    let (mut cap, api) = open(src, opts.cursor)?;
    let t0 = Instant::now();
    let (mut buf, mut nv12) = (Vec::new(), Vec::new());
    let mut enc: Option<encoder::Encoder> = None;
    let mut hardware_ok = true;
    let mut params = h264::ParamSets::default();
    let (mut have, mut last_out, mut next) = (false, t0, t0);
    let mut first = true;
    while !ctl.stopped() {
        let fps = ctl.fps();
        let period = Duration::from_secs_f64(1.0 / fps as f64);
        // pictures a hardware encoder finished in the meantime
        if let Some(e) = enc.as_mut() {
            let (w, h) = e.size();
            for (raw, t) in e.poll()? { if !emit(out, &mut params, &raw, t, (w, h), &mut first) { return Ok(()); } }
        }
        let now = Instant::now();
        if next > now { std::thread::sleep((next - now).min(Duration::from_millis(if enc.as_ref().is_some_and(|e| e.hardware()) { 3 } else { 100 }))); continue; }
        next = (next + period).max(now);
        let fresh = match cap.grab(&mut buf, Duration::ZERO)? {
            Grab::Closed => return Ok(()),
            Grab::None => false,
            Grab::Frame { w, h, bgra } => {
                let (dw, dh) = convert::fit(w, h, opts.max_size);
                if enc.as_ref().is_none_or(|e| e.size() != (dw, dh)) {
                    drop(enc.take()); // the old one goes first (one hardware session at a time on some GPUs)
                    let e = encoder::Encoder::new(dw, dh, fps, ctl.bitrate(), opts.keyframe_secs * fps, hardware_ok)?;
                    ctl.set_status(Status::Running { width: dw, height: dh, encoder: format!("{} + {api}", e.name) });
                    enc = Some(e);
                    first = true;
                }
                convert::to_nv12(convert::Picture { px: &buf, w, h, stride: w as usize * 4, bgra }, dw, dh, &mut nv12);
                have = true;
                true
            }
        };
        let key = ctl.take_keyframe() || first;
        if !have || !(fresh || key || last_out.elapsed() >= Duration::from_secs(1)) { continue; }
        let Some(e) = enc.as_mut() else { continue };
        if key && !first && !e.can_force_key() {
            // no key frame on request: a new encoder starts with one
            let (w, h) = e.size();
            drop(enc.take());
            enc = Some(encoder::Encoder::new(w, h, fps, ctl.bitrate(), opts.keyframe_secs * fps, hardware_ok)?);
        }
        let e = enc.as_mut().ok_or("no encoder")?;
        e.set_bitrate(ctl.bitrate());
        let pts = t0.elapsed();
        let aus = match e.encode(&nv12, (pts.as_nanos() / 100) as i64, key) {
            Ok(a) => a,
            Err(x) if e.hardware() => {
                // the GPU's encoder refused real input (e.g. it wants GPU surfaces): the software encoder takes over
                hardware_ok = false;
                let (w, h) = e.size();
                drop(enc.take());
                let mut s = encoder::Encoder::new(w, h, fps, ctl.bitrate(), opts.keyframe_secs * fps, false).map_err(|y| format!("{x}; {y}"))?;
                ctl.set_status(Status::Running { width: w, height: h, encoder: format!("{} + {api}", s.name) });
                let a = s.encode(&nv12, (pts.as_nanos() / 100) as i64, true)?;
                enc = Some(s);
                a
            }
            Err(x) => return Err(x),
        };
        let (w, h) = enc.as_ref().map_or((0, 0), |e| e.size());
        for (raw, t) in aus { if !emit(out, &mut params, &raw, t, (w, h), &mut first) { return Ok(()); } }
        last_out = Instant::now();
    }
    Ok(())
}

/// Sends one encoded picture (`t` in 100 ns units); false once nobody listens. The first key frame ends `first`.
fn emit(out: &Out, params: &mut h264::ParamSets, raw: &[u8], t: i64, (w, h): (u32, u32), first: &mut bool) -> bool {
    let data = params.fix(raw);
    let keyframe = h264::is_keyframe(&data);
    if keyframe { *first = false; }
    out.send(AccessUnit { data, pts_us: t.max(0) as u64 / 10, keyframe, width: w, height: h })
}
