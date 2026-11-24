//! macOS cameras: AVFoundation. `AVCaptureDevice` lists them; a capture session with an `AVCaptureVideoDataOutput` (32BGRA, late
//! frames dropped) hands each frame to a delegate on a dispatch queue of its own, which keeps only the newest one for `next`.
//! The camera permission is asked for the first time a camera is opened (the system prompt, NSCameraUsageDescription); when it is
//! refused the preview shows `Fail::Denied`.
#![allow(deprecated, unused_unsafe)] // devicesWithMediaType: the simple listing, still there on every macOS
use super::{CamDev, Fail, Mode, Pix, Raw, Source};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_av_foundation::{
    AVAuthorizationStatus, AVCaptureConnection, AVCaptureDevice, AVCaptureDeviceInput, AVCaptureOutput, AVCaptureSession,
    AVCaptureVideoDataOutput, AVCaptureVideoDataOutputSampleBufferDelegate, AVMediaTypeVideo,
};
use objc2_core_foundation::CFString;
use objc2_core_media::{CMSampleBuffer, CMVideoFormatDescriptionGetDimensions};
use objc2_core_video::{
    kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_32BGRA, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferGetHeight, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{NSDictionary, NSNumber, NSObject, NSObjectProtocol, NSString};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// The newest frame from the capture queue.
#[derive(Default)]
struct Slot { frame: Mutex<Option<Raw>>, cv: Condvar }

pub struct Ivars { slot: Arc<Slot> }

define_class!(
    // The sample buffer delegate: copies each picture out of its pixel buffer (the buffer goes back to the camera's pool).
    #[unsafe(super(NSObject))]
    #[name = "WeboxCameraDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl AVCaptureVideoDataOutputSampleBufferDelegate for Delegate {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        fn did_output(&self, _out: &AVCaptureOutput, sb: &CMSampleBuffer, _conn: &AVCaptureConnection) {
            let Some(raw) = (unsafe { sb.image_buffer() }).and_then(|pb| copy_bgra(&pb)) else { return };
            *self.ivars().slot.frame.lock().unwrap_or_else(|e| e.into_inner()) = Some(raw);
            self.ivars().slot.cv.notify_all();
        }
    }
);

impl Delegate {
    fn new(slot: Arc<Slot>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { slot });
        unsafe { msg_send![super(this), init] }
    }
}

fn copy_bgra(pb: &objc2_core_video::CVPixelBuffer) -> Option<Raw> {
    unsafe {
        if CVPixelBufferLockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly) != 0 { return None; }
        let (w, h, stride) = (CVPixelBufferGetWidth(pb), CVPixelBufferGetHeight(pb), CVPixelBufferGetBytesPerRow(pb));
        let base = CVPixelBufferGetBaseAddress(pb) as *const u8;
        let raw = (!base.is_null() && w > 0 && h > 0 && stride >= w * 4).then(|| Raw {
            pix: Pix::Bgra, w: w as u32, h: h as u32, stride, uv_off: 0, data: std::slice::from_raw_parts(base, stride * h).to_vec(),
        });
        CVPixelBufferUnlockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly);
        raw
    }
}

fn devices() -> Vec<Retained<AVCaptureDevice>> {
    let Some(t) = (unsafe { AVMediaTypeVideo }) else { return vec![] };
    unsafe { AVCaptureDevice::devicesWithMediaType(t) }.to_vec()
}

pub fn list() -> Vec<CamDev> {
    devices().iter().map(|d| unsafe { CamDev { id: d.uniqueID().to_string(), name: d.localizedName().to_string() } }).collect()
}

/// Asks for the camera permission when it was never asked (waits for the answer); false when refused.
fn allowed() -> bool {
    let Some(t) = (unsafe { AVMediaTypeVideo }) else { return false };
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(t) } {
        AVAuthorizationStatus::Authorized => true,
        AVAuthorizationStatus::NotDetermined => {
            let (tx, rx) = std::sync::mpsc::channel();
            let block = RcBlock::new(move |ok: Bool| { let _ = tx.send(ok.as_bool()); });
            unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(t, &block) };
            rx.recv_timeout(Duration::from_secs(120)).unwrap_or(false)
        }
        _ => false,
    }
}

