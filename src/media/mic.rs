//! The microphone chain, 48 kHz mono in 10 ms frames: echo cancellation (WebRTC AEC3 with high-pass filter, `sonora`)
//! against what the speaker plays, then noise removal (`denoise`), then automatic gain (AGC2 adaptive digital, `sonora`).
//! Echo cancellation and gain control run in calls only; outside a call (the level meter) only noise removal runs.
use super::denoise::{Denoiser, SmartMic, NN_FRAME};
use sonora::config::{EchoCanceller, GainController2, AdaptiveDigital, HighPassFilter};
use sonora::{AudioProcessing, Config, StreamConfig};
use std::collections::VecDeque;

/// Far-end audio waiting for the echo canceller is capped at this (older audio is dropped).
pub const FAR_MAX: usize = 25 * NN_FRAME;

fn apm(config: Config) -> AudioProcessing {
    AudioProcessing::builder().config(config).capture_config(StreamConfig::new(48_000, 1)).render_config(StreamConfig::new(48_000, 1)).build()
}
/// Echo canceller with its high-pass filter.
pub fn echo_canceller() -> AudioProcessing {
    apm(Config { echo_canceller: Some(EchoCanceller::default()), high_pass_filter: Some(HighPassFilter::default()), ..Default::default() })
}
/// Adaptive digital gain with limiter.
pub fn gain_control() -> AudioProcessing {
    apm(Config { gain_controller2: Some(GainController2 { adaptive_digital: Some(AdaptiveDigital::default()), ..Default::default() }), ..Default::default() })
}

pub struct MicChain {
    dn: Denoiser,
    aec: Option<AudioProcessing>,
    agc: Option<AudioProcessing>,
    pending: Vec<f32>,
    agc_pending: Vec<f32>,
    frame: [f32; NN_FRAME],
    out: [f32; NN_FRAME],
    clean: Vec<f32>,
}

impl Default for MicChain { fn default() -> Self { Self::new() } }

impl MicChain {
    pub fn new() -> MicChain {
        MicChain { dn: Denoiser::new(), aec: None, agc: None, pending: vec![], agc_pending: vec![], frame: [0.0; NN_FRAME], out: [0.0; NN_FRAME], clean: vec![] }
    }

    /// Processes microphone audio `input` and appends the result to `dst`. `call`: echo cancellation and gain control on;
    /// `far`: what the speaker played since the last call (consumed here).
    pub fn process(&mut self, mode: SmartMic, call: bool, far: &mut VecDeque<f32>, input: &[f32], dst: &mut Vec<f32>) {
        if !call {
            if self.aec.is_some() { self.aec = None; self.agc = None; dst.append(&mut self.pending); dst.append(&mut self.agc_pending); }
            far.clear();
            self.dn.process(mode, input, dst);
            return;
        }
        let aec = self.aec.get_or_insert_with(echo_canceller);
        while far.len() > FAR_MAX { far.pop_front(); }
        self.pending.extend_from_slice(input);
        let mut off = 0;
        while self.pending.len() - off >= NN_FRAME {
            // render (far end) before the capture frame it may echo into
            while far.len() >= NN_FRAME {
                for (d, s) in self.frame.iter_mut().zip(far.drain(..NN_FRAME)) { *d = s; }
                let _ = aec.process_render_f32(&[&self.frame], &mut [&mut self.out]);
            }
            self.frame.copy_from_slice(&self.pending[off..off + NN_FRAME]);
            off += NN_FRAME;
            let _ = aec.process_capture_f32(&[&self.frame], &mut [&mut self.out]);
            self.clean.clear();
            self.dn.process(mode, &self.out, &mut self.clean);
            self.agc_pending.extend_from_slice(&self.clean);
        }
        self.pending.drain(..off);
        let agc = self.agc.get_or_insert_with(gain_control);
        let mut off = 0;
        while self.agc_pending.len() - off >= NN_FRAME {
            self.frame.copy_from_slice(&self.agc_pending[off..off + NN_FRAME]);
            off += NN_FRAME;
            let _ = agc.process_capture_f32(&[&self.frame], &mut [&mut self.out]);
            dst.extend_from_slice(&self.out);
        }
        self.agc_pending.drain(..off);
    }

