//! The camera in a call, the sending side: capture -> plugins' frame filters (`hook::apply`, so the Video plugin's background,
//! filters and video-file source reach the far end) -> I420 at the call's size -> H.264 (OpenH264, Constrained Baseline,
//! SPS + PPS in front of every key frame) -> the sink (the call's `video/main` track, `cloud::media::Media::feed_camera`).
//!
//! Like the official client (`spark-windows-desktop-ui.dll`: `Connection::startVideoTrack`, `Connection::muteVideo`,
//! "muteVideoWithTrackOpen"; `wmeclient.dll` encodes with the `openh264.dll` it ships) the camera is one track of the call's
//! one media session; turning it off stops the capture, the line stays negotiated.
//!
//! Cost (Webox must stay lighter than the official client): one thread, which owns the device and the encoder. At most
//! 1280 x 720 at 30 fps (`caps`), 640 x 360 at 15 fps on a machine with fewer than 4 cores, and the rate halves by itself
//! when converting + encoding a picture takes longer than the frame time allows. The device is opened at the size sent (no
//! larger mode is decoded and thrown away). The self view is a small copy of what is sent (`cloud::video_rx::SELF`, at most
//! 480 x 270, every other picture), made from the encoder's input, never a second conversion of the camera picture.
use super::{hook, open, yuv, Aborter, Fail, Pattern, Pix, PreviewKey, Raw, Source, FRAME_TIMEOUT};
use crate::media::share::{convert::fit, h264::ParamSets};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What the call sends at most: picture size, rate and the encoder's bit rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caps { pub w: u32, pub h: u32, pub fps: u32, pub bitrate: u32 }

/// The caps for "Max. Kameraauflösung" `max_h` (0 = Auto) on a machine with `cores` logical cores: 720p / 30 fps, lower on
/// weak machines (fewer than 4 cores: 360p / 15 fps). Never above 720p: the server transcodes a 1:1 call anyway.
pub fn caps(max_h: u32, cores: usize) -> Caps {
    let weak = cores < 4;
    let want = if max_h == 0 { 1080 } else { max_h };
    let h = want.clamp(180, if weak { 360 } else { 720 }) & !1;
    let w = (h * 16 / 9 + 1) & !1;
    let fps = if weak { 15 } else { 30 };
    let full = match h { 720.. => 1_500_000, 540.. => 1_000_000, 360.. => 600_000, _ => 350_000 };
    Caps { w, h, fps, bitrate: if weak { full * 2 / 3 } else { full } }
}

/// The caps for this machine.
pub fn caps_here(max_h: u32) -> Caps { caps(max_h, std::thread::available_parallelism().map_or(2, |n| n.get())) }

/// Where the camera of the call is.
#[derive(Clone, Debug, PartialEq)]
pub enum SendState {
    Opening,
    /// Pictures are being sent: the device's name, the size sent and the current rate.
    Running { name: String, w: u32, h: u32, fps: u32 },
    Failed(Fail),
}

/// Gets every encoded access unit (Annex B) and its duration since the previous one.
pub type Sink = Box<dyn FnMut(Vec<u8>, Duration) + Send>;

struct Shared {
    stop: AtomicBool,
    /// Raised by the far end's picture loss indication: the next picture is a key frame.
    keyframe: Arc<AtomicBool>,
    state: Mutex<SendState>,
    sent: AtomicU64,
    done: AtomicBool,
    /// When the device last delivered (ms since start; 0 = never).
    last_ms: AtomicU64,
    abort: Mutex<Option<Aborter>>,
}

/// The running camera of a call; dropping it stops the capture and releases the device (a stuck device read is aborted
/// after a second, like the Settings preview).
pub struct CamSend { sh: Arc<Shared>, pub key: PreviewKey }

const STOP_GRACE: Duration = Duration::from_secs(1);

impl CamSend {
    #[allow(dead_code)] // the call uses `Media::feed_camera`; this is the same without a call (a local test of the camera)
    pub fn start(key: PreviewKey, keyframe: Arc<AtomicBool>, sink: Sink) -> CamSend { Self::start_with(key, keyframe, sink, None) }

