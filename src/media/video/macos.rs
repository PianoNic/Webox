//! macOS: AVFoundation. An `AVAssetReader` per playback position with one track output per stream: video as 32BGRA, scaled by the
//! reader to at most `MAX_SIZE` (aspect kept) and turned into RGBA here, sound as interleaved f32 PCM at the file's rate. The two
//! outputs are read in time order (the earlier of the next picture and the next block of sound first). VideoToolbox decodes on
//! the GPU where it can. A seek starts a new reader at the target (`timeRange`), which decodes from the key frame before it.
//!
//! `write` makes the test / sample videos: H.264 + AAC in MP4 with `AVAssetWriter`.
#![allow(deprecated, unused_unsafe)] // tracksWithMediaType / the CM free functions: the synchronous forms, on a worker thread
use super::sync::Frame;
use super::{Info, Packet, Source};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_av_foundation::{
    AVAssetReader, AVAssetReaderOutput, AVAssetReaderStatus, AVAssetReaderTrackOutput, AVAssetTrack, AVAssetWriter,
    AVAssetWriterInput, AVAssetWriterInputPixelBufferAdaptor, AVAssetWriterStatus, AVFileTypeMPEG4, AVMediaTypeAudio, AVMediaTypeVideo,
    AVURLAsset, AVVideoCodecKey, AVVideoCodecTypeH264, AVVideoHeightKey, AVVideoWidthKey,
};
use objc2_core_audio_types::AudioStreamBasicDescription;
use objc2_core_foundation::{CFRetained, CFString};
use objc2_core_media::{
    kCMTimePositiveInfinity, kCMTimeZero, CMAudioFormatDescriptionCreate, CMAudioFormatDescriptionGetStreamBasicDescription,
    CMAudioSampleBufferCreateReadyWithPacketDescriptions, CMBlockBuffer, CMBlockBufferCreateWithMemoryBlock,
    CMBlockBufferReplaceDataBytes, CMFormatDescription, CMSampleBuffer, CMTime, CMTimeRange,
};
use objc2_core_video::{
    kCVPixelBufferHeightKey, kCVPixelBufferPixelFormatTypeKey, kCVPixelBufferWidthKey, kCVPixelFormatType_32BGRA, CVPixelBuffer,
    CVPixelBufferCreate, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetWidth,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};
use std::path::Path;
use std::ptr::NonNull;

/// `kAudioFormatLinearPCM` / `kAudioFormatMPEG4AAC`.
const LPCM: u32 = 0x6C70_636D;
const AAC: u32 = 0x6161_6320;

/// A CoreFoundation string key as the NSString it is (toll-free bridged).
fn ns(s: &CFString) -> &NSString { unsafe { &*(s as *const CFString as *const NSString) } }

fn dict(pairs: &[(&NSString, Retained<NSNumber>)]) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys: Vec<&NSString> = pairs.iter().map(|p| p.0).collect();
    let vals: Vec<&AnyObject> = pairs.iter().map(|p| { let o: &AnyObject = &p.1; o }).collect();
    NSDictionary::from_slices(&keys, &vals)
}

/// The AVFAudio settings keys (`AVFormatIDKey`, ...): NSStrings whose value is their own name.
fn key(name: &str) -> Retained<NSString> { NSString::from_str(name) }

/// (w, h) scaled into `max` keeping the ratio, never enlarged.
fn fit(w: f64, h: f64, max: (u32, u32)) -> (u32, u32) {
    if w < 1.0 || h < 1.0 { return (0, 0); }
    let k = (max.0 as f64 / w).min(max.1 as f64 / h).min(1.0);
    (((w * k).round() as u32).max(1), ((h * k).round() as u32).max(1))
}

/// The 32BGRA picture of a pixel buffer as tightly packed RGBA.
pub(crate) fn bgra_to_rgba(pb: &CVPixelBuffer) -> Option<(usize, usize, Vec<u8>)> {
    unsafe {
        if CVPixelBufferLockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly) != 0 { return None; }
        let (w, h, stride) = (CVPixelBufferGetWidth(pb), CVPixelBufferGetHeight(pb), CVPixelBufferGetBytesPerRow(pb));
        let base = CVPixelBufferGetBaseAddress(pb) as *const u8;
        let out = (!base.is_null() && w > 0 && h > 0 && stride >= w * 4).then(|| {
            let src = std::slice::from_raw_parts(base, stride * (h - 1) + w * 4);
            let mut out = vec![0u8; w * h * 4];
            for y in 0..h {
                for (d, s) in out[y * w * 4..(y + 1) * w * 4].chunks_exact_mut(4).zip(src[y * stride..y * stride + w * 4].chunks_exact(4)) {
                    d.copy_from_slice(&[s[2], s[1], s[0], 255]);
                }
            }
            (w, h, out)
        });
        CVPixelBufferUnlockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly);
        out
    }
}

