//! Videos made on this machine, for the tests and for the offline sample data (`--sample`): nothing recorded, no file in the
//! repository. Windows encodes with Media Foundation's own encoders (H.264 + AAC in MP4, sink writer); Linux with GStreamer
//! (test sources -> MJPEG + PCM in AVI, gst-plugins-good).
use std::path::{Path, PathBuf};

/// The attachment key of the sample data's video (`cloud/sample.rs`); the message list plays `sample_clip()` for it.
pub const SAMPLE_KEY: &str = "sample-video:demo";

#[derive(Clone, Copy)]
pub struct Spec { pub w: u32, pub h: u32, pub fps: u32, pub frames: u32, pub rate: u32 }

/// Writes `spec.frames` pictures from `paint(i, rgba)` and a 440 Hz tone at `spec.rate` (stereo).
#[cfg(windows)]
pub fn write(path: &Path, spec: Spec, paint: &dyn Fn(u32, &mut [u8])) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Win32::Media::MediaFoundation::*;
    let e = |e: windows::core::Error| format!("{} (0x{:08X})", e.message(), e.code().0);
    let Spec { w: width, h: height, fps, frames, rate } = spec;
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(e)?;
        let res = (|| -> Result<(), String> {
            let w = MFCreateSinkWriterFromURL(&HSTRING::from(path.as_os_str()), None, None).map_err(e)?;
            let vt = |sub: &windows::core::GUID| -> Result<IMFMediaType, String> {
                let t = MFCreateMediaType().map_err(e)?;
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(e)?;
                t.SetGUID(&MF_MT_SUBTYPE, sub).map_err(e)?;
                t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32).map_err(e)?;
                t.SetUINT64(&MF_MT_FRAME_SIZE, ((width as u64) << 32) | height as u64).map_err(e)?;
                t.SetUINT64(&MF_MT_FRAME_RATE, ((fps as u64) << 32) | 1).map_err(e)?;
                t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1).map_err(e)?;
                Ok(t)
            };
            let out = vt(&MFVideoFormat_H264)?;
            out.SetUINT32(&MF_MT_AVG_BITRATE, 800_000).map_err(e)?;
            let vs = w.AddStream(&out).map_err(e)?;
            let input = vt(&MFVideoFormat_RGB32)?;
            input.SetUINT32(&MF_MT_DEFAULT_STRIDE, width * 4).map_err(e)?;
            w.SetInputMediaType(vs, &input, None).map_err(e)?;
            let at = |sub: &windows::core::GUID| -> Result<IMFMediaType, String> {
                let t = MFCreateMediaType().map_err(e)?;
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).map_err(e)?;
                t.SetGUID(&MF_MT_SUBTYPE, sub).map_err(e)?;
                t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate).map_err(e)?;
                t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2).map_err(e)?;
                t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16).map_err(e)?;
                Ok(t)
            };
            let aout = at(&MFAudioFormat_AAC)?;
            aout.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 16_000).map_err(e)?;
            let as_ = w.AddStream(&aout).map_err(e)?;
            let ain = at(&MFAudioFormat_PCM)?;
            ain.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4).map_err(e)?;
            ain.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, rate * 4).map_err(e)?;
            w.SetInputMediaType(as_, &ain, None).map_err(e)?;
            w.BeginWriting().map_err(e)?;
            let sample = |bytes: &[u8], t: i64, d: i64| -> Result<IMFSample, String> {
                let b = MFCreateMemoryBuffer(bytes.len() as u32).map_err(e)?;
                let mut p = std::ptr::null_mut();
                b.Lock(&mut p, None, None).map_err(e)?;
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
                b.Unlock().map_err(e)?;
                b.SetCurrentLength(bytes.len() as u32).map_err(e)?;
                let s = MFCreateSample().map_err(e)?;
                s.AddBuffer(&b).map_err(e)?;
                s.SetSampleTime(t).map_err(e)?;
                s.SetSampleDuration(d).map_err(e)?;
                Ok(s)
            };
            let fd = 10_000_000 / fps as i64;
            let mut rgba = vec![0u8; (width * height * 4) as usize];
            for i in 0..frames {
                paint(i, &mut rgba);
                let px: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], 255]).collect(); // RGB32 = B G R X
                w.WriteSample(vs, &sample(&px, i as i64 * fd, fd)?).map_err(e)?;
                let n = rate / fps;
                let pcm: Vec<u8> = (0..n).flat_map(|k| {
                    let t = (i * n + k) as f32 / rate as f32;
                    let b = (((t * 440.0 * std::f32::consts::TAU).sin() * 0.3 * i16::MAX as f32) as i16).to_le_bytes();
                    [b[0], b[1], b[0], b[1]]
                }).collect();
                w.WriteSample(as_, &sample(&pcm, i as i64 * fd, fd)?).map_err(e)?;
            }
            w.Finalize().map_err(e)
        })();
        let _ = MFShutdown();
        res
    }
}

