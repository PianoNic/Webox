//! Windows: Media Foundation's source reader. Plays what Windows can: the sources it ships (MP4 / MOV / M4V / 3GP, ASF / WMV,
//! AVI, MPEG-TS, MKV / WebM) with the decoders that are installed (H.264, MPEG-4 part 2, MJPEG, WMV, AAC / MP3 / WMA / AC-3 in
//! the box; HEVC, VP9, AV1 through their Store extensions).
//!
//! Hardware decoding: a D3D11 device (video support) behind an `IMFDXGIDeviceManager` is handed to the reader
//! (`MF_SOURCE_READER_D3D_MANAGER`, `MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS`), so the decoder MFT runs on DXVA where the GPU
//! has the codec; the advanced video processor converts to RGB32 (and scales down to `MAX_SIZE`) on the GPU, and locking the
//! sample's 2D buffer reads it back. Without a usable GPU the reader is created again without the device (software decoding).
//! Sound is decoded to 32-bit float at the file's own rate and channel count.
use super::{Info, Packet, Source};
use super::sync::Frame;
use std::path::Path;
use windows::core::{Interface, GUID, HSTRING, PROPVARIANT};
use windows::Win32::Media::MediaFoundation::*;

const ANY: u32 = MF_SOURCE_READER_ANY_STREAM.0 as u32;
const ALL: u32 = MF_SOURCE_READER_ALL_STREAMS.0 as u32;
const MEDIASOURCE: u32 = MF_SOURCE_READER_MEDIASOURCE.0 as u32;

struct Mf {
    reader: Option<IMFSourceReader>,
    video: Option<u32>,
    audio: Option<u32>,
    /// Current picture size and row stride of the reader's RGB32 output.
    w: usize,
    h: usize,
    stride: i32,
    rate: u32,
    ch: u16,
    ended_v: bool,
    ended_a: bool,
    /// Pictures delivered so far: a video stream that ends without one had no working decoder (seen with VP9 through DXVA on
    /// a machine without the VP9 extension: the reader opens, then only the sound comes).
    pictures: u64,
}

fn err(e: windows::core::Error) -> String { format!("Media Foundation: {} (0x{:08X})", e.message(), e.code().0) }

/// (w, h) packed in an MF_MT_FRAME_SIZE value.
fn unpack(v: u64) -> (u32, u32) { ((v >> 32) as u32, v as u32) }
/// Fits (w, h) into `max` keeping the ratio, even numbers (what the video processor wants).
pub(super) fn fit(w: u32, h: u32, max: (u32, u32)) -> (u32, u32) {
    if w <= max.0 && h <= max.1 || w == 0 || h == 0 { return (w, h); }
    let k = (max.0 as f64 / w as f64).min(max.1 as f64 / h as f64);
    (((w as f64 * k) as u32 & !1).max(2), ((h as f64 * k) as u32 & !1).max(2))
}

unsafe fn startup() -> Result<(), String> {
    let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(err)
}

/// A DXGI device manager over a new hardware D3D11 device with video support (None: no GPU decoding here).
unsafe fn gpu() -> Option<IMFDXGIDeviceManager> {
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::*;
    let mut dev: Option<ID3D11Device> = None;
    D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, None, D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut dev), None, None).ok()?;
    let dev = dev?;
    // the decoder and the reader use the device from their own threads
    if let Ok(mt) = dev.cast::<ID3D11Multithread>() { let _ = mt.SetMultithreadProtected(true); }
    let (mut token, mut mgr) = (0u32, None);
    MFCreateDXGIDeviceManager(&mut token, &mut mgr).ok()?;
    let mgr = mgr?;
    mgr.ResetDevice(&dev, token).ok()?;
    Some(mgr)
}

unsafe fn reader(path: &Path, hw: Option<&IMFDXGIDeviceManager>) -> Result<IMFSourceReader, String> {
    let mut a: Option<IMFAttributes> = None;
    MFCreateAttributes(&mut a, 4).map_err(err)?;
    let a = a.ok_or("attributes")?;
    // colour conversion (YUV -> RGB32) and scaling inside the reader
    a.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1).map_err(err)?;
    if let Some(m) = hw {
        a.SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, m).map_err(err)?;
        a.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1).map_err(err)?;
    }
    MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), &a).map_err(err)
}

/// Index of the first video and the first audio stream.
unsafe fn streams(r: &IMFSourceReader) -> (Option<u32>, Option<u32>) {
    let (mut v, mut a) = (None, None);
    for i in 0..64u32 {
        let Ok(t) = r.GetNativeMediaType(i, 0) else { break };
        match t.GetGUID(&MF_MT_MAJOR_TYPE) {
            Ok(g) if g == MFMediaType_Video && v.is_none() => v = Some(i),
            Ok(g) if g == MFMediaType_Audio && a.is_none() => a = Some(i),
            _ => {}
        }
    }
    (v, a)
}

