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

/// The audio host. Every cpal use goes through here: cpal keeps one WASAPI device enumerator for the whole process, made
/// in the apartment of the first thread that asks, so COM has to stay loaded after that thread ends (`keep_com_loaded`).
pub fn host() -> cpal::Host {
    super::keep_com_loaded();
    cpal::default_host()
}

#[cfg(not(target_os = "linux"))]
pub fn inputs() -> Vec<Dev> {
    let h = host();
    let def = h.default_input_device().and_then(|d| d.name().ok());
    h.input_devices().map(|it| it.filter_map(|d| d.name().ok()).map(|n| Dev { default: Some(&n) == def.as_ref(), name: n }).collect()).unwrap_or_default()
}
#[cfg(not(target_os = "linux"))]
pub fn outputs() -> Vec<Dev> {
    let h = host();
    let def = h.default_output_device().and_then(|d| d.name().ok());
    h.output_devices().map(|it| it.filter_map(|d| d.name().ok()).map(|n| Dev { default: Some(&n) == def.as_ref(), name: n }).collect()).unwrap_or_default()
}
/// Microphones and speakers in one go (on Linux one enumeration: cpal opens every ALSA PCM to list it).
#[cfg(not(target_os = "linux"))]
pub fn devices() -> (Vec<Dev>, Vec<Dev>) { (inputs(), outputs()) }
#[cfg(target_os = "linux")]
#[allow(dead_code)] // tests; the UI lists both sides with `devices`
pub fn inputs() -> Vec<Dev> { alsa_list(true, false).0 }
#[cfg(target_os = "linux")]
pub fn outputs() -> Vec<Dev> { alsa_list(false, true).1 }
#[cfg(target_os = "linux")]
pub fn devices() -> (Vec<Dev>, Vec<Dev>) { alsa_list(true, true) }

/// A sound server's ALSA PCM (`default` already goes through it).
#[cfg(target_os = "linux")]
fn is_server(name: &str) -> bool { matches!(name, "pipewire" | "pulse") }
/// ALSA PCMs that are not offered: raw hardware and plugin layers (`hw` / `plughw` grab the card exclusively, the rest
/// duplicate or bypass `default`), the sound server's own PCM (the same sink as `default`) and, with a sound server
/// (`server`), the per-card `sysdefault` (it bypasses the server and fights it for the card).
#[cfg(target_os = "linux")]
pub(crate) fn hidden(name: &str, server: bool) -> bool {
    const RAW: [&str; 27] = ["hw", "plughw", "front", "rear", "center_lfe", "side", "dmix", "dsnoop", "dshare", "iec958", "spdif", "hdmi",
        "usbstream", "modem", "phoneline", "pipewire", "pulse", "jack", "oss", "speex", "speexrate", "samplerate", "lavrate", "lavcrate",
        "a52", "upmix", "vdownmix"];
    let base = name.split(':').next().unwrap_or(name);
    base.starts_with("surround") || base == "null" || RAW.contains(&base) || (server && base == "sysdefault")
}
/// The ALSA PCMs worth offering. Hidden ones are dropped before their formats are probed (cpal has already opened them
/// to list them; the hint list itself is not reachable through cpal).
#[cfg(target_os = "linux")]
fn alsa_list(want_in: bool, want_out: bool) -> (Vec<Dev>, Vec<Dev>) {
    let mut server = false;
    let mut keep = vec![];
    if let Ok(it) = host().devices() {
        for d in it {
            let Ok(n) = d.name() else { continue };
            server |= is_server(&n);
            if !hidden(&n, false) { keep.push((n, d)); }
        }
    }
    keep.retain(|(n, _)| !hidden(n, server));
    let dev = |n: &String| Dev { default: n == "default", name: n.clone() };
    let ins = if want_in { keep.iter().filter(|(_, d)| d.supported_input_configs().is_ok_and(|mut c| c.next().is_some())).map(|(n, _)| dev(n)).collect() } else { vec![] };
    let outs = if want_out { keep.iter().filter(|(_, d)| d.supported_output_configs().is_ok_and(|mut c| c.next().is_some())).map(|(n, _)| dev(n)).collect() } else { vec![] };
    (ins, outs)
}
/// The saved device if it is still plugged in, else the system default (hot-plug safe).
#[cfg(not(target_os = "linux"))]
pub(crate) fn pick(name: &str, input: bool) -> Option<cpal::Device> {
    let h = host();
    if !name.is_empty() {
        let mut it = if input { h.input_devices().ok()? } else { h.output_devices().ok()? };
        if let Some(d) = it.find(|d| d.name().ok().as_deref() == Some(name)) { return Some(d); }
    }
    if input { h.default_input_device() } else { h.default_output_device() }
}
/// Linux: `default` and hidden PCMs (an older saved `hw:` / `sysdefault:` choice) go to `default` without enumerating;
/// other names are looked up without probing every device's formats on the way.
#[cfg(target_os = "linux")]
pub(crate) fn pick(name: &str, input: bool) -> Option<cpal::Device> {
    let h = host();
    if !name.is_empty() && name != "default" && !hidden(name, false) {
        let mut server = false; // the server's PCMs are listed before the per-card ones
        let found = h.devices().ok().and_then(|mut it| it.find(|d| { let n = d.name().unwrap_or_default(); server |= is_server(&n); n == name }));
        let ok = |d: &cpal::Device| if input { d.supported_input_configs().is_ok_and(|mut c| c.next().is_some()) } else { d.supported_output_configs().is_ok_and(|mut c| c.next().is_some()) };
        if let Some(d) = found.filter(|d| !hidden(name, server) && ok(d)) { return Some(d); }
    }
    if input { h.default_input_device() } else { h.default_output_device() }
}

