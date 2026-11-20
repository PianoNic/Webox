//! A video's sound on the speaker chosen in Settings > Audio (`audio_output`, "" = the system default; the same choice calls and
//! sounds use), through cpal on its own thread (cpal streams are not `Send`). The callback takes sample frames from the player's
//! queue, which advances the audio clock, resamples from the file's rate to the device's (linear) and maps the channels.
use super::Shared;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Starts the speaker for a stream of `rate` / `ch`; false when no output device could be opened.
pub(super) fn start(sh: Arc<Shared>, speaker: &str, rate: u32, ch: u16) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    let speaker = speaker.to_string();
    let spawned = std::thread::Builder::new().name("webox-video-audio".into()).spawn(move || {
        let stream = match open(&sh, &speaker, rate, ch) { Ok(s) => { let _ = tx.send(true); s } Err(e) => { eprintln!("video audio: {e}"); let _ = tx.send(false); return; } };
        while !sh.stop.load(Ordering::SeqCst) { std::thread::sleep(std::time::Duration::from_millis(50)); }
        drop(stream);
    });
    spawned.is_ok() && rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap_or(false)
}

/// The saved device if it is still there, else the default (as `media::audio::pick`).
fn device(name: &str) -> Option<cpal::Device> {
    let h = cpal::default_host();
    if !name.is_empty() {
        if let Some(d) = h.output_devices().ok().and_then(|mut it| it.find(|d| d.name().ok().as_deref() == Some(name))) { return Some(d); }
    }
    h.default_output_device()
}

/// Linear interpolation between whole sample frames, all channels at once.
pub(super) struct Resample { step: f64, pos: f64, a: Vec<f32>, b: Vec<f32>, primed: bool }
impl Resample {
    pub fn new(src_rate: u32, dst_rate: u32, ch: usize) -> Self { Resample { step: src_rate as f64 / dst_rate.max(1) as f64, pos: 0.0, a: vec![0.0; ch], b: vec![0.0; ch], primed: false } }
    /// One output frame into `out` (source channels); `next` fills the next source frame and returns false when there is none.
    pub fn next(&mut self, out: &mut [f32], next: &mut impl FnMut(&mut [f32]) -> bool) -> bool {
        if !self.primed { if !next(&mut self.b) { return false; } self.a.copy_from_slice(&self.b); self.primed = true; }
        while self.pos >= 1.0 {
            std::mem::swap(&mut self.a, &mut self.b);
            if !next(&mut self.b) { self.b.copy_from_slice(&self.a); self.pos -= 1.0; return false; }
            self.pos -= 1.0;
        }
        let k = self.pos as f32;
        for (o, (a, b)) in out.iter_mut().zip(self.a.iter().zip(&self.b)) { *o = a + (b - a) * k; }
        self.pos += self.step;
        true
    }
    pub fn reset(&mut self) { self.primed = false; self.pos = 0.0; }
}

/// Source channels -> device channels: equal counts pass through, mono goes to every speaker, more to fewer are mixed by
/// averaging the extra ones into the device's last channel (stereo -> mono: the mean), fewer to more leave the rest silent
/// except mono.
pub(super) fn map_channels(src: &[f32], dst: &mut [f32]) {
    let (s, d) = (src.len(), dst.len());
    if s == d { dst.copy_from_slice(src); return; }
    if s == 1 { dst.iter_mut().for_each(|x| *x = src[0]); return; }
    if d == 1 { dst[0] = src.iter().sum::<f32>() / s as f32; return; }
    if s < d { dst[..s].copy_from_slice(src); dst[s..].iter_mut().for_each(|x| *x = 0.0); return; }
    dst[..d - 1].copy_from_slice(&src[..d - 1]);
    dst[d - 1] = src[d - 1..].iter().sum::<f32>() / (s - d + 1) as f32;
}

fn open(sh: &Arc<Shared>, speaker: &str, rate: u32, ch: u16) -> Result<cpal::Stream, String> {
    let dev = device(speaker).ok_or("no output device")?;
    let cfg = dev.default_output_config().map_err(|e| e.to_string())?;
    let (dst_rate, dst_ch, fmt) = (cfg.sample_rate().0, cfg.channels() as usize, cfg.sample_format());
    let sh2 = sh.clone();
    let mut rs = Resample::new(rate, dst_rate, ch as usize);
    let (mut src_frame, mut dst_frame) = (vec![0.0f32; ch as usize], vec![0.0f32; dst_ch]);
    let mut fill = move |len: usize, put: &mut dyn FnMut(usize, f32)| {
        let (gain, playing) = (sh2.gain(), sh2.playing());
        let mut q = sh2.lock();
        for i in 0..len / dst_ch {
            let ok = playing && rs.next(&mut src_frame, &mut |f| q.pop_audio_frame(f));
            if ok { map_channels(&src_frame, &mut dst_frame) } else { dst_frame.iter_mut().for_each(|x| *x = 0.0) }
            for (c, v) in dst_frame.iter().enumerate() { put(i * dst_ch + c, v * gain); }
        }
        if q.audio.is_empty() { rs.reset(); }
        drop(q);
        sh2.cv.notify_all();
    };
    let err = |e| eprintln!("video audio: {e}");
    let cfg: cpal::StreamConfig = cfg.into();
    let st = match fmt {
        cpal::SampleFormat::F32 => dev.build_output_stream(&cfg, move |d: &mut [f32], _| { let n = d.len(); fill(n, &mut |i, v| d[i] = v) }, err, None),
        cpal::SampleFormat::I16 => dev.build_output_stream(&cfg, move |d: &mut [i16], _| { let n = d.len(); fill(n, &mut |i, v| d[i] = (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16) }, err, None),
        cpal::SampleFormat::U16 => dev.build_output_stream(&cfg, move |d: &mut [u16], _| { let n = d.len(); fill(n, &mut |i, v| d[i] = ((v.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16) }, err, None),
        f => return Err(format!("sample format {f:?}")),
    }.map_err(|e| e.to_string())?;
    st.play().map_err(|e| e.to_string())?;
    Ok(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_are_mapped() {
        let mut d = [0.0; 2];
        map_channels(&[0.5], &mut d);
        assert_eq!(d, [0.5, 0.5]);
        let mut m = [0.0; 1];
        map_channels(&[0.2, 0.6], &mut m);
        assert!((m[0] - 0.4).abs() < 1e-6);
        let mut s = [0.0; 2];
        map_channels(&[0.1, 0.2, 0.3, 0.5], &mut s); // 4 -> 2: the extra channels join the last one
        assert!((s[0] - 0.1).abs() < 1e-6 && (s[1] - (0.2 + 0.3 + 0.5) / 3.0).abs() < 1e-6);
        let mut six = [9.0; 6];
        map_channels(&[0.1, 0.2], &mut six);
        assert_eq!(six, [0.1, 0.2, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn resampling_keeps_the_duration() {
        // 1 s at 44.1 kHz stereo -> 48 kHz
        let mut src = (0..44_100).map(|i| [i as f32, -(i as f32)]);
        let mut rs = Resample::new(44_100, 48_000, 2);
        let mut out = [0.0f32; 2];
        let mut n = 0;
        while rs.next(&mut out, &mut |f| match src.next() { Some(v) => { f.copy_from_slice(&v); true } None => false }) { n += 1; assert_eq!(out[0], -out[1]); }
        assert!((n as i64 - 48_000).abs() <= 2, "{n}");
    }
}
