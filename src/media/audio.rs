//! Audio devices and the capture / playback pipeline for calls (pure Rust: `cpal` for the devices).
//!
//! Everything is 48 kHz mono `i16` on the call side (what Opus wants); this module converts from / to whatever the chosen
//! device runs at. The cpal streams are not `Send`, so one thread owns them and `Engine` talks to it by messages.
//! Device choice is persisted in the settings JSON (`audio_input`, `audio_output`, `audio_ringtone_output`,
//! `audio_volume`, `smart_mic`); an empty name means "the system default" ("Systemstandard").
//! The microphone runs through `mic::MicChain` (echo cancellation and gain control in calls, noise removal always) on its
//! own thread before the level meter and the call sender; the speaker output is its echo reference.
pub use super::denoise::SmartMic;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

pub const RATE: u32 = 48_000;
/// 20 ms of 48 kHz mono, the Opus frame the call path uses.
pub const FRAME: usize = 960;
const MAX_BUFFER: usize = RATE as usize; // 1 s
/// Captured audio waiting for the call sender is capped at 60 ms: if the microphone's clock runs faster than the sender's,
/// the oldest 20 ms are dropped instead of the delay growing.
const MAX_CAPTURED: usize = 3 * FRAME;

#[derive(Clone, Debug, PartialEq)]
pub struct Dev { pub name: String, pub default: bool }

/// The user's choices (names; empty = system default). `smart` is Settings > Smart Audio > Mikrofonaudio (`smart_mic`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Choice { pub input: String, pub output: String, pub ringtone: String, pub volume: f32, pub smart: SmartMic }

impl Choice {
    pub fn load(s: &serde_json::Value) -> Choice {
        let t = |k: &str| s[k].as_str().unwrap_or("").to_string();
        // `audio_noise_removal: false` (an older setting) only matters when `smart_mic` was never set.
        let smart = match s["smart_mic"].as_str() { Some(k) => SmartMic::from_key(k), None if s["audio_noise_removal"] == false => SmartMic::Off, None => SmartMic::Auto };
        Choice { input: t("audio_input"), output: t("audio_output"), ringtone: t("audio_ringtone_output"),
            volume: s["audio_volume"].as_f64().map_or(1.0, |v| v.clamp(0.0, 1.0) as f32), smart }
    }
    /// Puts the Smart Audio choice on an engine's microphone path.
    pub fn apply_smart(&self, io: &Io) { io.set_smart(self.smart); }
    pub fn store(&self, s: &mut serde_json::Value) {
        s["audio_input"] = self.input.clone().into(); s["audio_output"] = self.output.clone().into();
        s["audio_ringtone_output"] = self.ringtone.clone().into(); s["audio_volume"] = (self.volume as f64).into();
        s["smart_mic"] = self.smart.key().into();
    }
}

pub fn inputs() -> Vec<Dev> {
    let h = cpal::default_host();
    let def = h.default_input_device().and_then(|d| d.name().ok());
    h.input_devices().map(|it| it.filter_map(|d| d.name().ok()).map(|n| Dev { default: Some(&n) == def.as_ref(), name: n }).collect()).unwrap_or_default()
}
pub fn outputs() -> Vec<Dev> {
    let h = cpal::default_host();
    let def = h.default_output_device().and_then(|d| d.name().ok());
    h.output_devices().map(|it| it.filter_map(|d| d.name().ok()).map(|n| Dev { default: Some(&n) == def.as_ref(), name: n }).collect()).unwrap_or_default()
}
/// The saved device if it is still plugged in, else the system default (hot-plug safe).
fn pick(name: &str, input: bool) -> Option<cpal::Device> {
    let h = cpal::default_host();
    if !name.is_empty() {
        let mut it = if input { h.input_devices().ok()? } else { h.output_devices().ok()? };
        if let Some(d) = it.find(|d| d.name().ok().as_deref() == Some(name)) { return Some(d); }
    }
    if input { h.default_input_device() } else { h.default_output_device() }
}

