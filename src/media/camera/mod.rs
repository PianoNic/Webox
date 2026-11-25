//! Cameras: enumeration, capture and the Settings > Video preview.
//!
//! Platform back ends: Windows Media Foundation (`mf.rs`: `MFEnumDeviceSources` + `IMFSourceReader`, the reader decodes
//! MJPG / converts to NV12), Linux V4L2 (`v4l2.rs`: raw ioctls, mmap streaming; MJPG decoded with the in-tree `image` crate,
//! whose zune-jpeg back end fills in the default Huffman tables webcams leave out), macOS AVFoundation (`avf.rs`: a capture session
//! delivering BGRA). Everything runs on worker threads;
//! the UI only uploads the finished RGBA preview frame as a texture.
//!
//! The preview applies what the page's options mean for the user's own view: mirror, rotate, zoom to fit (fill the preview
//! instead of showing the whole frame), and the video enhancements as plain image adjustments (`Look`): brightness /
//! contrast / saturation, "Belichtung anpassen" as an automatic exposure gain, "Aussehen auffrischen" as a smoothing blur
//! blended in by its level. None of this reaches a call: the call's camera (`send.rs`) sends the picture as the camera (and
//! the plugins) deliver it, and the far end decides how to show it.
//!
//! Plugins (`hook.rs`): every captured picture passes `hook::apply` right after decoding, where plugins' frame filters edit it
//! under a time budget (`camera.filter`); a plugin can also put a video file or a picture in place of the device
//! (`camera.source`, `PreviewKey::src`).
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(windows)]
mod mf;
#[cfg(target_os = "linux")]
pub(crate) mod v4l2;
#[cfg(target_os = "macos")]
mod avf; // AVFoundation capture session
pub(crate) mod v4l2_parse;
pub mod hook; // plugin frame filters and file sources (camera.filter / camera.source / media.*)
pub mod send; // the camera in a call: capture -> filters -> H.264 for the call's video/main line

/// A camera as listed in the picker. `id`: MF symbolic link / V4L2 device path. The settings store the name (stable across
/// USB ports, like the audio devices).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CamDev { pub id: String, pub name: String }

/// Pixel formats the converters understand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// `Rgba`: a frame after the plugins' frame filters (`hook`) or from a file source; no device delivers it.
pub enum Pix { Nv12, Yuyv, Mjpg, Bgra, Rgb24, Rgba }

impl Pix {
    pub fn name(self) -> &'static str { match self { Pix::Nv12 => "NV12", Pix::Yuyv => "YUY2", Pix::Mjpg => "MJPG", Pix::Bgra => "RGB32", Pix::Rgb24 => "RGB24", Pix::Rgba => "RGBA" } }
    /// Preference among modes of equal size and rate: uncompressed first (cheapest to convert).
    fn rank(self) -> u8 { match self { Pix::Nv12 => 0, Pix::Yuyv => 1, Pix::Bgra => 2, Pix::Rgba => 2, Pix::Rgb24 => 3, Pix::Mjpg => 4 } }
}

/// One capture mode of a camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mode { pub pix: Pix, pub w: u32, pub h: u32, pub fps: f32 }

/// "Max camera resolution": the largest mode not taller than `max_h` (Auto = 1080), preferring at least 15 fps (webcams often
/// offer 1080p uncompressed at only 5 fps but 30 fps as MJPG), then the larger frame, then the higher rate (up to 30),
/// then the cheaper format. Falls back to the smallest mode if every mode is taller.
pub fn pick_mode(modes: &[Mode], max_h: u32) -> Option<Mode> {
    let ok: Vec<&Mode> = modes.iter().filter(|m| m.h <= max_h && m.w > 0 && m.h > 0).collect();
    if ok.is_empty() { return modes.iter().filter(|m| m.w > 0).min_by_key(|m| m.w as u64 * m.h as u64).copied(); }
    ok.into_iter().max_by(|a, b| {
        let key = |m: &Mode| (m.fps >= 14.5, m.w as u64 * m.h as u64, (m.fps.min(30.0) * 10.0) as u32, 255 - m.pix.rank());
        key(a).cmp(&key(b))
    }).copied()
}

/// Heights offered under "Max camera resolution" (the pack only has "Auto ({{0}}P)"). Only heights the camera reaches are
/// offered.
pub const HEIGHTS: [u32; 4] = [1080, 720, 540, 360];
pub fn offered_heights(modes: &[Mode]) -> Vec<u32> {
    let top = modes.iter().map(|m| m.h).max().unwrap_or(1080);
    HEIGHTS.iter().copied().filter(|&h| h <= top).collect()
}
/// The camera's best height up to 1080 (shown as "Auto (1080P)").
pub fn auto_height(modes: &[Mode]) -> u32 { modes.iter().map(|m| m.h).filter(|&h| h <= 1080).max().unwrap_or(1080) }

/// A captured frame as delivered by the device / reader.
pub struct Raw { pub pix: Pix, pub w: u32, pub h: u32, pub stride: usize, pub uv_off: usize, pub data: Vec<u8> }

