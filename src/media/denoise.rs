//! Smart Audio > Mikrofonaudio: noise removal on the microphone path (48 kHz mono, before the level meter and the call sender).
//!
//! Engines (both open source, pure Rust, 480-sample frames = 10 ms at 48 kHz):
//! - DeepFilterNet 3 (libDF + tract, MIT / Apache-2.0; model in assets/models/deepfilternet3). The primary engine. It runs on
//!   its own `deepfilter` thread, which first builds the model (takes a moment) and then processes the frames it is sent.
//! - RNNoise (`nnnoiseless`, BSD-3-Clause). Used while DeepFilterNet is still loading, if it fails to load, if it misses a
//!   frame deadline or costs more than 70 % of the real-time budget on average (then for the rest of the session), and in
//!   eco mode.
//!
//! Modes (pack: "Removes all background noise.", "Removes all background noise and background speech.", "Others hear the
//! original sound when you play an instrument or sing."):
//! - `Auto` ("Automatisch"): noise removal on, moderate: attenuation limited to `AUTO_LIM_DB`, so the voice stays natural.
//! - `Voice` ("Nur Stimme"): strongest suppression: no attenuation limit plus libDF's post filter (RNNoise: plus a gate on its
//!   voice-activity estimate). Neither engine has a speaker model, so "only my voice" is approximated by suppressing harder.
//! - `Off` ("Aus"): no processing. `All` ("Alle Geräusche", music mode): no suppression, the original sound passes.
use df::tract::{DfParams, DfTract, ReduceMask, RuntimeParams};
use nnnoiseless::DenoiseState;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Frame of both engines: 10 ms at 48 kHz.
pub const NN_FRAME: usize = 480;
const _: () = assert!(DenoiseState::FRAME_SIZE == NN_FRAME);
/// Latency of the RNNoise path: the 10 ms look-behind of `Denoiser` plus RNNoise's own frame of overlap.
#[allow(dead_code)]
pub const DELAY: usize = 2 * NN_FRAME;
/// "Automatisch": DeepFilterNet's attenuation limit (dB): moderate, keeps some room tone so the voice sounds natural.
pub const AUTO_LIM_DB: f32 = 24.0;
/// "Nur Stimme": libDF's post-filter strength (its documented default).
const VOICE_PF_BETA: f32 = 0.02;
/// A DeepFilterNet frame that takes longer than this is late: the stream falls back to RNNoise.
const DEADLINE: Duration = Duration::from_millis(40);
/// Average DeepFilterNet cost per 10 ms frame above which it can't keep up with headroom (70 % of real time).
const SLOW_MS: f32 = 7.0;