/// The file's bytes are a container `format::sniff` knows: AVFoundation would also follow a playlist (HLS) to other URLs.
fn known_container(path: &Path) -> Result<(), String> {
    use std::io::Read;
    let mut head = vec![0u8; 4096];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).map_err(|e| e.to_string())?;
    head.truncate(n);
    super::format::sniff(&head).map(|_| ()).ok_or_else(|| "not a known video container".to_string())
}

struct Avf {
    asset: Retained<AVURLAsset>,
    video: Option<Retained<AVAssetTrack>>,
    audio: Option<Retained<AVAssetTrack>>,
    size: (u32, u32),
    reader: Retained<AVAssetReader>,
    vout: Option<Retained<AVAssetReaderTrackOutput>>,
    aout: Option<Retained<AVAssetReaderTrackOutput>>,
    /// The next sample of each output, read ahead to pick the earlier one.
    vnext: Option<Packet>,
    anext: Option<Packet>,
    v_eos: bool,
    a_eos: bool,
}

pub(super) fn open(path: &Path, max: (u32, u32), with_audio: bool) -> Result<(Box<dyn Source>, Info), String> {
    known_container(path)?;
    let abs = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    unsafe {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&abs.to_string_lossy()));
        let opts = dict(&[(&key("AVURLAssetPreferPreciseDurationAndTimingKey"), NSNumber::new_bool(true))]);
        let asset = AVURLAsset::URLAssetWithURL_options(&url, Some(&opts));
        let first = |t: Option<&'static NSString>| t.and_then(|t| asset.tracksWithMediaType(t).firstObject());
        let video = first(AVMediaTypeVideo);
        let audio = if with_audio { first(AVMediaTypeAudio) } else { None };
        if video.is_none() && audio.is_none() { return Err("no audio or video track (or a format AVFoundation cannot read)".into()); }
        let size = video.as_ref().map_or((0, 0), |t| { let s = t.naturalSize(); fit(s.width.abs(), s.height.abs(), max) });
        let duration = { let d = asset.duration().seconds(); if d.is_finite() { d } else { 0.0 } };
        let mut s = Avf { reader: reader(&asset, None)?, asset, video, audio, size, vout: None, aout: None, vnext: None, anext: None, v_eos: false, a_eos: false };
        s.start(0.0)?;
        let info = Info { duration, width: size.0, height: size.1, has_video: s.video.is_some(), has_audio: s.audio.is_some(), hardware: false, ready: true, error: None };
        Ok((Box::new(s), info))
    }
}

fn reader(asset: &AVURLAsset, _: Option<()>) -> Result<Retained<AVAssetReader>, String> {
    unsafe { AVAssetReader::assetReaderWithAsset_error(asset).map_err(|e| e.localizedDescription().to_string()) }
}