impl Raw {
    /// Decodes MJPG to RGB24 (other formats are sampled directly).
    fn decoded(self) -> Result<Raw, String> {
        if self.pix != Pix::Mjpg { return Ok(self); }
        let img = image::load_from_memory_with_format(&self.data, image::ImageFormat::Jpeg).map_err(|e| format!("MJPG: {e}"))?.to_rgb8();
        let (w, h) = img.dimensions();
        Ok(Raw { pix: Pix::Rgb24, w, h, stride: w as usize * 3, uv_off: 0, data: img.into_raw() })
    }
    /// The colour at (x, y), RGB.
    #[inline]
    fn rgb(&self, x: u32, y: u32) -> [u8; 3] {
        let (x, y) = (x.min(self.w - 1) as usize, y.min(self.h - 1) as usize);
        let d = &self.data;
        match self.pix {
            Pix::Rgb24 => { let i = y * self.stride + x * 3; [d[i], d[i + 1], d[i + 2]] }
            Pix::Bgra => { let i = y * self.stride + x * 4; [d[i + 2], d[i + 1], d[i]] }
            Pix::Rgba => { let i = y * self.stride + x * 4; [d[i], d[i + 1], d[i + 2]] }
            Pix::Yuyv => { let i = y * self.stride + (x & !1) * 2; yuv(d[i + (x & 1) * 2], d[i + 1], d[i + 3]) }
            Pix::Nv12 => { let c = self.uv_off + (y / 2) * self.stride + (x & !1); yuv(d[y * self.stride + x], d[c], d[c + 1]) }
            Pix::Mjpg => [0, 0, 0],
        }
    }
    /// Whether `data` is large enough for the declared layout (a short buffer would index out of range).
    fn valid(&self) -> bool {
        if self.w == 0 || self.h == 0 { return false; }
        let (w, h, s) = (self.w as usize, self.h as usize, self.stride);
        match self.pix {
            Pix::Rgb24 => s >= w * 3 && self.data.len() >= s * (h - 1) + w * 3,
            Pix::Bgra | Pix::Rgba => s >= w * 4 && self.data.len() >= s * (h - 1) + w * 4,
            Pix::Yuyv => s >= w * 2 && self.data.len() >= s * (h - 1) + w * 2 && w % 2 == 0,
            // the last chroma pair read is at uv_off + s*((h-1)/2) + ((w-1) & !1), +1 (an exact w*h*3/2 frame must pass)
            Pix::Nv12 => s >= w && self.uv_off >= s * h && self.data.len() >= self.uv_off + s * ((h - 1) / 2) + ((w - 1) & !1) + 2,
            Pix::Mjpg => !self.data.is_empty(),
        }
    }
}

/// BT.601 limited range (what webcams deliver) to RGB.
#[inline]
pub fn yuv(y: u8, u: u8, v: u8) -> [u8; 3] {
    let c = (y as i32 - 16).max(0) * 298;
    let (d, e) = (u as i32 - 128, v as i32 - 128);
    let cl = |x: i32| ((x + 128) >> 8).clamp(0, 255) as u8;
    [cl(c + 409 * e), cl(c - 100 * d - 208 * e), cl(c + 516 * d)]
}

/// How the preview shows the picture: the page's options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// "Meine Videovorschau spiegeln"
    pub mirror: bool,
    /// The rotate button: quarter turns clockwise (0..3).
    pub rotation: u8,
    /// "Auf optimale Darstellung zoomen": fill the preview (crop) instead of fitting the whole frame (letterbox).
    pub fill: bool,
    /// "Belichtung anpassen": automatic exposure gain toward a mid-grey average.
    pub lighting: bool,
    /// "Customize brightness, contrast, and saturation": the three sliders, 0..100 (50 = unchanged).
    pub bcs: Option<[f32; 3]>,
    /// "Aussehen auffrischen" level 0..100.
    pub touch_up: Option<f32>,
}
impl Default for Look { fn default() -> Look { Look { mirror: true, rotation: 0, fill: true, lighting: false, bcs: None, touch_up: None } } }

/// Renders `src` into an `ow` x `oh` RGBA image with the look applied.
pub fn render(src: &Raw, ow: usize, oh: usize, look: &Look) -> Vec<u8> {
    let mut out = vec![0u8; ow * oh * 4];
    for px in out.chunks_exact_mut(4) { px[3] = 255; }
    if ow == 0 || oh == 0 || !src.valid() { return out; }
    let rot = look.rotation % 4;
    let (rw, rh) = if rot % 2 == 1 { (src.h as f32, src.w as f32) } else { (src.w as f32, src.h as f32) };
    let (sx, sy) = (ow as f32 / rw, oh as f32 / rh);
    let scale = if look.fill { sx.max(sy) } else { sx.min(sy) };
    // a small box filter when shrinking a lot, so a 1080p frame doesn't shimmer in a 424 px preview
    let k = ((1.0 / scale).round() as i32).clamp(1, 3);
    let lut = lut(src, look);
    let (cx, cy) = (ow as f32 / 2.0, oh as f32 / 2.0);
    let (w1, h1) = (src.w as f32 - 1.0, src.h as f32 - 1.0);
    for oy in 0..oh {
        for ox in 0..ow {
            let mut u = (ox as f32 + 0.5 - cx) / scale + rw / 2.0;
            let v = (oy as f32 + 0.5 - cy) / scale + rh / 2.0;
            if u < 0.0 || v < 0.0 || u >= rw || v >= rh { continue; } // letterbox: stays black
            if look.mirror { u = rw - u; }
            let (fx, fy) = match rot { 0 => (u, v), 1 => (v, h1 + 1.0 - u), 2 => (w1 + 1.0 - u, h1 + 1.0 - v), _ => (w1 + 1.0 - v, u) };
            let (x0, y0) = ((fx - k as f32 / 2.0).max(0.0) as u32, (fy - k as f32 / 2.0).max(0.0) as u32);
            let mut acc = [0u32; 3];
            for dy in 0..k as u32 { for dx in 0..k as u32 { let c = src.rgb(x0 + dx, y0 + dy); for i in 0..3 { acc[i] += c[i] as u32; } } }
            let n = (k * k) as u32;
            let i = (oy * ow + ox) * 4;
            for c in 0..3 { out[i + c] = lut[(acc[c] / n) as usize]; }
        }
    }
    if let Some([_, _, s]) = look.bcs { saturate(&mut out, s / 50.0); }
    if let Some(level) = look.touch_up { smooth(&mut out, ow, oh, (level / 100.0).clamp(0.0, 1.0) * 0.75); }
    out
}