/// Linear-interpolating resampler (push style): good enough for speech, no external crate.
pub struct Lin { ratio: f64, pos: f64, last: f32 }
impl Lin {
    pub fn new(in_rate: u32, out_rate: u32) -> Lin { Lin { ratio: in_rate as f64 / out_rate as f64, pos: 0.0, last: 0.0 } }
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        for &x in input {
            while self.pos < 1.0 { out.push(self.last + (x - self.last) * self.pos as f32); self.pos += self.ratio; }
            self.pos -= 1.0;
            self.last = x;
        }
    }
}
/// Pull-style resampler for the playback callback: reads source samples on demand.
struct Pull { ratio: f64, pos: f64, a: f32, b: f32 }
impl Pull {
    fn new(src_rate: u32, dst_rate: u32) -> Pull { Pull { ratio: src_rate as f64 / dst_rate as f64, pos: 1.0, a: 0.0, b: 0.0 } }
    fn next(&mut self, src: &mut impl FnMut() -> f32) -> f32 {
        while self.pos >= 1.0 { self.a = self.b; self.b = src(); self.pos -= 1.0; }
        let v = self.a + (self.b - self.a) * self.pos as f32;
        self.pos += self.ratio;
        v
    }
}

/// State shared between the audio thread and the rest of the app.
#[derive(Default)]
pub struct Io {
    /// Captured microphone audio, 48 kHz mono, oldest first (the call sender drains it).
    pub captured: Mutex<VecDeque<i16>>,
    /// Audio to play, 48 kHz mono (the call receiver and the tone generator fill it).
    pub to_play: Mutex<VecDeque<i16>>,
    /// Microphone level 0..1 (f32 bits), smoothed — the meter next to the mute button.
    level: AtomicU32,
    /// Playback gain 0..1 (f32 bits).
    volume: AtomicU32,
    /// Smart Audio mode of the microphone (`SmartMic::to_u8`), read for every block.
    smart: AtomicU8,
    /// In a call: echo cancellation and gain control on, and the speaker output is kept as their reference.
    call: AtomicBool,
    /// What the speaker played (48 kHz, after the volume), waiting for the echo canceller.
    pub far: Mutex<VecDeque<f32>>,
}
impl Io {
    pub fn level(&self) -> f32 { f32::from_bits(self.level.load(Ordering::Relaxed)) }
    pub fn set_smart(&self, m: SmartMic) { self.smart.store(m.to_u8(), Ordering::Relaxed) }
    pub fn smart(&self) -> SmartMic { SmartMic::from_u8(self.smart.load(Ordering::Relaxed)) }
    pub fn set_call(&self, on: bool) { self.call.store(on, Ordering::Relaxed) }
    pub fn in_call(&self) -> bool { self.call.load(Ordering::Relaxed) }
    pub fn set_volume(&self, v: f32) { self.volume.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed) }
    fn gain(&self) -> f32 { f32::from_bits(self.volume.load(Ordering::Relaxed)) }
    /// Takes exactly one 20 ms frame of microphone audio if there is one.
    pub fn take_frame(&self) -> Option<[i16; FRAME]> {
        let mut q = self.captured.lock().ok()?;
        if q.len() < FRAME { return None; }
        let mut f = [0i16; FRAME];
        for s in f.iter_mut() { *s = q.pop_front().unwrap_or(0); }
        Some(f)
    }
    pub fn play(&self, pcm: &[i16]) {
        if let Ok(mut q) = self.to_play.lock() {
            q.extend(pcm.iter().copied());
            while q.len() > MAX_BUFFER { q.pop_front(); }
        }
    }
    /// Like `play` but never drops audio (ringtones / sound files longer than the 1 s call buffer).
    pub fn play_long(&self, pcm: &[i16]) { if let Ok(mut q) = self.to_play.lock() { q.extend(pcm.iter().copied()); } }
    pub fn play_pending(&self) -> usize { self.to_play.lock().map_or(0, |q| q.len()) }
}

