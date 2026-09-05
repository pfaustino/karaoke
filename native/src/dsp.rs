use crate::fx::{ColorFx, FxKnob};
use crate::pitch::{freq_to_midi, midi_to_freq, snap_midi};

const MIN_HZ: f32 = 70.0;
const MAX_HZ: f32 = 1200.0;
const YIN_THRESHOLD: f32 = 0.15;
const PITCH_WINDOW: usize = 1024;
const DOWNSAMPLE: usize = 4;
const DETECT_HOP: usize = 256;
const GRAIN: usize = 512;
const DELAY: usize = 2048;
const GATE_RMS: f32 = 0.012;
const MIN_RATIO: f32 = 0.5;
const MAX_RATIO: f32 = 2.0;

pub struct VoiceChain {
    hpf: Biquad,
    compressor: Compressor,
    autotune: Autotune,
    color: ColorFx,
    reverb: Reverb,
    echo: Echo,
    sample_rate: f32,
    last_room: f32,
}

impl VoiceChain {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            hpf: Biquad::highpass(sample_rate, 80.0),
            compressor: Compressor::new(sample_rate),
            autotune: Autotune::new(sample_rate),
            color: ColorFx::new(sample_rate),
            reverb: Reverb::new(sample_rate, 0.55),
            echo: Echo::new(sample_rate),
            sample_rate,
            last_room: 0.55,
        }
    }

    pub fn process(&mut self, input: f32, p: &Params) -> (f32, PitchReadout) {
        let mut x = self.hpf.process(input) * p.voice_gain;
        x = self.compressor.process(x);
        let (tuned, pitch) = self.autotune.process(x, p);
        let tuned = self.color.process(tuned, &p.fx);
        if (p.reverb_size - self.last_room).abs() > 0.02 {
            self.reverb = Reverb::new(self.sample_rate, p.reverb_size);
            self.last_room = p.reverb_size;
        }
        let wet_rev = self.reverb.process(tuned);
        let wet_echo = self.echo.process(tuned, p.echo_time);
        let mixed = tuned + wet_rev * p.reverb_mix * 0.85 + wet_echo * p.echo_mix * 0.55;
        (mixed * p.master_gain, pitch)
    }
}

#[derive(Clone, Copy)]
pub struct Params {
    pub voice_gain: f32,
    pub master_gain: f32,
    pub tune_on: bool,
    pub tune_amount: f32,
    pub tune_retune: f32,
    pub key: i32,
    pub scale: usize,
    pub reverb_mix: f32,
    pub reverb_size: f32,
    pub echo_mix: f32,
    pub echo_time: f32,
    pub fx: FxKnob,
}

#[derive(Clone, Copy, Default)]
pub struct PitchReadout {
    pub freq: f32,
    pub target_midi: f32,
}

struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn highpass(sample_rate: f32, cutoff: f32) -> Self {
        let w0 = 2.0 * std::f32::consts::PI * cutoff / sample_rate;
        let cos = w0.cos();
        let sin = w0.sin();
        let alpha = sin / (2.0 * 0.707);
        let b0 = (1.0 + cos) * 0.5;
        let b1 = -(1.0 + cos);
        let b2 = (1.0 + cos) * 0.5;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos;
        let a2 = 1.0 - alpha;
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

struct Compressor {
    env: f32,
    threshold: f32,
    att: f32,
    rel: f32,
}

impl Compressor {
    fn new(sample_rate: f32) -> Self {
        Self {
            env: 0.0,
            threshold: 10f32.powf(-22.0 / 20.0),
            att: (-1.0 / (0.004 * sample_rate)).exp(),
            rel: (-1.0 / (0.16 * sample_rate)).exp(),
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let abs = x.abs();
        let coeff = if abs > self.env { self.att } else { self.rel };
        self.env = coeff * self.env + (1.0 - coeff) * abs;
        if self.env <= self.threshold {
            return x;
        }
        let over = self.env / self.threshold;
        let gain = (over.powf(-0.75)).min(1.0);
        x * gain
    }
}

struct Autotune {
    ring: Vec<f32>,
    ring_pos: usize,
    ring_filled: bool,
    samples_since_detect: usize,
    down: Vec<f32>,
    diff: Vec<f32>,
    delay: Vec<f32>,
    write_pos: usize,
    grains: [(f32, usize); 2],
    hann: Vec<f32>,
    ratio: f32,
    freq: f32,
    target_midi: f32,
    detect_rate: f32,
}

impl Autotune {
    fn new(sample_rate: f32) -> Self {
        let mut hann = vec![0.0; GRAIN];
        for (i, slot) in hann.iter_mut().enumerate() {
            *slot = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / GRAIN as f32).cos());
        }
        Self {
            ring: vec![0.0; PITCH_WINDOW * DOWNSAMPLE],
            ring_pos: 0,
            ring_filled: false,
            samples_since_detect: 0,
            down: vec![0.0; PITCH_WINDOW],
            diff: vec![0.0; PITCH_WINDOW / 2],
            delay: vec![0.0; DELAY],
            write_pos: 0,
            grains: [(0.0, 0), (GRAIN as f32 / 2.0, GRAIN / 2)],
            hann,
            ratio: 1.0,
            freq: 0.0,
            target_midi: 0.0,
            detect_rate: sample_rate / DOWNSAMPLE as f32,
        }
    }

    fn process(&mut self, x: f32, p: &Params) -> (f32, PitchReadout) {
        let len = self.ring.len();
        self.ring[self.ring_pos] = x;
        self.ring_pos += 1;
        if self.ring_pos >= len {
            self.ring_pos = 0;
            self.ring_filled = true;
        }
        self.samples_since_detect += 1;
        if self.samples_since_detect >= DETECT_HOP {
            self.samples_since_detect = 0;
            self.detect();
        }

        if !p.tune_on {
            return (
                x,
                PitchReadout {
                    freq: self.freq,
                    target_midi: self.target_midi,
                },
            );
        }

        let mut target_ratio = 1.0;
        if self.freq >= MIN_HZ && self.freq <= MAX_HZ {
            let midi = freq_to_midi(self.freq);
            self.target_midi = snap_midi(midi, p.key, p.scale) as f32;
            target_ratio = (midi_to_freq(self.target_midi) / self.freq).clamp(MIN_RATIO, MAX_RATIO);
        }
        let slew = p.tune_retune.clamp(0.02, 1.0);
        self.ratio += (target_ratio - self.ratio) * slew;
        let wet = self.shift(x, self.ratio);
        let y = x * (1.0 - p.tune_amount) + wet * p.tune_amount;
        (
            y,
            PitchReadout {
                freq: self.freq,
                target_midi: self.target_midi,
            },
        )
    }

    fn detect(&mut self) {
        if !self.ring_filled {
            return;
        }
        let len = self.ring.len();
        let mut energy = 0.0;
        for i in 0..PITCH_WINDOW {
            let sample = self.ring[(self.ring_pos + i * DOWNSAMPLE) % len];
            self.down[i] = sample;
            energy += sample * sample;
        }
        let rms = (energy / PITCH_WINDOW as f32).sqrt();
        if rms < GATE_RMS {
            self.freq = 0.0;
            return;
        }
        self.freq = yin_pitch(&self.down, self.detect_rate, &mut self.diff);
    }

    fn shift(&mut self, input: f32, ratio: f32) -> f32 {
        self.delay[self.write_pos] = input;
        let mut mix = 0.0;
        let mut weight = 0.0;
        for grain in &mut self.grains {
            let amp = self.hann[grain.1];
            mix += read_cubic(&self.delay, grain.0) * amp;
            weight += amp;
            grain.0 += ratio;
            grain.1 += 1;
            if grain.1 >= GRAIN {
                grain.0 = (self.write_pos as f32 - GRAIN as f32 + DELAY as f32) % DELAY as f32;
                grain.1 = 0;
            }
        }
        self.write_pos = (self.write_pos + 1) % DELAY;
        if weight > 1e-6 {
            mix / weight
        } else {
            0.0
        }
    }
}

fn read_cubic(buffer: &[f32], pos: f32) -> f32 {
    let length = buffer.len() as f32;
    let wrapped = ((pos % length) + length) % length;
    let i = wrapped.floor() as i32;
    let f = wrapped - i as f32;
    let len = buffer.len() as i32;
    let at = |idx: i32| buffer[(((idx % len) + len) % len) as usize];
    let ym1 = at(i - 1);
    let y0 = at(i);
    let y1 = at(i + 1);
    let y2 = at(i + 2);
    let c1 = 0.5 * (y1 - ym1);
    let c2 = ym1 - 2.5 * y0 + 2.0 * y1 - 0.5 * y2;
    let c3 = 0.5 * (y2 - ym1) + 1.5 * (y0 - y1);
    ((c3 * f + c2) * f + c1) * f + y0
}