/// Stream configs to try, best first, ending with the device's default. On Linux first 48 kHz (what calls and PipeWire run
/// at; cpal's default picks 44.1 kHz from PipeWire's continuous range, so everything got resampled twice), with a fixed
/// buffer of `buf` frames where the device allows it, then 48 kHz with the default buffer.
pub(crate) fn stream_configs(dev: &cpal::Device, input: bool, buf: Option<u32>) -> Result<Vec<(cpal::StreamConfig, cpal::SampleFormat)>, String> {
    let def = if input { dev.default_input_config() } else { dev.default_output_config() }.map_err(|e| e.to_string())?;
    let mut v = vec![];
    #[cfg(target_os = "linux")]
    {
        let ranges: Vec<cpal::SupportedStreamConfigRange> = if input { dev.supported_input_configs().map(|c| c.collect()) } else { dev.supported_output_configs().map(|c| c.collect()) }.unwrap_or_default();
        let fits = |r: &&cpal::SupportedStreamConfigRange| r.sample_format() == def.sample_format() && r.min_sample_rate().0 <= RATE && RATE <= r.max_sample_rate().0;
        if let Some(r) = ranges.iter().filter(fits).find(|r| r.channels() == def.channels()).or_else(|| ranges.iter().find(fits)) {
            let mut c = r.clone().with_sample_rate(cpal::SampleRate(RATE)).config();
            if let (Some(b), cpal::SupportedBufferSize::Range { min, max }) = (buf, r.buffer_size()) {
                if *min <= b && b <= *max { v.push((cpal::StreamConfig { buffer_size: cpal::BufferSize::Fixed(b), ..c.clone() }, def.sample_format())); }
            }
            c.buffer_size = cpal::BufferSize::Default;
            v.push((c, def.sample_format()));
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = buf;
    v.push((def.config(), def.sample_format()));
    Ok(v)
}
/// Call audio's fixed buffer on Linux: 20 ms (cpal's default asks ALSA for 100 ms).
const BUF_FRAMES: u32 = FRAME as u32;

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
    /// A stream reported an error (device unplugged, sound server restarted): the engine reopens its devices.
    failed: AtomicBool,
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

/// Stream errors this close together (or more) mean the stream is broken, not just one glitch.
const FATAL_ERRORS: usize = 3;
const FATAL_WINDOW: std::time::Duration = std::time::Duration::from_secs(1);

/// Whether a stream error should reopen the devices: at once when the device is gone, else only when errors keep coming
/// (`FATAL_ERRORS` within `FATAL_WINDOW`). One ALSA overrun / underrun (xrun) is transient: reopening for it would tear down
/// both streams and reset the echo canceller. `recent` holds the times of this stream's earlier errors.
fn stream_error_fatal(device_gone: bool, recent: &mut VecDeque<std::time::Instant>, now: std::time::Instant) -> bool {
    recent.retain(|t| now.saturating_duration_since(*t) < FATAL_WINDOW);
    recent.push_back(now);
    device_gone || recent.len() >= FATAL_ERRORS
}

/// The error callback of a stream (`side` names it in the log): sets `failed` for the engine only for a fatal error.
fn stream_error(io: Arc<Io>, side: &'static str) -> impl FnMut(cpal::StreamError) + Send + 'static {
    let mut recent = VecDeque::new();
    move |e| {
        if !stream_error_fatal(matches!(e, cpal::StreamError::DeviceNotAvailable), &mut recent, std::time::Instant::now()) {
            eprintln!("audio {side} (transient): {e}");
        } else if !io.failed.swap(true, Ordering::Relaxed) { eprintln!("audio {side}: {e}"); }
    }
}

/// Devices to open: `None` leaves that side closed (an empty name is the system default, not "none").
enum Cmd { Open(Option<String>, Option<String>, Sender<Result<String, String>>), Close }

/// Owns the audio thread. `open` (re)starts capture + playback on the chosen devices; `close` stops them.
/// Sounds open only the speaker (`open_output_async`), the level meter only the microphone (`open_input_async`).
pub struct Engine { tx: Sender<Cmd>, pub io: Arc<Io> }

impl Engine {
    pub fn new() -> Engine {
        let io = Arc::new(Io::default());
        io.volume.store(1.0f32.to_bits(), Ordering::Relaxed);
        let (tx, rx) = channel::<Cmd>();
        let io2 = io.clone();
        std::thread::spawn(move || {
            use std::sync::mpsc::RecvTimeoutError;
            let mut streams: Vec<cpal::Stream> = vec![];
            // The devices of the running streams, to reopen them when one fails (a headset unplugged, the default changed).
            let mut last: Option<(Option<String>, Option<String>)> = None;
            loop {
                let cmd = match rx.recv_timeout(std::time::Duration::from_secs(2)) {
                    Ok(c) => c,
                    Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {
                        let Some((i, o)) = last.clone() else { continue };
                        if !io2.failed.swap(false, Ordering::Relaxed) { continue; }
                        streams.clear();
                        // the same devices, else the defaults; still failing: tried again on the next tick
                        let r = open_streams(&io2, i.as_deref(), o.as_deref()).or_else(|_| open_streams(&io2, i.as_ref().map(|_| ""), o.as_ref().map(|_| "")));
                        match r { Ok((s, info)) => { eprintln!("audio: reopened ({info})"); streams = s; } Err(e) => { eprintln!("audio: reopen failed: {e}"); io2.failed.store(true, Ordering::Relaxed); } }
                        continue;
                    }
                };
                streams.clear(); // dropping stops them
                io2.failed.store(false, Ordering::Relaxed);
                last = None;
                if let Cmd::Open(i, o, reply) = cmd {
                    let r = open_streams(&io2, i.as_deref(), o.as_deref());
                    let _ = reply.send(match r { Ok((s, info)) => { streams = s; last = Some((i, o)); Ok(info) } Err(e) => Err(e) });
                }
            }
        });
        Engine { tx, io }
    }
    /// Starts the streams. Returns a short description ("In: …, Out: …") or the reason it failed. Blocks briefly.
    pub fn open(&self, input: &str, output: &str) -> Result<String, String> { self.open_wait(Some(input), Some(output)) }
    fn open_wait(&self, input: Option<&str>, output: Option<&str>) -> Result<String, String> {
        let rrx = self.send_open(input, output);
        rrx.recv_timeout(std::time::Duration::from_secs(10)).map_err(|_| "audio devices didn't answer".to_string())?
    }
    /// `open_output` without waiting: the device opens on the engine's thread and the answer arrives on the returned
    /// channel (disconnected if the thread is gone). Sound queued meanwhile plays once the output is open.
    pub fn open_output_async(&self, output: &str) -> std::sync::mpsc::Receiver<Result<String, String>> { self.send_open(None, Some(output)) }
    /// Only the microphone, without waiting (the level meter).
    pub fn open_input_async(&self, input: &str) -> std::sync::mpsc::Receiver<Result<String, String>> { self.send_open(Some(input), None) }
    fn send_open(&self, input: Option<&str>, output: Option<&str>) -> std::sync::mpsc::Receiver<Result<String, String>> {
        let (rtx, rrx) = channel();
        let _ = self.tx.send(Cmd::Open(input.map(String::from), output.map(String::from), rtx));
        rrx
    }
    pub fn close(&self) { let _ = self.tx.send(Cmd::Close); }
}

fn to_f32<T: cpal::Sample>(v: T) -> f32 where T: cpal::SizedSample, f32: cpal::FromSample<T> { v.to_sample::<f32>() }

/// Builds the stream with the first config that works (`stream_configs` order); the last one's error otherwise.
pub(crate) fn first_ok(cfgs: &[(cpal::StreamConfig, cpal::SampleFormat)], mut build: impl FnMut(&cpal::StreamConfig, cpal::SampleFormat) -> Result<cpal::Stream, String>) -> Result<cpal::Stream, String> {
    let mut last = Err("no stream config".to_string());
    for (c, f) in cfgs { last = build(c, *f); if last.is_ok() { break; } }
    last
}

fn open_streams(io: &Arc<Io>, input: Option<&str>, output: Option<&str>) -> Result<(Vec<cpal::Stream>, String), String> {
    let mut streams = vec![];
    let mut info = vec![];
    if let Some(dev) = input.and_then(|i| pick(i, true)) {
        let cfgs = stream_configs(&dev, true, Some(BUF_FRAMES)).map_err(|e| format!("Mikrofon: {e}"))?;
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
        let build = |cfg: &cpal::StreamConfig, fmt: cpal::SampleFormat| -> Result<cpal::Stream, String> {
            let (ch, blocks) = (cfg.channels as usize, blocks.clone());
            let (mut lin, mut mono, mut res) = (Lin::new(cfg.sample_rate.0, RATE), Vec::<f32>::new(), Vec::<f32>::new());
            let mut handle = move |frames: &mut dyn Iterator<Item = f32>| {
                mono.clear();
                let mut acc = 0.0; let mut n = 0;
                for s in frames { acc += s; n += 1; if n == ch { mono.push(acc / ch as f32); acc = 0.0; n = 0; } }
                res.clear();
                lin.push(&mono, &mut res);
                let _ = blocks.send(res.clone());
            };
            let io4 = io.clone();
            let err = stream_error(io4, "input");
            match fmt {
                cpal::SampleFormat::F32 => dev.build_input_stream(cfg, move |d: &[f32], _| handle(&mut d.iter().copied()), err, None),
                cpal::SampleFormat::I16 => dev.build_input_stream(cfg, move |d: &[i16], _| handle(&mut d.iter().map(|&s| to_f32(s))), err, None),
                cpal::SampleFormat::U16 => dev.build_input_stream(cfg, move |d: &[u16], _| handle(&mut d.iter().map(|&s| to_f32(s))), err, None),
                f => return Err(format!("Mikrofon: Format {f:?} wird nicht unterstützt")),
            }.map_err(|e| format!("Mikrofon: {e}"))
        };
        let st = first_ok(&cfgs, build)?;
        drop(blocks); // the stream holds the sender now; the chain's thread ends with it
        st.play().map_err(|e| format!("Mikrofon: {e}"))?;
        streams.push(st);
        info.push(format!("In: {}", dev.name().unwrap_or_default()));
    }
    if let Some(dev) = output.and_then(|o| pick(o, false)) {
        let cfgs = stream_configs(&dev, false, Some(BUF_FRAMES)).map_err(|e| format!("Lautsprecher: {e}"))?;
        let build = |cfg: &cpal::StreamConfig, fmt: cpal::SampleFormat| -> Result<cpal::Stream, String> {
            let (rate, ch) = (cfg.sample_rate.0, cfg.channels as usize);
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
            let io4 = io.clone();
            let err = stream_error(io4, "output");
            match fmt {
                cpal::SampleFormat::F32 => dev.build_output_stream(cfg, move |d: &mut [f32], _| { let n = d.len(); play_into(n, Box::new(|i, v| d[i] = v)) }, err, None),
                cpal::SampleFormat::I16 => dev.build_output_stream(cfg, move |d: &mut [i16], _| { let n = d.len(); play_into(n, Box::new(|i, v| d[i] = (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)) }, err, None),
                cpal::SampleFormat::U16 => dev.build_output_stream(cfg, move |d: &mut [u16], _| { let n = d.len(); play_into(n, Box::new(|i, v| d[i] = ((v.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16)) }, err, None),
                f => return Err(format!("Lautsprecher: Format {f:?} wird nicht unterstützt")),
            }.map_err(|e| format!("Lautsprecher: {e}"))
        };
        let st = first_ok(&cfgs, build)?;
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
    fn only_lasting_stream_errors_reopen_the_devices() {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut r = VecDeque::new();
        assert!(!stream_error_fatal(false, &mut r, ms(0)), "one overrun is transient");
        assert!(!stream_error_fatal(false, &mut r, ms(500)));
        assert!(stream_error_fatal(false, &mut r, ms(900)), "three within a second");
        // Spread out, errors never add up.
        let mut r = VecDeque::new();
        for i in 0..20 { assert!(!stream_error_fatal(false, &mut r, ms(i * 600)), "{i}"); }
        assert!(r.len() <= 2);
        // A device that is gone counts at once.
        assert!(stream_error_fatal(true, &mut VecDeque::new(), ms(0)));
    }

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

    /// Raw ALSA PCMs stay out of the device lists; `default` and user-defined PCMs stay in, `sysdefault` only without a
    /// sound server. Pure name logic, opens nothing.
    #[cfg(target_os = "linux")]
    #[test]
    fn raw_alsa_pcms_are_hidden() {
        for n in ["hw:CARD=PCH,DEV=0", "plughw:CARD=PCH,DEV=0", "front:CARD=PCH,DEV=0", "surround51:CARD=PCH,DEV=0", "dmix:CARD=PCH,DEV=0",
                  "dsnoop:CARD=PCH,DEV=0", "iec958:CARD=PCH,DEV=0", "hdmi:CARD=HDMI,DEV=0", "usbstream:CARD=Headset", "pipewire", "pulse", "null"] {
            assert!(hidden(n, false), "{n}");
        }
        for n in ["default", "sysdefault:CARD=Headset", "my_asoundrc_pcm"] { assert!(!hidden(n, false), "{n}"); }
        assert!(hidden("sysdefault:CARD=Headset", true) && !hidden("default", true));
    }

    #[test]
    fn enumeration_does_not_panic() {
        let _ = (inputs(), outputs());
    }

    /// Regression (Windows CI, 0xc0000005 in `IMMDeviceEnumerator::GetDefaultAudioEndpoint`): cpal makes its device
    /// enumerator once per process in the apartment of the first thread that lists devices. That thread ending used to take
    /// COM down with it, and the next lookup from another thread (the engine's) called into an unloaded DLL. Needs no
    /// device and opens none: with nothing plugged in the lookups just come back empty.
    #[test]
    fn devices_listed_on_a_thread_that_ended_stay_usable() {
        for _ in 0..3 {
            std::thread::spawn(|| { let _ = (inputs(), outputs(), pick("", false), pick("", true)); }).join().unwrap();
        }
        let _ = (inputs(), outputs());
    }

    /// A real speaker-only engine at volume 0 (silent): it opens no microphone and plays its queue out. Skips on CI runners
    /// and on machines without an output device.
    #[test]
    fn output_only_engine_plays_out_silently() {
        if crate::media::ci() { eprintln!("SKIPPED: CI runner, no speaker to open"); return; }
        if outputs().is_empty() { eprintln!("SKIPPED: no audio output device"); return; }
        let e = Engine::new();
        e.io.set_volume(0.0);
        e.io.play_long(&tone(440.0, 300, 0.5));
        let info = match e.open_output_async("").recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(Ok(i)) => i,
            other => { eprintln!("SKIPPED: the output device did not open: {other:?}"); return; }
        };
        assert!(info.starts_with("Out: ") && !info.contains("In: "), "{info}");
        let t0 = std::time::Instant::now();
        while e.io.play_pending() > 0 && t0.elapsed() < std::time::Duration::from_secs(5) { std::thread::sleep(std::time::Duration::from_millis(20)); }
        assert_eq!(e.io.play_pending(), 0, "the speaker pulled the queued sound");
        assert_eq!(e.io.level(), 0.0, "no microphone ran");
    }
}