/// A sine beep of `ms` milliseconds (speaker test), 48 kHz mono, with a short fade in / out.
#[cfg(test)]
pub fn tone(freq: f32, ms: u32, amp: f32) -> Vec<i16> {
    let n = (RATE as u64 * ms as u64 / 1000) as usize;
    let fade = (RATE as usize / 100).min(n / 2).max(1);
    (0..n).map(|i| {
        let env = (i.min(n - 1 - i).min(fade)) as f32 / fade as f32;
        ((i as f32 * freq * std::f32::consts::TAU / RATE as f32).sin() * amp * env * i16::MAX as f32) as i16
    }).collect()
}

/// RMS of a block as 0..1.
pub fn rms(pcm: &[i16]) -> f32 {
    if pcm.is_empty() { return 0.0; }
    (pcm.iter().map(|&s| { let f = s as f32 / i16::MAX as f32; f * f }).sum::<f32>() / pcm.len() as f32).sqrt()
}

enum Cmd { Open(String, String, Sender<Result<String, String>>), Close }

/// Owns the audio thread. `open` (re)starts capture + playback on the chosen devices; `close` stops them.
pub struct Engine { tx: Sender<Cmd>, pub io: Arc<Io> }

impl Engine {
    pub fn new() -> Engine {
        let io = Arc::new(Io::default());
        io.volume.store(1.0f32.to_bits(), Ordering::Relaxed);
        let (tx, rx) = channel::<Cmd>();
        let io2 = io.clone();
        std::thread::spawn(move || {
            let mut streams: Vec<cpal::Stream> = vec![];
            for cmd in rx {
                streams.clear(); // dropping stops them
                if let Cmd::Open(i, o, reply) = cmd {
                    let r = open_streams(&io2, &i, &o);
                    let _ = reply.send(match r { Ok((s, info)) => { streams = s; Ok(info) } Err(e) => Err(e) });
                }
            }
        });
        Engine { tx, io }
    }
    /// Starts the streams. Returns a short description ("In: …, Out: …") or the reason it failed. Blocks briefly.
    pub fn open(&self, input: &str, output: &str) -> Result<String, String> {
        let (rtx, rrx) = channel();
        self.tx.send(Cmd::Open(input.into(), output.into(), rtx)).map_err(|_| "audio thread gone".to_string())?;
        rrx.recv_timeout(std::time::Duration::from_secs(10)).map_err(|_| "audio devices didn't answer".to_string())?
    }
    /// `open` without waiting: the devices open on the engine's thread and the answer arrives on the returned channel
    /// (disconnected if the thread is gone). Sound queued meanwhile plays once the output is open.
    pub fn open_async(&self, input: &str, output: &str) -> std::sync::mpsc::Receiver<Result<String, String>> {
        let (rtx, rrx) = channel();
        let _ = self.tx.send(Cmd::Open(input.into(), output.into(), rtx));
        rrx
    }
    pub fn close(&self) { let _ = self.tx.send(Cmd::Close); }
}

fn to_f32<T: cpal::Sample>(v: T) -> f32 where T: cpal::SizedSample, f32: cpal::FromSample<T> { v.to_sample::<f32>() }

