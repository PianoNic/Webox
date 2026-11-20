//! Windows cameras through Media Foundation: `MFEnumDeviceSources` (video capture sources) for the list, an
//! `IMFSourceReader` in synchronous mode for frames. The native mode is chosen by `pick_mode` (Max. Kameraauflösung);
//! uncompressed modes (NV12 / YUY2 / RGB32) are read as they are, MJPG is decoded to NV12 by the reader's decoder MFT,
//! which may run on the GPU when "Hardware-Beschleunigung aktivieren" is on (`MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS`).
//! Everything here runs on the preview's worker thread (COM MTA there).
use super::{pick_mode, CamDev, Fail, Mode, Pix, Raw, Source};
use windows::core::{GUID, HRESULT, PWSTR};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED};

const FIRST_VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

/// COM + Media Foundation for the calling thread, undone on drop.
struct MfInit { com: bool }
impl MfInit {
    fn new() -> Result<MfInit, Fail> {
        unsafe {
            let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_LITE) { if com { CoUninitialize(); } return Err(Fail::Other(format!("MFStartup: {e}"))); }
            Ok(MfInit { com })
        }
    }
}
impl Drop for MfInit { fn drop(&mut self) { unsafe { let _ = MFShutdown(); if self.com { CoUninitialize(); } } } }

/// The video capture sources (not activated yet).
unsafe fn activates() -> windows::core::Result<Vec<IMFActivate>> {
    let mut attrs: Option<IMFAttributes> = None;
    MFCreateAttributes(&mut attrs, 1)?;
    let attrs = attrs.ok_or_else(|| windows::core::Error::from(HRESULT(0x80004005u32 as i32)))?;
    attrs.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)?;
    let mut pp: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut n = 0u32;
    MFEnumDeviceSources(&attrs, &mut pp, &mut n)?;
    let mut v = Vec::with_capacity(n as usize);
    if !pp.is_null() {
        for i in 0..n as usize { if let Some(a) = (*pp.add(i)).take() { v.push(a); } }
        CoTaskMemFree(Some(pp as *const _));
    }
    Ok(v)
}

