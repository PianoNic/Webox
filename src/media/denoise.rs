//! Microphone noise removal (Smart audio > Microphone audio), 48 kHz mono in 10 ms frames.
//!
//! - `Auto` ("Auto"): RNNoise (`nnnoiseless`). Low delay, almost no CPU.
//! - `Voice` ("Voice only"): DeepFilterNet 3 (libDF + tract) without an attenuation limit, plus its post filter. Loaded on
//!   first use on a `deepfilter` thread; RNNoise with a voice gate covers until it is ready, and for the rest of the
//!   session if it fails to load, misses a frame deadline or uses more than 70 % of real time.
//! - `Off` ("Off") and `All` ("All sounds"): the signal passes unchanged.
use df::tract::{DfParams, DfTract, ReduceMask, RuntimeParams};
use nnnoiseless::DenoiseState;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Frame of both engines: 10 ms at 48 kHz.
pub const NN_FRAME: usize = 480;
const _: () = assert!(DenoiseState::FRAME_SIZE == NN_FRAME);
/// DeepFilterNet post-filter strength.
const VOICE_PF_BETA: f32 = 0.02;
/// A DeepFilterNet frame that takes longer than this is late: the stream falls back to RNNoise.
const DEADLINE: Duration = Duration::from_millis(40);
/// Average DeepFilterNet cost per 10 ms frame above which it is dropped (70 % of real time).
const SLOW_MS: f32 = 7.0;