    /// `test_src`: a source instead of the device (tests).
    pub fn start_with(key: PreviewKey, keyframe: Arc<AtomicBool>, mut sink: Sink, test_src: Option<Box<dyn Source + Send>>) -> CamSend {
        let sh = Arc::new(Shared { stop: AtomicBool::new(false), keyframe, state: Mutex::new(SendState::Opening), sent: AtomicU64::new(0),
            done: AtomicBool::new(false), last_ms: AtomicU64::new(0), abort: Mutex::new(None) });
        let (sh2, k2) = (sh.clone(), key.clone());
        let t0 = Instant::now();
        let _ = std::thread::Builder::new().name("webox-camera-send".into()).spawn(move || {
            let set = |s: SendState| if let Ok(mut g) = sh2.state.lock() { if !matches!(*g, SendState::Failed(_)) { *g = s; } };
            let caps = caps_here(k2.max_h);
            let src: Result<Box<dyn Source>, Fail> = if let Some(s) = test_src { Ok(s as Box<dyn Source>) }
                else if let Some(p) = &k2.src { hook::open_source(p, caps.h) }
                else if k2.sample { Ok(Box::new(Pattern { t0: Instant::now(), next_at: Instant::now() })) }
                else { match &k2.dev { Some(d) => open(d, caps.h, k2.hw), None => Err(Fail::NoCamera) } };
            match src {
                Ok(mut src) => {
                    if let Ok(mut a) = sh2.abort.lock() { *a = src.aborter(); }
                    if let Err(f) = run(&mut *src, caps, &sh2, &mut sink, t0, &set) { set(SendState::Failed(f)); }
                }
                Err(f) => set(SendState::Failed(f)),
            }
            crate::cloud::video_rx::clear_self();
            sh2.done.store(true, Ordering::SeqCst);
            crate::ui::wake();
        });
        let sh3 = sh.clone();
        let _ = std::thread::Builder::new().name("webox-camera-watchdog".into()).spawn(move || {
            let abort = |sh: &Shared| if let Some(a) = sh.abort.lock().ok().and_then(|mut a| a.take()) { (a.0)(); };
            let mut stop_at: Option<Instant> = None;
            while !sh3.done.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(100));
                if sh3.stop.load(Ordering::SeqCst) {
                    if stop_at.get_or_insert_with(Instant::now).elapsed() > STOP_GRACE { abort(&sh3); }
                    continue;
                }
                let running = matches!(sh3.state.lock().map(|s| s.clone()), Ok(SendState::Running { .. }));
                let now_ms = t0.elapsed().as_millis() as u64;
                let quiet = Duration::from_millis(now_ms.saturating_sub(sh3.last_ms.load(Ordering::Relaxed).max(1)));
                if running && quiet > FRAME_TIMEOUT {
                    if let Ok(mut s) = sh3.state.lock() { *s = SendState::Failed(Fail::NoPicture); }
                    sh3.stop.store(true, Ordering::SeqCst);
                    abort(&sh3);
                }
            }
        });
        CamSend { sh, key }
    }

    pub fn state(&self) -> SendState { self.sh.state.lock().map(|s| s.clone()).unwrap_or(SendState::Opening) }
    /// Pictures handed to the sink so far.
    pub fn sent(&self) -> u64 { self.sh.sent.load(Ordering::Relaxed) }
    /// The thread has returned (the device is released).
    pub fn finished(&self) -> bool { self.sh.done.load(Ordering::SeqCst) }
}

impl Drop for CamSend { fn drop(&mut self) { self.sh.stop.store(true, Ordering::SeqCst); } }