impl Avf {
    /// A new reader from `at` seconds to the end.
    fn start(&mut self, at: f64) -> Result<(), String> {
        unsafe {
            self.reader.cancelReading();
            let r = reader(&self.asset, None)?;
            if at > 0.0 { r.setTimeRange(CMTimeRange { start: CMTime::with_seconds(at, 600), duration: kCMTimePositiveInfinity }); }
            self.vout = None;
            self.aout = None;
            if let Some(t) = &self.video {
                let mut v = vec![(ns(kCVPixelBufferPixelFormatTypeKey), NSNumber::new_u32(kCVPixelFormatType_32BGRA))];
                if self.size.0 > 0 { v.push((ns(kCVPixelBufferWidthKey), NSNumber::new_u32(self.size.0))); v.push((ns(kCVPixelBufferHeightKey), NSNumber::new_u32(self.size.1))); }
                let o = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(t, Some(&dict(&v)));
                o.setAlwaysCopiesSampleData(false);
                if r.canAddOutput(&o) { r.addOutput(&o); self.vout = Some(o); }
            }
            if let Some(t) = &self.audio {
                let (f, i, b, n, e) = (key("AVFormatIDKey"), key("AVLinearPCMIsFloatKey"), key("AVLinearPCMBitDepthKey"), key("AVLinearPCMIsNonInterleaved"), key("AVLinearPCMIsBigEndianKey"));
                let s = dict(&[(&f, NSNumber::new_u32(LPCM)), (&i, NSNumber::new_bool(true)), (&b, NSNumber::new_u32(32)), (&n, NSNumber::new_bool(false)), (&e, NSNumber::new_bool(false))]);
                let o = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(t, Some(&s));
                if r.canAddOutput(&o) { r.addOutput(&o); self.aout = Some(o); }
            }
            if !r.startReading() {
                return Err(r.error().map_or("AVAssetReader could not start".into(), |e| e.localizedDescription().to_string()));
            }
            self.reader = r;
            self.vnext = None; self.anext = None;
            self.v_eos = self.vout.is_none();
            self.a_eos = self.aout.is_none();
            Ok(())
        }
    }

    fn failed(&self) -> Option<String> {
        unsafe { (self.reader.status() == AVAssetReaderStatus::Failed).then(|| self.reader.error().map_or("decode error".into(), |e| e.localizedDescription().to_string())) }
    }

    /// The next picture, None at the end.
    fn next_video(&mut self) -> Result<Option<Packet>, String> {
        let Some(o) = &self.vout else { return Ok(None) };
        loop {
            let Some(sb) = (unsafe { AsRef::<AVAssetReaderOutput>::as_ref(&**o).copyNextSampleBuffer() }) else { return self.failed().map_or(Ok(None), Err) };
            let pts = unsafe { sb.output_presentation_time_stamp().seconds() };
            let Some(pb) = (unsafe { sb.image_buffer() }) else { continue }; // a marker sample without a picture
            let Some((w, h, rgba)) = bgra_to_rgba(&pb) else { continue };
            return Ok(Some(Packet::Video(Frame { pts: if pts.is_finite() { pts } else { 0.0 }, w, h, rgba })));
        }
    }

    /// The next block of sound, None at the end.
    fn next_audio(&mut self) -> Result<Option<Packet>, String> {
        let Some(o) = &self.aout else { return Ok(None) };
        loop {
            let Some(sb) = (unsafe { AsRef::<AVAssetReaderOutput>::as_ref(&**o).copyNextSampleBuffer() }) else { return self.failed().map_or(Ok(None), Err) };
            if let Some(p) = audio_packet(&sb) { return Ok(Some(p)); }
        }
    }
}

fn audio_packet(sb: &CMSampleBuffer) -> Option<Packet> {
    unsafe {
        let fmt = sb.format_description()?;
        let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(&fmt).as_ref()?;
        let (rate, ch) = (asbd.mSampleRate.round() as u32, asbd.mChannelsPerFrame as u16);
        let bb = sb.data_buffer()?;
        let len = bb.data_length();
        if rate == 0 || ch == 0 || len < 4 { return None; }
        let mut data = vec![0f32; len / 4];
        if bb.copy_data_bytes(0, data.len() * 4, NonNull::new(data.as_mut_ptr() as *mut _)?) != 0 { return None; }
        let pts = sb.output_presentation_time_stamp().seconds();
        Some(Packet::Audio { pts: if pts.is_finite() { pts } else { 0.0 }, rate, ch, data })
    }
}

fn pts(p: &Packet) -> f64 { match p { Packet::Video(f) => f.pts, Packet::Audio { pts, .. } => *pts } }

impl Source for Avf {
    fn read(&mut self) -> Result<Option<Packet>, String> {
        if self.vnext.is_none() && !self.v_eos { self.vnext = self.next_video()?; self.v_eos = self.vnext.is_none(); }
        if self.anext.is_none() && !self.a_eos { self.anext = self.next_audio()?; self.a_eos = self.anext.is_none(); }
        let video_first = match (&self.vnext, &self.anext) { (Some(v), Some(a)) => pts(v) <= pts(a), (Some(_), None) => true, _ => false };
        Ok(if video_first { self.vnext.take() } else { self.anext.take() })
    }

    fn seek(&mut self, secs: f64) -> Result<(), String> { self.start(secs.max(0.0)) }
}