/// Per-channel tone curve: exposure gain ("Belichtung anpassen"), then brightness and contrast.
fn lut(src: &Raw, look: &Look) -> [u8; 256] {
    let gain = if look.lighting { exposure_gain(mean_luma(src)) } else { 1.0 };
    let (b, c) = look.bcs.map_or((0.0, 1.0), |[b, c, _]| ((b - 50.0) / 200.0, 1.0 + (c - 50.0) / 50.0 * 0.6));
    let mut t = [0u8; 256];
    for (i, o) in t.iter_mut().enumerate() {
        let mut v = i as f32 / 255.0;
        // soft-knee gain: brightens dark pictures without clipping the highlights
        v = v * gain / (1.0 + (gain - 1.0) * v);
        v = (v - 0.5) * c + 0.5 + b;
        *o = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    }
    t
}
/// Average luma 0..1 from a sparse grid of the frame.
pub fn mean_luma(src: &Raw) -> f32 {
    if !src.valid() { return 0.5; }
    let (mut acc, mut n) = (0.0f32, 0u32);
    for gy in 0..24 { for gx in 0..32 {
        let c = src.rgb(gx * src.w / 32, gy * src.h / 24);
        acc += (0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32) / 255.0; n += 1;
    } }
    acc / n as f32
}
/// Gain that moves the average toward 0.45 (limited, so a dark room isn't turned into noise).
pub fn exposure_gain(mean: f32) -> f32 {
    // the soft-knee curve maps m to m*g/(1+(g-1)m); solving for 0.45 gives g = 0.45(1-m) / (m(1-0.45))
    let m = mean.clamp(0.02, 0.98);
    (0.45 * (1.0 - m) / (m * 0.55)).clamp(0.6, 3.0)
}
fn saturate(px: &mut [u8], s: f32) {
    if (s - 1.0).abs() < 0.01 { return; }
    for p in px.chunks_exact_mut(4) {
        let l = 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
        for c in &mut p[..3] { *c = (l + (*c as f32 - l) * s).clamp(0.0, 255.0) as u8; }
    }
}
/// Blends a 5x5 box blur into the picture by `amount` (0..1): the "smoothing effect and softer focus" of the touch-up.
fn smooth(px: &mut [u8], w: usize, h: usize, amount: f32) {
    if amount <= 0.01 || w < 5 || h < 5 { return; }
    let r = 2usize;
    let mut tmp = vec![0u16; w * h * 3];
    // horizontal pass (running sum)
    for y in 0..h {
        for c in 0..3 {
            let row = |x: usize| px[(y * w + x.min(w - 1)) * 4 + c] as u32;
            let mut s: u32 = (0..=r).map(row).sum::<u32>() + row(0) * r as u32;
            for x in 0..w {
                tmp[(y * w + x) * 3 + c] = (s / (2 * r as u32 + 1)) as u16;
                s += row(x + r + 1);
                s -= row(x.saturating_sub(r));
            }
        }
    }
    let a = (amount * 256.0) as u32;
    for x in 0..w {
        for c in 0..3 {
            let col = |y: usize| tmp[(y.min(h - 1) * w + x) * 3 + c] as u32;
            let mut s: u32 = (0..=r).map(col).sum::<u32>() + col(0) * r as u32;
            for y in 0..h {
                let blur = s / (2 * r as u32 + 1);
                let i = (y * w + x) * 4 + c;
                px[i] = ((px[i] as u32 * (256 - a) + blur * a) >> 8) as u8;
                s += col(y + r + 1);
                s -= col(y.saturating_sub(r));
            }
        }
    }
}

/// The --sample / no-device stand-in: colour bars, a grey ramp and a square that moves, 1280x720 RGB.
pub fn test_pattern(t: f32) -> Raw {
    let (w, h) = (1280u32, 720u32);
    let bars: [[u8; 3]; 8] = [[235, 235, 235], [235, 235, 16], [16, 235, 235], [16, 235, 16], [235, 16, 235], [235, 16, 16], [16, 16, 235], [16, 16, 16]];
    let mut d = vec![0u8; (w * h * 3) as usize];
    let sq = ((t * 0.25).fract() * (w - 120) as f32) as u32;
    for y in 0..h {
        for x in 0..w {
            let c = if y < h * 2 / 3 { bars[(x * 8 / w) as usize] }
                else if y < h * 5 / 6 { let g = (x * 255 / (w - 1)) as u8; [g, g, g] }
                else { [24, 24, 40] };
            let c = if x >= sq && x < sq + 120 && y >= h * 5 / 6 + 10 && y < h - 10 { [250, 250, 250] } else { c };
            // a marker in the top-left corner, so mirror and rotation are visible
            let c = if x < 160 && y < 90 && (x / 20 + y / 20) % 2 == 0 { [255, 120, 0] } else { c };
            let i = ((y * w + x) * 3) as usize;
            d[i..i + 3].copy_from_slice(&c);
        }
    }
    Raw { pix: Pix::Rgb24, w, h, stride: (w * 3) as usize, uv_off: 0, data: d }
}

/// Why the preview has no picture (drives the error panel: warning icon, title, text).
#[derive(Clone, Debug, PartialEq)]
pub enum Fail { NoCamera, InUse(String), Denied, Unplugged(String), Other(String),
    /// Opened, but no frame arrived for `FRAME_TIMEOUT` (another app holds the camera, Windows camera privacy, a stuck driver).
    NoPicture }

#[derive(Clone, Debug, PartialEq)]
pub enum State { Opening, Running { mode: Mode, modes: Vec<Mode>, name: String }, Failed(Fail) }

/// A live device source.
pub trait Source {
    fn mode(&self) -> Mode;
    /// Every mode the device offers (the "Max camera resolution" list is built from it).
    fn modes(&self) -> Vec<Mode>;
    fn name(&self) -> String;
    /// The next frame (blocks up to about one frame time); `Ok(None)` = no frame this time.
    fn next(&mut self) -> Result<Option<Raw>, Fail>;
    /// A handle that shuts the device down from another thread, making a blocked `next` return.
    fn aborter(&self) -> Option<Aborter> { None }
}