/// The send loop: until stopped, or the device fails (the error).
fn run(src: &mut dyn Source, caps: Caps, sh: &Shared, sink: &mut Sink, t0: Instant, set: &dyn Fn(SendState)) -> Result<(), Fail> {
    use openh264::encoder::{BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Profile, RateControlMode, UsageType};
    let mut fps = caps.fps;
    let cfg = EncoderConfig::new().bitrate(BitRate::from_bps(caps.bitrate)).max_frame_rate(FrameRate::from_hz(fps as f32))
        .usage_type(UsageType::CameraVideoRealTime).rate_control_mode(RateControlMode::Bitrate).profile(Profile::Baseline)
        .intra_frame_period(IntraFramePeriod::from_num_frames(fps * 10)).num_threads(1).skip_frames(true);
    let mut enc = Encoder::with_api_config(openh264::OpenH264API::from_source(), cfg).map_err(|e| Fail::Other(format!("H.264: {e}")))?;
    let name = src.name();
    let (mut last, mut last_pts, mut n) = (None::<Instant>, None::<Instant>, 0u64);
    let mut cost_ms = 0.0f32; // smoothed time per picture (convert + encode)
    let mut params = ParamSets::default();
    let mut size = (0u32, 0u32);
    while !sh.stop.load(Ordering::SeqCst) {
        let raw = match src.next()? { Some(r) => r, None => continue };
        sh.last_ms.store(t0.elapsed().as_millis().max(1) as u64, Ordering::Relaxed);
        let interval = 1.0 / fps as f32;
        if last.is_some_and(|t| t.elapsed().as_secs_f32() < interval - 0.004) { continue; } // drain the device, skip the picture
        last = Some(Instant::now());
        let busy = Instant::now();
        let Ok(raw) = raw.decoded() else { continue };
        n += 1;
        let raw = hook::apply(raw, n, t0.elapsed().as_micros() as u64);
        if !raw.valid() { continue; }
        let (tw, th) = fit(raw.w, raw.h, (caps.w, caps.h));
        if (tw, th) != size {
            size = (tw, th);
            crate::cloud::dbg_conn(&format!("call: camera: sending {tw}x{th} at {fps} fps ({} kbit/s)", caps.bitrate / 1000));
            set(SendState::Running { name: name.clone(), w: tw, h: th, fps });
            sh.keyframe.store(true, Ordering::SeqCst);
        }
        let i420 = to_i420(&raw, tw as usize, th as usize);
        if n % 2 == 1 {
            let (sw, shh) = fit(tw, th, (480, 270));
            crate::cloud::video_rx::SELF.publish(sw as usize, shh as usize, &i420_rgba(&i420, tw as usize, th as usize, sw as usize, shh as usize));
        }
        if sh.keyframe.swap(false, Ordering::SeqCst) { enc.force_intra_frame(); }
        let yuv = openh264::formats::YUVBuffer::from_vec(i420, tw as usize, th as usize);
        let bits = match enc.encode_at(&yuv, openh264::Timestamp::from_millis(t0.elapsed().as_millis() as u64)) {
            Ok(b) => b.to_vec(),
            Err(e) => { crate::cloud::dbg_conn(&format!("call: camera: encode failed: {e}")); continue }
        };
        // adaptive rate: a machine that can't keep up sends half the pictures instead of falling behind
        cost_ms = if cost_ms == 0.0 { busy.elapsed().as_secs_f32() * 1000.0 } else { cost_ms * 0.9 + busy.elapsed().as_secs_f32() * 100.0 };
        if fps > 15 && n > 30 && cost_ms > interval * 1000.0 * 0.85 {
            fps = (fps / 2).max(15);
            crate::cloud::dbg_conn(&format!("call: camera: {cost_ms:.0} ms per picture, lowering to {fps} fps"));
            set(SendState::Running { name: name.clone(), w: tw, h: th, fps });
        }
        if bits.is_empty() { continue; } // the rate control skipped this picture
        let au = params.fix(&bits);
        let now = Instant::now();
        let dur = last_pts.map_or(Duration::from_secs_f32(interval), |p| now.duration_since(p).clamp(Duration::from_millis(1), Duration::from_secs(1)));
        last_pts = Some(now);
        sink(au, dur);
        if sh.sent.fetch_add(1, Ordering::Relaxed) == 0 { crate::cloud::dbg_conn("call: camera: first picture sent"); }
    }
    Ok(())
}