impl Drop for Avf {
    fn drop(&mut self) { unsafe { self.reader.cancelReading(); } }
}

/// Writes `spec.frames` pictures from `paint(i, rgba)` (H.264) and a 440 Hz stereo tone at `spec.rate` (AAC) into an MP4.
pub(super) fn write(path: &Path, spec: super::synth::Spec, paint: &dyn Fn(u32, &mut [u8])) -> Result<(), String> {
    let super::synth::Spec { w, h, fps, frames, rate } = spec;
    let _ = std::fs::remove_file(path); // the writer refuses an existing file
    unsafe {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let ft = AVFileTypeMPEG4.ok_or("AVFileTypeMPEG4")?;
        let writer = AVAssetWriter::assetWriterWithURL_fileType_error(&url, ft).map_err(|e| e.localizedDescription().to_string())?;

        let vs = {
            let keys = [AVVideoCodecKey.ok_or("AVVideoCodecKey")?, AVVideoWidthKey.ok_or("AVVideoWidthKey")?, AVVideoHeightKey.ok_or("AVVideoHeightKey")?];
            let (w_n, h_n) = (NSNumber::new_u32(w), NSNumber::new_u32(h));
            let vals: [&AnyObject; 3] = [AVVideoCodecTypeH264.ok_or("H.264")?, &w_n, &h_n];
            NSDictionary::<NSString, AnyObject>::from_slices(&keys, &vals)
        };
        let vin = AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(AVMediaTypeVideo.ok_or("video")?, Some(&vs));
        vin.setExpectsMediaDataInRealTime(false);
        let attrs = dict(&[(ns(kCVPixelBufferPixelFormatTypeKey), NSNumber::new_u32(kCVPixelFormatType_32BGRA))]);
        let adaptor = AVAssetWriterInputPixelBufferAdaptor::assetWriterInputPixelBufferAdaptorWithAssetWriterInput_sourcePixelBufferAttributes(&vin, Some(&attrs));

        let (f, r, c, b) = (key("AVFormatIDKey"), key("AVSampleRateKey"), key("AVNumberOfChannelsKey"), key("AVEncoderBitRateKey"));
        let as_ = dict(&[(&f, NSNumber::new_u32(AAC)), (&r, NSNumber::new_f64(rate as f64)), (&c, NSNumber::new_u32(2)), (&b, NSNumber::new_u32(128_000))]);
        let ain = AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(AVMediaTypeAudio.ok_or("audio")?, Some(&as_));
        ain.setExpectsMediaDataInRealTime(false);
        if !writer.canAddInput(&vin) || !writer.canAddInput(&ain) { return Err("AVAssetWriter refuses the inputs".into()); }
        writer.addInput(&vin);
        writer.addInput(&ain);
        let err = |wr: &AVAssetWriter| wr.error().map_or("AVAssetWriter failed".into(), |e| e.localizedDescription().to_string());
        if !writer.startWriting() { return Err(err(&writer)); }
        writer.startSessionAtSourceTime(kCMTimeZero);

        let wait = |i: &AVAssetWriterInput, wr: &AVAssetWriter| -> Result<(), String> {
            for _ in 0..2000 {
                if i.isReadyForMoreMediaData() { return Ok(()); }
                if wr.status() == AVAssetWriterStatus::Failed { return Err(err(wr)); }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err("AVAssetWriter input not ready".into())
        };

        // Pictures and sound interleaved per frame, so neither input waits for the other.
        let fmt = audio_format(rate)?;
        let per_frame = (rate / fps.max(1)) as usize;
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        let mut done_samples = 0usize;
        for i in 0..frames {
            paint(i, &mut rgba);
            let pb = bgra_buffer(w, h, &rgba)?;
            wait(&vin, &writer)?;
            if !adaptor.appendPixelBuffer_withPresentationTime(&pb, CMTime::new(i as i64, fps as i32)) { return Err(err(&writer)); }
            let mut pcm = Vec::with_capacity(per_frame * 2);
            for n in done_samples..done_samples + per_frame {
                let v = (2.0 * std::f32::consts::PI * 440.0 * n as f32 / rate as f32).sin() * 0.5;
                pcm.extend([v, v]);
            }
            let sb = audio_buffer(&fmt, &pcm, CMTime::new(done_samples as i64, rate as i32))?;
            wait(&ain, &writer)?;
            if !ain.appendSampleBuffer(&sb) { return Err(err(&writer)); }
            done_samples += per_frame;
        }
        vin.markAsFinished();
        ain.markAsFinished();
        if !writer.finishWriting() || writer.status() != AVAssetWriterStatus::Completed { return Err(err(&writer)); }
        Ok(())
    }
}

/// Interleaved stereo f32 PCM at `rate`.
fn audio_format(rate: u32) -> Result<CFRetained<CMFormatDescription>, String> {
    let asbd = AudioStreamBasicDescription {
        mSampleRate: rate as f64, mFormatID: LPCM, mFormatFlags: 1 | 8, // kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked
        mBytesPerPacket: 8, mFramesPerPacket: 1, mBytesPerFrame: 8, mChannelsPerFrame: 2, mBitsPerChannel: 32, mReserved: 0,
    };
    let mut out: *const CMFormatDescription = std::ptr::null();
    let st = unsafe { CMAudioFormatDescriptionCreate(None, NonNull::from(&asbd), 0, std::ptr::null(), 0, std::ptr::null(), None, NonNull::from(&mut out)) };
    NonNull::new(out as *mut CMFormatDescription).filter(|_| st == 0).map(|p| unsafe { CFRetained::from_raw(p) }).ok_or_else(|| format!("audio format ({st})"))
}

fn audio_buffer(fmt: &CMFormatDescription, pcm: &[f32], at: CMTime) -> Result<CFRetained<CMSampleBuffer>, String> {
    unsafe {
        let len = pcm.len() * 4;
        let mut bb: *mut CMBlockBuffer = std::ptr::null_mut();
        let st = CMBlockBufferCreateWithMemoryBlock(None, std::ptr::null_mut(), len, None, std::ptr::null(), 0, len, 1, NonNull::from(&mut bb)); // kCMBlockBufferAssureMemoryNowFlag
        let bb = NonNull::new(bb).filter(|_| st == 0).map(|p| CFRetained::from_raw(p)).ok_or_else(|| format!("block buffer ({st})"))?;
        let st = CMBlockBufferReplaceDataBytes(NonNull::new(pcm.as_ptr() as *mut _).ok_or("pcm")?, &bb, 0, len);
        if st != 0 { return Err(format!("block buffer copy ({st})")); }
        let mut sb: *mut CMSampleBuffer = std::ptr::null_mut();
        let st = CMAudioSampleBufferCreateReadyWithPacketDescriptions(None, &bb, fmt, (pcm.len() / 2) as isize, at, std::ptr::null(), NonNull::from(&mut sb));
        NonNull::new(sb).filter(|_| st == 0).map(|p| CFRetained::from_raw(p)).ok_or_else(|| format!("audio sample buffer ({st})"))
    }
}

/// A 32BGRA pixel buffer from an RGBA picture.
fn bgra_buffer(w: u32, h: u32, rgba: &[u8]) -> Result<CFRetained<CVPixelBuffer>, String> {
    unsafe {
        let mut pb: *mut CVPixelBuffer = std::ptr::null_mut();
        let st = CVPixelBufferCreate(None, w as usize, h as usize, kCVPixelFormatType_32BGRA, None, NonNull::from(&mut pb));
        let pb = NonNull::new(pb).filter(|_| st == 0).map(|p| CFRetained::from_raw(p)).ok_or_else(|| format!("pixel buffer ({st})"))?;
        if CVPixelBufferLockBaseAddress(&pb, CVPixelBufferLockFlags(0)) != 0 { return Err("pixel buffer lock".into()); }
        let (stride, base) = (CVPixelBufferGetBytesPerRow(&pb), CVPixelBufferGetBaseAddress(&pb) as *mut u8);
        if !base.is_null() {
            let dst = std::slice::from_raw_parts_mut(base, stride * h as usize);
            for y in 0..h as usize {
                for (d, s) in dst[y * stride..y * stride + w as usize * 4].chunks_exact_mut(4).zip(rgba[y * w as usize * 4..(y + 1) * w as usize * 4].chunks_exact(4)) {
                    d.copy_from_slice(&[s[2], s[1], s[0], 255]);
                }
            }
        }
        CVPixelBufferUnlockBaseAddress(&pb, CVPixelBufferLockFlags(0));
        Ok(pb)
    }
}