unsafe fn string(a: &IMFAttributes, key: &GUID) -> Option<String> {
    let mut p = PWSTR::null();
    let mut len = 0u32;
    a.GetAllocatedString(key, &mut p, &mut len).ok()?;
    let s = p.to_string().ok();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

fn dev_of(a: &IMFActivate) -> Option<CamDev> {
    unsafe {
        let name = string(a, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME)?;
        let id = string(a, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK).unwrap_or_else(|| name.clone());
        Some(CamDev { id, name })
    }
}

pub fn list() -> Vec<CamDev> {
    let Ok(_init) = MfInit::new() else { return vec![] };
    unsafe { activates().map(|v| v.iter().filter_map(dev_of).collect()).unwrap_or_default() }
}

pub(super) fn pix_of(sub: &GUID) -> Option<Pix> {
    if *sub == MFVideoFormat_NV12 { Some(Pix::Nv12) } else if *sub == MFVideoFormat_YUY2 { Some(Pix::Yuyv) }
    else if *sub == MFVideoFormat_MJPG { Some(Pix::Mjpg) } else if *sub == MFVideoFormat_RGB32 { Some(Pix::Bgra) } else { None }
}
/// `MF_MT_FRAME_SIZE` / `MF_MT_FRAME_RATE` pack two u32 into one u64 (high = width / numerator).
pub(super) fn split(v: u64) -> (u32, u32) { ((v >> 32) as u32, v as u32) }

unsafe fn mode_of(t: &IMFMediaType) -> Option<Mode> {
    let pix = pix_of(&t.GetGUID(&MF_MT_SUBTYPE).ok()?)?;
    let (w, h) = split(t.GetUINT64(&MF_MT_FRAME_SIZE).ok()?);
    let fps = t.GetUINT64(&MF_MT_FRAME_RATE).ok().map(split).map_or(30.0, |(n, d)| if d == 0 { 30.0 } else { n as f32 / d as f32 });
    Some(Mode { pix, w, h, fps })
}

fn fail(e: &windows::core::Error, name: &str) -> Fail {
    match e.code().0 as u32 {
        0x8007_0005 => Fail::Denied, // E_ACCESSDENIED: camera access is off in Windows privacy settings
        0xC00D_3704 | 0x8007_0020 | 0xC00D_3EA3 | 0x8007_00AA => Fail::InUse(name.into()), // HW MFT start failed, sharing violation, preempted, busy
        0xC00D_3EA2 | 0x8007_048F | 0xC00D_36B4 => Fail::Unplugged(name.into()), // device invalidated / not connected / shut down
        _ => Fail::Other(format!("{name}: {e}")),
    }
}

pub struct MfSource {
    reader: IMFSourceReader,
    src: IMFMediaSource,
    act: IMFActivate,
    mode: Mode,
    modes: Vec<Mode>,
    name: String,
    /// What the reader delivers (may differ from the native mode: MJPG -> NV12).
    out: Pix,
    w: u32,
    h: u32,
    stride: i32,
    _init: MfInit, // last: shut MF down after the COM objects above are released
}

pub fn open(dev: &CamDev, max_h: u32, hw: bool) -> Result<MfSource, Fail> {
    let init = MfInit::new()?;
    unsafe {
        let all = activates().map_err(|e| fail(&e, &dev.name))?;
        if all.is_empty() { return Err(Fail::NoCamera); }
        let devs: Vec<Option<CamDev>> = all.iter().map(dev_of).collect();
        let i = devs.iter().position(|d| d.as_ref().is_some_and(|d| d.id == dev.id))
            .or_else(|| devs.iter().position(|d| d.as_ref().is_some_and(|d| d.name == dev.name)))
            .ok_or_else(|| Fail::Unplugged(dev.name.clone()))?;
        let act = all[i].clone();
        let src: IMFMediaSource = act.ActivateObject().map_err(|e| fail(&e, &dev.name))?;
        let mut attrs: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attrs, 3).map_err(|e| fail(&e, &dev.name))?;
        let attrs = attrs.ok_or(Fail::Other("MFCreateAttributes".into()))?;
        let _ = attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, hw as u32);
        let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
        let reader = match MFCreateSourceReaderFromMediaSource(&src, &attrs) {
            Ok(r) => r,
            Err(e) => { let _ = src.Shutdown(); let _ = act.ShutdownObject(); return Err(fail(&e, &dev.name)); }
        };
        let mut natives: Vec<(Mode, IMFMediaType)> = vec![];
        let mut k = 0;
        while let Ok(t) = reader.GetNativeMediaType(FIRST_VIDEO, k) { if let Some(m) = mode_of(&t) { natives.push((m, t)); } k += 1; }
        let modes: Vec<Mode> = natives.iter().map(|n| n.0).collect();
        let mode = pick_mode(&modes, max_h).ok_or_else(|| Fail::Other(format!("{}: kein unterstütztes Videoformat", dev.name)))?;
        let native = &natives.iter().find(|n| n.0 == mode).ok_or(Fail::NoCamera)?.1;
        let mut s = MfSource { reader, src, act, mode, modes, name: dev.name.clone(), out: mode.pix, w: mode.w, h: mode.h, stride: 0, _init: init };
        s.reader.SetCurrentMediaType(FIRST_VIDEO, None, native).map_err(|e| fail(&e, &dev.name))?;
        if mode.pix == Pix::Mjpg {
            let t = MFCreateMediaType().map_err(|e| fail(&e, &dev.name))?;
            let set = t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).and_then(|_| t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12))
                .and_then(|_| t.SetUINT64(&MF_MT_FRAME_SIZE, (mode.w as u64) << 32 | mode.h as u64));
            set.and_then(|_| s.reader.SetCurrentMediaType(FIRST_VIDEO, None, &t)).map_err(|e| fail(&e, &dev.name))?;
        }
        let cur = s.reader.GetCurrentMediaType(FIRST_VIDEO).map_err(|e| fail(&e, &dev.name))?;
        if let Some(m) = mode_of(&cur) { s.out = m.pix; s.w = m.w; s.h = m.h; }
        s.stride = cur.GetUINT32(&MF_MT_DEFAULT_STRIDE).map_or(0, |v| v as i32);
        let _ = s.reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
        s.reader.SetStreamSelection(FIRST_VIDEO, true).map_err(|e| fail(&e, &dev.name))?;
        Ok(s)
    }
}

impl Drop for MfSource {
    fn drop(&mut self) { unsafe { let _ = self.src.Shutdown(); let _ = self.act.ShutdownObject(); } }
}