fn open_streams(io: &Arc<Io>, input: &str, output: &str) -> Result<(Vec<cpal::Stream>, String), String> {
    let mut streams = vec![];
    let mut info = vec![];
    if let Some(dev) = pick(input, true) {
        let cfg = dev.default_input_config().map_err(|e| format!("Mikrofon: {e}"))?;
        let (rate, ch, fmt) = (cfg.sample_rate().0, cfg.channels() as usize, cfg.sample_format());
        let mut lin = Lin::new(rate, RATE);
        let mut mono: Vec<f32> = vec![];
        let mut res: Vec<f32> = vec![];
        // The microphone chain runs on its own thread so it never holds up the device callback. The callback only mixes
        // down, resamples to 48 kHz and hands the block over; the thread ends when the stream (and so `blocks`) is dropped.
        let (blocks, rx) = channel::<Vec<f32>>();
        let io3 = io.clone();
        std::thread::Builder::new().name("mic-dsp".into()).spawn(move || {
            let mut chain = super::mic::MicChain::new();
            let (mut clean, mut far) = (Vec::<f32>::new(), VecDeque::<f32>::new());
            for b in rx {
                if let Ok(mut f) = io3.far.lock() { far.extend(f.drain(..)); }
                clean.clear();
                chain.process(io3.smart(), io3.in_call(), &mut far, &b, &mut clean);
                let pcm: Vec<i16> = clean.iter().map(|v| (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).collect();
                let level = rms(&pcm).min(1.0);
                let old = f32::from_bits(io3.level.load(Ordering::Relaxed));
                io3.level.store((if level > old { level } else { old * 0.85 + level * 0.15 }).to_bits(), Ordering::Relaxed); // fast attack, slow release
                if let Ok(mut q) = io3.captured.lock() {
                    q.extend(pcm);
                    while q.len() > MAX_CAPTURED { q.drain(..FRAME); }
                }
            }
        }).map_err(|e| format!("Mikrofon: {e}"))?;
        let mut handle = move |frames: &mut dyn Iterator<Item = f32>| {
            mono.clear();
            let mut acc = 0.0; let mut n = 0;
            for s in frames { acc += s; n += 1; if n == ch { mono.push(acc / ch as f32); acc = 0.0; n = 0; } }
            res.clear();
            lin.push(&mono, &mut res);
            let _ = blocks.send(res.clone());
        };
        let err = |e| eprintln!("audio input: {e}");
        let st = match fmt {
            cpal::SampleFormat::F32 => dev.build_input_stream(&cfg.into(), move |d: &[f32], _| handle(&mut d.iter().copied()), err, None),
            cpal::SampleFormat::I16 => dev.build_input_stream(&cfg.into(), move |d: &[i16], _| handle(&mut d.iter().map(|&s| to_f32(s))), err, None),
            cpal::SampleFormat::U16 => dev.build_input_stream(&cfg.into(), move |d: &[u16], _| handle(&mut d.iter().map(|&s| to_f32(s))), err, None),
            f => return Err(format!("Mikrofon: Format {f:?} wird nicht unterstützt")),
        }.map_err(|e| format!("Mikrofon: {e}"))?;
        st.play().map_err(|e| format!("Mikrofon: {e}"))?;
        streams.push(st);
        info.push(format!("In: {}", dev.name().unwrap_or_default()));
    }
    if let Some(dev) = pick(output, false) {
        let cfg = dev.default_output_config().map_err(|e| format!("Lautsprecher: {e}"))?;
        let (rate, ch, fmt) = (cfg.sample_rate().0, cfg.channels() as usize, cfg.sample_format());
        let io2 = io.clone();
        let mut pull = Pull::new(RATE, rate);
        let play_into = move |len: usize, mut put: Box<dyn FnMut(usize, f32) + '_>| {
            let gain = io2.gain();
            let mut q = io2.to_play.lock().ok();
            // in a call, what is played (at 48 kHz, after the volume) is the echo canceller's reference
            let mut far = if io2.in_call() { io2.far.lock().ok() } else { None };
            let mut src = || {
                let v = q.as_mut().and_then(|q| q.pop_front()).map_or(0.0, |s| s as f32 / i16::MAX as f32);
                if let Some(f) = far.as_mut() { f.push_back(v * gain); }
                v
            };
            for i in 0..len / ch { let v = pull.next(&mut src) * gain; for c in 0..ch { put(i * ch + c, v); } }
            if let Some(f) = far.as_mut() { while f.len() > super::mic::FAR_MAX { f.pop_front(); } }
        };
        let mut play_into = play_into;
        let err = |e| eprintln!("audio output: {e}");
        let st = match fmt {
            cpal::SampleFormat::F32 => dev.build_output_stream(&cfg.into(), move |d: &mut [f32], _| { let n = d.len(); play_into(n, Box::new(|i, v| d[i] = v)) }, err, None),
            cpal::SampleFormat::I16 => dev.build_output_stream(&cfg.into(), move |d: &mut [i16], _| { let n = d.len(); play_into(n, Box::new(|i, v| d[i] = (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)) }, err, None),
            cpal::SampleFormat::U16 => dev.build_output_stream(&cfg.into(), move |d: &mut [u16], _| { let n = d.len(); play_into(n, Box::new(|i, v| d[i] = ((v.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16)) }, err, None),
            f => return Err(format!("Lautsprecher: Format {f:?} wird nicht unterstützt")),
        }.map_err(|e| format!("Lautsprecher: {e}"))?;
        st.play().map_err(|e| format!("Lautsprecher: {e}"))?;
        streams.push(st);
        info.push(format!("Out: {}", dev.name().unwrap_or_default()));
    }
    if streams.is_empty() { return Err("Keine Audiogeräte gefunden".into()); }
    Ok((streams, info.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_round_trip() {
        let mut v = serde_json::json!({});
        let c = Choice { input: "Mic".into(), output: "Spk".into(), ringtone: "".into(), volume: 0.5, smart: SmartMic::Voice };
        c.store(&mut v);
        assert_eq!(Choice::load(&v), c);
        assert_eq!(Choice::load(&serde_json::json!({})).volume, 1.0);
        assert_eq!(Choice::load(&serde_json::json!({})).smart, SmartMic::Auto);
        assert_eq!(Choice::load(&serde_json::json!({"audio_noise_removal": false})).smart, SmartMic::Off);
        assert_eq!(Choice::load(&serde_json::json!({"smart_mic": "all", "audio_noise_removal": false})).smart, SmartMic::All);
    }

    #[test]
    fn resampler_keeps_length_and_tone() {
        let src: Vec<f32> = (0..4410).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 44_100.0).sin() * 0.5).collect();
        let mut out = vec![];
        Lin::new(44_100, RATE).push(&src, &mut out);
        assert!((out.len() as i64 - 4800).abs() <= 2, "len {}", out.len());
        let pcm: Vec<i16> = out.iter().map(|v| (v * i16::MAX as f32) as i16).collect();
        assert!((rms(&pcm) - 0.3535).abs() < 0.03, "rms {}", rms(&pcm));
    }

    #[test]
    fn tone_has_energy_and_frames() {
        let t = tone(440.0, 200, 0.5);
        assert_eq!(t.len(), 9600);
        assert!(rms(&t) > 0.2);
        let io = Io::default();
        io.play(&t);
        assert_eq!(io.play_pending(), 9600);
        io.captured.lock().unwrap().extend(t.iter().take(FRAME + 5));
        assert!(io.take_frame().is_some() && io.take_frame().is_none());
    }

    /// The codec path of a call without any network or device: 48 kHz mono tone -> Opus (20 ms frames) -> decode.
    #[test]
    fn opus_loopback_keeps_the_tone() {
        use ropus::{Application, Channels, DecodeMode, Decoder, Encoder};
        let mut enc = Encoder::builder(RATE, Channels::Mono, Application::Voip).build().unwrap();
        let mut dec = Decoder::new(RATE, Channels::Mono).unwrap();
        let pcm = tone(440.0, 1000, 0.5);
        let (mut out, mut bytes) = (vec![], 0);
        for f in pcm.chunks_exact(FRAME) {
            let mut pkt = [0u8; 1275];
            let n = enc.encode(f, &mut pkt).unwrap();
            bytes += n;
            let mut dst = [0i16; FRAME];
            assert_eq!(dec.decode(&pkt[..n], &mut dst, DecodeMode::Normal).unwrap(), FRAME);
            out.extend_from_slice(&dst);
        }
        assert!(bytes < pcm.len() * 2 / 4, "compressed {bytes} bytes");
        let tail = &out[out.len() / 2..];
        assert!((rms(tail) - 0.3535).abs() < 0.08, "rms {}", rms(tail)); // the tone survives the codec
    }

    #[test]
    fn enumeration_does_not_panic() {
        let _ = (inputs(), outputs());
    }
}