/// The picture as I420 (BT.601 limited range) at `tw` x `th` (even), nearest-neighbour scaled. NV12 and YUY2 are copied
/// sample by sample (no colour conversion); other formats are converted.
pub fn to_i420(src: &Raw, tw: usize, th: usize) -> Vec<u8> {
    let mut out = vec![0u8; tw * th * 3 / 2];
    if !src.valid() || tw < 2 || th < 2 { return out; }
    let (w, h) = (src.w as usize, src.h as usize);
    let xs: Vec<usize> = (0..tw).map(|x| x * w / tw).collect();
    let ys: Vec<usize> = (0..th).map(|y| y * h / th).collect();
    let (yp, uv) = out.split_at_mut(tw * th);
    let (up, vp) = uv.split_at_mut(tw * th / 4);
    let (d, s) = (&src.data, src.stride);
    match src.pix {
        Pix::Nv12 => {
            for (oy, &sy) in ys.iter().enumerate() { let row = sy * s; for (ox, &sx) in xs.iter().enumerate() { yp[oy * tw + ox] = d[row + sx]; } }
            for cy in 0..th / 2 {
                let row = src.uv_off + (ys[cy * 2] / 2) * s;
                for cx in 0..tw / 2 { let i = row + (xs[cx * 2] & !1); up[cy * (tw / 2) + cx] = d[i]; vp[cy * (tw / 2) + cx] = d[i + 1]; }
            }
        }
        Pix::Yuyv => {
            for (oy, &sy) in ys.iter().enumerate() { let row = sy * s; for (ox, &sx) in xs.iter().enumerate() { yp[oy * tw + ox] = d[row + sx * 2]; } }
            for cy in 0..th / 2 {
                let row = ys[cy * 2] * s;
                for cx in 0..tw / 2 { let i = row + (xs[cx * 2] & !1) * 2; up[cy * (tw / 2) + cx] = d[i + 1]; vp[cy * (tw / 2) + cx] = d[i + 3]; }
            }
        }
        _ => {
            for (oy, &sy) in ys.iter().enumerate() {
                for (ox, &sx) in xs.iter().enumerate() {
                    let [r, g, b] = src.rgb(sx as u32, sy as u32).map(i32::from);
                    yp[oy * tw + ox] = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8;
                    if oy % 2 == 0 && ox % 2 == 0 {
                        let c = (oy / 2) * (tw / 2) + ox / 2;
                        up[c] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
                        vp[c] = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
                    }
                }
            }
        }
    }
    out
}