/// The DeepFilterNet 3 model (enc / erb_dec / df_dec ONNX + config.ini), shipped with Webox.
static MODEL: &[u8] = include_bytes!("../../assets/models/deepfilternet3/DeepFilterNet3_onnx.tar.gz");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SmartMic { #[default] Auto, Off, Voice, All }

impl SmartMic {
    /// Settings value (`smart_mic`): "auto" | "off" | "voice" | "all".
    pub fn from_key(s: &str) -> SmartMic { match s { "off" => SmartMic::Off, "voice" => SmartMic::Voice, "all" => SmartMic::All, _ => SmartMic::Auto } }
    pub fn key(self) -> &'static str { match self { SmartMic::Auto => "auto", SmartMic::Off => "off", SmartMic::Voice => "voice", SmartMic::All => "all" } }
    pub fn to_u8(self) -> u8 { self as u8 }
    pub fn from_u8(v: u8) -> SmartMic { match v { 1 => SmartMic::Off, 2 => SmartMic::Voice, 3 => SmartMic::All, _ => SmartMic::Auto } }
    /// Whether the mode runs the noise remover.
    pub fn denoises(self) -> bool { matches!(self, SmartMic::Auto | SmartMic::Voice) }
}

/// What the microphone path is running (shown on the Smart Audio page).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine { Idle, Loading, DeepFilter, RnnFallback, RnnEco, LoadFailed }
static STATUS: AtomicU8 = AtomicU8::new(0);
/// Set once DeepFilterNet missed its deadline / was too slow on this machine: later streams start on RNNoise.
static DFN_SLOW: AtomicBool = AtomicBool::new(false);
fn set_status(e: Engine) { STATUS.store(e as u8, Ordering::Relaxed); }
pub fn engine() -> Engine {
    match STATUS.load(Ordering::Relaxed) { 1 => Engine::Loading, 2 => Engine::DeepFilter, 3 => Engine::RnnFallback, 4 => Engine::RnnEco, 5 => Engine::LoadFailed, _ => Engine::Idle }
}

/// Builds DeepFilterNet from the shipped model (checks it runs at 48 kHz with 10 ms hops).
pub fn build_dfn() -> Result<DfTract, String> {
    let p = DfParams::from_bytes(MODEL).map_err(|e| format!("DeepFilterNet model: {e:#}"))?;
    // one microphone channel: averaging the mask across channels (the default MEAN) would only add nodes to the ERB decoder
    let rp = RuntimeParams::default_with_ch(1).with_atten_lim(AUTO_LIM_DB).with_post_filter(0.0).with_mask_reduce(ReduceMask::NONE);
    let m = DfTract::new(p, &rp).map_err(|e| format!("DeepFilterNet: {e:#}"))?;
    if m.sr != 48_000 || m.hop_size != NN_FRAME { return Err(format!("DeepFilterNet: {} Hz / hop {}", m.sr, m.hop_size)); }
    Ok(m)
}
/// Mode -> DeepFilterNet parameters (changed in place, no reload).
pub fn dfn_mode(m: &mut DfTract, mode: SmartMic) {
    if mode == SmartMic::Voice { m.set_atten_lim(100.0); m.set_pf_beta(VOICE_PF_BETA); } else { m.set_atten_lim(AUTO_LIM_DB); m.set_pf_beta(0.0); }
}
/// One 10 ms frame through DeepFilterNet (samples −1..1).
pub fn dfn_frame(m: &mut DfTract, input: &[f32; NN_FRAME], out: &mut [f32; NN_FRAME]) -> Result<f32, String> {
    let noisy = ndarray::ArrayView2::from_shape((1, NN_FRAME), &input[..]).map_err(|e| e.to_string())?;
    let enh = ndarray::ArrayViewMut2::from_shape((1, NN_FRAME), &mut out[..]).map_err(|e| e.to_string())?;
    m.process(noisy, enh).map_err(|e| e.to_string())
}

struct Job { frame: Box<[f32; NN_FRAME]>, mode: SmartMic, reset: bool }
/// The `deepfilter` thread: builds the model, then processes frames in order. `ready`: 0 loading, 1 ready, 2 failed.
struct Dfn { tx: Sender<Job>, rx: Receiver<Result<Box<[f32; NN_FRAME]>, String>>, ready: Arc<AtomicU8> }
impl Dfn {
    fn spawn() -> Dfn {
        let (tx, jobs) = channel::<Job>();
        let (otx, rx) = channel();
        let ready = Arc::new(AtomicU8::new(0));
        let r2 = ready.clone();
        let spawned = std::thread::Builder::new().name("deepfilter".into()).spawn(move || {
            let mut m = match build_dfn() { Ok(m) => m, Err(e) => { eprintln!("{e}"); r2.store(2, Ordering::Relaxed); return; } };
            r2.store(1, Ordering::Relaxed);
            let mut cur = None;
            for j in jobs {
                if j.reset { let _ = m.init(); }
                if cur != Some(j.mode) { dfn_mode(&mut m, j.mode); cur = Some(j.mode); }
                let mut out = Box::new([0f32; NN_FRAME]);
                let r = dfn_frame(&mut m, &j.frame, &mut out).map(|_| out);
                if otx.send(r).is_err() { break; }
            }
        });
        if spawned.is_err() { ready.store(2, Ordering::Relaxed); }
        Dfn { tx, rx, ready }
    }
    fn state(&self) -> u8 { self.ready.load(Ordering::Relaxed) }
}

/// Streaming wrapper: any block size in, the same number of samples out.
///
/// Both engines work on whole 480-sample frames. Input waits in `pending` until a frame is full; processed audio waits in
/// `ready` until it is handed out. Starting with one frame of silence in `pending` (10 ms) guarantees that `ready` always
/// holds at least as many samples as were put in, so every call returns exactly `input.len()` samples.
/// Call it from a processing thread, never from the device callback: a DeepFilterNet frame waits for the `deepfilter` thread.
pub struct Denoiser {
    rnn: Box<DenoiseState<'static>>,
    dfn: Option<Dfn>,
    eco: bool,
    pending: Vec<f32>,
    ready: VecDeque<f32>,
    frame_in: [f32; NN_FRAME],
    frame_out: [f32; NN_FRAME],
    gate: f32,
    active: bool,
    dfn_reset: bool,
    /// DeepFilterNet cost per frame (ms, moving average) and frames measured.
    pub cost_ms: f32,
    cost_n: u32,
    /// Voice probability (RNNoise) of the last frame, for diagnostics.
    pub vad: f32,
}

impl Default for Denoiser { fn default() -> Self { Self::new() } }

impl Denoiser {
    /// DeepFilterNet preferred (starts loading now; RNNoise meanwhile), unless it was too slow earlier in this session.
    pub fn new() -> Denoiser { let mut d = Self::rnnoise_only(); if !DFN_SLOW.load(Ordering::Relaxed) { d.dfn = Some(Dfn::spawn()); } d }
    /// RNNoise only.
    pub fn rnnoise_only() -> Denoiser {
        Denoiser { rnn: DenoiseState::new(), dfn: None, eco: false, pending: Vec::with_capacity(NN_FRAME * 4), ready: VecDeque::new(),
            frame_in: [0.0; NN_FRAME], frame_out: [0.0; NN_FRAME], gate: 1.0, active: false, dfn_reset: false, cost_ms: 0.0, cost_n: 0, vad: 0.0 }
    }
    /// Eco mode: RNNoise even when DeepFilterNet is available.
    pub fn set_eco(&mut self, eco: bool) { self.eco = eco; }
    /// Waits until DeepFilterNet is loaded (true) or unavailable (false). For tests and benchmarks.
    #[allow(dead_code)]
    pub fn wait_dfn(&self, timeout: Duration) -> bool {
        let t0 = Instant::now();
        while let Some(d) = &self.dfn { match d.state() { 1 => return true, 2 => return false, _ if t0.elapsed() > timeout => return false, _ => std::thread::sleep(Duration::from_millis(10)) } }
        false
    }
    /// Whether DeepFilterNet ran the last frame.
    #[allow(dead_code)]
    pub fn on_dfn(&self) -> bool { !self.eco && self.dfn.as_ref().is_some_and(|d| d.state() == 1) }

    /// Processes `input` (−1..1) in `mode` and appends the result to `dst`.
    pub fn process(&mut self, mode: SmartMic, input: &[f32], dst: &mut Vec<f32>) {
        if !mode.denoises() {
            // switching off: hand back what was still buffered so nothing is lost (at most 10 ms extra, once)
            if self.active { dst.extend(self.ready.drain(..)); dst.extend(self.pending.drain(..)); self.active = false; }
            set_status(Engine::Idle);
            dst.extend_from_slice(input);
            return;
        }
        if !self.active {
            // a fresh start: new engine states (no stale noise estimate) and one frame of silence as look-behind
            self.rnn = DenoiseState::new();
            self.dfn_reset = true;
            self.pending.clear();
            self.ready.clear();
            self.pending.resize(NN_FRAME, 0.0);
            self.gate = 1.0;
            self.active = true;
        }
        self.pending.extend_from_slice(input);
        let mut off = 0;
        while self.pending.len() - off >= NN_FRAME {
            self.frame_in.copy_from_slice(&self.pending[off..off + NN_FRAME]);
            off += NN_FRAME;
            let gain = self.frame(mode);
            let g0 = self.gate;
            // smooth the gate over frames (fast open, slower close) so it doesn't click
            let g1 = g0 + (gain - g0) * if gain > g0 { 0.6 } else { 0.15 };
            self.gate = g1;
            for (i, s) in self.frame_out.iter().enumerate() {
                let g = g0 + (g1 - g0) * i as f32 / NN_FRAME as f32;
                self.ready.push_back((s * g).clamp(-1.0, 1.0));
            }
        }
        self.pending.drain(..off);
        let n = input.len().min(self.ready.len()); // always input.len(), see the type's docs
        dst.extend(self.ready.drain(..n));
    }

    /// One frame `frame_in` -> `frame_out` (−1..1). Returns the gate target for it.
    fn frame(&mut self, mode: SmartMic) -> f32 {
        if !self.eco {
            match self.dfn.as_ref().map(|d| d.state()) {
                Some(1) => {
                    if self.dfn_frame(mode) { set_status(Engine::DeepFilter); return 1.0; }
                    // late or broken: RNNoise for this stream; too slow is remembered for the session
                    self.dfn = None;
                    DFN_SLOW.store(true, Ordering::Relaxed);
                    set_status(Engine::RnnFallback);
                }
                Some(2) => { self.dfn = None; set_status(Engine::LoadFailed); }
                Some(_) => set_status(Engine::Loading),
                // no DeepFilterNet on this stream: it failed to load (keep saying so) or was too slow
                None => if engine() != Engine::LoadFailed { set_status(Engine::RnnFallback); },
            }
        } else { set_status(Engine::RnnEco); }
        let mut i = [0f32; NN_FRAME];
        for (d, s) in i.iter_mut().zip(&self.frame_in) { *d = s * 32768.0; }
        let mut o = [0f32; NN_FRAME];
        let vad = self.rnn.process_frame(&mut o, &i);
        self.vad = vad;
        for (d, s) in self.frame_out.iter_mut().zip(&o) { *d = s / 32768.0; }
        if mode == SmartMic::Voice { voice_gate(vad) } else { 1.0 }
    }

    /// Sends the frame to the `deepfilter` thread and waits for it. False when it is late, failed or too slow on average.
    fn dfn_frame(&mut self, mode: SmartMic) -> bool {
        let Some(d) = &self.dfn else { return false };
        let t0 = Instant::now();
        let job = Job { frame: Box::new(self.frame_in), mode, reset: std::mem::take(&mut self.dfn_reset) };
        if d.tx.send(job).is_err() { return false; }
        let Ok(Ok(out)) = d.rx.recv_timeout(DEADLINE) else { return false };
        self.frame_out = *out;
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        // the first frames include warm-up (allocation, caches): measure from frame 20 on
        self.cost_n += 1;
        if self.cost_n > 20 { self.cost_ms = if self.cost_n == 21 { ms } else { self.cost_ms * 0.95 + ms * 0.05 }; }
        !(self.cost_n > 120 && self.cost_ms > SLOW_MS)
    }
}

/// "Nur Stimme" on RNNoise: gain from its voice probability — full above 0.6, down to −20 dB below 0.2.
pub fn voice_gate(vad: f32) -> f32 {
    let x = ((vad - 0.2) / 0.4).clamp(0.0, 1.0);
    0.1 + 0.9 * x * x * (3.0 - 2.0 * x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(v: &[f32]) -> f32 { if v.is_empty() { 0.0 } else { (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt() } }
    fn db(x: f32) -> f32 { 20.0 * x.max(1e-9).log10() }
    /// Deterministic white noise (xorshift), −amp..amp.
    fn noise(n: usize, amp: f32, seed: u32) -> Vec<f32> {
        let mut s = seed.max(1);
        (0..n).map(|_| { s ^= s << 13; s ^= s >> 17; s ^= s << 5; (s as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp }).collect()
    }
    /// Fan / air-conditioning-like noise: white noise through a one-pole low-pass.
    fn fan(n: usize, amp: f32, seed: u32) -> Vec<f32> {
        let mut y = 0.0f32;
        noise(n, 1.0, seed).into_iter().map(|x| { y = 0.97 * y + 0.03 * x; y * amp * 8.0 }).collect()
    }
    /// Speech-like test signal (source-filter vowel synthesis): a glottal pulse train with pitch movement and jitter,
    /// spectrally tilted, through three formant resonators; syllables at 4 Hz cycling the vowels /a/ /i/ /u/ (envelope 0 at
    /// t = k/4 s, peak at k/4 + 1/8 s). Speech models treat a plain harmonic buzz as tonal noise; this one they keep.
    fn speechlike(n: usize, amp: f32) -> Vec<f32> {
        const FS: f32 = 48_000.0;
        let vowels = [[(730.0, 80.0), (1090.0, 90.0), (2440.0, 120.0)], [(270.0, 60.0), (2290.0, 100.0), (3010.0, 120.0)], [(300.0, 60.0), (870.0, 80.0), (2240.0, 120.0)]];
        let mut rng = 0x2545_f491u32;
        let mut jitter = || { rng ^= rng << 13; rng ^= rng >> 17; rng ^= rng << 5; rng as f32 / u32::MAX as f32 - 0.5 };
        let (mut phase, mut tilt) = (0.0f32, 0.0f32);
        let mut st = [[0.0f32; 2]; 3];
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / FS;
            let syl = (t * 4.0) as usize;
            let env = (0.5 - 0.5 * (t * 4.0 * std::f32::consts::TAU).cos()).powf(0.7);
            let f0 = 120.0 + 25.0 * (t * 4.0 * std::f32::consts::PI).sin() * if syl % 2 == 0 { 1.0 } else { -1.0 };
            phase += f0 * (1.0 + 0.01 * jitter()) / FS;
            let pulse = if phase >= 1.0 { phase -= 1.0; 1.0 } else { 0.0 };
            tilt = 0.96 * tilt + pulse; // −12 dB/octave-ish glottal spectrum
            let mut y = 0.0;
            for (k, &(f, b)) in vowels[syl % 3].iter().enumerate() {
                let r = (-std::f32::consts::PI * b / FS).exp();
                let c = 2.0 * r * (std::f32::consts::TAU * f / FS).cos();
                let v = (1.0 - r) * tilt + c * st[k][0] - r * r * st[k][1];
                st[k][1] = st[k][0]; st[k][0] = v;
                y += v / (k + 1) as f32;
            }
            out.push(y * env);
        }
        let peak = out.iter().fold(0.0f32, |a, &b| a.max(b.abs())).max(1e-9);
        out.iter_mut().for_each(|s| *s *= amp / peak);
        out
    }
    /// Feeds `x` in device-sized blocks (441 samples, not a multiple of the frame) and collects the output.
    fn feed(d: &mut Denoiser, mode: SmartMic, x: &[f32]) -> Vec<f32> {
        let mut out = vec![];
        for b in x.chunks(441) { d.process(mode, b, &mut out); }
        out
    }
    fn run(mode: SmartMic, x: &[f32]) -> Vec<f32> { feed(&mut Denoiser::rnnoise_only(), mode, x) }
    /// DeepFilterNet, loaded before the first sample.
    fn dfn() -> Denoiser { let d = Denoiser::new(); assert!(d.wait_dfn(Duration::from_secs(60)), "DeepFilterNet loads"); d }

    /// Segmental SNR of a voiced signal at the syllable peaks against the pauses (both 25 ms windows), skipping the first
    /// second; `lag` = the processing delay to undo.
    /// Talk / pause test signal: `SEG` of speech, `SEG` of silence, repeated (20 ms ramps), so there are real pauses as in a
    /// conversation. Returns the signal and whether segment k is speech.
    const SEG: usize = 28_800; // 0.6 s
    fn talk(n: usize, amp: f32) -> Vec<f32> {
        let s = speechlike(n, amp);
        let ramp = 960.0;
        s.iter().enumerate().map(|(i, v)| {
            let (k, p) = (i / SEG, (i % SEG) as f32);
            let g = if k % 2 == 0 { (p / ramp).min((SEG as f32 - p) / ramp).min(1.0) } else { 0.0 };
            v * g
        }).collect()
    }
    /// Speech-to-noise ratio (dB): mean power of the speech segments' middles against the pauses' middles (the middle 0.3 s
    /// of each 0.6 s segment, so a processing delay of up to 0.15 s doesn't matter), skipping the first 1.2 s; `lag` undoes
    /// the processing delay.
    fn talk_snr(y: &[f32], lag: usize) -> f32 {
        let (mut s, mut n) = (0.0f64, 0.0f64);
        for k in 2..y.len() / SEG - 1 {
            let c = k * SEG + SEG / 4 + lag;
            let p = y[c..c + SEG / 2].iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
            if k % 2 == 0 { s += p } else { n += p }
        }
        (10.0 * (s / n.max(1e-18)).log10()) as f32
    }
    /// Processing delay of `y` against `x`: the shift (0..150 ms) that best matches their 10 ms energy envelopes (the
    /// waveform is periodic and the denoisers change it, the envelope is neither).
    fn lag_of(x: &[f32], y: &[f32]) -> usize {
        let env = |v: &[f32]| v.chunks(480).map(|c| c.iter().map(|s| s * s).sum::<f32>().sqrt()).collect::<Vec<f32>>();
        let (ex, ey) = (env(x), env(y));
        let best = (0..15).max_by(|&a, &b| {
            let c = |l: usize| (0..ex.len() - 15).map(|i| ex[i] * ey[i + l]).sum::<f32>();
            c(a).partial_cmp(&c(b)).unwrap()
        }).unwrap();
        best * 480
    }

    #[test]
    fn modes_round_trip() {
        for m in [SmartMic::Auto, SmartMic::Off, SmartMic::Voice, SmartMic::All] {
            assert_eq!(SmartMic::from_key(m.key()), m);
            assert_eq!(SmartMic::from_u8(m.to_u8()), m);
        }
        assert_eq!(SmartMic::from_key("bogus"), SmartMic::Auto);
    }

    #[test]
    fn off_and_music_mode_pass_through_unchanged() {
        let x = noise(48_000, 0.3, 7);
        assert_eq!(run(SmartMic::Off, &x), x);
        assert_eq!(run(SmartMic::All, &x), x);
        let mut d = dfn();
        assert_eq!(feed(&mut d, SmartMic::All, &x), x);
    }

    #[test]
    fn denoising_keeps_the_stream_length() {
        let x = noise(48_000 + 123, 0.1, 3);
        for m in [SmartMic::Auto, SmartMic::Voice] { assert_eq!(run(m, &x).len(), x.len()); }
        let mut d = dfn();
        assert_eq!(feed(&mut d, SmartMic::Auto, &x).len(), x.len());
    }

    #[test]
    fn rnnoise_removes_noise_alone() {
        let x = fan(48_000 * 2, 0.1, 11);
        let y = run(SmartMic::Auto, &x);
        let (a, b) = (rms(&x[48_000..]), rms(&y[48_000..])); // after the first second (the model's noise estimate settles)
        assert!(b < a * 0.25, "noise rms {a} -> {b}");
    }

    #[test]
    fn deepfilter_removes_noise_alone_and_respects_the_limits() {
        let x = fan(48_000 * 3, 0.1, 11);
        let mut d = dfn();
        let auto = feed(&mut d, SmartMic::Auto, &x);
        assert!(d.on_dfn());
        let voice = feed(&mut dfn(), SmartMic::Voice, &x);
        let (a, b, v) = (db(rms(&x[48_000..])), db(rms(&auto[48_000..])), db(rms(&voice[48_000..])));
        eprintln!("DeepFilterNet noise only: in {a:.1} dB, Automatisch {b:.1} dB, Nur Stimme {v:.1} dB");
        // Automatisch is limited to AUTO_LIM_DB of attenuation (a little slack for the analysis window); Nur Stimme goes further
        assert!(a - b > AUTO_LIM_DB - 6.0 && a - b < AUTO_LIM_DB + 3.0, "auto attenuation {:.1} dB", a - b);
        assert!(v < b - 6.0, "voice-only {v:.1} vs auto {b:.1}");
    }

    #[test]
    fn snr_improves_with_both_engines() {
        let n = SEG * 12; // 7.2 s: six speech bursts, six pauses
        let clean = talk(n, 0.5);
        let x: Vec<f32> = clean.iter().zip(&fan(n, 0.05, 5)).map(|(a, b)| a + b).collect();
        let snr_in = talk_snr(&x, 0);
        let mut results = vec![];
        let mut runs: Vec<(&str, Vec<f32>)> = vec![("RNNoise Automatisch", run(SmartMic::Auto, &x)), ("RNNoise Nur Stimme", run(SmartMic::Voice, &x))];
        runs.push(("DeepFilterNet Automatisch", feed(&mut dfn(), SmartMic::Auto, &x)));
        runs.push(("DeepFilterNet Nur Stimme", feed(&mut dfn(), SmartMic::Voice, &x)));
        // the voice itself must be kept: speech-segment power within 3 dB of the clean signal's (noise removed from it too)
        let speech_db = |y: &[f32], lag: usize| {
            let p: f64 = (2..n / SEG - 1).filter(|k| k % 2 == 0).map(|k| { let c = k * SEG + SEG / 4 + lag; y[c..c + SEG / 2].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() }).sum();
            (10.0 * p.log10()) as f32
        };
        let clean_db = speech_db(&clean, 0);
        for (name, y) in &runs {
            let lag = lag_of(&x, y);
            let snr = talk_snr(y, lag);
            let kept = speech_db(y, lag) - clean_db;
            eprintln!("{name}: speech-to-noise {snr_in:.1} -> {snr:.1} dB (+{:.1}), speech level {kept:+.1} dB, delay {:.0} ms", snr - snr_in, lag as f32 / 48.0);
            results.push((name.to_string(), snr, kept));
        }
        for (name, snr, kept) in &results {
            assert!(*snr > snr_in + 10.0, "{name}: {snr_in:.1} -> {snr:.1} dB");
            assert!(kept.abs() < 3.0, "{name}: speech level {kept:+.1} dB");
        }
        // Automatisch is limited (AUTO_LIM_DB); Nur Stimme removes more
        assert!(results[3].1 > results[2].1 + 3.0, "DeepFilterNet Nur Stimme {:.1} vs Automatisch {:.1}", results[3].1, results[2].1);
        assert!(results[1].1 >= results[0].1 - 0.5, "RNNoise Nur Stimme {:.1} vs Automatisch {:.1}", results[1].1, results[0].1);
    }

    #[test]
    fn rnnoise_voice_only_is_stronger_than_auto() {
        let x = fan(48_000 * 2, 0.1, 13);
        let (a, v) = (run(SmartMic::Auto, &x), run(SmartMic::Voice, &x));
        assert!(rms(&v[48_000..]) <= rms(&a[48_000..]) + 1e-6, "voice {} auto {}", rms(&v[48_000..]), rms(&a[48_000..]));
    }

    #[test]
    fn eco_mode_uses_rnnoise() {
        let x = fan(48_000, 0.1, 19);
        let mut d = dfn();
        d.set_eco(true);
        let y = feed(&mut d, SmartMic::Auto, &x);
        assert!(!d.on_dfn());
        assert_eq!(y, run(SmartMic::Auto, &x), "eco = the RNNoise result");
    }

    #[test]
    fn gate_curve() {
        assert!((voice_gate(1.0) - 1.0).abs() < 1e-6);
        assert!((voice_gate(0.0) - 0.1).abs() < 1e-6);
        assert!(voice_gate(0.4) > voice_gate(0.3));
    }

    #[test]
    fn switching_modes_mid_stream_keeps_length() {
        let x = noise(48_000, 0.1, 17);
        let mut d = dfn();
        let mut out = vec![];
        for (i, b) in x.chunks(500).enumerate() { d.process(if i % 3 == 0 { SmartMic::Off } else { SmartMic::Auto }, b, &mut out); }
        // switching off returns what was buffered, so the total can only exceed the input by the buffered look-behind
        assert!(out.len() >= x.len() && out.len() <= x.len() + 40 * NN_FRAME, "{} vs {}", out.len(), x.len());
    }

    /// Frame-timing benchmark: `cargo test --release --bin webox media::denoise::tests::bench -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench() {
        let x: Vec<f32> = talk(48_000 * 10, 0.5).iter().zip(&fan(48_000 * 10, 0.05, 3)).map(|(a, b)| a + b).collect();
        let t = Instant::now();
        let m = build_dfn().unwrap();
        eprintln!("DeepFilterNet model build: {:.0} ms", t.elapsed().as_secs_f32() * 1000.0);
        let mut m = m;
        for (name, mode) in [("Automatisch", SmartMic::Auto), ("Nur Stimme", SmartMic::Voice)] {
            dfn_mode(&mut m, mode);
            let mut times = vec![];
            let mut out = [0f32; NN_FRAME];
            for f in x.chunks_exact(NN_FRAME) {
                let f: &[f32; NN_FRAME] = f.try_into().unwrap();
                let t = Instant::now();
                dfn_frame(&mut m, f, &mut out).unwrap();
                times.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            report(&format!("DeepFilterNet {name}"), &mut times);
        }
        let mut st = DenoiseState::new();
        let mut times = vec![];
        let mut o = [0f32; NN_FRAME];
        for f in x.chunks_exact(NN_FRAME) {
            let i: Vec<f32> = f.iter().map(|s| s * 32768.0).collect();
            let t = Instant::now();
            st.process_frame(&mut o, &i);
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        report("RNNoise", &mut times);
        // the whole path as the microphone runs it: Denoiser + the deepfilter thread hand-off
        let mut d = dfn();
        let t = Instant::now();
        let y = feed(&mut d, SmartMic::Auto, &x);
        let el = t.elapsed().as_secs_f64();
        eprintln!("Denoiser (DeepFilterNet, thread hand-off): 10 s of audio in {:.0} ms = {:.1} % of real time, still on DeepFilterNet: {}",
            el * 1000.0, el / 10.0 * 100.0, d.on_dfn());
        assert_eq!(y.len(), x.len());
    }
    fn report(name: &str, t: &mut [f64]) {
        let warm = &mut t[20..];
        warm.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mean = warm.iter().sum::<f64>() / warm.len() as f64;
        let p = |q: f64| warm[((warm.len() - 1) as f64 * q) as usize];
        eprintln!("{name}: per 10 ms frame mean {mean:.3} ms, median {:.3}, p99 {:.3}, max {:.3} -> {:.1} % of one core",
            p(0.5), p(0.99), warm[warm.len() - 1], mean / 10.0 * 100.0);
    }
}
