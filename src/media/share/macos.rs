//! macOS share pipeline: ScreenCaptureKit -> BGRA on the CPU -> `convert::to_nv12` (scaling) -> VideoToolbox H.264.
//! `SCShareableContent` lists the displays and the on-screen windows (Webox's own left out); an `SCStream` delivers 32BGRA
//! pictures to a delegate on a dispatch queue, which keeps the newest one for the pump. `VTCompressionSession` encodes
//! Constrained Baseline in real time without B-frames; its length-prefixed (AVCC) output is turned into Annex B with SPS + PPS in
//! front of every key frame. Capturing needs the Screen Recording permission (System Settings > Privacy & Security), which macOS
//! only grants to an app started after the user allowed it: `unavailable` says so.
#![allow(deprecated, unused_unsafe)] // the CM free functions
use super::{convert, h264, AccessUnit, Kind, Opts, Out, ShareSource, Status};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_core_foundation::{CFDictionary, CFRetained, CFString, CFType};
use objc2_core_media::{kCMTimeInvalid, kCMVideoCodecType_H264, CMSampleBuffer, CMTime, CMVideoFormatDescriptionGetH264ParameterSetAtIndex};
use objc2_core_video::{
    kCVPixelFormatType_32BGRA, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddress,
    CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRow, CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferGetHeight,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSNumber, NSObject, NSObjectProtocol, NSString};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamDelegate, SCStreamOutput, SCStreamOutputType, SCWindow,
};
use objc2_video_toolbox::{
    kVTCompressionPropertyKey_AllowFrameReordering, kVTCompressionPropertyKey_AverageBitRate, kVTCompressionPropertyKey_ExpectedFrameRate,
    kVTCompressionPropertyKey_MaxKeyFrameInterval, kVTCompressionPropertyKey_ProfileLevel, kVTCompressionPropertyKey_RealTime,
    kVTEncodeFrameOptionKey_ForceKeyFrame, kVTProfileLevel_H264_Baseline_AutoLevel, kVTProfileLevel_H264_ConstrainedBaseline_AutoLevel,
    VTCompressionSession, VTEncodeInfoFlags, VTSessionSetProperty,
};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGDisplayPixelsWide(display: u32) -> usize;
    fn CGDisplayPixelsHigh(display: u32) -> usize;
    fn CGMainDisplayID() -> u32;
}

/// Screen Recording is allowed for this app (no prompt).
fn permitted() -> bool { unsafe { CGPreflightScreenCaptureAccess() } }

/// Why nothing can be shared: the Screen Recording permission is missing. The system prompt is asked for once per run; macOS
/// applies a new permission only after a restart of the app.
pub(super) fn unavailable() -> Option<String> {
    if permitted() { return None; }
    static ASKED: std::sync::Once = std::sync::Once::new();
    ASKED.call_once(|| { unsafe { CGRequestScreenCaptureAccess(); } });
    Some("Bildschirmfreigabe nicht erlaubt: Erlauben Sie Webox unter Systemeinstellungen > Datenschutz & Sicherheit > Bildschirm- & Systemaudioaufnahme und starten Sie Webox danach neu.".into())
}

/// A Retained moved from a completion handler's thread to the waiting one.
struct Handoff<T>(T);
unsafe impl<T> Send for Handoff<T> {}

/// The displays and on-screen windows (blocking, at most 5 s).
fn content() -> Option<Retained<SCShareableContent>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let block = RcBlock::new(move |c: *mut SCShareableContent, _e: *mut NSError| {
        let _ = tx.send(Handoff(unsafe { Retained::retain(c) }));
    });
    unsafe { SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(true, true, &block) };
    rx.recv_timeout(Duration::from_secs(5)).ok().and_then(|h| h.0)
}

fn own_pid() -> i32 { std::process::id() as i32 }

fn is_own(w: &SCWindow) -> bool { unsafe { w.owningApplication() }.is_some_and(|a| unsafe { a.processID() } == own_pid()) }