/// The cameras on this machine. Blocks (tens of ms): call from a worker thread.
pub fn list() -> Vec<CamDev> {
    #[cfg(windows)] { mf::list() }
    #[cfg(target_os = "linux")] { v4l2::list() }
    #[cfg(target_os = "macos")] { avf::list() }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))] { vec![] }
}
fn open(dev: &CamDev, max_h: u32, hw: bool) -> Result<Box<dyn Source>, Fail> {
    #[cfg(windows)] { mf::open(dev, max_h, hw).map(|s| Box::new(s) as Box<dyn Source>) }
    #[cfg(target_os = "linux")] { let _ = hw; v4l2::open(dev, max_h).map(|s| Box::new(s) as Box<dyn Source>) }
    #[cfg(target_os = "macos")] { let _ = hw; avf::open(dev, max_h).map(|s| Box::new(s) as Box<dyn Source>) }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))] { let _ = (dev, max_h, hw); Err(Fail::NoCamera) }
}

/// The synthetic source of sample mode.
struct Pattern { t0: Instant, next_at: Instant }
impl Source for Pattern {
    fn mode(&self) -> Mode { Mode { pix: Pix::Rgb24, w: 1280, h: 720, fps: 30.0 } }
    fn modes(&self) -> Vec<Mode> { vec![self.mode()] }
    fn name(&self) -> String { "Testbild".into() }
    fn next(&mut self) -> Result<Option<Raw>, Fail> {
        let now = Instant::now();
        if now < self.next_at { std::thread::sleep(self.next_at - now); }
        self.next_at += Duration::from_millis(33);
        Ok(Some(test_pattern(self.t0.elapsed().as_secs_f32())))
    }
}

/// A finished preview frame.
pub struct Frame { pub w: usize, pub h: usize, pub rgba: Vec<u8>, pub seq: u64 }

struct Shared {
    look: Mutex<Look>, size: Mutex<(usize, usize)>, frame: Mutex<Option<Frame>>, state: Mutex<State>, seq: AtomicU64, stop: AtomicBool,
    /// When the worker last got a frame from the device (ms since the preview started; 0 = none yet).
    last_ms: AtomicU64,
    /// The worker thread has returned (the device is released).
    done: AtomicBool,
    /// The frames arriving have been near-constant black for more than `DARK_SECS`.
    dark: AtomicBool,
    /// Shuts the device down from another thread (unblocks a device read that never returns).
    abort: Mutex<Option<Aborter>>,
}

/// The running preview: one worker thread owns the device; dropping the `Preview` stops it and releases the camera.
/// A watchdog thread next to it turns "no frame for `FRAME_TIMEOUT`" into `Fail::NoPicture` and aborts the device, and
/// aborts it too when the preview is dropped but the worker is stuck in a read.
pub struct Preview { sh: Arc<Shared>, pub key: PreviewKey }

/// What the capture was opened with: a change reopens the device (look changes don't).
#[derive(Clone, Debug, PartialEq, Eq)]
/// `src`: a plugin's video file or image in place of the camera (`hook::source()`); a change reopens the preview.
pub struct PreviewKey { pub dev: Option<CamDev>, pub max_h: u32, pub hw: bool, pub sample: bool, pub src: Option<std::path::PathBuf> }

/// Preview rate: at most this many frames per second are converted and handed to the UI.
const PREVIEW_FPS: f32 = 30.0;
/// No frame from the device for this long (after opening, or while running): no picture, release the device.
pub const FRAME_TIMEOUT: Duration = Duration::from_secs(5);
/// Grace for the worker to stop by itself after the preview is dropped, before the device is aborted.
const STOP_GRACE: Duration = Duration::from_secs(1);
/// Frames darker than this (mean luma 0..255) with almost no detail count as black.
const DARK_LUMA: f32 = 10.0;
/// How long the picture must stay black before the preview says so.
const DARK_SECS: f32 = 2.0;

/// Shuts a device down from any thread.
pub struct Aborter(pub Box<dyn Fn() + Send + Sync>);

/// Whether a frame is (near-)black: dark on average and without detail (a closed cover, a camera that sends nothing).
pub fn is_black(src: &Raw) -> bool {
    if !src.valid() { return false; }
    let mut hi = 0u8;
    for gy in 0..24 { for gx in 0..32 { let c = src.rgb(gx * src.w / 32, gy * src.h / 24); hi = hi.max(c[0]).max(c[1]).max(c[2]); } }
    mean_luma(src) * 255.0 < DARK_LUMA && hi < 48
}