/// Builds a `Raw` from a locked buffer: works out stride and plane offset from the buffer length when the media type has no
/// default stride, and turns a bottom-up RGB32 frame (negative stride) the right way up.
pub(super) fn raw_from(pix: Pix, w: u32, h: u32, stride: i32, data: &[u8]) -> Option<Raw> {
    let (wu, hu, len) = (w as usize, h as usize, data.len());
    if wu == 0 || hu == 0 { return None; }
    match pix {
        Pix::Nv12 => {
            let s = if stride != 0 { stride.unsigned_abs() as usize } else { len * 2 / (hu * 3) };
            if s < wu || len < s * hu + s * (hu / 2) { return None; }
            Some(Raw { pix, w, h, stride: s, uv_off: len - s * (hu / 2), data: data.to_vec() })
        }
        Pix::Yuyv => {
            let s = if stride != 0 { stride.unsigned_abs() as usize } else { len / hu };
            if s < wu * 2 || len < s * hu { return None; }
            Some(Raw { pix, w, h, stride: s, uv_off: 0, data: data.to_vec() })
        }
        Pix::Bgra => {
            let s = if stride != 0 { stride.unsigned_abs() as usize } else { len / hu };
            if s < wu * 4 || len < s * hu { return None; }
            let d = if stride < 0 { data[..s * hu].chunks_exact(s).rev().flatten().copied().collect() } else { data.to_vec() };
            Some(Raw { pix, w, h, stride: s, uv_off: 0, data: d })
        }
        Pix::Mjpg | Pix::Rgb24 => Some(Raw { pix, w, h, stride: wu * 3, uv_off: 0, data: data.to_vec() }),
    }
}

impl Source for MfSource {
    fn mode(&self) -> Mode { self.mode }
    fn modes(&self) -> Vec<Mode> { self.modes.clone() }
    fn name(&self) -> String { self.name.clone() }
    fn next(&mut self) -> Result<Option<Raw>, Fail> {
        unsafe {
            let mut flags = 0u32;
            let mut sample: Option<IMFSample> = None;
            self.reader.ReadSample(FIRST_VIDEO, 0, None, Some(&mut flags), None, Some(&mut sample)).map_err(|e| fail(&e, &self.name))?;
            if flags & (MF_SOURCE_READERF_ERROR.0 as u32 | MF_SOURCE_READERF_ENDOFSTREAM.0 as u32) != 0 { return Err(Fail::Unplugged(self.name.clone())); }
            let Some(sample) = sample else { return Ok(None) };
            let buf = sample.ConvertToContiguousBuffer().map_err(|e| fail(&e, &self.name))?;
            let mut p: *mut u8 = std::ptr::null_mut();
            let mut len = 0u32;
            buf.Lock(&mut p, None, Some(&mut len)).map_err(|e| fail(&e, &self.name))?;
            let raw = if p.is_null() { None } else { raw_from(self.out, self.w, self.h, self.stride, std::slice::from_raw_parts(p, len as usize)) };
            let _ = buf.Unlock();
            Ok(raw)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_type_parsing() {
        assert_eq!(split((1920u64 << 32) | 1080), (1920, 1080));
        assert_eq!(split((30_000u64 << 32) | 1001), (30_000, 1001));
        assert_eq!(pix_of(&MFVideoFormat_MJPG), Some(Pix::Mjpg));
        assert_eq!(pix_of(&MFVideoFormat_NV12), Some(Pix::Nv12));
        assert_eq!(pix_of(&MFVideoFormat_YUY2), Some(Pix::Yuyv));
        assert_eq!(pix_of(&MFVideoFormat_RGB32), Some(Pix::Bgra));
        assert_eq!(pix_of(&MFMediaType_Video), None);
    }

    #[test]
    fn buffer_layouts() {
        // NV12 640x480 without a stride attribute: stride and UV offset from the length
        let r = raw_from(Pix::Nv12, 640, 480, 0, &vec![0; 640 * 480 * 3 / 2]).unwrap();
        assert_eq!((r.stride, r.uv_off), (640, 640 * 480));
        // 1080 rows padded to 1088 (some drivers): the UV plane is at the end
        let r = raw_from(Pix::Nv12, 1920, 1080, 1920, &vec![0; 1920 * 1088 * 3 / 2]).unwrap();
        assert_eq!(r.uv_off, 1920 * 1088 * 3 / 2 - 1920 * 540);
        assert!(raw_from(Pix::Nv12, 640, 480, 0, &[0; 100]).is_none());
        // bottom-up RGB32: the first row of the result is the last one in memory
        let mut d = vec![0u8; 2 * 4 * 2];
        d[8..16].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let r = raw_from(Pix::Bgra, 2, 2, -8, &d).unwrap();
        assert_eq!(&r.data[..8], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(raw_from(Pix::Yuyv, 4, 2, 0, &[0; 16]).unwrap().stride, 8);
    }

    /// Runs on the build machine: lists whatever cameras there are, without opening one.
    #[test]
    fn enumeration_does_not_fail() {
        let l = list();
        for d in &l { assert!(!d.name.is_empty() && !d.id.is_empty()); }
        eprintln!("cameras: {:?}", l.iter().map(|d| &d.name).collect::<Vec<_>>());
    }
}