/// Pixels per point of the main display (2 on Retina).
fn main_scale(c: &SCShareableContent) -> f64 {
    let main = unsafe { CGMainDisplayID() };
    unsafe { c.displays() }.iter().find(|d| unsafe { d.displayID() } == main)
        .map_or(1.0, |d| { let pts = unsafe { d.width() } as f64; if pts > 0.0 { (unsafe { CGDisplayPixelsWide(main) }) as f64 / pts } else { 1.0 } })
        .max(1.0)
}

pub(super) fn sources() -> Vec<ShareSource> {
    if !permitted() { return vec![]; }
    let Some(c) = content() else { return vec![] };
    let main = unsafe { CGMainDisplayID() };
    let mut v = vec![];
    for (i, d) in unsafe { c.displays() }.iter().enumerate() {
        let id = unsafe { d.displayID() };
        let size = unsafe { (CGDisplayPixelsWide(id) as u32, CGDisplayPixelsHigh(id) as u32) };
        v.push(ShareSource { kind: Kind::Screen, handle: id as isize, index: i as u32, name: format!("Bildschirm {}", i + 1), app: String::new(), size, primary: id == main, minimized: false });
    }
    v.sort_by_key(|s| !s.primary);
    let scale = main_scale(&c);
    for w in unsafe { c.windows() }.iter() {
        let (title, layer, frame) = unsafe { (w.title().map(|t| t.to_string()).unwrap_or_default(), w.windowLayer(), w.frame()) };
        if layer != 0 || title.is_empty() || is_own(&w) || frame.size.width < 50.0 || frame.size.height < 50.0 { continue; }
        let app = unsafe { w.owningApplication() }.map(|a| unsafe { a.applicationName() }.to_string()).unwrap_or_default();
        let size = ((frame.size.width * scale) as u32, (frame.size.height * scale) as u32);
        v.push(ShareSource { kind: Kind::Window, handle: unsafe { w.windowID() } as isize, index: 0, name: title, app, size, primary: false, minimized: false });
    }
    v
}

pub(super) fn thumbnail(s: &ShareSource, max: (u32, u32)) -> Option<(u32, u32, Vec<u8>)> {
    if !permitted() || !matches!(s.kind, Kind::Screen | Kind::Window) { return None; }
    let (w, h) = convert::fit(s.size.0, s.size.1, max);
    let cap = Capture::start(s, (w, h), 5, false).ok()?;
    let t0 = Instant::now();
    let mut buf = vec![];
    while t0.elapsed() < Duration::from_secs(3) {
        if let Grab::Frame { w, h } = cap.grab(&mut buf, Duration::from_millis(100)) {
            return Some(convert::thumb(convert::Picture { px: &buf, w, h, stride: w as usize * 4, bgra: true }, max));
        }
    }
    None
}

// Capture.

#[derive(Default)]
struct Latest { frame: Mutex<Option<(u32, u32, Vec<u8>)>>, cv: Condvar, stopped: AtomicBool }

pub struct Ivars { latest: Arc<Latest> }

define_class!(
    // The stream's output and delegate: keeps the newest picture, notes when the stream stops (window closed, display gone).
    #[unsafe(super(NSObject))]
    #[name = "WeboxShareOutput"]
    #[ivars = Ivars]
    struct StreamSink;

    unsafe impl NSObjectProtocol for StreamSink {}

    unsafe impl SCStreamOutput for StreamSink {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn did_output(&self, _s: &SCStream, sb: &CMSampleBuffer, ty: SCStreamOutputType) {
            if ty != SCStreamOutputType::Screen { return; }
            // idle frames (nothing changed) carry no picture
            let Some(px) = (unsafe { sb.image_buffer() }).and_then(|pb| copy_bgra(&pb)) else { return };
            *self.ivars().latest.frame.lock().unwrap_or_else(|e| e.into_inner()) = Some(px);
            self.ivars().latest.cv.notify_all();
        }
    }

    unsafe impl SCStreamDelegate for StreamSink {
        #[unsafe(method(stream:didStopWithError:))]
        fn did_stop(&self, _s: &SCStream, _e: &NSError) {
            self.ivars().latest.stopped.store(true, Ordering::SeqCst);
            self.ivars().latest.cv.notify_all();
        }
    }
);

