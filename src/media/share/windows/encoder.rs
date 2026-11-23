//! Media Foundation H.264 encoder MFT: the GPU's (asynchronous, event driven) when one is registered and takes NV12 from
//! system memory, else Microsoft's software encoder (synchronous). Baseline profile, no B-frames, low-latency mode, CBR at
//! the requested rate, a key frame every `gop` pictures and on request (`CODECAPI_AVEncVideoForceKeyFrame`).
//! `WEBOX_SHARE_SOFTWARE=1` skips the hardware encoders.
use std::mem::ManuallyDrop;
use std::time::{Duration, Instant};
use windows::core::{Interface, GUID, VARIANT};
use windows::Win32::Media::MediaFoundation::*;

fn err(e: windows::core::Error) -> String { format!("Media Foundation: {} (0x{:08X})", e.message(), e.code().0) }

pub(crate) struct Encoder {
    mft: IMFTransform,
    codec: Option<ICodecAPI>,
    /// Asynchronous (hardware) MFTs report through events.
    events: Option<IMFMediaEventGenerator>,
    need: u32,
    provides: bool,
    out_size: u32,
    w: u32,
    h: u32,
    bitrate: u32,
    frame_dur: i64,
    pub name: String,
}

/// The registered H.264 encoders for NV12 input, best first.
unsafe fn candidates(hardware: bool) -> Vec<(IMFActivate, String)> {
    let input = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_NV12 };
    let output = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_H264 };
    let flags = if hardware { MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER } else { MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER };
    let (mut p, mut n): (*mut Option<IMFActivate>, u32) = (std::ptr::null_mut(), 0);
    let r = MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER, flags, Some(&input), Some(&output), &mut p, &mut n);
    if r.is_err() || p.is_null() { return vec![]; }
    let mut v = vec![];
    for i in 0..n as usize {
        if let Some(a) = std::ptr::read(p.add(i)) {
            let mut s = windows::core::PWSTR::null();
            let mut len = 0u32;
            let name = if a.GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut s, &mut len).is_ok() {
                let t = s.to_string().unwrap_or_default();
                windows::Win32::System::Com::CoTaskMemFree(Some(s.0 as _));
                t
            } else { String::new() };
            v.push((a, name));
        }
    }
    windows::Win32::System::Com::CoTaskMemFree(Some(p as _));
    v
}

unsafe fn video_type(sub: &GUID, w: u32, h: u32, fps: u32) -> Result<IMFMediaType, String> {
    let t = MFCreateMediaType().map_err(err)?;
    t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(err)?;
    t.SetGUID(&MF_MT_SUBTYPE, sub).map_err(err)?;
    t.SetUINT64(&MF_MT_FRAME_SIZE, ((w as u64) << 32) | h as u64).map_err(err)?;
    t.SetUINT64(&MF_MT_FRAME_RATE, ((fps as u64) << 32) | 1).map_err(err)?;
    t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1).map_err(err)?;
    t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32).map_err(err)?;
    Ok(t)
}

fn set(codec: &ICodecAPI, api: &GUID, v: u32) -> bool { unsafe { codec.SetValue(api, &VARIANT::from(v)).is_ok() } }

impl Encoder {
    /// A configured encoder for `w` x `h` NV12 pictures.
    /// `hardware` false: the software encoder only (after a hardware one failed on real input).
    pub fn new(w: u32, h: u32, fps: u32, bitrate: u32, gop: u32, hardware: bool) -> Result<Encoder, String> {
        let mut last = "Media Foundation: no H.264 encoder is installed".to_string();
        let mut gpu_err = String::new();
        let hw = hardware && std::env::var_os("WEBOX_SHARE_SOFTWARE").is_none();
        unsafe {
            for hardware in [true, false] {
                if hardware && !hw { continue; }
                for (a, name) in candidates(hardware) {
                    match Self::open(&a, w, h, fps, bitrate, gop, hardware) {
                        Ok(mut e) => {
                            e.name = format!("Media Foundation ({}): {name}", if hardware { "hardware" } else { "software" });
                            if !gpu_err.is_empty() { e.name += &format!(" (GPU encoder: {gpu_err})"); }
                            return Ok(e);
                        }
                        Err(x) => { let _ = a.ShutdownObject(); last = format!("{name}: {x}"); if hardware { gpu_err = last.clone(); } }
                    }
                }
            }
        }
        Err(last)
    }