unsafe fn set_video(r: &IMFSourceReader, i: u32, max: (u32, u32)) -> Result<(), String> {
    let native = r.GetNativeMediaType(i, 0).map_err(err)?;
    let (w, h) = native.GetUINT64(&MF_MT_FRAME_SIZE).map(unpack).unwrap_or((0, 0));
    let t = MFCreateMediaType().map_err(err)?;
    t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(err)?;
    t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).map_err(err)?;
    let (fw, fh) = fit(w, h, max);
    if (fw, fh) != (w, h) && t.SetUINT64(&MF_MT_FRAME_SIZE, ((fw as u64) << 32) | fh as u64).is_ok() && r.SetCurrentMediaType(i, None, &t).is_ok() { return Ok(()); }
    let t = MFCreateMediaType().map_err(err)?;
    t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(err)?;
    t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).map_err(err)?;
    r.SetCurrentMediaType(i, None, &t).map_err(err)
}

unsafe fn set_audio(r: &IMFSourceReader, i: u32) -> Result<(), String> {
    let t = MFCreateMediaType().map_err(err)?;
    t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).map_err(err)?;
    t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float).map_err(err)?;
    r.SetCurrentMediaType(i, None, &t).map_err(err)
}

impl Mf {
    unsafe fn read_types(&mut self) {
        let Some(r) = &self.reader else { return };
        if let Some(i) = self.video {
            if let Ok(t) = r.GetCurrentMediaType(i) {
                let (w, h) = t.GetUINT64(&MF_MT_FRAME_SIZE).map(unpack).unwrap_or((0, 0));
                self.w = w as usize;
                self.h = h as usize;
                // MF_MT_DEFAULT_STRIDE is an INT32 stored as UINT32; negative = bottom-up rows
                self.stride = t.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| s as i32).unwrap_or(w as i32 * 4);
            }
        }
        if let Some(i) = self.audio {
            if let Ok(t) = r.GetCurrentMediaType(i) {
                self.rate = t.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(0);
                self.ch = t.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(0) as u16;
            }
        }
    }

    /// RGB32 (B, G, R, X per pixel) -> RGBA, through the 2D buffer when there is one (its pitch is the truth, also for GPU surfaces).
    unsafe fn picture(&self, s: &IMFSample, pts: f64) -> Result<Frame, String> {
        let (w, h) = (self.w, self.h);
        let mut rgba = vec![0u8; w * h * 4];
        let copy = |base: *const u8, pitch: isize, rgba: &mut [u8]| {
            for y in 0..h {
                let row = std::slice::from_raw_parts(base.offset(y as isize * pitch), w * 4);
                for (d, s) in rgba[y * w * 4..(y + 1) * w * 4].chunks_exact_mut(4).zip(row.chunks_exact(4)) { d.copy_from_slice(&[s[2], s[1], s[0], 255]); }
            }
        };
        let buf = s.GetBufferByIndex(0).map_err(err)?;
        if let Ok(b2) = buf.cast::<IMF2DBuffer>() {
            let (mut p, mut pitch) = (std::ptr::null_mut(), 0i32);
            if b2.Lock2D(&mut p, &mut pitch).is_ok() {
                if !p.is_null() && pitch.unsigned_abs() as usize >= w * 4 { copy(p, pitch as isize, &mut rgba); }
                let _ = b2.Unlock2D();
                return Ok(Frame { pts, w, h, rgba });
            }
        }
        let buf = s.ConvertToContiguousBuffer().map_err(err)?;
        let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
        buf.Lock(&mut p, None, Some(&mut len)).map_err(err)?;
        let stride = if self.stride == 0 { (w * 4) as i32 } else { self.stride };
        if !p.is_null() && len as usize >= stride.unsigned_abs() as usize * h {
            // bottom-up: the first row in memory is the last one on screen
            let (base, pitch) = if stride < 0 { (p.add(stride.unsigned_abs() as usize * (h - 1)) as *const u8, stride as isize) } else { (p as *const u8, stride as isize) };
            copy(base, pitch, &mut rgba);
        }
        let _ = buf.Unlock();
        Ok(Frame { pts, w, h, rgba })
    }

    unsafe fn sound(&self, s: &IMFSample) -> Result<Vec<f32>, String> {
        let buf = s.ConvertToContiguousBuffer().map_err(err)?;
        let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
        buf.Lock(&mut p, None, Some(&mut len)).map_err(err)?;
        let v = if p.is_null() { vec![] } else { std::slice::from_raw_parts(p as *const f32, len as usize / 4).to_vec() };
        let _ = buf.Unlock();
        Ok(v)
    }
}