impl StreamSink {
    fn new(latest: Arc<Latest>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { latest });
        unsafe { msg_send![super(this), init] }
    }
}

/// A 32BGRA pixel buffer as tightly packed rows.
fn copy_bgra(pb: &CVPixelBuffer) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        if CVPixelBufferLockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly) != 0 { return None; }
        let (w, h, stride) = (CVPixelBufferGetWidth(pb), CVPixelBufferGetHeight(pb), CVPixelBufferGetBytesPerRow(pb));
        let base = CVPixelBufferGetBaseAddress(pb) as *const u8;
        let out = (!base.is_null() && w > 0 && h > 0 && stride >= w * 4).then(|| {
            let src = std::slice::from_raw_parts(base, stride * (h - 1) + w * 4);
            let mut px = Vec::with_capacity(w * h * 4);
            for y in 0..h { px.extend_from_slice(&src[y * stride..y * stride + w * 4]); }
            (w as u32, h as u32, px)
        });
        CVPixelBufferUnlockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly);
        out
    }
}

enum Grab { Frame { w: u32, h: u32 }, None, Closed }

struct Capture { stream: Retained<SCStream>, _sink: Retained<StreamSink>, latest: Arc<Latest> }

impl Capture {
    /// Starts an SCStream of the screen or window `s`, delivering at most `fps` pictures of `size` pixels.
    fn start(s: &ShareSource, size: (u32, u32), fps: u32, cursor: bool) -> Result<Capture, String> {
        let c = content().ok_or("ScreenCaptureKit: keine Bildschirminhalte (Bildschirmaufnahme nicht erlaubt?)")?;
        unsafe {
            let filter = match s.kind {
                Kind::Screen => {
                    let d: Retained<SCDisplay> = c.displays().iter().find(|d| d.displayID() as isize == s.handle).ok_or("Der Bildschirm ist nicht mehr da")?;
                    let own: Vec<Retained<SCWindow>> = c.windows().iter().filter(|w| is_own(w)).collect();
                    SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), &d, &NSArray::from_retained_slice(&own))
                }
                _ => {
                    let w: Retained<SCWindow> = c.windows().iter().find(|w| w.windowID() as isize == s.handle).ok_or("Das Fenster ist nicht mehr da")?;
                    SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &w)
                }
            };
            let cfg = SCStreamConfiguration::new();
            cfg.setWidth(size.0.max(2) as usize);
            cfg.setHeight(size.1.max(2) as usize);
            cfg.setMinimumFrameInterval(CMTime::new(1, fps.max(1) as i32));
            cfg.setPixelFormat(kCVPixelFormatType_32BGRA);
            cfg.setShowsCursor(cursor);
            cfg.setQueueDepth(3);
            let latest = Arc::new(Latest::default());
            let sink = StreamSink::new(latest.clone());
            let stream = SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &cfg, Some(ProtocolObject::from_ref(&*sink)));
            let queue = dispatch2::DispatchQueue::new("webox.share", None);
            stream.addStreamOutput_type_sampleHandlerQueue_error(ProtocolObject::from_ref(&*sink), SCStreamOutputType::Screen, Some(&queue))
                .map_err(|e| e.localizedDescription().to_string())?;
            let (tx, rx) = std::sync::mpsc::channel();
            let done = RcBlock::new(move |e: *mut NSError| { let _ = tx.send(e.as_ref().map(|e| e.localizedDescription().to_string())); });
            stream.startCaptureWithCompletionHandler(Some(&done));
            match rx.recv_timeout(Duration::from_secs(10)) {
                Ok(None) => Ok(Capture { stream, _sink: sink, latest }),
                Ok(Some(e)) => Err(format!("ScreenCaptureKit: {e}")),
                Err(_) => Err("ScreenCaptureKit: keine Antwort".into()),
            }
        }
    }

    /// The newest picture into `buf` (BGRA, `w * 4` per row), waiting at most `wait`.
    fn grab(&self, buf: &mut Vec<u8>, wait: Duration) -> Grab {
        if self.latest.stopped.load(Ordering::SeqCst) { return Grab::Closed; }
        let g = self.latest.frame.lock().unwrap_or_else(|e| e.into_inner());
        let (mut g, _) = self.latest.cv.wait_timeout_while(g, wait, |f| f.is_none() && !self.latest.stopped.load(Ordering::SeqCst)).unwrap_or_else(|e| e.into_inner());
        match g.take() { Some((w, h, px)) => { *buf = px; Grab::Frame { w, h } } None => Grab::None }
    }
}