    pub fn hardware(&self) -> bool { self.events.is_some() }

    /// The encoder takes key frame requests (else the caller starts a new encoder, whose first picture is one).
    pub fn can_force_key(&self) -> bool { self.codec.as_ref().is_some_and(|c| unsafe { c.IsSupported(&CODECAPI_AVEncVideoForceKeyFrame).is_ok() }) }

    #[allow(clippy::too_many_arguments)]
    unsafe fn open(a: &IMFActivate, w: u32, h: u32, fps: u32, bitrate: u32, gop: u32, hardware: bool) -> Result<Encoder, String> {
        let mft: IMFTransform = a.ActivateObject().map_err(err)?;
        let mut events = None;
        if let Ok(attrs) = mft.GetAttributes() {
            let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
            if hardware { let _ = attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1); }
        }
        if hardware { events = mft.cast::<IMFMediaEventGenerator>().ok(); }
        let codec = mft.cast::<ICodecAPI>().ok();
        // rate control before the types: some encoders read it only then
        if let Some(c) = &codec {
            set(c, &CODECAPI_AVEncCommonRateControlMode, eAVEncCommonRateControlMode_CBR.0 as u32);
            set(c, &CODECAPI_AVEncCommonMeanBitRate, bitrate);
            let _ = c.SetValue(&CODECAPI_AVLowLatencyMode, &VARIANT::from(true)); // VT_BOOL
            set(c, &CODECAPI_AVEncMPVDefaultBPictureCount, 0);
            if gop > 0 { set(c, &CODECAPI_AVEncMPVGOPSize, gop); }
        }
        let out = video_type(&MFVideoFormat_H264, w, h, fps)?;
        out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate).map_err(err)?;
        let mut ok = false;
        for profile in [eAVEncH264VProfile_ConstrainedBase.0, eAVEncH264VProfile_Base.0] {
            out.SetUINT32(&MF_MT_MPEG2_PROFILE, profile as u32).map_err(err)?;
            if mft.SetOutputType(0, &out, 0).is_ok() { ok = true; break; }
        }
        if !ok { return Err("the encoder takes no Baseline output".into()); }
        let input = video_type(&MFVideoFormat_NV12, w, h, fps)?;
        mft.SetInputType(0, &input, 0).map_err(err)?;
        if let Some(c) = &codec {
            set(c, &CODECAPI_AVEncCommonMeanBitRate, bitrate);
            if gop > 0 { set(c, &CODECAPI_AVEncMPVGOPSize, gop); }
        }
        let info = mft.GetOutputStreamInfo(0).map_err(err)?;
        let provides = info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32) != 0;
        mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0).map_err(err)?;
        mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0).map_err(err)?;
        Ok(Encoder { mft, codec, events, need: 0, provides, out_size: info.cbSize.max(w * h * 3 / 2), w, h, bitrate, frame_dur: 10_000_000 / fps.max(1) as i64, name: String::new() })
    }

    pub fn size(&self) -> (u32, u32) { (self.w, self.h) }

    pub fn set_bitrate(&mut self, bps: u32) {
        if bps == self.bitrate { return; }
        self.bitrate = bps;
        if let Some(c) = &self.codec { set(c, &CODECAPI_AVEncCommonMeanBitRate, bps); }
    }

    /// Encodes one NV12 picture (`pts` in 100 ns units); returns the access units that came out (zero or more) with their times.
    /// `key`: force a key frame; false when the encoder cannot (the caller then makes a new encoder).
    pub fn encode(&mut self, nv12: &[u8], pts: i64, key: bool) -> Result<Vec<(Vec<u8>, i64)>, String> {
        unsafe {
            if key { if let Some(c) = &self.codec { set(c, &CODECAPI_AVEncVideoForceKeyFrame, 1); } }
            let sample = self.sample(nv12, pts)?;
            let mut out = vec![];
            if self.events.is_some() {
                // asynchronous: wait for METransformNeedInput, collect what is ready, then wait briefly for this picture
                let t0 = Instant::now();
                while self.need == 0 {
                    self.pump(&mut out)?;
                    if self.need == 0 { if t0.elapsed() > Duration::from_millis(500) { return Err("the encoder takes no input".into()); } std::thread::sleep(Duration::from_millis(1)); }
                }
                self.need -= 1;
                self.mft.ProcessInput(0, &sample, 0).map_err(err)?;
                // the picture usually comes out within a few ms; later ones are collected by `poll`
                let t0 = Instant::now();
                while out.is_empty() && t0.elapsed() < Duration::from_millis(8) { self.pump(&mut out)?; if out.is_empty() { std::thread::sleep(Duration::from_millis(1)); } }
            } else {
                self.mft.ProcessInput(0, &sample, 0).map_err(err)?;
                while let Some(au) = self.pull()? { out.push(au); }
            }
            Ok(out)
        }
    }

    /// Pictures an asynchronous encoder finished since the last call (nothing for a synchronous one).
    pub fn poll(&mut self) -> Result<Vec<(Vec<u8>, i64)>, String> {
        let mut out = vec![];
        unsafe { self.pump(&mut out)?; }
        Ok(out)
    }

    /// Handles the pending events of an asynchronous encoder.
    unsafe fn pump(&mut self, out: &mut Vec<(Vec<u8>, i64)>) -> Result<(), String> {
        let Some(ev) = self.events.clone() else { return Ok(()) };
        loop {
            let e = match ev.GetEvent(MF_EVENT_FLAG_NO_WAIT) { Ok(e) => e, Err(e) if e.code() == MF_E_NO_EVENTS_AVAILABLE => return Ok(()), Err(e) => return Err(err(e)) };
            match e.GetType().map_err(err)? {
                t if t == METransformNeedInput.0 as u32 => self.need += 1,
                t if t == METransformHaveOutput.0 as u32 => { if let Some(au) = self.pull()? { out.push(au); } }
                _ => {}
            }
        }
    }

    unsafe fn sample(&self, nv12: &[u8], pts: i64) -> Result<IMFSample, String> {
        let b = MFCreateMemoryBuffer(nv12.len() as u32).map_err(err)?;
        let mut p = std::ptr::null_mut();
        b.Lock(&mut p, None, None).map_err(err)?;
        std::ptr::copy_nonoverlapping(nv12.as_ptr(), p, nv12.len());
        b.Unlock().map_err(err)?;
        b.SetCurrentLength(nv12.len() as u32).map_err(err)?;
        let s = MFCreateSample().map_err(err)?;
        s.AddBuffer(&b).map_err(err)?;
        s.SetSampleTime(pts).map_err(err)?;
        s.SetSampleDuration(self.frame_dur).map_err(err)?;
        Ok(s)
    }

    /// One output sample's bytes and time, None when the encoder needs more input. An asynchronous encoder is asked exactly
    /// once per METransformHaveOutput (a second call fails with E_UNEXPECTED).
    unsafe fn pull(&mut self) -> Result<Option<(Vec<u8>, i64)>, String> {
        for _ in 0..if self.events.is_some() { 1 } else { 4 } {
            let mine = if self.provides { None } else {
                let s = MFCreateSample().map_err(err)?;
                s.AddBuffer(&MFCreateMemoryBuffer(self.out_size).map_err(err)?).map_err(err)?;
                Some(s)
            };
            let mut buf = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: ManuallyDrop::new(mine), dwStatus: 0, pEvents: ManuallyDrop::new(None) }];
            let mut status = 0u32;
            let r = self.mft.ProcessOutput(0, &mut buf, &mut status);
            let sample = ManuallyDrop::take(&mut buf[0].pSample);
            drop(ManuallyDrop::take(&mut buf[0].pEvents));
            match r {
                Ok(()) => {
                    let Some(s) = sample else { return Ok(None) };
                    let b = s.ConvertToContiguousBuffer().map_err(err)?;
                    let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
                    b.Lock(&mut p, None, Some(&mut len)).map_err(err)?;
                    let v = if p.is_null() { vec![] } else { std::slice::from_raw_parts(p, len as usize).to_vec() };
                    let _ = b.Unlock();
                    return Ok(Some((v, s.GetSampleTime().unwrap_or(0))));
                }
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    // the encoder settled its output type (e.g. after the first picture): take it and ask again
                    let t = self.mft.GetOutputAvailableType(0, 0).map_err(err)?;
                    self.mft.SetOutputType(0, &t, 0).map_err(err)?;
                    if let Ok(i) = self.mft.GetOutputStreamInfo(0) { self.out_size = i.cbSize.max(self.out_size); }
                }
                Err(e) => return Err(err(e)),
            }
        }
        Ok(None)
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            let _ = self.mft.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            if let Some(ev) = self.events.take() { if let Ok(s) = ev.cast::<IMFShutdown>() { let _ = s.Shutdown(); } }
        }
    }
}