/// Every (size, top rate) the camera offers; the index of each mode's format in `formats`.
fn modes_of(d: &AVCaptureDevice) -> Vec<(Mode, usize)> {
    let mut v = vec![];
    for (i, f) in unsafe { d.formats() }.iter().enumerate() {
        let dim = unsafe { CMVideoFormatDescriptionGetDimensions(&f.formatDescription()) };
        let fps = unsafe { f.videoSupportedFrameRateRanges() }.iter().map(|r| unsafe { r.maxFrameRate() }).fold(0.0f64, f64::max);
        let m = Mode { pix: Pix::Bgra, w: dim.width.max(0) as u32, h: dim.height.max(0) as u32, fps: fps as f32 };
        if !v.iter().any(|(o, _): &(Mode, usize)| o.w == m.w && o.h == m.h && o.fps == m.fps) { v.push((m, i)); }
    }
    v
}

pub struct AvfSource {
    session: Retained<AVCaptureSession>,
    _output: Retained<AVCaptureVideoDataOutput>,
    _delegate: Retained<Delegate>,
    slot: Arc<Slot>,
    mode: Mode,
    modes: Vec<Mode>,
    name: String,
}

pub fn open(dev: &CamDev, max_h: u32) -> Result<AvfSource, Fail> {
    let all = devices();
    let d = all.iter().find(|d| unsafe { d.uniqueID().to_string() } == dev.id)
        .or_else(|| all.iter().find(|d| unsafe { d.localizedName().to_string() } == dev.name))
        .ok_or_else(|| Fail::Unplugged(dev.name.clone()))?;
    if !allowed() { return Err(Fail::Denied); }
    let name = unsafe { d.localizedName().to_string() };
    let with_idx = modes_of(d);
    let modes: Vec<Mode> = with_idx.iter().map(|m| m.0).collect();
    let mode = super::pick_mode(&modes, max_h).ok_or(Fail::NoCamera)?;
    unsafe {
        let input = AVCaptureDeviceInput::deviceInputWithDevice_error(d).map_err(|e| {
            let msg = e.localizedDescription().to_string();
            if e.code() == -11852 { Fail::Denied } else { Fail::InUse(msg) } // AVErrorApplicationIsNotAuthorizedToUseDevice
        })?;
        let session = AVCaptureSession::new();
        session.beginConfiguration();
        if !session.canAddInput(&input) { return Err(Fail::InUse(name)); }
        session.addInput(&input);
        // the chosen format (the session preset would pick its own otherwise)
        if let Some((_, i)) = with_idx.iter().find(|(m, _)| *m == mode) {
            if d.lockForConfiguration().is_ok() {
                let f = d.formats();
                if *i < f.count() { d.setActiveFormat(&f.objectAtIndex(*i)); }
                d.unlockForConfiguration();
            }
        }
        let output = AVCaptureVideoDataOutput::new();
        let key = &*(kCVPixelBufferPixelFormatTypeKey as *const CFString as *const NSString);
        let fmt = NSNumber::new_u32(kCVPixelFormatType_32BGRA);
        let fmt: &AnyObject = &fmt;
        output.setVideoSettings(Some(&NSDictionary::from_slices(&[key], &[fmt])));
        output.setAlwaysDiscardsLateVideoFrames(true);
        let slot = Arc::new(Slot::default());
        let delegate = Delegate::new(slot.clone());
        let queue = dispatch2::DispatchQueue::new("webox.camera", None);
        output.setSampleBufferDelegate_queue(Some(ProtocolObject::from_ref(&*delegate)), Some(&queue));
        if !session.canAddOutput(&output) { return Err(Fail::Other("AVCaptureVideoDataOutput".into())); }
        session.addOutput(&output);
        session.commitConfiguration();
        session.startRunning();
        Ok(AvfSource { session, _output: output, _delegate: delegate, slot, mode, modes, name })
    }
}

impl Source for AvfSource {
    fn mode(&self) -> Mode { self.mode }
    fn modes(&self) -> Vec<Mode> { self.modes.clone() }
    fn name(&self) -> String { self.name.clone() }
    fn next(&mut self) -> Result<Option<Raw>, Fail> {
        let g = self.slot.frame.lock().unwrap_or_else(|e| e.into_inner());
        let (mut g, _) = self.slot.cv.wait_timeout_while(g, Duration::from_millis(100), |f| f.is_none()).unwrap_or_else(|e| e.into_inner());
        Ok(g.take())
    }
}

impl Drop for AvfSource {
    fn drop(&mut self) { unsafe { self.session.stopRunning(); } }
}