    /// Echo return loss enhancement the echo canceller reports (dB), once it has an estimate.
    #[allow(dead_code)]
    pub fn erle_db(&self) -> Option<f64> { self.aec.as_ref().and_then(|a| a.statistics().echo_return_loss_enhancement) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::denoise::tests::{db, noise, report, rms, speechlike_seed};
    use std::time::Instant;

    /// A room: the far-end signal comes back into the microphone `delay` samples later through a decaying 30 ms impulse
    /// response, `gain` overall.
    fn room(far: &[f32], delay: usize, gain: f32) -> Vec<f32> {
        let taps: Vec<f32> = noise(1440, 1.0, 99).iter().enumerate().map(|(i, r)| r * (-(i as f32) / 240.0).exp()).collect();
        let norm = taps.iter().map(|t| t * t).sum::<f32>().sqrt();
        let mut y = vec![0.0f32; far.len()];
        for (i, out) in y.iter_mut().enumerate().skip(delay) {
            let j = i - delay;
            *out = taps.iter().enumerate().take(j + 1).map(|(k, t)| t * far[j - k]).sum::<f32>() * gain / norm;
        }
        y
    }
    /// Runs the chain over a call: each 10 ms the speaker plays `far` and the microphone records `mic`.
    fn call(chain: &mut MicChain, mode: SmartMic, far: &[f32], mic: &[f32]) -> Vec<f32> {
        let mut q = VecDeque::new();
        let mut out = vec![];
        for (f, m) in far.chunks(NN_FRAME).zip(mic.chunks(NN_FRAME)) {
            q.extend(f.iter().copied());
            chain.process(mode, true, &mut q, m, &mut out);
        }
        out
    }

    #[test]
    fn echo_is_cancelled() {
        let n = 48_000 * 10;
        let far = speechlike_seed(n, 0.5, 1);
        let echo = room(&far, 1920, 0.5); // 40 ms acoustic + device delay, −6 dB
        let floor = noise(n, 0.001, 4);
        let mic: Vec<f32> = echo.iter().zip(&floor).map(|(a, b)| a + b).collect();
        let mut chain = MicChain::new();
        let out = call(&mut chain, SmartMic::Off, &far, &mic);
        let tail = 48_000 * 5..n; // after convergence
        let erle = db(rms(&mic[tail.clone()])) - db(rms(&out[tail.clone()]));
        eprintln!("single talk: ERLE {erle:.1} dB (measured), reported {:?} dB", chain.erle_db());
        assert!(erle > 20.0, "ERLE {erle:.1} dB");
    }

    #[test]
    fn near_end_speech_survives_double_talk() {
        let n = 48_000 * 10;
        let far = speechlike_seed(n, 0.5, 1);
        let near = speechlike_seed(n, 0.3, 7);
        // the near talker starts after 5 s, when the canceller has converged
        let near: Vec<f32> = near.iter().enumerate().map(|(i, v)| if i >= 48_000 * 5 { *v } else { 0.0 }).collect();
        let echo = room(&far, 1920, 0.5);
        let mic: Vec<f32> = echo.iter().zip(&near).map(|(a, b)| a + b).collect();
        let mut chain = MicChain::new();
        let out = call(&mut chain, SmartMic::Off, &far, &mic);
        // how much of the near talker comes through: projection of the output onto the near signal over the last 4 s
        let r = 48_000 * 6..n;
        let (on, nn) = (r.clone().map(|i| out[i] * near[i]).sum::<f32>(), r.clone().map(|i| near[i] * near[i]).sum::<f32>());
        let kept = db((on / nn).abs().sqrt());
        eprintln!("double talk: near-end level through the canceller {kept:+.1} dB");
        assert!(kept > -10.0, "near end {kept:+.1} dB");
    }

    #[test]
    fn quiet_speech_is_brought_up() {
        let n = 48_000 * 10;
        let quiet = speechlike_seed(n, 0.02, 3); // about −40 dBFS
        let mut chain = MicChain::new();
        let out = call(&mut chain, SmartMic::Off, &vec![0.0; n], &quiet);
        let tail = 48_000 * 6..n;
        let gain = db(rms(&out[tail.clone()])) - db(rms(&quiet[tail.clone()]));
        eprintln!("AGC: quiet speech raised by {gain:.1} dB");
        assert!(gain > 10.0, "gain {gain:.1} dB");
        assert!(out.iter().all(|s| s.abs() <= 1.0));
    }

    #[test]
    fn outside_a_call_only_noise_removal_runs() {
        let x = noise(48_000, 0.2, 5);
        let mut chain = MicChain::new();
        let mut out = vec![];
        let mut q = VecDeque::from(vec![0.5; 4800]);
        chain.process(SmartMic::Off, false, &mut q, &x, &mut out);
        assert_eq!(out, x, "Aus outside a call: untouched");
        assert!(q.is_empty() && chain.aec.is_none());
    }

    /// Delay of one processing stage on clean speech.
    fn stage_delay(mut p: AudioProcessing, x: &[f32]) -> (usize, f32) {
        let mut y = vec![];
        let mut out = [0f32; NN_FRAME];
        for f in x.chunks_exact(NN_FRAME) { p.process_capture_f32(&[f], &mut [&mut out]).unwrap(); y.extend_from_slice(&out); }
        y.resize(x.len() + 48_000, 0.0);
        crate::media::denoise::tests::exact_lag(x, &y)
    }

    #[test]
    fn chain_delay() {
        let x = speechlike_seed(48_000 * 4, 0.3, 2);
        let (a, ac) = stage_delay(echo_canceller(), &x);
        let (g, gc) = stage_delay(gain_control(), &x);
        let mut chain = MicChain::new();
        let mut y = call(&mut chain, SmartMic::Off, &vec![0.0; x.len()], &x);
        y.resize(x.len() + 48_000, 0.0);
        let (l, c) = crate::media::denoise::tests::exact_lag(&x, &y);
        eprintln!("delay: echo canceller {a} samples = {:.2} ms (corr {ac:.3}), gain control {g} = {:.2} ms (corr {gc:.3}), \
                   chain with noise removal off {l} = {:.2} ms (corr {c:.3})", a as f32 / 48.0, g as f32 / 48.0, l as f32 / 48.0);
        assert!(l < NN_FRAME, "chain {l} samples");
    }

    /// Cost per 10 ms frame: `cargo test --release --bin webox media::mic::tests::bench -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench() {
        let n = 48_000 * 10;
        let far = speechlike_seed(n, 0.5, 1);
        let mic = room(&far, 1920, 0.5);
        let (mut aec, mut agc) = (echo_canceller(), gain_control());
        let (mut t_aec, mut t_agc) = (vec![], vec![]);
        let mut out = [0f32; NN_FRAME];
        for (f, m) in far.chunks_exact(NN_FRAME).zip(mic.chunks_exact(NN_FRAME)) {
            let t = Instant::now();
            aec.process_render_f32(&[f], &mut [&mut out]).unwrap();
            aec.process_capture_f32(&[m], &mut [&mut out]).unwrap();
            t_aec.push(t.elapsed().as_secs_f64() * 1000.0);
            let t = Instant::now();
            agc.process_capture_f32(&[m], &mut [&mut out]).unwrap();
            t_agc.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        report("Echo canceller (render + capture)", &mut t_aec);
        report("Gain control", &mut t_agc);
    }
}