impl Drop for Capture {
    fn drop(&mut self) { unsafe { self.stream.stopCaptureWithCompletionHandler(None); } }
}

// Encoder.

/// Finished pictures from the encoder callback: (Annex B access unit, pts in µs).
type Done = Mutex<Vec<(Vec<u8>, i64)>>;

struct Encoder { session: CFRetained<VTCompressionSession>, done: Box<Done>, w: u32, h: u32, bitrate: u32, nv12: Option<CFRetained<CVPixelBuffer>> }

/// A Foundation object as the CoreFoundation value it bridges to (NSNumber -> CFNumber / CFBoolean, NSDictionary -> CFDictionary).
fn cf<T>(o: &T) -> &CFType { unsafe { &*(o as *const T as *const CFType) } }

unsafe extern "C-unwind" fn on_encoded(refcon: *mut c_void, _frame: *mut c_void, status: i32, _flags: VTEncodeInfoFlags, sb: *mut CMSampleBuffer) {
    if status != 0 || sb.is_null() || refcon.is_null() { return; }
    let (done, sb) = (&*(refcon as *const Done), &*sb);
    let Some(au) = annex_b(sb) else { return };
    let pts = sb.presentation_time_stamp();
    let us = if pts.timescale > 0 { pts.value * 1_000_000 / pts.timescale as i64 } else { 0 };
    done.lock().unwrap_or_else(|e| e.into_inner()).push((au, us));
}

/// The sample's AVCC NAL units (4-byte big-endian lengths) as Annex B; SPS + PPS from the format description in front of a key frame.
fn annex_b(sb: &CMSampleBuffer) -> Option<Vec<u8>> {
    unsafe {
        let bb = sb.data_buffer()?;
        let len = bb.data_length();
        let mut data = vec![0u8; len];
        if bb.copy_data_bytes(0, len, NonNull::new(data.as_mut_ptr() as *mut c_void)?) != 0 { return None; }
        let mut nals: Vec<&[u8]> = vec![];
        let mut i = 0;
        while i + 4 <= data.len() {
            let n = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
            if n == 0 || i + 4 + n > data.len() { break; }
            nals.push(&data[i + 4..i + 4 + n]);
            i += 4 + n;
        }
        let mut params: Vec<Vec<u8>> = vec![];
        if nals.iter().any(|n| h264::nal_type(n) == h264::IDR) {
            if let Some(fmt) = sb.format_description() {
                for idx in 0..2 {
                    let (mut p, mut size, mut count, mut hl) = (std::ptr::null::<u8>(), 0usize, 0usize, 0i32);
                    if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(&fmt, idx, &mut p, &mut size, &mut count, &mut hl) == 0 && !p.is_null() {
                        params.push(std::slice::from_raw_parts(p, size).to_vec());
                    }
                }
            }
        }
        Some(h264::annex_b(params.iter().map(|p| p.as_slice()).chain(nals)))
    }
}