impl Preview {
    pub fn start(key: PreviewKey, look: Look, size: (usize, usize), wake: impl Fn() + Send + Sync + 'static) -> Preview {
        Self::start_with(key, look, size, wake, None)
    }
    /// `test_src`: a source to use instead of the device (tests).
    fn start_with(key: PreviewKey, look: Look, size: (usize, usize), wake: impl Fn() + Send + Sync + 'static, test_src: Option<Box<dyn Source + Send>>) -> Preview {
        let sh = Arc::new(Shared { look: Mutex::new(look), size: Mutex::new(size), frame: Mutex::new(None), state: Mutex::new(State::Opening),
            seq: AtomicU64::new(0), stop: AtomicBool::new(false), last_ms: AtomicU64::new(0), done: AtomicBool::new(false), dark: AtomicBool::new(false), abort: Mutex::new(None) });
        let wake = Arc::new(wake);
        let t0 = Instant::now();
        let (sh2, k2, wake2) = (sh.clone(), key.clone(), wake.clone());
        let _ = std::thread::Builder::new().name("camera-preview".into()).spawn(move || {
            let set = |s: State| { if let Ok(mut g) = sh2.state.lock() { if !matches!(*g, State::Failed(Fail::NoPicture)) { *g = s; } } };
            let finish = |f: Option<Fail>| { if let Some(f) = f { set(State::Failed(f)); } sh2.done.store(true, Ordering::Relaxed); (*wake2)(); };
            let src: Result<Box<dyn Source>, Fail> = if let Some(s) = test_src { Ok(s as Box<dyn Source>) }
                // a plugin's video file or image instead of the camera (`camera.source`): the device stays closed
                else if let Some(p) = &k2.src { hook::open_source(p, k2.max_h) }
                else if k2.sample { Ok(Box::new(Pattern { t0: Instant::now(), next_at: Instant::now() })) }
                else { match &k2.dev { Some(d) => open(d, k2.max_h, k2.hw), None => Err(Fail::NoCamera) } };
            let mut src = match src { Ok(s) => s, Err(f) => return finish(Some(f)) };
            if let Ok(mut a) = sh2.abort.lock() { *a = src.aborter(); }
            set(State::Running { mode: src.mode(), modes: src.modes(), name: src.name() });
            (*wake2)();
            let mut last = Instant::now() - Duration::from_secs(1);
            let mut dark_since: Option<Instant> = None;
            let mut n = 0u64;
            while !sh2.stop.load(Ordering::Relaxed) {
                let raw = match src.next() { Ok(Some(r)) => r, Ok(None) => continue, Err(f) => return finish(Some(f)) };
                sh2.last_ms.store(t0.elapsed().as_millis().max(1) as u64, Ordering::Relaxed);
                if last.elapsed().as_secs_f32() < 1.0 / PREVIEW_FPS - 0.004 { continue; } // keep draining the device, skip the frame
                last = Instant::now();
                let Ok(raw) = raw.decoded() else { continue };
                let black = is_black(&raw);
                let since = if black { *dark_since.get_or_insert_with(Instant::now) } else { dark_since = None; Instant::now() };
                sh2.dark.store(black && since.elapsed().as_secs_f32() > DARK_SECS, Ordering::Relaxed);
                // every camera picture passes the plugins' frame filters here (`camera.filter`), before anything uses it (the
                // black check above looks at the camera itself)
                n += 1;
                let raw = hook::apply(raw, n, t0.elapsed().as_micros() as u64);
                let look = sh2.look.lock().map(|l| *l).unwrap_or_default();
                let (w, h) = sh2.size.lock().map(|s| *s).unwrap_or((424, 240));
                let rgba = render(&raw, w, h, &look);
                let seq = sh2.seq.fetch_add(1, Ordering::Relaxed) + 1;
                if let Ok(mut f) = sh2.frame.lock() { *f = Some(Frame { w, h, rgba, seq }); }
                (*wake2)();
            }
            drop(src);
            finish(None);
        });
        // watchdog
        let sh3 = sh.clone();
        let _ = std::thread::Builder::new().name("camera-watchdog".into()).spawn(move || {
            let mut stop_at: Option<Instant> = None;
            let abort = |sh: &Shared| { if let Some(a) = sh.abort.lock().ok().and_then(|mut a| a.take()) { (a.0)(); } };
            while !sh3.done.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
                if sh3.stop.load(Ordering::Relaxed) {
                    if stop_at.get_or_insert_with(Instant::now).elapsed() > STOP_GRACE { abort(&sh3); }
                    continue;
                }
                let running = matches!(sh3.state.lock().map(|s| s.clone()), Ok(State::Running { .. }));
                let last = sh3.last_ms.load(Ordering::Relaxed);
                let quiet = Duration::from_millis(t0.elapsed().as_millis() as u64 - last.min(t0.elapsed().as_millis() as u64));
                if running && quiet > FRAME_TIMEOUT {
                    if let Ok(mut s) = sh3.state.lock() { *s = State::Failed(Fail::NoPicture); }
                    sh3.stop.store(true, Ordering::Relaxed);
                    abort(&sh3);
                    (*wake)();
                }
            }
        });
        Preview { sh, key }
    }
    pub fn set_look(&self, l: Look) { if let Ok(mut g) = self.sh.look.lock() { *g = l; } }
    pub fn set_size(&self, w: usize, h: usize) { if let Ok(mut g) = self.sh.size.lock() { *g = (w.clamp(16, 1920), h.clamp(16, 1080)); } }
    pub fn state(&self) -> State { self.sh.state.lock().map(|s| s.clone()).unwrap_or(State::Opening) }
    /// The picture has been black for more than 2 s (frames arrive, but show nothing).
    pub fn dark(&self) -> bool { self.sh.dark.load(Ordering::Relaxed) }
    /// The newest frame if it is newer than `seen`.
    pub fn take(&self, seen: u64) -> Option<Frame> {
        if self.sh.seq.load(Ordering::Relaxed) <= seen { return None; }
        self.sh.frame.lock().ok()?.take()
    }
}
impl Drop for Preview { fn drop(&mut self) { self.sh.stop.store(true, Ordering::Relaxed); } }

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pix: Pix, w: u32, h: u32, fps: f32) -> Mode { Mode { pix, w, h, fps } }

    #[test]
    fn mode_choice_follows_the_max_resolution() {
        let modes = [m(Pix::Yuyv, 1920, 1080, 5.0), m(Pix::Mjpg, 1920, 1080, 30.0), m(Pix::Yuyv, 1280, 720, 10.0), m(Pix::Mjpg, 1280, 720, 30.0),
            m(Pix::Nv12, 640, 360, 30.0), m(Pix::Yuyv, 640, 360, 30.0), m(Pix::Yuyv, 640, 480, 30.0)];
        assert_eq!(pick_mode(&modes, 1080), Some(m(Pix::Mjpg, 1920, 1080, 30.0))); // 1080p YUY2 runs at 5 fps only
        assert_eq!(pick_mode(&modes, 720), Some(m(Pix::Mjpg, 1280, 720, 30.0)));
        assert_eq!(pick_mode(&modes, 540), Some(m(Pix::Yuyv, 640, 480, 30.0)));
        assert_eq!(pick_mode(&modes, 360), Some(m(Pix::Nv12, 640, 360, 30.0))); // equal size and rate: NV12 before YUY2
        assert_eq!(pick_mode(&modes, 200), Some(m(Pix::Nv12, 640, 360, 30.0))); // nothing small enough: the smallest
        assert_eq!(pick_mode(&[], 1080), None);
        assert_eq!(offered_heights(&modes), vec![1080, 720, 540, 360]);
        assert_eq!(offered_heights(&[m(Pix::Yuyv, 1280, 720, 30.0)]), vec![720, 540, 360]);
        assert_eq!(auto_height(&modes), 1080);
    }

    #[test]
    fn yuv_conversion_hits_the_reference_colours() {
        assert_eq!(yuv(235, 128, 128), [255, 255, 255]);
        assert_eq!(yuv(16, 128, 128), [0, 0, 0]);
        let r = yuv(81, 90, 240); // BT.601 red
        assert!(r[0] > 250 && r[1] < 5 && r[2] < 5, "{r:?}");
        let g = yuv(145, 54, 34);
        assert!(g[1] > 250 && g[0] < 5 && g[2] < 5, "{g:?}");
    }

    /// 4x2 frames: left half red, right half white, in every format.
    fn frames() -> Vec<Raw> {
        let (rd, wh) = ([81u8, 90, 240], [235u8, 128, 128]);
        // YUYV: Y0 U Y1 V per 2 pixels
        let row: Vec<u8> = [rd, wh].iter().flat_map(|c| [c[0], c[1], c[0], c[2]]).collect();
        let yuyv = Raw { pix: Pix::Yuyv, w: 4, h: 2, stride: 8, uv_off: 0, data: [row.clone(), row].concat() };
        // NV12 with a padded stride of 6: Y plane, then interleaved UV for each 2x2 block
        let mut nv = vec![0u8; 6 * 2 + 6];
        for y in 0..2 { for x in 0..4 { nv[y * 6 + x] = if x < 2 { rd[0] } else { wh[0] }; } }
        nv[12..16].copy_from_slice(&[rd[1], rd[2], wh[1], wh[2]]);
        let nv12 = Raw { pix: Pix::Nv12, w: 4, h: 2, stride: 6, uv_off: 12, data: nv };
        let bgra: Vec<u8> = (0..8).flat_map(|i| if i % 4 < 2 { [0, 0, 255, 255] } else { [255, 255, 255, 255] }).collect();
        let bgra = Raw { pix: Pix::Bgra, w: 4, h: 2, stride: 16, uv_off: 0, data: bgra };
        vec![yuyv, nv12, bgra]
    }

    #[test]
    fn every_format_converts_to_the_same_picture() {
        for f in frames() {
            assert!(f.valid(), "{:?}", f.pix);
            let left = f.rgb(0, 1); let right = f.rgb(3, 0);
            assert!(left[0] > 245 && left[1] < 10 && left[2] < 10, "{:?} left {left:?}", f.pix);
            assert!(right.iter().all(|&c| c > 245), "{:?} right {right:?}", f.pix);
        }
    }

    #[test]
    fn mjpg_frames_decode() {
        let mut img = image::RgbImage::new(32, 16);
        for (x, _, p) in img.enumerate_pixels_mut() { *p = if x < 16 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 255]) }; }
        let mut jpg = vec![];
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 95).encode_image(&img).unwrap();
        let raw = Raw { pix: Pix::Mjpg, w: 32, h: 16, stride: 0, uv_off: 0, data: jpg }.decoded().unwrap();
        assert_eq!((raw.pix, raw.w, raw.h), (Pix::Rgb24, 32, 16));
        let (l, r) = (raw.rgb(4, 8), raw.rgb(28, 8));
        assert!(l[0] > 220 && l[2] < 40 && r[2] > 220 && r[0] < 40, "{l:?} {r:?}");
        assert!(Raw { pix: Pix::Mjpg, w: 32, h: 16, stride: 0, uv_off: 0, data: vec![1, 2, 3] }.decoded().is_err());
    }

    /// Regression (black preview on a real camera): frames whose buffer has exactly the size of their layout, as Media
    /// Foundation delivers NV12 (also MJPG decoded to NV12) and V4L2 YUYV, must convert to the picture, not to black.
    #[test]
    fn exact_size_frames_convert() {
        // mid-grey with a red left half, in every layout, at the camera's real sizes and the smallest ones
        // (4x2: the smallest NV12 frame with two chroma blocks, one per half)
        for (w, h) in [(1280u32, 720u32), (640, 480), (4, 2), (8, 4)] {
            let (wu, hu) = (w as usize, h as usize);
            let (red, grey) = ([81u8, 90, 240], [126u8, 128, 128]);
            let px = |x: usize| if x < wu / 2 { red } else { grey };
            // NV12: Y plane then interleaved UV per 2x2 block, exactly w*h*3/2 bytes
            let mut nv: Vec<u8> = (0..hu).flat_map(|_| (0..wu).map(|x| px(x)[0]).collect::<Vec<_>>()).collect();
            let uv_off = nv.len();
            for _ in 0..hu.div_ceil(2) { for bx in 0..wu.div_ceil(2) { let c = px(bx * 2); nv.push(c[1]); nv.push(c[2]); } }
            let nv12 = Raw { pix: Pix::Nv12, w, h, stride: wu, uv_off, data: nv };
            if w % 2 == 0 && h % 2 == 0 { assert_eq!(nv12.data.len(), wu * hu * 3 / 2); }
            assert!(nv12.valid(), "NV12 {w}x{h} exact size is valid");
            // YUYV (even widths only, by definition): Y0 U Y1 V
            let yuyv = (w % 2 == 0).then(|| Raw { pix: Pix::Yuyv, w, h, stride: wu * 2, uv_off: 0,
                data: (0..hu).flat_map(|_| (0..wu / 2).flat_map(|b| { let c = px(b * 2); [c[0], c[1], c[0], c[2]] }).collect::<Vec<_>>()).collect() });
            for f in std::iter::once(nv12).chain(yuyv) {
                let out = render(&f, wu, hu, &Look { mirror: false, fill: true, ..Look::default() });
                let (l, r) = (px_at(&out, wu, 0, hu / 2), px_at(&out, wu, wu - 1, hu / 2));
                assert!(l[0] > 200 && l[1] < 60, "{:?} {w}x{h} left is red: {l:?}", f.pix);
                assert!((r[0] as i32 - r[1] as i32).abs() < 12 && r[0] > 90 && r[0] < 160, "{:?} {w}x{h} right is grey: {r:?}", f.pix);
                assert!(!is_black(&f), "{:?} {w}x{h}", f.pix);
            }
        }
        // MJPG: decoded to RGB24 of exactly w*h*3
        let mut img = image::RgbImage::new(64, 36);
        for (x, _, p) in img.enumerate_pixels_mut() { *p = if x < 32 { image::Rgb([220, 30, 30]) } else { image::Rgb([128, 128, 128]) }; }
        let mut jpg = vec![];
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 90).encode_image(&img).unwrap();
        let rgb = Raw { pix: Pix::Mjpg, w: 64, h: 36, stride: 0, uv_off: 0, data: jpg }.decoded().unwrap();
        assert!(rgb.valid() && !is_black(&rgb));
        let out = render(&rgb, 64, 36, &Look { mirror: false, ..Look::default() });
        assert!(px_at(&out, 64, 2, 18)[0] > 180, "MJPG left is red");
    }
    fn px_at(img: &[u8], w: usize, x: usize, y: usize) -> [u8; 3] { let i = (y * w + x) * 4; [img[i], img[i + 1], img[i + 2]] }

    #[test]
    fn short_buffers_are_rejected_not_read() {
        let r = Raw { pix: Pix::Nv12, w: 640, h: 480, stride: 640, uv_off: 640 * 480, data: vec![0; 1000] };
        assert!(!r.valid());
        assert_eq!(render(&r, 8, 4, &Look::default()).len(), 8 * 4 * 4); // black, no panic
    }

    fn px(img: &[u8], w: usize, x: usize, y: usize) -> [u8; 3] { let i = (y * w + x) * 4; [img[i], img[i + 1], img[i + 2]] }

    #[test]
    fn mirror_and_rotation() {
        // 4x2: left red, right white
        let f = &frames()[2];
        let plain = Look { mirror: false, fill: true, ..Look::default() };
        let a = render(f, 4, 2, &plain);
        assert!(px(&a, 4, 0, 0)[1] < 10 && px(&a, 4, 3, 0)[1] > 245);
        let b = render(f, 4, 2, &Look { mirror: true, ..plain });
        assert!(px(&b, 4, 0, 0)[1] > 245 && px(&b, 4, 3, 0)[1] < 10, "mirrored");
        // a quarter turn clockwise: the left (red) half ends up on top
        let c = render(f, 2, 4, &Look { rotation: 1, ..plain });
        assert!(px(&c, 2, 0, 0)[1] < 10 && px(&c, 2, 1, 3)[1] > 245, "rotated");
        let d = render(f, 4, 2, &Look { rotation: 2, ..plain });
        assert!(px(&d, 4, 0, 0)[1] > 245, "half turn");
    }

    #[test]
    fn fit_letterboxes_and_fill_crops() {
        let f = test_pattern(0.0); // 16:9
        let fit = render(&f, 100, 100, &Look { fill: false, mirror: false, ..Look::default() });
        assert_eq!(px(&fit, 100, 50, 2), [0, 0, 0], "fit: black bar on top");
        let fill = render(&f, 100, 100, &Look { fill: true, mirror: false, ..Look::default() });
        assert_ne!(px(&fill, 100, 50, 2), [0, 0, 0], "fill: picture up to the edge");
    }

    #[test]
    fn adjustments_change_the_picture() {
        let grey = Raw { pix: Pix::Rgb24, w: 8, h: 8, stride: 24, uv_off: 0, data: vec![60; 8 * 8 * 3] };
        let base = render(&grey, 8, 8, &Look { mirror: false, ..Look::default() })[0];
        let bright = render(&grey, 8, 8, &Look { bcs: Some([90.0, 50.0, 50.0]), ..Look::default() })[0];
        let dark = render(&grey, 8, 8, &Look { bcs: Some([10.0, 50.0, 50.0]), ..Look::default() })[0];
        let lit = render(&grey, 8, 8, &Look { lighting: true, ..Look::default() })[0];
        assert!(bright > base && dark < base && lit > base, "{base} {bright} {dark} {lit}");
        assert_eq!(render(&grey, 8, 8, &Look { bcs: Some([50.0, 50.0, 50.0]), ..Look::default() })[0], base, "50 = unchanged");
        let red = Raw { pix: Pix::Rgb24, w: 8, h: 8, stride: 24, uv_off: 0, data: [200u8, 40, 40].repeat(64) };
        let grey_out = render(&red, 8, 8, &Look { bcs: Some([50.0, 50.0, 0.0]), ..Look::default() });
        assert!((grey_out[0] as i32 - grey_out[1] as i32).abs() <= 1, "saturation 0 = grey");
        assert!((exposure_gain(0.45) - 1.0).abs() < 0.01 && exposure_gain(0.1) > 1.5 && exposure_gain(0.9) < 1.0);
    }

    #[test]
    fn touch_up_smooths_edges() {
        let f = test_pattern(0.0);
        let a = render(&f, 128, 72, &Look { mirror: false, ..Look::default() });
        let b = render(&f, 128, 72, &Look { mirror: false, touch_up: Some(100.0), ..Look::default() });
        // at a bar boundary the smoothed picture has a smaller jump
        let jump = |img: &[u8]| (0..127).map(|x| (px(img, 128, x + 1, 20)[0] as i32 - px(img, 128, x, 20)[0] as i32).abs()).max().unwrap();
        assert!(jump(&b) < jump(&a), "{} vs {}", jump(&b), jump(&a));
    }

    /// A device that opens but never delivers a frame (its read waits until it is aborted).
    struct Stuck(Arc<AtomicBool>);
    impl Source for Stuck {
        fn mode(&self) -> Mode { m(Pix::Nv12, 640, 360, 30.0) }
        fn modes(&self) -> Vec<Mode> { vec![self.mode()] }
        fn name(&self) -> String { "stuck".into() }
        fn next(&mut self) -> Result<Option<Raw>, Fail> {
            while !self.0.load(Ordering::Relaxed) { std::thread::sleep(Duration::from_millis(10)); }
            Err(Fail::Other("aborted".into()))
        }
        fn aborter(&self) -> Option<Aborter> { let f = self.0.clone(); Some(Aborter(Box::new(move || f.store(true, Ordering::Relaxed)))) }
    }
    /// A device that delivers black NV12 frames at 30 fps.
    struct Black;
    impl Source for Black {
        fn mode(&self) -> Mode { m(Pix::Nv12, 64, 36, 30.0) }
        fn modes(&self) -> Vec<Mode> { vec![self.mode()] }
        fn name(&self) -> String { "black".into() }
        fn next(&mut self) -> Result<Option<Raw>, Fail> {
            std::thread::sleep(Duration::from_millis(33));
            let mut d = vec![16u8; 64 * 36]; d.extend(std::iter::repeat(128u8).take(64 * 18));
            Ok(Some(Raw { pix: Pix::Nv12, w: 64, h: 36, stride: 64, uv_off: 64 * 36, data: d }))
        }
    }
    fn wait_until(secs: f32, f: impl Fn() -> bool) -> bool {
        let t0 = Instant::now();
        while t0.elapsed().as_secs_f32() < secs { if f() { return true; } std::thread::sleep(Duration::from_millis(20)); }
        false
    }
    fn key() -> PreviewKey { PreviewKey { dev: None, max_h: 1080, hw: false, sample: false, src: None } }

    #[test]
    fn a_camera_without_frames_becomes_no_picture_and_is_released() {
        let aborted = Arc::new(AtomicBool::new(false));
        let p = Preview::start_with(key(), Look::default(), (32, 18), || {}, Some(Box::new(Stuck(aborted.clone()))));
        assert!(wait_until(1.0, || matches!(p.state(), State::Running { .. })));
        let t0 = Instant::now();
        assert!(wait_until(FRAME_TIMEOUT.as_secs_f32() + 2.0, || p.state() == State::Failed(Fail::NoPicture)), "state {:?}", p.state());
        assert!(t0.elapsed() >= FRAME_TIMEOUT - Duration::from_millis(300));
        assert!(wait_until(1.0, || aborted.load(Ordering::Relaxed)), "device aborted");
        assert!(wait_until(1.0, || p.sh.done.load(Ordering::Relaxed)), "worker returned");
        assert_eq!(p.state(), State::Failed(Fail::NoPicture), "the abort error doesn't replace NoPicture");
    }

    #[test]
    fn leaving_the_page_aborts_a_stuck_read() {
        let aborted = Arc::new(AtomicBool::new(false));
        let p = Preview::start_with(key(), Look::default(), (32, 18), || {}, Some(Box::new(Stuck(aborted.clone()))));
        assert!(wait_until(1.0, || matches!(p.state(), State::Running { .. })));
        let sh = p.sh.clone();
        drop(p);
        assert!(wait_until(STOP_GRACE.as_secs_f32() + 1.0, || aborted.load(Ordering::Relaxed) && sh.done.load(Ordering::Relaxed)), "released after the grace");
    }

    #[test]
    fn black_frames_are_reported_after_two_seconds() {
        let p = Preview::start_with(key(), Look::default(), (32, 18), || {}, Some(Box::new(Black)));
        assert!(wait_until(1.0, || p.take(0).is_some()), "black frames still reach the texture");
        assert!(!p.dark(), "not yet");
        assert!(wait_until(DARK_SECS + 1.5, || p.dark()), "dark after 2 s");
        assert!(matches!(p.state(), State::Running { .. }), "a black picture is not an error");
    }

    #[test]
    fn black_detection() {
        let black = Raw { pix: Pix::Yuyv, w: 4, h: 2, stride: 8, uv_off: 0, data: [16u8, 128].repeat(8) };
        assert!(is_black(&black));
        assert!(!is_black(&test_pattern(0.0)));
        for f in frames() { assert!(!is_black(&f), "{:?}", f.pix); }
        // a dim but real picture (a dark room) is not black
        let dim: Vec<u8> = (0..64 * 36).map(|i| if i % 64 < 32 { 20 } else { 90 }).chain(std::iter::repeat(128).take(64 * 18)).collect();
        assert!(!is_black(&Raw { pix: Pix::Nv12, w: 64, h: 36, stride: 64, uv_off: 64 * 36, data: dim }));
    }

    #[test]
    fn sample_preview_delivers_frames_and_stops() {
        let key = PreviewKey { dev: None, max_h: 1080, hw: false, sample: true, src: None };
        let p = Preview::start(key, Look::default(), (64, 36), || {});
        let t0 = Instant::now();
        let mut got = None;
        while t0.elapsed() < Duration::from_secs(5) { if let Some(f) = p.take(0) { got = Some(f); break; } std::thread::sleep(Duration::from_millis(5)); }
        let f = got.expect("a frame");
        assert_eq!((f.w, f.h, f.rgba.len()), (64, 36, 64 * 36 * 4));
        assert!(matches!(p.state(), State::Running { .. }));
        drop(p);
        let none = Preview::start(PreviewKey { dev: None, max_h: 1080, hw: false, sample: false, src: None }, Look::default(), (8, 8), || {});
        let t0 = Instant::now();
        while none.state() == State::Opening && t0.elapsed() < Duration::from_secs(2) { std::thread::sleep(Duration::from_millis(5)); }
        assert_eq!(none.state(), State::Failed(Fail::NoCamera));
    }
}
