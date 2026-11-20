//! Decode tests on a video made by the test itself (no real file in the repository): 2 s, 160 x 120 at 10 fps, the first second
//! red, the second blue, with a 440 Hz tone. Windows encodes it with Media Foundation's own H.264 + AAC encoders into MP4;
//! Linux with GStreamer (videotestsrc / audiotestsrc -> MJPEG + PCM in AVI, plugins -base and -good).
use super::*;
use std::path::PathBuf;

const W: u32 = 160;
const H: u32 = 120;
const FPS: u32 = 10;
const FRAMES: u32 = 20;
const RATE: u32 = 48_000;

fn temp(ext: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("webox-video-test-{}", crate::jose::uuid()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(format!("synthetic.{ext}"))
}

/// The colour a test frame should have (red for the first second, blue after).
fn want(pts: f64) -> [u8; 3] { if pts < 0.95 { [255, 0, 0] } else { [0, 0, 255] } }

fn centre(f: &Frame) -> [u8; 3] { let i = ((f.h / 2) * f.w + f.w / 2) * 4; [f.rgba[i], f.rgba[i + 1], f.rgba[i + 2]] }
fn near(a: [u8; 3], b: [u8; 3]) -> bool { a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() < 70) }

fn spec() -> synth::Spec { synth::Spec { w: W, h: H, fps: FPS, frames: FRAMES, rate: RATE } }

#[cfg(windows)]
fn make(path: &std::path::Path) -> Result<(), String> {
    synth::write(path, spec(), &|i, rgba| { let [r, g, b] = want(i as f64 / FPS as f64); for p in rgba.chunks_exact_mut(4) { p.copy_from_slice(&[r, g, b, 255]); } })
}

#[cfg(target_os = "linux")]
fn make(path: &std::path::Path) -> Result<(), String> {
    // first second red, then blue: two videotestsrc runs joined by `concat`
    let half = FRAMES / 2;
    let caps = format!("video/x-raw,width={W},height={H},framerate={FPS}/1");
    synth::write_gst(path, spec(), &format!("concat name=c videotestsrc num-buffers={half} pattern=red ! {caps} ! c. videotestsrc num-buffers={half} pattern=blue ! {caps} ! c. c."))
}
/// The test video, or None when this machine has no encoder (Windows N without the Media Feature Pack, Linux without
/// gst-plugins-good): the test then says so and passes.
fn synthetic() -> Option<PathBuf> {
    let p = temp(if cfg!(windows) { "mp4" } else { "avi" });
    match make(&p) {
        Ok(()) if std::fs::metadata(&p).is_ok_and(|m| m.len() > 0) => Some(p),
        r => { eprintln!("SKIPPED: cannot make a test video here: {r:?}"); None }
    }
}