static MODEL: &[u8] = include_bytes!("../../assets/models/deepfilternet3/DeepFilterNet3_onnx.tar.gz");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SmartMic { #[default] Auto, Off, Voice, All }

impl SmartMic {
    /// Settings value (`smart_mic`): "auto" | "off" | "voice" | "all".
    pub fn from_key(s: &str) -> SmartMic { match s { "off" => SmartMic::Off, "voice" => SmartMic::Voice, "all" => SmartMic::All, _ => SmartMic::Auto } }
    pub fn key(self) -> &'static str { match self { SmartMic::Auto => "auto", SmartMic::Off => "off", SmartMic::Voice => "voice", SmartMic::All => "all" } }
    pub fn to_u8(self) -> u8 { self as u8 }
    pub fn from_u8(v: u8) -> SmartMic { match v { 1 => SmartMic::Off, 2 => SmartMic::Voice, 3 => SmartMic::All, _ => SmartMic::Auto } }
    pub fn denoises(self) -> bool { matches!(self, SmartMic::Auto | SmartMic::Voice) }
}

/// What the microphone path runs (shown on the Smart Audio page).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine { Idle, Rnnoise, Loading, DeepFilter, RnnFallback, LoadFailed }
static STATUS: AtomicU8 = AtomicU8::new(0);
/// DeepFilterNet was too slow on this machine: later streams don't try it again.
static DFN_SLOW: AtomicBool = AtomicBool::new(false);
fn set_status(e: Engine) { STATUS.store(e as u8, Ordering::Relaxed); }
pub fn engine() -> Engine {
    match STATUS.load(Ordering::Relaxed) { 1 => Engine::Rnnoise, 2 => Engine::Loading, 3 => Engine::DeepFilter, 4 => Engine::RnnFallback, 5 => Engine::LoadFailed, _ => Engine::Idle }
}

/// DeepFilterNet from the bundled model, set up for "Nur Stimme" (48 kHz, 10 ms hops).
pub fn build_dfn() -> Result<DfTract, String> {
    let p = DfParams::from_bytes(MODEL).map_err(|e| format!("DeepFilterNet model: {e:#}"))?;
    let rp = RuntimeParams::default_with_ch(1).with_atten_lim(100.0).with_post_filter(VOICE_PF_BETA).with_mask_reduce(ReduceMask::NONE);
    let m = DfTract::new(p, &rp).map_err(|e| format!("DeepFilterNet: {e:#}"))?;
    if m.sr != 48_000 || m.hop_size != NN_FRAME { return Err(format!("DeepFilterNet: {} Hz / hop {}", m.sr, m.hop_size)); }
    Ok(m)
}
/// One 10 ms frame through DeepFilterNet (samples −1..1).
pub fn dfn_frame(m: &mut DfTract, input: &[f32; NN_FRAME], out: &mut [f32; NN_FRAME]) -> Result<f32, String> {
    let noisy = ndarray::ArrayView2::from_shape((1, NN_FRAME), &input[..]).map_err(|e| e.to_string())?;
    let enh = ndarray::ArrayViewMut2::from_shape((1, NN_FRAME), &mut out[..]).map_err(|e| e.to_string())?;
    m.process(noisy, enh).map_err(|e| e.to_string())
}

struct Job { frame: Box<[f32; NN_FRAME]>, reset: bool }
/// The `deepfilter` thread: builds the model, then processes the frames it is sent in order.
/// `ready`: 0 loading, 1 ready, 2 failed.
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
            for j in jobs {
                if j.reset { let _ = m.init(); }
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

/// Streaming noise remover. Input of any block size is cut into 10 ms frames; each finished frame is appended to the
/// output right away, so a call returns whole frames only and keeps less than one frame back (no added look-behind).
/// Call it from a processing thread: a DeepFilterNet frame waits for the `deepfilter` thread.
pub struct Denoiser {
    rnn: Box<DenoiseState<'static>>,
    dfn: Option<Dfn>,
    /// Never start DeepFilterNet (tests of the RNNoise path).
    rnn_only: bool,
    pending: Vec<f32>,
    frame_in: [f32; NN_FRAME],
    frame_out: [f32; NN_FRAME],
    gate: f32,
    active: bool,
    dfn_reset: bool,
    /// DeepFilterNet cost per frame (ms, moving average after warm-up).
    pub cost_ms: f32,
    cost_n: u32,
}

impl Default for Denoiser { fn default() -> Self { Self::new() } }

impl Denoiser {
    pub fn new() -> Denoiser {
        Denoiser { rnn: DenoiseState::new(), dfn: None, rnn_only: false, pending: Vec::with_capacity(NN_FRAME * 2), frame_in: [0.0; NN_FRAME],
            frame_out: [0.0; NN_FRAME], gate: 1.0, active: false, dfn_reset: false, cost_ms: 0.0, cost_n: 0 }
    }
    #[allow(dead_code)]
    pub fn rnnoise_only() -> Denoiser { Denoiser { rnn_only: true, ..Self::new() } }
    /// Starts loading DeepFilterNet now and waits for it (tests and benchmarks).
    #[allow(dead_code)]
    pub fn wait_dfn(&mut self, timeout: Duration) -> bool {
        if self.dfn.is_none() { self.dfn = Some(Dfn::spawn()); }
        let t0 = Instant::now();
        while let Some(d) = &self.dfn { match d.state() { 1 => return true, 2 => return false, _ if t0.elapsed() > timeout => return false, _ => std::thread::sleep(Duration::from_millis(10)) } }
        false
    }
    /// Whether DeepFilterNet is loaded and in use.
    #[allow(dead_code)]
    pub fn on_dfn(&self) -> bool { self.dfn.as_ref().is_some_and(|d| d.state() == 1) }

    /// Processes `input` (−1..1) in `mode` and appends the result to `dst`.
    pub fn process(&mut self, mode: SmartMic, input: &[f32], dst: &mut Vec<f32>) {
        if !mode.denoises() {
            // switching off: hand back the unfinished frame unprocessed
            if self.active { dst.append(&mut self.pending); self.active = false; }
            set_status(Engine::Idle);
            dst.extend_from_slice(input);
            return;
        }
        if !self.active {
            self.rnn = DenoiseState::new();
            self.dfn_reset = true;
            self.pending.clear();
            self.gate = 1.0;
            self.active = true;
        }
        if mode == SmartMic::Voice && self.dfn.is_none() && !self.rnn_only && !DFN_SLOW.load(Ordering::Relaxed) && engine() != Engine::LoadFailed {
            self.dfn = Some(Dfn::spawn());
        }
        self.pending.extend_from_slice(input);
        let mut off = 0;
        while self.pending.len() - off >= NN_FRAME {
            self.frame_in.copy_from_slice(&self.pending[off..off + NN_FRAME]);
            off += NN_FRAME;
            let gain = self.frame(mode);
            // smooth the gate over frames (fast open, slower close) so it doesn't click
            let g0 = self.gate;
            let g1 = g0 + (gain - g0) * if gain > g0 { 0.6 } else { 0.15 };
            self.gate = g1;
            dst.extend(self.frame_out.iter().enumerate().map(|(i, s)| (s * (g0 + (g1 - g0) * i as f32 / NN_FRAME as f32)).clamp(-1.0, 1.0)));
        }
        self.pending.drain(..off);
    }

    /// `frame_in` -> `frame_out`. Returns the gate target for the frame.
    fn frame(&mut self, mode: SmartMic) -> f32 {
        if mode == SmartMic::Voice {
            match self.dfn.as_ref().map(|d| d.state()) {
                Some(1) => {
                    if self.dfn_frame() { set_status(Engine::DeepFilter); return 1.0; }
                    self.dfn = None;
                    DFN_SLOW.store(true, Ordering::Relaxed);
                    set_status(Engine::RnnFallback);
                }
                Some(2) => { self.dfn = None; set_status(Engine::LoadFailed); }
                Some(_) => set_status(Engine::Loading),
                None => if engine() != Engine::LoadFailed { set_status(Engine::RnnFallback); },
            }
        } else {
            set_status(Engine::Rnnoise);
        }
        let mut i = [0f32; NN_FRAME];
        for (d, s) in i.iter_mut().zip(&self.frame_in) { *d = s * 32768.0; }
        let mut o = [0f32; NN_FRAME];
        let vad = self.rnn.process_frame(&mut o, &i);
        for (d, s) in self.frame_out.iter_mut().zip(&o) { *d = s / 32768.0; }
        if mode == SmartMic::Voice { voice_gate(vad) } else { 1.0 }
    }

    /// Sends the frame to the `deepfilter` thread and waits for it. False when late, failed or too slow on average.
    fn dfn_frame(&mut self) -> bool {
        let Some(d) = &self.dfn else { return false };
        let t0 = Instant::now();
        if d.tx.send(Job { frame: Box::new(self.frame_in), reset: std::mem::take(&mut self.dfn_reset) }).is_err() { return false; }
        let Ok(Ok(out)) = d.rx.recv_timeout(DEADLINE) else { return false };
        self.frame_out = *out;
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        self.cost_n += 1;
        if self.cost_n > 20 { self.cost_ms = if self.cost_n == 21 { ms } else { self.cost_ms * 0.95 + ms * 0.05 }; }
        !(self.cost_n > 120 && self.cost_ms > SLOW_MS)
    }
}

/// RNNoise "Nur Stimme": gain from its voice probability, full above 0.6, down to −20 dB below 0.2.
pub fn voice_gate(vad: f32) -> f32 {
    let x = ((vad - 0.2) / 0.4).clamp(0.0, 1.0);
    0.1 + 0.9 * x * x * (3.0 - 2.0 * x)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn rms(v: &[f32]) -> f32 { if v.is_empty() { 0.0 } else { (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt() } }
    pub(crate) fn db(x: f32) -> f32 { 20.0 * x.max(1e-9).log10() }
    /// Deterministic white noise (xorshift), −amp..amp.
    pub(crate) fn noise(n: usize, amp: f32, seed: u32) -> Vec<f32> {
        let mut s = seed.max(1);
        (0..n).map(|_| { s ^= s << 13; s ^= s >> 17; s ^= s << 5; (s as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp }).collect()
    }
    /// Fan-like noise: white noise through a one-pole low-pass.
    pub(crate) fn fan(n: usize, amp: f32, seed: u32) -> Vec<f32> {
        let mut y = 0.0f32;
        noise(n, 1.0, seed).into_iter().map(|x| { y = 0.97 * y + 0.03 * x; y * amp * 8.0 }).collect()
    }
    /// Speech-like signal: a jittered glottal pulse train through three formant resonators, syllables at 4 Hz cycling the
    /// vowels /a/ /i/ /u/. `seed` varies the jitter.
    pub(crate) fn speechlike_seed(n: usize, amp: f32, seed: u32) -> Vec<f32> {
        const FS: f32 = 48_000.0;
        let vowels = [[(730.0, 80.0), (1090.0, 90.0), (2440.0, 120.0)], [(270.0, 60.0), (2290.0, 100.0), (3010.0, 120.0)], [(300.0, 60.0), (870.0, 80.0), (2240.0, 120.0)]];
        let mut rng = 0x2545_f491u32 ^ seed;
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
            tilt = 0.96 * tilt + pulse;
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
    pub(crate) fn speechlike(n: usize, amp: f32) -> Vec<f32> { speechlike_seed(n, amp, 0) }
    /// Feeds `x` in device-sized blocks (441 samples, not a multiple of the frame) and collects the output.
    fn feed(d: &mut Denoiser, mode: SmartMic, x: &[f32]) -> Vec<f32> {
        let mut out = vec![];
        for b in x.chunks(441) { d.process(mode, b, &mut out); }
        out
    }
    fn run(mode: SmartMic, x: &[f32]) -> Vec<f32> { feed(&mut Denoiser::rnnoise_only(), mode, x) }
    /// DeepFilterNet, loaded before the first sample.
    fn dfn() -> Denoiser { let mut d = Denoiser::new(); assert!(d.wait_dfn(Duration::from_secs(60)), "DeepFilterNet loads"); d }

    /// Talk / pause signal: 0.6 s of speech, 0.6 s of silence, repeated (20 ms ramps).
    const SEG: usize = 28_800;
    fn talk(n: usize, amp: f32) -> Vec<f32> {
        let s = speechlike(n, amp);
        s.iter().enumerate().map(|(i, v)| {
            let (k, p) = (i / SEG, (i % SEG) as f32);
            v * if k % 2 == 0 { (p / 960.0).min((SEG as f32 - p) / 960.0).min(1.0) } else { 0.0 }
        }).collect()
    }
    /// Speech-to-noise ratio (dB): speech segments' middles against the pauses' middles, from 1.2 s on; `lag` undoes the
    /// processing delay.
    fn talk_snr(y: &[f32], lag: usize) -> f32 {
        let (mut s, mut n) = (0.0f64, 0.0f64);
        for k in 2..y.len() / SEG - 1 {
            let c = k * SEG + SEG / 4 + lag;
            let p = y[c..c + SEG / 2].iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
            if k % 2 == 0 { s += p } else { n += p }
        }
        (10.0 * (s / n.max(1e-18)).log10()) as f32
    }
    /// Sample-exact delay of `y` against `x` and the normalised correlation there: coarse from 10 ms energy envelopes
    /// (0..200 ms), then every sample within ±20 ms of it over 2 s of signal.
    pub(crate) fn exact_lag(x: &[f32], y: &[f32]) -> (usize, f32) {
        let env = |v: &[f32]| v.chunks(480).map(|c| c.iter().map(|s| s * s).sum::<f32>().sqrt()).collect::<Vec<f32>>();
        let (ex, ey) = (env(x), env(y));
        let coarse = (0..20).max_by(|&a, &b| {
            let c = |l: usize| (0..ex.len().saturating_sub(20).min(ey.len().saturating_sub(20))).map(|i| ex[i] * ey[i + l]).sum::<f32>();
            c(a).partial_cmp(&c(b)).unwrap()
        }).unwrap() * 480;
        let r = 48_000..48_000 * 3;
        let nx = r.clone().map(|i| x[i] * x[i]).sum::<f32>().sqrt();
        (coarse.saturating_sub(960)..coarse + 960).map(|l| {
            let ny = r.clone().map(|i| y[i + l] * y[i + l]).sum::<f32>().sqrt().max(1e-9);
            (l, r.clone().map(|i| x[i] * y[i + l]).sum::<f32>() / (nx * ny))
        }).max_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).unwrap()
    }
    /// Delay through the denoiser on clean speech (both engines pass it almost unchanged, so the correlation peak is sharp).
    fn latency_of(mut d: Denoiser, mode: SmartMic) -> (usize, f32) {
        let x = speechlike(48_000 * 4, 0.5);
        let mut y = feed(&mut d, mode, &x);
        y.resize(x.len() + 48_000, 0.0);
        exact_lag(&x, &y)
    }

    #[test]
    fn processing_delay() {
        let (r, rc) = latency_of(Denoiser::new(), SmartMic::Auto);
        let (d, dc) = latency_of(dfn(), SmartMic::Voice);
        eprintln!("delay: Automatisch (RNNoise) {r} samples = {:.2} ms (corr {rc:.3}), Nur Stimme (DeepFilterNet) {d} samples = {:.2} ms (corr {dc:.3})",
            r as f32 / 48.0, d as f32 / 48.0);
        // RNNoise: its one frame of overlap; DeepFilterNet: STFT overlap + two frames of model lookahead
        assert!(r.abs_diff(NN_FRAME) <= 8 && d.abs_diff(3 * NN_FRAME) <= 8, "{r} / {d} samples");
        assert!(rc > 0.9 && dc > 0.7, "clean speech passes nearly unchanged ({rc}, {dc})");
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
        assert_eq!(feed(&mut dfn(), SmartMic::All, &x), x);
    }

    #[test]
    fn output_is_whole_frames_with_less_than_a_frame_held_back() {
        let x = noise(48_000 + 123, 0.1, 3);
        for mut d in [Denoiser::rnnoise_only(), dfn()] {
            for m in [SmartMic::Auto, SmartMic::Voice] {
                let y = feed(&mut d, m, &x);
                assert_eq!(y.len(), x.len() / NN_FRAME * NN_FRAME, "{m:?}");
            }
        }
        // frame-sized blocks come back at once
        let mut d = Denoiser::rnnoise_only();
        let mut out = vec![];
        d.process(SmartMic::Auto, &x[..NN_FRAME], &mut out);
        assert_eq!(out.len(), NN_FRAME);
    }

    #[test]
    fn automatisch_runs_rnnoise_and_removes_noise() {
        let x = fan(48_000 * 2, 0.1, 11);
        let mut d = Denoiser::new();
        let y = feed(&mut d, SmartMic::Auto, &x);
        assert!(!d.on_dfn() && d.dfn.is_none(), "Automatisch doesn't load DeepFilterNet");
        let (a, b) = (rms(&x[48_000..]), rms(&y[48_000 - NN_FRAME..y.len() - NN_FRAME]));
        assert!(b < a * 0.25, "noise rms {a} -> {b}");
    }

    #[test]
    fn nur_stimme_runs_deepfilternet_and_removes_more() {
        let x = fan(48_000 * 3, 0.1, 11);
        let mut d = dfn();
        let voice = feed(&mut d, SmartMic::Voice, &x);
        assert!(d.on_dfn());
        let auto = run(SmartMic::Auto, &x);
        let (a, r, v) = (db(rms(&x[48_000..])), db(rms(&auto[48_000..])), db(rms(&voice[48_000..])));
        eprintln!("noise only: in {a:.1} dB, Automatisch (RNNoise) {r:.1} dB, Nur Stimme (DeepFilterNet) {v:.1} dB");
        assert!(v < r - 6.0, "Nur Stimme {v:.1} vs Automatisch {r:.1}");
    }

    #[test]
    fn snr_improves_and_speech_is_kept() {
        let n = SEG * 12;
        let clean = talk(n, 0.5);
        let x: Vec<f32> = clean.iter().zip(&fan(n, 0.05, 5)).map(|(a, b)| a + b).collect();
        let snr_in = talk_snr(&x, 0);
        let pad = |mut y: Vec<f32>| { y.resize(n, 0.0); y };
        let runs = [("Automatisch (RNNoise)", pad(run(SmartMic::Auto, &x)), NN_FRAME),
            ("Nur Stimme (DeepFilterNet)", pad(feed(&mut dfn(), SmartMic::Voice, &x)), 3 * NN_FRAME),
            ("Nur Stimme fallback (RNNoise + gate)", pad(run(SmartMic::Voice, &x)), NN_FRAME)];
        let speech_db = |y: &[f32], lag: usize| {
            let p: f64 = (2..n / SEG - 1).filter(|k| k % 2 == 0).map(|k| { let c = k * SEG + SEG / 4 + lag; y[c..c + SEG / 2].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() }).sum();
            (10.0 * p.log10()) as f32
        };
        let clean_db = speech_db(&clean, 0);
        for (name, y, lag) in &runs {
            let snr = talk_snr(y, *lag);
            let kept = speech_db(y, *lag) - clean_db;
            eprintln!("{name}: speech-to-noise {snr_in:.1} -> {snr:.1} dB (+{:.1}), speech level {kept:+.1} dB", snr - snr_in);
            assert!(snr > snr_in + 10.0, "{name}: {snr_in:.1} -> {snr:.1} dB");
            assert!(kept.abs() < 3.0, "{name}: speech level {kept:+.1} dB");
        }
    }

    #[test]
    fn gate_curve() {
        assert!((voice_gate(1.0) - 1.0).abs() < 1e-6);
        assert!((voice_gate(0.0) - 0.1).abs() < 1e-6);
        assert!(voice_gate(0.4) > voice_gate(0.3));
    }

    #[test]
    fn switching_modes_mid_stream_loses_nothing() {
        let x = noise(48_000, 0.1, 17);
        let mut d = Denoiser::rnnoise_only();
        let mut out = vec![];
        for (i, b) in x.chunks(500).enumerate() { d.process(if i % 3 == 0 { SmartMic::Off } else { SmartMic::Auto }, b, &mut out); }
        // what is still held back is less than one frame
        assert!(out.len() <= x.len() && x.len() - out.len() < NN_FRAME, "{} vs {}", out.len(), x.len());
    }

    /// Frame cost: `cargo test --release --bin webox media::denoise::tests::bench -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench() {
        let x: Vec<f32> = talk(48_000 * 10, 0.5).iter().zip(&fan(48_000 * 10, 0.05, 3)).map(|(a, b)| a + b).collect();
        let t = Instant::now();
        let mut m = build_dfn().unwrap();
        eprintln!("DeepFilterNet model build: {:.0} ms", t.elapsed().as_secs_f32() * 1000.0);
        let mut times = vec![];
        let mut out = [0f32; NN_FRAME];
        for f in x.chunks_exact(NN_FRAME) {
            let t = Instant::now();
            dfn_frame(&mut m, f.try_into().unwrap(), &mut out).unwrap();
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        report("DeepFilterNet", &mut times);
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
    }
    pub(crate) fn report(name: &str, t: &mut [f64]) {
        let warm = &mut t[20..];
        warm.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mean = warm.iter().sum::<f64>() / warm.len() as f64;
        let p = |q: f64| warm[((warm.len() - 1) as f64 * q) as usize];
        eprintln!("{name}: per 10 ms frame mean {mean:.3} ms, median {:.3}, p99 {:.3}, max {:.3} -> {:.1} % of one core",
            p(0.5), p(0.99), warm[warm.len() - 1], mean / 10.0 * 100.0);
    }
}