/// Runs `video` (a gst-launch description producing raw video, ending without a sink) for its buffers, muxed with a 440 Hz tone
/// of the same length.
#[cfg(target_os = "linux")]
pub fn write_gst(path: &Path, spec: Spec, video: &str) -> Result<(), String> {
    use gst::prelude::*;
    use gstreamer as gst;
    gst::init().map_err(|e| e.to_string())?;
    let Spec { w, h, fps, frames, rate } = spec;
    let desc = format!(
        "{video} ! video/x-raw,width={w},height={h},framerate={fps}/1 ! videoconvert ! jpegenc ! avimux name=m ! filesink location=\"{}\" \
         audiotestsrc num-buffers={frames} samplesperbuffer={spb} freq=440 volume=0.3 ! audio/x-raw,format=S16LE,rate={rate},channels=2 ! m.",
        path.display(), spb = rate / fps);
    let p = gst::parse_launch(&desc).map_err(|e| e.to_string())?;
    p.set_state(gst::State::Playing).map_err(|e| e.to_string())?;
    let bus = p.bus().ok_or("bus")?;
    let r = match bus.timed_pop_filtered(gst::ClockTime::from_seconds(60), &[gst::MessageType::Eos, gst::MessageType::Error]) {
        Some(m) => match m.view() { gst::MessageView::Error(e) => Err(e.error().to_string()), _ => Ok(()) },
        None => Err("timeout".into()),
    };
    let _ = p.set_state(gst::State::Null);
    r
}

/// The sample data's video ("Demo-Clip"): 8 s, 640 x 360, a colour wheel with a moving bar and the running time as blocks. Made
/// once per run in the temp folder; None when this machine cannot encode (the card then shows an error).
pub fn sample_clip() -> Option<PathBuf> {
    static CLIP: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    CLIP.get_or_init(|| {
        let dir = std::env::temp_dir().join("webox-sample-video");
        let _ = std::fs::create_dir_all(&dir);
        let spec = Spec { w: 640, h: 360, fps: 25, frames: 200, rate: 48_000 };
        #[cfg(windows)]
        let (p, r) = { let p = dir.join("Demo-Clip.mp4"); let r = if p.exists() { Ok(()) } else { write(&p, spec, &paint_demo) }; (p, r) };
        #[cfg(target_os = "linux")]
        let (p, r) = { let p = dir.join("Demo-Clip.avi"); let r = if p.exists() { Ok(()) } else { write_gst(&p, spec, &format!("videotestsrc num-buffers={} pattern=ball", spec.frames)) }; (p, r) };
        #[cfg(not(any(windows, target_os = "linux")))]
        let (p, r): (PathBuf, Result<(), String>) = { let _ = spec; (dir.join("Demo-Clip"), Err("no video encoder on this platform".into())) };
        match r { Ok(()) => Some(p), Err(e) => { let _ = std::fs::remove_file(&p); eprintln!("sample video: {e}"); None } }
    }).clone()
}

/// Frame `i` of the demo clip (640 x 360 RGBA): a slowly turning hue, a white bar crossing every 2 s, a progress strip.
#[cfg(windows)]
fn paint_demo(i: u32, rgba: &mut [u8]) {
    let (w, h) = (640usize, 360usize);
    let t = i as f32 / 25.0;
    let bar = ((t / 2.0).fract() * w as f32) as usize;
    for y in 0..h {
        for x in 0..w {
            let hue = (x as f32 / w as f32 + t / 8.0).fract() * 6.0;
            let (r, g, b) = match hue as u32 { 0 => (1.0, hue.fract(), 0.0), 1 => (1.0 - hue.fract(), 1.0, 0.0), 2 => (0.0, 1.0, hue.fract()), 3 => (0.0, 1.0 - hue.fract(), 1.0), 4 => (hue.fract(), 0.0, 1.0), _ => (1.0, 0.0, 1.0 - hue.fract()) };
            let shade = 0.35 + 0.4 * (y as f32 / h as f32);
            let mut px = [(r * shade * 255.0) as u8, (g * shade * 255.0) as u8, (b * shade * 255.0) as u8, 255];
            if x.abs_diff(bar) < 6 { px = [240, 240, 240, 255]; }
            if y >= h - 12 && (x as f32) < t / 8.0 * w as f32 { px = [255, 255, 255, 255]; }
            rgba[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&px);
        }
    }
}