impl Encoder {
    fn new(w: u32, h: u32, fps: u32, bitrate: u32, key_interval: u32) -> Result<Encoder, String> {
        let done: Box<Done> = Box::default();
        let mut s: *mut VTCompressionSession = std::ptr::null_mut();
        let st = unsafe {
            VTCompressionSession::create(None, w as i32, h as i32, kCMVideoCodecType_H264, None, None, None, Some(on_encoded),
                &*done as *const Done as *mut c_void, NonNull::from(&mut s))
        };
        let session = NonNull::new(s).filter(|_| st == 0).map(|p| unsafe { CFRetained::from_raw(p) })
            .ok_or_else(|| format!("no H.264 encoder (VideoToolbox: {st})"))?;
        unsafe {
            let set = |k: &CFString, v: &CFType| VTSessionSetProperty(&session, k, Some(v));
            let yes = NSNumber::new_bool(true);
            let no = NSNumber::new_bool(false);
            set(kVTCompressionPropertyKey_RealTime, cf(&*yes));
            if set(kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_ConstrainedBaseline_AutoLevel) != 0 {
                set(kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_Baseline_AutoLevel);
            }
            set(kVTCompressionPropertyKey_AllowFrameReordering, cf(&*no));
            set(kVTCompressionPropertyKey_ExpectedFrameRate, cf(&*NSNumber::new_u32(fps)));
            // 0 = key frames only on request
            set(kVTCompressionPropertyKey_MaxKeyFrameInterval, cf(&*NSNumber::new_u32(if key_interval == 0 { 1_000_000 } else { key_interval })));
        }
        let mut e = Encoder { session, done, w, h, bitrate: 0, nv12: None };
        e.set_bitrate(bitrate);
        Ok(e)
    }

    fn set_bitrate(&mut self, bps: u32) {
        if bps == self.bitrate { return; }
        self.bitrate = bps;
        unsafe { VTSessionSetProperty(&self.session, kVTCompressionPropertyKey_AverageBitRate, Some(cf(&*NSNumber::new_u32(bps)))); }
    }

    /// Encodes one NV12 picture of the session's size; returns what the encoder finished (normally that picture).
    fn encode(&mut self, nv12: &[u8], pts_us: i64, key: bool) -> Result<Vec<(Vec<u8>, i64)>, String> {
        let (w, h) = (self.w as usize, self.h as usize);
        if self.nv12.is_none() {
            let mut pb: *mut CVPixelBuffer = std::ptr::null_mut();
            let st = unsafe { CVPixelBufferCreate(None, w, h, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, None, NonNull::from(&mut pb)) };
            self.nv12 = Some(NonNull::new(pb).filter(|_| st == 0).map(|p| unsafe { CFRetained::from_raw(p) }).ok_or_else(|| format!("pixel buffer ({st})"))?);
        }
        let pb = self.nv12.as_ref().ok_or("pixel buffer")?;
        unsafe {
            if CVPixelBufferLockBaseAddress(pb, CVPixelBufferLockFlags(0)) != 0 { return Err("pixel buffer lock".into()); }
            for (plane, rows, off) in [(0usize, h, 0usize), (1, h / 2, w * h)] {
                let (base, stride) = (CVPixelBufferGetBaseAddressOfPlane(pb, plane) as *mut u8, CVPixelBufferGetBytesPerRowOfPlane(pb, plane));
                if base.is_null() || stride < w { continue; }
                for y in 0..rows {
                    let src = &nv12[off + y * w..off + y * w + w];
                    std::ptr::copy_nonoverlapping(src.as_ptr(), base.add(y * stride), w);
                }
            }
            CVPixelBufferUnlockBaseAddress(pb, CVPixelBufferLockFlags(0));
            let yes = NSNumber::new_bool(true);
            let yes: &AnyObject = &yes;
            let force = NSDictionary::<NSString, AnyObject>::from_slices(&[&*(kVTEncodeFrameOptionKey_ForceKeyFrame as *const CFString as *const NSString)], &[yes]);
            let props = key.then(|| &*(&*force as *const NSDictionary<NSString, AnyObject> as *const CFDictionary));
            let t = CMTime::new(pts_us, 1_000_000);
            let st = self.session.encode_frame(pb, t, kCMTimeInvalid, props, std::ptr::null_mut(), std::ptr::null_mut());
            if st != 0 { return Err(format!("VideoToolbox encode failed ({st})")); }
            self.session.complete_frames(t);
        }
        Ok(std::mem::take(&mut *self.done.lock().unwrap_or_else(|e| e.into_inner())))
    }
}