impl Source for Mf {
    fn read(&mut self) -> Result<Option<Packet>, String> {
        unsafe {
            loop {
                if self.video.map_or(true, |_| self.ended_v) && self.audio.map_or(true, |_| self.ended_a) { return Ok(None); }
                let r = self.reader.as_ref().ok_or("closed")?;
                let (mut idx, mut flags, mut ts, mut sample) = (0u32, 0u32, 0i64, None);
                r.ReadSample(ANY, 0, Some(&mut idx), Some(&mut flags), Some(&mut ts), Some(&mut sample)).map_err(err)?;
                let f = flags as i32;
                if f & MF_SOURCE_READERF_ERROR.0 != 0 { return Err("Media Foundation: decoding failed".into()); }
                if f & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 != 0 { self.read_types(); }
                if f & MF_SOURCE_READERF_ENDOFSTREAM.0 != 0 {
                    if Some(idx) == self.video { self.ended_v = true; if self.pictures == 0 { return Err("Media Foundation: no decoder for this video".into()); } }
                    if Some(idx) == self.audio { self.ended_a = true; }
                }
                let Some(s) = sample else { continue };
                let pts = ts as f64 / 1e7;
                if Some(idx) == self.video { self.pictures += 1; return self.picture(&s, pts).map(|f| Some(Packet::Video(f))); }
                if Some(idx) == self.audio {
                    let data = self.sound(&s)?;
                    return Ok(Some(Packet::Audio { pts, rate: self.rate, ch: self.ch, data }));
                }
            }
        }
    }

    fn seek(&mut self, secs: f64) -> Result<(), String> {
        let r = self.reader.as_ref().ok_or("closed")?;
        let pos = PROPVARIANT::from((secs.max(0.0) * 1e7) as i64);
        unsafe { r.SetCurrentPosition(&GUID::zeroed(), &pos).map_err(err)?; }
        self.ended_v = false;
        self.ended_a = false;
        Ok(())
    }
}

impl Drop for Mf {
    fn drop(&mut self) { self.reader = None; unsafe { let _ = MFShutdown(); } }
}

/// Opens `path`; `with_audio` false selects only the picture (poster frames).
pub(super) fn open(path: &Path, max: (u32, u32), with_audio: bool) -> Result<(Box<dyn Source>, Info), String> {
    unsafe {
        startup()?;
        // the Mf value owns the MFStartup from here on (its Drop calls MFShutdown)
        let mut mf = Mf { reader: None, video: None, audio: None, w: 0, h: 0, stride: 0, rate: 0, ch: 0, ended_v: false, ended_a: false, pictures: 0 };
        // WEBOX_VIDEO_SOFTWARE=1: no DXVA (a driver whose decoder misbehaves)
        let gpu = if std::env::var_os("WEBOX_VIDEO_SOFTWARE").is_some() { None } else { gpu() };
        let mut hardware = false;
        let mut last = String::new();
        for hw in [gpu.as_ref(), None] {
            if hw.is_none() && hardware { break; }
            let r = match reader(path, hw) { Ok(r) => r, Err(e) => { last = e; continue } };
            let (v, a) = streams(&r);
            let _ = r.SetStreamSelection(ALL, false);
            let mut ok_v = None;
            if let Some(i) = v { if r.SetStreamSelection(i, true).is_ok() && set_video(&r, i, max).is_ok() { ok_v = Some(i); } else { let _ = r.SetStreamSelection(i, false); } }
            if v.is_some() && ok_v.is_none() && hw.is_some() { last = "no decoder on the GPU".into(); continue; } // try software
            let mut ok_a = None;
            if with_audio { if let Some(i) = a { if r.SetStreamSelection(i, true).is_ok() && set_audio(&r, i).is_ok() { ok_a = Some(i); } else { let _ = r.SetStreamSelection(i, false); } } }
            if ok_v.is_none() && ok_a.is_none() { last = if v.is_some() { "Media Foundation: no decoder for this video".into() } else { "Media Foundation: no video in the file".into() }; break; }
            mf.reader = Some(r);
            mf.video = ok_v;
            mf.audio = ok_a;
            hardware = hw.is_some();
            break;
        }
        let Some(r) = mf.reader.clone() else { return Err(if last.is_empty() { "Media Foundation: cannot open the file".into() } else { last }) };
        mf.read_types();
        let duration = r.GetPresentationAttribute(MEDIASOURCE, &MF_PD_DURATION).ok().and_then(|v| u64::try_from(&v).ok()).map_or(0.0, |d| d as f64 / 1e7);
        let info = Info { duration, width: mf.w as u32, height: mf.h as u32, has_video: mf.video.is_some(), has_audio: mf.audio.is_some(), hardware, ready: true, error: None };
        Ok((Box::new(mf), info))
    }
}
