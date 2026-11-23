//! Offline tests of the share pipeline: the H.264 helpers, the picture conversion, and round trips through the real
//! encoder and the playback decoder (`media::video`): the built-in test picture, a window the test opens itself, and the
//! primary monitor. Encoded streams go to a temp folder (MPEG-TS, see `ts.rs`) that is deleted afterwards; nothing touches
//! the network.
//!
//! The tests that capture or encode run one at a time, also against other test runs on the same machine (`exclusive`):
//! desktop duplication slots, the GPU encoder and the timer resolution are shared. Tests that need the live desktop skip
//! where there is none to capture (`no_desktop`: CI runners, a locked or disconnected session), and every encoder test
//! skips where the system has no H.264 encoder or decoder; the pure tests always run.
use super::*;
use std::path::PathBuf;
use std::time::Instant;

fn temp_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("webox-share-test-{}", crate::jose::uuid()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Returns from the test with the reason when there is one.
macro_rules! skip_if {
    ($reason:expr) => { if let Some(r) = $reason { eprintln!("SKIPPED: {r}"); return; } };
}

/// A CI runner (GitHub Actions and the like): no interactive desktop to capture, often no GPU and no media codecs.
fn ci() -> bool { std::env::var_os("CI").is_some() || std::env::var_os("GITHUB_ACTIONS").is_some() }

/// Why the live desktop cannot be captured right now, None when it can.
fn no_desktop() -> Option<String> {
    if ci() { return Some("CI runner without an interactive desktop".into()); }
    #[cfg(windows)]
    unsafe {
        use ::windows::Win32::System::StationsAndDesktops::{CloseDesktop, OpenInputDesktop, SwitchDesktop, DESKTOP_CONTROL_FLAGS, DESKTOP_SWITCHDESKTOP};
        // the input desktop is ours and switchable only while the session is unlocked and attached to a console
        let ok = match OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_SWITCHDESKTOP) {
            Ok(h) => { let ok = SwitchDesktop(h).is_ok(); let _ = CloseDesktop(h); ok }
            Err(_) => false,
        };
        if !ok { return Some("the desktop is locked or not interactive".into()); }
    }
    None
}

/// A missing piece of the system (no encoder / decoder / plugin) rather than a bug.
fn missing(e: &str) -> bool {
    ["no H.264 encoder", "is missing", "no decoder", "not installed", "not registered"].iter().any(|m| e.contains(m))
}