impl Drop for Encoder {
    // the callback's refcon (`done`) must outlive every call: invalidate waits for pending frames
    fn drop(&mut self) { unsafe { self.session.complete_frames(kCMTimeInvalid); self.session.invalidate(); } }
}

// The pump.

enum Src { Test { i: u32 }, Live(Capture) }

pub(super) fn run(src: &ShareSource, opts: Opts, out: &Out) -> Result<(), String> {
    let ctl = &out.ctl;
    let mut cap = match src.kind {
        Kind::Test => Src::Test { i: 0 },
        Kind::Screen | Kind::Window => {
            if !permitted() { return Err(unavailable().unwrap_or_default()); }
            let (w, h) = convert::fit(src.size.0, src.size.1, opts.max_size);
            Src::Live(Capture::start(src, (w, h), ctl.fps(), opts.cursor)?)
        }
        Kind::Portal => return Err("not available on macOS".into()),
    };
    let api = if matches!(cap, Src::Test { .. }) { "test picture" } else { "ScreenCaptureKit" };
    let t0 = Instant::now();
    let (mut buf, mut nv12) = (Vec::new(), Vec::new());
    let mut enc: Option<Encoder> = None;
    let mut params = h264::ParamSets::default();
    let (mut have, mut last_out, mut next) = (false, t0, t0);
    let mut first = true;
    while !ctl.stopped() {
        let fps = ctl.fps();
        let period = Duration::from_secs_f64(1.0 / fps as f64);
        let now = Instant::now();
        if next > now { std::thread::sleep((next - now).min(Duration::from_millis(5))); continue; }
        next = (next + period).max(now);
        let frame = match &mut cap {
            Src::Test { i } => {
                let (w, h) = src.size;
                buf.resize((w * h * 4) as usize, 0);
                convert::test_picture(*i, w, h, &mut buf);
                *i += 1;
                Some((w, h, false))
            }
            Src::Live(c) => match c.grab(&mut buf, Duration::ZERO) { Grab::Closed => return Ok(()), Grab::None => None, Grab::Frame { w, h } => Some((w, h, true)) },
        };
        let fresh = frame.is_some();
        if let Some((w, h, bgra)) = frame {
            let (dw, dh) = convert::fit(w, h, opts.max_size);
            if enc.as_ref().is_none_or(|e| (e.w, e.h) != (dw, dh)) {
                drop(enc.take());
                enc = Some(Encoder::new(dw, dh, fps, ctl.bitrate(), opts.keyframe_secs * fps)?);
                ctl.set_status(Status::Running { width: dw, height: dh, encoder: format!("VideoToolbox + {api}") });
                first = true;
            }
            convert::to_nv12(convert::Picture { px: &buf, w, h, stride: w as usize * 4, bgra }, dw, dh, &mut nv12);
            have = true;
        }
        let key = ctl.take_keyframe() || first;
        if !have || !(fresh || key || last_out.elapsed() >= Duration::from_secs(1)) { continue; }
        let Some(e) = enc.as_mut() else { continue };
        e.set_bitrate(ctl.bitrate());
        let (w, h) = (e.w, e.h);
        for (raw, us) in e.encode(&nv12, t0.elapsed().as_micros() as i64, key)? {
            let data = params.fix(&raw);
            let keyframe = h264::is_keyframe(&data);
            if keyframe { first = false; }
            if !out.send(AccessUnit { data, pts_us: us.max(0) as u64, keyframe, width: w, height: h }) { return Ok(()); }
        }
        last_out = Instant::now();
    }
    Ok(())
}