fn yin_pitch(samples: &[f32], detect_rate: f32, diff: &mut [f32]) -> f32 {
    let n = samples.len();
    let tau_min = (detect_rate / MAX_HZ).floor().max(2.0) as usize;
    let tau_max = ((detect_rate / MIN_HZ).floor() as usize)
        .min(n.saturating_sub(2))
        .min(diff.len().saturating_sub(1));
    if tau_max <= tau_min {
        return 0.0;
    }

    for tau in 1..=tau_max {
        let mut sum = 0.0;
        let limit = n - tau;
        for i in 0..limit {
            let delta = samples[i] - samples[i + tau];
            sum += delta * delta;
        }
        diff[tau] = sum;
    }

    diff[0] = 1.0;
    let mut running = 0.0;
    for tau in 1..=tau_max {
        running += diff[tau];
        diff[tau] = (diff[tau] * tau as f32) / running;
    }

    let mut tau_estimate = 0usize;
    for tau in tau_min..=tau_max {
        if diff[tau] >= YIN_THRESHOLD {
            continue;
        }
        let mut local = tau;
        let walk_limit = tau_max.min(tau + 16);
        for step in (tau + 1)..=walk_limit {
            if diff[step] >= diff[local] {
                break;
            }
            local = step;
        }
        tau_estimate = local;
        break;
    }

    if tau_estimate == 0 || diff[tau_estimate] >= YIN_THRESHOLD {
        return 0.0;
    }

    let prev = if tau_estimate > 1 {
        diff[tau_estimate - 1]
    } else {
        diff[tau_estimate]
    };
    let next = if tau_estimate < tau_max {
        diff[tau_estimate + 1]
    } else {
        diff[tau_estimate]
    };
    let denom = 2.0 * (2.0 * diff[tau_estimate] - next - prev);
    let shift = if denom.abs() > 1e-9 {
        (next - prev) / denom
    } else {
        0.0
    };
    let better = tau_estimate as f32 + shift;
    if better <= 0.0 {
        0.0
    } else {
        detect_rate / better
    }
}

struct Echo {
    buf: Vec<f32>,
    pos: usize,
    sample_rate: f32,
}

impl Echo {
    fn new(sample_rate: f32) -> Self {
        Self {
            buf: vec![0.0; (sample_rate * 1.2) as usize],
            pos: 0,
            sample_rate,
        }
    }

    fn process(&mut self, x: f32, time_sec: f32) -> f32 {
        let delay = (time_sec.clamp(0.08, 0.9) * self.sample_rate) as usize;
        let delay = delay.min(self.buf.len() - 1);
        let read = (self.pos + self.buf.len() - delay) % self.buf.len();
        let y = self.buf[read];
        self.buf[self.pos] = x + y * 0.28;
        self.pos = (self.pos + 1) % self.buf.len();
        y
    }
}

struct Comb {
    buf: Vec<f32>,
    pos: usize,
    feedback: f32,
}

impl Comb {
    fn new(len: usize, feedback: f32) -> Self {
        Self {
            buf: vec![0.0; len.max(2)],
            pos: 0,
            feedback,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.buf[self.pos];
        self.buf[self.pos] = x + y * self.feedback;
        self.pos = (self.pos + 1) % self.buf.len();
        y
    }
}

struct Allpass {
    buf: Vec<f32>,
    pos: usize,
    feedback: f32,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(2)],
            pos: 0,
            feedback: 0.5,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let buf = self.buf[self.pos];
        let y = -x + buf;
        self.buf[self.pos] = x + buf * self.feedback;
        self.pos = (self.pos + 1) % self.buf.len();
        y
    }
}

struct Reverb {
    combs: [Comb; 4],
    allpass: [Allpass; 2],
}

impl Reverb {
    fn new(sample_rate: f32, room: f32) -> Self {
        let scale = (sample_rate / 44_100.0) * (0.7 + room * 0.8);
        let fb = 0.72 + room * 0.18;
        Self {
            combs: [
                Comb::new((1116.0 * scale) as usize, fb),
                Comb::new((1188.0 * scale) as usize, fb),
                Comb::new((1277.0 * scale) as usize, fb),
                Comb::new((1356.0 * scale) as usize, fb),
            ],
            allpass: [
                Allpass::new((225.0 * scale) as usize),
                Allpass::new((556.0 * scale) as usize),
            ],
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let mut acc = 0.0;
        for comb in &mut self.combs {
            acc += comb.process(x);
        }
        acc *= 0.25;
        for ap in &mut self.allpass {
            acc = ap.process(acc);
        }
        acc
    }
}