/// Holds the capture / encoder tests to one at a time: in this process (a mutex) and across processes (a named mutex).
struct Exclusive { _local: std::sync::MutexGuard<'static, ()>, #[cfg(windows)] named: Option<::windows::Win32::Foundation::HANDLE> }

fn exclusive() -> Exclusive {
    static LOCAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let local = LOCAL.lock().unwrap_or_else(|e| e.into_inner());
    #[cfg(windows)]
    let named = unsafe {
        use ::windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
        CreateMutexW(None, false, ::windows::core::w!("Local\\webox-share-tests")).ok().inspect(|&h| { WaitForSingleObject(h, 120_000); })
    };
    Exclusive { _local: local, #[cfg(windows)] named }
}

#[cfg(windows)]
impl Drop for Exclusive {
    fn drop(&mut self) {
        if let Some(h) = self.named.take() { unsafe { let _ = ::windows::Win32::System::Threading::ReleaseMutex(h); let _ = ::windows::Win32::Foundation::CloseHandle(h); } }
    }
}

/// Collects access units for `secs` once the share runs (calling `idle` in between, e.g. to pump the test window's
/// messages). Opening a GPU encoder can take a second, so the clock starts with the first picture (waiting at most 15 s).
/// None: skipped (the reason is printed) because the system lacks an encoder, or a CI runner cannot run the share.
fn collect(s: &ShareStream, secs: f64, mut idle: impl FnMut()) -> Option<Vec<AccessUnit>> {
    let t0 = Instant::now();
    while s.status() == Status::Starting && t0.elapsed() < Duration::from_secs(15) { idle(); std::thread::sleep(Duration::from_millis(10)); }
    let (t0, mut v) = (Instant::now(), vec![]);
    while t0.elapsed().as_secs_f64() < secs {
        if let Some(au) = s.recv(Duration::from_millis(15)) { v.push(au); }
        idle();
        if let Status::Failed(e) = s.status() {
            if missing(&e) || (ci() && v.is_empty()) { eprintln!("SKIPPED: {e}"); return None; }
            panic!("share failed: {e}");
        }
    }
    Some(v)
}

/// Writes the units as MPEG-TS and decodes them with the playback decoder; None when this machine has no decoder for it.
fn decode(aus: &[AccessUnit]) -> Option<(crate::media::video::Info, Vec<crate::media::video::Frame>)> {
    let dir = temp_dir();
    let path = dir.join("share.ts");
    let mut ts = ts::Ts::default();
    for au in aus { ts.push(&au.data, au.pts_us, au.keyframe); }
    std::fs::write(&path, &ts.out).unwrap();
    let r = crate::media::video::decode_all(&path);
    let _ = std::fs::remove_dir_all(&dir);
    match r {
        Ok(x) => Some(x),
        Err(e) if cfg!(target_os = "linux") || ci() => { eprintln!("SKIPPED: no decoder for H.264 in MPEG-TS here: {e}"); None }
        Err(e) => panic!("decode: {e}"),
    }
}

fn check_stream(aus: &[AccessUnit], size: (u32, u32)) {
    assert!(!aus.is_empty(), "no access units");
    let first = &aus[0];
    assert!(first.keyframe, "the stream starts with a key frame");
    let units = h264::nals(&first.data);
    let types: Vec<u8> = units.iter().map(|n| h264::nal_type(n)).collect();
    assert_eq!(&types[..2], &[h264::SPS, h264::PPS], "SPS + PPS lead the key frame: {types:?}");
    // Constrained Baseline: profile 66 with constraint_set1 (42c0xx / 42e0xx)
    let (profile, flags, _) = h264::sps_profile(units[0]).unwrap();
    assert!(profile == 66 && flags & 0x40 != 0, "Constrained Baseline, got {:?}", h264::profile_level_id(&first.data));
    assert_eq!(h264::sps_size(units[0]), Some(size));
    assert!(aus.iter().all(|a| (a.width, a.height) == size));
    assert!(aus.windows(2).all(|w| w[1].pts_us >= w[0].pts_us), "time runs forward");
    assert!(aus.iter().all(|a| !h264::nals(&a.data).iter().any(|n| h264::nal_type(n) == h264::AUD)));
}

fn mean_diff(a: &[u8], b: &[u8]) -> f64 {
    a.chunks(4).zip(b.chunks(4)).map(|(x, y)| (0..3).map(|i| (x[i] as i32 - y[i] as i32).abs()).sum::<i32>() as f64 / 3.0).sum::<f64>() / (a.len() / 4) as f64
}

#[test]
fn h264_helpers() {
    let sps = [0x67, 0x42, 0x00, 0x1f, 0xab];
    let au = [&[0, 0, 0, 1][..], &sps, &[0, 0, 1, 0x68, 0xce, 0x3c, 0x80], &[0, 0, 0, 1, 0x65, 0x88, 0x00, 0x00]].concat();
    let n = h264::nals(&au);
    assert_eq!(n.len(), 3);
    assert_eq!(n.iter().map(|x| h264::nal_type(x)).collect::<Vec<_>>(), vec![7, 8, 5]);
    assert_eq!(n[2], &[0x65, 0x88], "trailing zeros are not part of the unit");
    assert_eq!(h264::profile_level_id(&au).as_deref(), Some("42001f"));
    let mut p = h264::ParamSets::default();
    assert_eq!(h264::profile_level_id(&p.fix(&au)).as_deref(), Some("42c01f"), "marked constrained");
    // a key frame without parameter sets gets the last ones; a delimiter is dropped
    let bare = p.fix(&[0, 0, 0, 1, 9, 0xf0, 0, 0, 0, 1, 0x65, 0x88][..]);
    assert_eq!(h264::nals(&bare).iter().map(|x| h264::nal_type(x)).collect::<Vec<_>>(), vec![7, 8, 5]);
    let delta = p.fix(&[0, 0, 0, 1, 0x41, 0x9a][..]);
    assert_eq!(h264::nals(&delta).len(), 1);
    assert!(h264::is_keyframe(&bare) && !h264::is_keyframe(&delta));
}

#[test]
fn fit_and_colour_conversion() {
    assert_eq!(convert::fit(3840, 2160, (1920, 1080)), (1920, 1080));
    assert_eq!(convert::fit(1001, 501, (1280, 720)), (1000, 500), "even, never enlarged");
    assert_eq!(convert::fit(2560, 1600, (1280, 720)), (1152, 720));
    let (w, h) = (64u32, 48u32);
    let red: Vec<u8> = (0..w * h).flat_map(|_| [0u8, 0, 255, 255]).collect(); // BGRA
    let mut nv12 = vec![];
    convert::to_nv12(convert::Picture { px: &red, w, h, stride: w as usize * 4, bgra: true }, 32, 24, &mut nv12);
    assert_eq!(nv12.len(), 32 * 24 * 3 / 2);
    assert!(nv12[..32 * 24].iter().all(|&y| (80..=83).contains(&y)), "Y of red {}", nv12[0]);
    assert!(nv12[32 * 24..].chunks(2).all(|c| (88..=92).contains(&c[0]) && (238..=242).contains(&c[1])), "UV of red {:?}", &nv12[32 * 24..32 * 24 + 2]);
    let (tw, th, t) = convert::thumb(convert::Picture { px: &red, w, h, stride: w as usize * 4, bgra: true }, (16, 16));
    assert_eq!((tw, th, &t[..4]), (16, 12, &[255u8, 0, 0, 255][..]));
}

#[test]
fn presets_differ_in_size_and_rate() {
    let (t, m) = (Opts::preset(Optimize::Text), Opts::preset(Optimize::Motion));
    assert!(t.max_size.0 > m.max_size.0 && t.fps < m.fps);
}

/// The test picture through the encoder and back: size, profile, key frames on request, colours.
#[test]
fn test_picture_round_trip() {
    let _x = exclusive();
    let (w, h) = (640u32, 360u32);
    let s = start(&ShareSource::test_pattern(w, h), Opts { max_size: (1280, 720), fps: 30, bitrate: 1_500_000, keyframe_secs: 0, cursor: false });
    let Some(mut aus) = collect(&s, 1.0, || {}) else { return };
    let Status::Running { encoder, .. } = s.status() else { panic!("{:?}", s.status()) };
    eprintln!("encoder: {encoder}");
    s.request_keyframe();
    let asked = aus.len();
    aus.extend(collect(&s, 0.6, || {}).unwrap_or_default());
    s.set_bitrate(500_000);
    aus.extend(collect(&s, 0.4, || {}).unwrap_or_default());
    s.stop();
    check_stream(&aus, (w, h));
    // the rate once running (opening a GPU encoder takes a moment)
    let secs = (aus.last().unwrap().pts_us - aus[0].pts_us) as f64 / 1e6;
    assert!(secs > 0.5 && aus.len() as f64 >= secs * 30.0 * 0.85, "{} pictures in {secs:.2} s at 30 fps", aus.len());
    assert!(aus[asked..asked + 3.min(aus.len() - asked)].iter().any(|a| a.keyframe), "a key frame follows the request");
    assert!(aus[1..asked].iter().all(|a| !a.keyframe), "no key frames without a request (keyframe_secs 0)");
    let Some((info, frames)) = decode(&aus) else { return };
    assert_eq!((info.width, info.height), (w, h));
    assert!(frames.len() + 2 >= aus.len(), "{} of {} pictures decoded", frames.len(), aus.len());
    let mut want = vec![0u8; (w * h * 4) as usize];
    convert::test_picture(0, w, h, &mut want);
    let d = mean_diff(&frames[0].rgba, &want);
    assert!(d < 10.0, "first picture differs by {d:.1} per channel");
}

/// A window this test opens (solid orange), shared by handle: never listed among the sources (it is Webox's own), its
/// picture arrives and decodes to the same colour.
#[cfg(windows)]
#[test]
fn shares_a_window_of_its_own() {
    skip_if!(no_desktop());
    let _x = exclusive();
    use ::windows::core::w;
    use ::windows::Win32::Foundation::*;
    use ::windows::Win32::Graphics::Gdi::{CreateSolidBrush, DeleteObject};
    use ::windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use ::windows::Win32::UI::WindowsAndMessaging::*;
    unsafe extern "system" fn proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT { DefWindowProcW(h, m, w, l) }
    unsafe {
        let brush = CreateSolidBrush(COLORREF(0x0020_80f0)); // R f0 G 80 B 20
        let inst = GetModuleHandleW(None).unwrap();
        let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: HINSTANCE(inst.0), hbrBackground: brush, lpszClassName: w!("WeboxShareTest"), ..Default::default() };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(WS_EX_NOACTIVATE, w!("WeboxShareTest"), w!("Webox share test"), WS_OVERLAPPEDWINDOW, 40, 40, 480, 320, None, None, inst, None).unwrap();
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let pump = || { let mut m = MSG::default(); while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() { let _ = TranslateMessage(&m); DispatchMessageW(&m); } };
        pump();
        assert!(!sources().iter().any(|s| s.handle == hwnd.0 as isize), "own windows are not offered");
        let src = ShareSource { kind: Kind::Window, handle: hwnd.0 as isize, index: 0, name: "test".into(), app: String::new(), size: (480, 320), primary: false, minimized: false };
        let thumb = thumbnail(&src, (160, 120));
        let s = start(&src, Opts { max_size: (1920, 1080), fps: 15, bitrate: 1_000_000, keyframe_secs: 2, cursor: false });
        let aus = collect(&s, 1.5, pump);
        let status = s.status();
        let Some(aus) = aus else { let _ = DestroyWindow(hwnd); let _ = DeleteObject(brush); return };
        s.stop();
        let _ = DestroyWindow(hwnd);
        pump();
        let _ = DeleteObject(brush);
        let (tw, th, t) = thumb.expect("thumbnail");
        assert!(tw <= 160 && th <= 120 && t.len() == (tw * th * 4) as usize);
        let size = (aus.first().unwrap_or_else(|| panic!("no pictures: {status:?}")).width, aus[0].height);
        check_stream(&aus, size);
        assert!(size.0 >= 400 && size.1 >= 250, "window size {size:?}");
        let Some((_, frames)) = decode(&aus) else { return };
        let f = frames.last().unwrap();
        let i = ((f.h / 2) * f.w + f.w / 2) * 4;
        let c = &f.rgba[i..i + 3];
        assert!((c[0] as i32 - 0xf0).abs() < 24 && (c[1] as i32 - 0x80).abs() < 24 && (c[2] as i32 - 0x20).abs() < 24, "centre {c:?}");
    }
}

/// The primary monitor: WGC (or the duplication fallback) delivers, the stream decodes at the fitted size. The capture
/// only lives in memory and in a temp file that is deleted.
#[cfg(windows)]
#[test]
fn shares_the_primary_monitor() {
    skip_if!(no_desktop());
    let _x = exclusive();
    let Some(mon) = sources().into_iter().find(|s| s.kind == Kind::Screen && s.primary) else { eprintln!("SKIPPED: no monitor"); return };
    let opts = Opts { fps: 10, ..Opts::preset(Optimize::Text) };
    let s = start(&mon, opts);
    // a still screen gives one picture a second: key frame requests make the encoder repeat the last one
    let mut n = 0;
    let aus = collect(&s, 2.5, || { n += 1; if n % 10 == 0 { s.request_keyframe(); } });
    let status = s.status();
    s.stop();
    let Some(aus) = aus else { return };
    eprintln!("{} pictures, {status:?}", aus.len());
    // the session got locked (or the screen went to the secure desktop) while the test ran: no picture to judge
    if aus.is_empty() { skip_if!(no_desktop()); }
    let size = convert::fit(mon.size.0, mon.size.1, opts.max_size);
    check_stream(&aus, size);
    let Some((info, frames)) = decode(&aus) else { return };
    assert_eq!((info.width, info.height), size);
    assert!(!frames.is_empty());
}

/// The DXGI Desktop Duplication fallback reads the primary monitor at its full size.
#[cfg(windows)]
#[test]
fn duplication_fallback_reads_the_monitor() {
    use super::windows::{dupl::Dupl, Capture, Grab};
    skip_if!(no_desktop());
    let _x = exclusive();
    let Some(mon) = sources().into_iter().find(|s| s.kind == Kind::Screen && s.primary) else { eprintln!("SKIPPED: no monitor"); return };
    // no duplication here (remote session, a GPU without it, all slots taken by other programs)
    let mut d = match Dupl::open(mon.handle) { Ok(d) => d, Err(e) => { eprintln!("SKIPPED: {e}"); return } };
    let mut buf = vec![];
    let (mut got, mut last) = (None, String::new());
    let t0 = Instant::now();
    while got.is_none() && t0.elapsed() < Duration::from_secs(5) {
        match d.grab(&mut buf, Duration::from_millis(100)) { Ok(Grab::Frame { w, h, .. }) => got = Some((w, h)), Ok(_) => {} Err(e) => last = e }
    }
    if got.is_none() { skip_if!(no_desktop()); }
    assert_eq!(got, Some(mon.size), "{last}");
    assert_eq!(buf.len(), (mon.size.0 * mon.size.1 * 4) as usize);
}

/// Microsoft's software encoder on its own: Baseline output, key frames on request.
#[cfg(windows)]
#[test]
fn software_encoder_forces_key_frames() {
    use super::windows::encoder::Encoder;
    let _x = exclusive();
    unsafe {
        let _ = ::windows::Win32::System::Com::CoInitializeEx(None, ::windows::Win32::System::Com::COINIT_MULTITHREADED);
        let _ = ::windows::Win32::Media::MediaFoundation::MFStartup(::windows::Win32::Media::MediaFoundation::MF_VERSION, ::windows::Win32::Media::MediaFoundation::MFSTARTUP_FULL);
    }
    let (w, h) = (320u32, 240u32);
    let mut e = match Encoder::new(w, h, 30, 600_000, 0, false) {
        Ok(e) => e,
        Err(e) if missing(&e) || ci() => { unsafe { let _ = ::windows::Win32::Media::MediaFoundation::MFShutdown(); } eprintln!("SKIPPED: {e}"); return }
        Err(e) => panic!("software encoder: {e}"),
    };
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let mut nv12 = vec![];
    let mut keys = vec![];
    for i in 0..20u32 {
        convert::test_picture(i, w, h, &mut rgba);
        convert::to_nv12(convert::Picture { px: &rgba, w, h, stride: w as usize * 4, bgra: false }, w, h, &mut nv12);
        for (au, _) in e.encode(&nv12, i as i64 * 333_333, i == 10).unwrap() { keys.push((i, h264::is_keyframe(&au))); }
    }
    drop(e);
    unsafe { let _ = ::windows::Win32::Media::MediaFoundation::MFShutdown(); }
    assert!(keys.first().is_some_and(|k| k.1), "{keys:?}");
    assert!(keys.iter().any(|&(i, k)| i >= 10 && i <= 12 && k), "key frame after the request: {keys:?}");
    assert_eq!(keys.iter().filter(|k| k.1).count(), 2, "{keys:?}");
}