/// A small RGBA copy (`ow` x `oh`) of an I420 picture (`w` x `h`), nearest-neighbour: the self view.
pub fn i420_rgba(i420: &[u8], w: usize, h: usize, ow: usize, oh: usize) -> Vec<u8> {
    let mut out = vec![255u8; ow * oh * 4];
    if w < 2 || h < 2 || i420.len() < w * h * 3 / 2 { return out; }
    let (yp, uv) = i420.split_at(w * h);
    let (up, vp) = uv.split_at(w * h / 4);
    for oy in 0..oh {
        let sy = oy * h / oh;
        for ox in 0..ow {
            let sx = ox * w / ow;
            let c = (sy / 2) * (w / 2) + sx / 2;
            let px = yuv(yp[sy * w + sx], up[c], vp[c]);
            out[(oy * ow + ox) * 4..(oy * ow + ox) * 4 + 3].copy_from_slice(&px);
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use super::super::Mode;

    #[test]
    fn caps_follow_the_setting_and_the_machine() {
        assert_eq!(caps(0, 8), Caps { w: 1280, h: 720, fps: 30, bitrate: 1_500_000 }, "Auto: 720p30 on a normal machine");
        assert_eq!(caps(1080, 8).h, 720, "never above 720p");
        assert_eq!(caps(360, 8), Caps { w: 640, h: 360, fps: 30, bitrate: 600_000 });
        assert_eq!(caps(0, 2), Caps { w: 640, h: 360, fps: 15, bitrate: 400_000 }, "weak machine: 360p15");
        assert_eq!(caps(540, 16).h, 540);
        assert!(caps(100, 8).h >= 180);
    }

    /// The colour bars survive the trip into I420 from every format (red stays red, white white), and the self view of it.
    #[test]
    fn pictures_convert_to_i420() {
        let pat = super::super::test_pattern(0.0); // RGB24 1280x720, first bar white, second yellow
        let i = to_i420(&pat, 320, 180);
        assert_eq!(i.len(), 320 * 180 * 3 / 2);
        let px = |i420: &[u8], x: usize, y: usize| { let (w, h) = (320, 180); let c = (y / 2) * (w / 2) + x / 2; yuv(i420[y * w + x], i420[w * h + c], i420[w * h + w * h / 4 + c]) };
        let white = px(&i, 20, 60); // bars are 40 px wide at 320: white, yellow, cyan, …
        assert!(white.iter().all(|&c| c > 220), "{white:?}");
        let yellow = px(&i, 60, 60);
        assert!(yellow[0] > 200 && yellow[1] > 200 && yellow[2] < 60, "{yellow:?}");
        // NV12 and YUY2 straight from a camera: copied, not converted
        let (red, grey) = ([81u8, 90, 240], [126u8, 128, 128]);
        let (w, h) = (64usize, 36usize);
        let c = |x: usize| if x < w / 2 { red } else { grey };
        let mut nv: Vec<u8> = (0..h).flat_map(|_| (0..w).map(|x| c(x)[0]).collect::<Vec<_>>()).collect();
        let uv_off = nv.len();
        for _ in 0..h / 2 { for b in 0..w / 2 { nv.push(c(b * 2)[1]); nv.push(c(b * 2)[2]); } }
        let nv12 = Raw { pix: Pix::Nv12, w: w as u32, h: h as u32, stride: w, uv_off, data: nv };
        let yuyv = Raw { pix: Pix::Yuyv, w: w as u32, h: h as u32, stride: w * 2, uv_off: 0,
            data: (0..h).flat_map(|_| (0..w / 2).flat_map(|b| { let p = c(b * 2); [p[0], p[1], p[0], p[2]] }).collect::<Vec<_>>()).collect() };
        for f in [nv12, yuyv] {
            let i = to_i420(&f, 32, 18);
            assert_eq!((i[2], i[32 * 18]), (81, 90), "{:?}: red samples copied", f.pix);
            let rgba = i420_rgba(&i, 32, 18, 16, 8);
            assert!(rgba[0] > 200 && rgba[1] < 60, "{:?} self view red", f.pix);
            let r = (16 * 4 - 4) as usize;
            assert!((rgba[r] as i32 - rgba[r + 1] as i32).abs() < 12, "{:?} self view grey", f.pix);
        }
        assert_eq!(to_i420(&Raw { pix: Pix::Nv12, w: 64, h: 36, stride: 64, uv_off: 64 * 36, data: vec![0; 10] }, 32, 18).len(), 32 * 18 * 3 / 2, "short buffer: no panic");
    }

    /// A 30 fps source of moving colour bars at 160x96 (small: the tests run unoptimised on slow CI runners) (the call's pipeline without a camera).
    pub(crate) struct Bars { pub n: u32, pub next_at: Instant }
    impl Source for Bars {
        fn mode(&self) -> Mode { Mode { pix: Pix::Rgb24, w: 160, h: 96, fps: 30.0 } }
        fn modes(&self) -> Vec<Mode> { vec![self.mode()] }
        fn name(&self) -> String { "Testbalken".into() }
        fn next(&mut self) -> Result<Option<Raw>, Fail> {
            let now = Instant::now();
            if now < self.next_at { std::thread::sleep(self.next_at - now); }
            self.next_at += Duration::from_millis(33);
            self.n += 1;
            let (w, h, n) = (160u32, 96u32, self.n);
            let data = (0..h).flat_map(|_| (0..w).flat_map(move |x| { let b = ((x + n * 4) / 40) % 8; [if b & 1 > 0 { 230 } else { 20 }, if b & 2 > 0 { 230 } else { 20 }, if b & 4 > 0 { 230 } else { 20 }] })).collect();
            Ok(Some(Raw { pix: Pix::Rgb24, w, h, stride: w as usize * 3, uv_off: 0, data }))
        }
    }
    pub(crate) fn bars() -> Box<dyn Source + Send> { Box::new(Bars { n: 0, next_at: Instant::now() }) }
    pub(crate) fn key() -> PreviewKey { PreviewKey { dev: None, max_h: 720, hw: false, sample: false, src: None } }

    /// The pipeline encodes real H.264 that OpenH264 decodes back to the picture's size, starts with a key frame, makes a key
    /// frame when asked, keeps the frame rate, and releases everything when dropped.
    #[test]
    fn camera_pictures_become_decodable_h264() {
        let aus: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
        let a2 = aus.clone();
        let kf = Arc::new(AtomicBool::new(false));
        let s = CamSend::start_with(key(), kf.clone(), Box::new(move |au, _| a2.lock().unwrap().push(au)), Some(bars()));
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(10) && aus.lock().unwrap().len() < 20 { std::thread::sleep(Duration::from_millis(20)); }
        assert!(matches!(s.state(), SendState::Running { w: 160, h: 96, .. }), "{:?}", s.state());
        let first = aus.lock().unwrap().first().cloned().expect("pictures");
        assert!(crate::media::share::h264::is_keyframe(&first), "starts with a key frame");
        let pli = crate::media::share::h264::profile_level_id(&first).expect("SPS in front of the key frame");
        assert!(pli.starts_with("42") && u8::from_str_radix(&pli[2..4], 16).unwrap() & 0xc0 == 0xc0, "Constrained Baseline: {pli}");
        kf.store(true, Ordering::SeqCst);
        let before = aus.lock().unwrap().len();
        let t1 = Instant::now();
        while t1.elapsed() < Duration::from_secs(5) && !aus.lock().unwrap()[before..].iter().any(|a| crate::media::share::h264::is_keyframe(a)) { std::thread::sleep(Duration::from_millis(20)); }
        assert!(aus.lock().unwrap()[before..].iter().any(|a| crate::media::share::h264::is_keyframe(a)), "a key frame when asked");
        let mut dec = openh264::decoder::Decoder::new().unwrap();
        let mut decoded = 0;
        for au in aus.lock().unwrap().iter() {
            for nal in openh264::nal_units(au) {
                if let Ok(Some(y)) = dec.decode(nal) { use openh264::formats::YUVSource; assert_eq!(y.dimensions(), (160, 96)); decoded += 1; }
            }
        }
        assert!(decoded >= 15, "decoded {decoded}");
        let t4 = Instant::now(); // the self view is one picture for the whole app: another test's camera may just have ended
        while t4.elapsed() < Duration::from_secs(2) && crate::cloud::video_rx::self_view().is_none() { std::thread::sleep(Duration::from_millis(10)); }
        assert!(crate::cloud::video_rx::self_view().is_some_and(|p| p.image.size == [160, 96]), "self view of what is sent");
        let sent = s.sent();
        assert!(sent >= 20, "sent {sent}");
        let t2 = Instant::now();
        while t2.elapsed() < Duration::from_secs(2) && s.sent() < sent + 30 { std::thread::sleep(Duration::from_millis(10)); }
        let fps = (s.sent() - sent) as f32 / t2.elapsed().as_secs_f32();
        assert!(fps >= 15.0, "{fps:.1} fps");
        let sh = s.sh.clone();
        drop(s);
        let t3 = Instant::now();
        while t3.elapsed() < Duration::from_secs(3) && !sh.done.load(Ordering::SeqCst) { std::thread::sleep(Duration::from_millis(10)); }
        assert!(sh.done.load(Ordering::SeqCst), "stopped and released");
    }

    #[test]
    fn no_camera_fails_without_sending() {
        let s = CamSend::start_with(key(), Arc::default(), Box::new(|_, _| panic!("nothing to send")), None);
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(3) && !s.finished() { std::thread::sleep(Duration::from_millis(10)); }
        assert_eq!(s.state(), SendState::Failed(Fail::NoCamera));
    }
}