#[test]
fn decodes_frames_and_sound_of_a_synthetic_video() {
    let Some(path) = synthetic() else { return };
    let mut head = vec![0u8; 64];
    { use std::io::Read; let n = std::fs::File::open(&path).unwrap().read(&mut head).unwrap(); head.truncate(n); }
    let c = format::detect(&head, &path.to_string_lossy(), "").expect("container");
    assert_eq!(c, if cfg!(windows) { format::Container::Mp4 } else { format::Container::Avi });
    let (mut src, info) = backend::open(&path, sync::MAX_SIZE, true).expect("open");
    assert!(info.has_video && info.has_audio, "{info:?}");
    assert_eq!((info.width, info.height), (W, H), "{info:?}");
    assert!((info.duration - 2.0).abs() < 0.3, "duration {}", info.duration);
    eprintln!("decoder: hardware = {}", info.hardware);
    let (mut frames, mut samples, mut energy, mut rate) = (vec![], 0usize, 0f64, 0u32);
    while let Some(p) = src.read().expect("read") {
        match p {
            Packet::Video(f) => { assert_eq!((f.w, f.h, f.rgba.len()), (W as usize, H as usize, (W * H * 4) as usize)); frames.push((f.pts, centre(&f))); }
            Packet::Audio { rate: r, ch, data, .. } => { rate = r; assert_eq!(ch, 2); samples += data.len() / 2; energy += data.iter().map(|v| (*v as f64).powi(2)).sum::<f64>(); }
        }
    }
    assert!(frames.len() >= FRAMES as usize - 2, "{} frames", frames.len());
    for (pts, rgb) in &frames { if (pts - 0.95).abs() > 0.15 { assert!(near(*rgb, want(*pts)), "frame at {pts}: {rgb:?}"); } }
    assert!(frames.windows(2).all(|w| w[1].0 >= w[0].0), "frames in order");
    let secs = samples as f64 / rate.max(1) as f64;
    assert!((secs - 2.0).abs() < 0.25, "{secs} s of sound at {rate} Hz");
    assert!(energy / (samples.max(1) * 2) as f64 > 0.01, "the tone is there");
    // seeking into the blue second gives blue pictures at or after the target (the loop decodes up to it)
    src.seek(1.5).expect("seek");
    let mut after = None;
    while let Some(p) = src.read().expect("read") { if let Packet::Video(f) = p { if f.pts + 0.02 >= 1.5 { after = Some(f); break; } } }
    let f = after.expect("a frame after the seek");
    assert!(near(centre(&f), [0, 0, 255]), "{:?}", centre(&f));
    let (poster, pinfo) = first_frame(&path).expect("first frame");
    assert!(near(centre(&poster), [255, 0, 0]) && poster.pts < 0.2 && pinfo.has_video);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// By hand: `WEBOX_VIDEO_PROBE="a.mp4;b.webm" cargo test --bin webox probe -- --ignored --nocapture` decodes each file (made-up
/// test files only) and prints what this platform's framework makes of it.
#[test]
#[ignore]
fn probe() {
    for f in std::env::var("WEBOX_VIDEO_PROBE").unwrap_or_default().split(';').filter(|f| !f.is_empty()) {
        let r = backend::open(std::path::Path::new(f), sync::MAX_SIZE, true).and_then(|(mut src, info)| {
            let (mut v, mut a) = (0, 0usize);
            while let Some(p) = src.read()? { match p { Packet::Video(_) => v += 1, Packet::Audio { data, .. } => a += data.len() } }
            Ok(format!("{}x{} {:.1}s hw={} video frames {v}, audio samples {a}", info.width, info.height, info.duration, info.hardware))
        });
        println!("PROBE {f}: {r:?}");
    }
}

/// The sample data's clip (640 x 360, 25 fps, made by `synth`) opens in the player and gives pictures.
#[test]
fn sample_clip_plays() {
    let Some(path) = synth::sample_clip() else { eprintln!("SKIPPED: no encoder"); return };
    let p = Player::open(&path, std::env::var("WEBOX_TEST_SPEAKER").ok().as_deref(), None);
    for _ in 0..300 { if p.info().ready || p.info().error.is_some() { break; } std::thread::sleep(Duration::from_millis(10)); }
    assert!(p.info().error.is_none(), "{:?}", p.info().error);
    assert_eq!((p.info().width, p.info().height), (640, 360));
    p.play();
    let mut got = 0;
    for _ in 0..100 { if p.frame().0.is_some() { got += 1; } std::thread::sleep(Duration::from_millis(10)); }
    assert!(got >= 5, "{got} pictures in 1 s");
    assert!(p.info().error.is_none(), "{:?}", p.info().error);
}

/// The whole player on its thread, without a speaker (wall clock): it opens paused, plays forward in time, seeks, ends.
#[test]
fn player_plays_seeks_and_ends() {
    let Some(path) = synthetic() else { return };
    let p = Player::open(&path, None, None);
    let wait = |ok: &dyn Fn() -> bool| { for _ in 0..300 { if ok() { return true; } std::thread::sleep(Duration::from_millis(10)); } false };
    assert!(wait(&|| p.info().ready || p.info().error.is_some()));
    assert!(p.info().error.is_none(), "{:?}", p.info().error);
    assert!(!p.audible()); // opened without sound
    // paused: the first picture still arrives (the poster of the inline player)
    let mut shown = None;
    for _ in 0..300 { if let (Some(f), _) = p.frame() { shown = Some(f.pts); break; } std::thread::sleep(Duration::from_millis(10)); }
    assert!(shown.is_some_and(|t| t < 0.2), "{shown:?}");
    assert_eq!(p.position(), 0.0);
    p.play();
    std::thread::sleep(Duration::from_millis(400));
    let t = p.position();
    assert!((0.3..0.8).contains(&t), "position {t}");
    let mut latest = 0.0;
    for _ in 0..20 { if let (Some(f), _) = p.frame() { latest = f.pts; } std::thread::sleep(Duration::from_millis(10)); }
    assert!(latest > 0.15 && latest <= p.position() + 0.01, "frame {latest} at {}", p.position());
    p.seek(1.6);
    assert!((p.position() - 1.6).abs() < 0.05);
    assert!(wait(&|| p.ended()), "ends after the last frame");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
