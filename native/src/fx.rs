const GRAIN: usize = 512;
const DELAY: usize = 2048;

#[derive(Clone, Copy)]
pub struct FxKnob {
    pub alien: f32,
    pub alien_rate: f32,
    pub chipmunk: f32,
    pub chipmunk_height: f32,
    pub demon: f32,
    pub demon_depth: f32,
    pub robot: f32,
    pub robot_crunch: f32,
    pub telephone: f32,
    pub telephone_tone: f32,
    pub chorus: f32,
    pub chorus_rate: f32,
    pub radio: f32,
    pub radio_static: f32,
    pub vader: f32,
    pub vader_dark: f32,
    pub flange: f32,
    pub flange_rate: f32,
    pub phaser: f32,
    pub phaser_rate: f32,
    pub vibrato: f32,
    pub vibrato_rate: f32,
    pub overdrive: f32,
    pub overdrive_drive: f32,
    pub underwater: f32,
    pub underwater_depth: f32,
}

impl Default for FxKnob {
    fn default() -> Self {
        Self {
            alien: 0.0,
            alien_rate: 0.45,
            chipmunk: 0.0,
            chipmunk_height: 0.7,
            demon: 0.0,
            demon_depth: 0.7,
            robot: 0.0,
            robot_crunch: 0.55,
            telephone: 0.0,
            telephone_tone: 0.45,
            chorus: 0.0,
            chorus_rate: 0.35,
            radio: 0.0,
            radio_static: 0.3,
            vader: 0.0,
            vader_dark: 0.7,
            flange: 0.0,
            flange_rate: 0.35,
            phaser: 0.0,
            phaser_rate: 0.4,
            vibrato: 0.0,
            vibrato_rate: 0.45,
            overdrive: 0.0,
            overdrive_drive: 0.55,
            underwater: 0.0,
            underwater_depth: 0.6,
        }
    }
}

pub struct ColorFx {
    chipmunk: GrainShift,
    demon: GrainShift,
    alien_phase: f32,
    alien_vib: f32,
    robot_hold: f32,
    robot_age: u32,
    telephone: Biquad,
    radio_bp: Biquad,
    radio_noise: u32,
    chorus: Vec<f32>,
    chorus_pos: usize,
    chorus_lfo: f32,
    vader: GrainShift,
    vader_lp: Biquad,
    flange: Vec<f32>,
    flange_pos: usize,
    flange_lfo: f32,
    flange_fb: f32,
    phaser: [Biquad; 4],
    phaser_lfo: f32,
    phaser_fb: f32,
    vibrato: Vec<f32>,
    vibrato_pos: usize,
    vibrato_lfo: f32,
    underwater_lp: Biquad,
    underwater_lfo: f32,
    sample_rate: f32,
}

impl ColorFx {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            chipmunk: GrainShift::new(),
            demon: GrainShift::new(),
            alien_phase: 0.0,
            alien_vib: 0.0,
            robot_hold: 0.0,
            robot_age: 0,
            telephone: Biquad::bandpass(sample_rate, 1400.0, 0.7),
            radio_bp: Biquad::bandpass(sample_rate, 1100.0, 0.9),
            radio_noise: 1,
            chorus: vec![0.0; (sample_rate * 0.04) as usize + 8],
            chorus_pos: 0,
            chorus_lfo: 0.0,
            vader: GrainShift::new(),
            vader_lp: Biquad::lowpass(sample_rate, 700.0, 0.7),
            flange: vec![0.0; (sample_rate * 0.02) as usize + 8],
            flange_pos: 0,
            flange_lfo: 0.0,
            flange_fb: 0.0,
            phaser: [
                Biquad::allpass(sample_rate, 200.0, 0.6),
                Biquad::allpass(sample_rate, 400.0, 0.6),
                Biquad::allpass(sample_rate, 800.0, 0.6),
                Biquad::allpass(sample_rate, 1600.0, 0.6),
            ],
            phaser_lfo: 0.0,
            phaser_fb: 0.0,
            vibrato: vec![0.0; (sample_rate * 0.018) as usize + 8],
            vibrato_pos: 0,
            vibrato_lfo: 0.0,
            underwater_lp: Biquad::lowpass(sample_rate, 500.0, 0.7),
            underwater_lfo: 0.0,
            sample_rate,
        }
    }

    pub fn process(&mut self, input: f32, p: &FxKnob) -> f32 {
        let mut x = input;
        if p.chipmunk > 0.001 {
            let semitones = 4.0 + p.chipmunk_height * 8.0;
            let wet = self.chipmunk.process(x, 2f32.powf(semitones / 12.0));
            x = x * (1.0 - p.chipmunk) + wet * p.chipmunk;
        }
        if p.demon > 0.001 {
            let semitones = 4.0 + p.demon_depth * 8.0;
            let wet = self.demon.process(x, 2f32.powf(-semitones / 12.0));
            x = x * (1.0 - p.demon) + wet * p.demon;
        }
        if p.alien > 0.001 {
            let hz = 28.0 + p.alien_rate * 240.0;
            self.alien_phase += hz / self.sample_rate;
            if self.alien_phase > 1.0 {
                self.alien_phase -= 1.0;
            }
            self.alien_vib += (1.8 + p.alien_rate * 4.0) / self.sample_rate;
            if self.alien_vib > 1.0 {
                self.alien_vib -= 1.0;
            }
            let ring = (self.alien_phase * std::f32::consts::TAU).sin();
            let vib = 1.0 + 0.12 * (self.alien_vib * std::f32::consts::TAU).sin();
            let wet = (x * ring * 1.6 * vib).clamp(-1.0, 1.0);
            x = x * (1.0 - p.alien) + wet * p.alien;
        }
        if p.robot > 0.001 {
            let hold = 2 + (p.robot_crunch * 28.0) as u32;
            if self.robot_age == 0 {
                self.robot_hold = x;
            }
            self.robot_age += 1;
            if self.robot_age >= hold {
                self.robot_age = 0;
            }
            let levels = 3.0 + (1.0 - p.robot_crunch) * 28.0;
            let crushed = (self.robot_hold * levels).round() / levels;
            x = x * (1.0 - p.robot) + crushed * p.robot;
        }
        if p.telephone > 0.001 {
            let tone = 900.0 + p.telephone_tone * 1600.0;
            self.telephone.set_bandpass(self.sample_rate, tone, 0.85);
            let wet = self.telephone.process(x) * 1.6;
            x = x * (1.0 - p.telephone) + wet.clamp(-1.0, 1.0) * p.telephone;
        }
        if p.radio > 0.001 {
            self.radio_bp.set_bandpass(self.sample_rate, 1050.0, 1.1);
            let filtered = self.radio_bp.process(x);
            self.radio_noise = self.radio_noise.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = (self.radio_noise as f32 / u32::MAX as f32) * 2.0 - 1.0;
            let crunch = (filtered * (1.8 + p.radio * 3.0)).tanh();
            let wet = (crunch + noise * p.radio_static * 0.22).clamp(-1.0, 1.0);
            x = x * (1.0 - p.radio) + wet * p.radio;
        }
        if p.chorus > 0.001 {
            let hz = 0.15 + p.chorus_rate * 3.2;
            self.chorus_lfo += hz / self.sample_rate;
            if self.chorus_lfo > 1.0 {
                self.chorus_lfo -= 1.0;
            }
            let sweep = (self.chorus_lfo * std::f32::consts::TAU).sin();
            let delay = (0.008 + (0.006 + sweep.abs() * 0.012)) * self.sample_rate;
            let wet = read_delay(&self.chorus, self.chorus_pos, delay);
            self.chorus[self.chorus_pos] = x;
            self.chorus_pos = (self.chorus_pos + 1) % self.chorus.len();
            x = x * (1.0 - p.chorus * 0.55) + wet * p.chorus;
        }
        if p.vader > 0.001 {
            let semitones = 5.0 + p.vader_dark * 5.0;
            let shifted = self.vader.process(x, 2f32.powf(-semitones / 12.0));
            let cutoff = 1100.0 - p.vader_dark * 750.0;
            self.vader_lp.set_lowpass(self.sample_rate, cutoff, 0.75);
            let dark = self.vader_lp.process(shifted);
            let wet = (dark * 1.45).tanh();
            x = x * (1.0 - p.vader) + wet * p.vader;
        }
        if p.flange > 0.001 {
            let hz = 0.08 + p.flange_rate * 1.6;
            self.flange_lfo += hz / self.sample_rate;
            if self.flange_lfo > 1.0 {
                self.flange_lfo -= 1.0;
            }
            let sweep = 0.5 + 0.5 * (self.flange_lfo * std::f32::consts::TAU).sin();
            let delay = (0.0004 + sweep * 0.0075) * self.sample_rate;
            let delayed = read_delay(&self.flange, self.flange_pos, delay);
            let wet = (x + delayed).clamp(-1.0, 1.0);
            self.flange_fb = (delayed * 0.45).clamp(-0.95, 0.95);
            self.flange[self.flange_pos] = (x + self.flange_fb).clamp(-1.0, 1.0);
            self.flange_pos = (self.flange_pos + 1) % self.flange.len();
            x = x * (1.0 - p.flange * 0.65) + wet * p.flange;
        }
        if p.phaser > 0.001 {
            let hz = 0.12 + p.phaser_rate * 2.4;
            self.phaser_lfo += hz / self.sample_rate;
            if self.phaser_lfo > 1.0 {
                self.phaser_lfo -= 1.0;
            }
            let sweep = 0.5 + 0.5 * (self.phaser_lfo * std::f32::consts::TAU).sin();
            let bases = [180.0, 420.0, 840.0, 1680.0];
            let mut staged = (x + self.phaser_fb * 0.35).clamp(-1.0, 1.0);
            for (filter, base) in self.phaser.iter_mut().zip(bases) {
                filter.set_allpass(self.sample_rate, base * (0.55 + sweep * 1.6), 0.55);
                staged = filter.process(staged);
            }
            self.phaser_fb = staged;
            x = x * (1.0 - p.phaser * 0.6) + staged * p.phaser;
        }
        if p.vibrato > 0.001 {
            let hz = 2.0 + p.vibrato_rate * 7.0;
            self.vibrato_lfo += hz / self.sample_rate;
            if self.vibrato_lfo > 1.0 {
                self.vibrato_lfo -= 1.0;
            }
            let sweep = 0.5 + 0.5 * (self.vibrato_lfo * std::f32::consts::TAU).sin();
            let delay = (0.001 + sweep * (0.003 + p.vibrato * 0.007)) * self.sample_rate;
            let wet = read_delay(&self.vibrato, self.vibrato_pos, delay);
            self.vibrato[self.vibrato_pos] = x;
            self.vibrato_pos = (self.vibrato_pos + 1) % self.vibrato.len();
            x = x * (1.0 - p.vibrato) + wet * p.vibrato;
        }
        if p.overdrive > 0.001 {
            let drive = 1.6 + p.overdrive_drive * 8.0;
            let wet = (x * drive).tanh();
            x = x * (1.0 - p.overdrive) + wet * p.overdrive;
        }
        if p.underwater > 0.001 {
            let cutoff = 900.0 - p.underwater_depth * 620.0;
            self.underwater_lp.set_lowpass(self.sample_rate, cutoff, 0.8);
            let filtered = self.underwater_lp.process(x);
            self.underwater_lfo += (0.35 + p.underwater_depth * 1.4) / self.sample_rate;
            if self.underwater_lfo > 1.0 {
                self.underwater_lfo -= 1.0;
            }
            let wobble = 0.72 + 0.28 * (self.underwater_lfo * std::f32::consts::TAU).sin();
            let wet = (filtered * wobble * 1.15).clamp(-1.0, 1.0);
            x = x * (1.0 - p.underwater) + wet * p.underwater;
        }
        x
    }
}

struct GrainShift {
    delay: Vec<f32>,
    write_pos: usize,
    grains: [(f32, usize); 2],
    hann: Vec<f32>,
}

impl GrainShift {
    fn new() -> Self {
        let mut hann = vec![0.0; GRAIN];
        for (i, slot) in hann.iter_mut().enumerate() {
            *slot = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / GRAIN as f32).cos());
        }
        Self {
            delay: vec![0.0; DELAY],
            write_pos: 0,
            grains: [(0.0, 0), (GRAIN as f32 / 2.0, GRAIN / 2)],
            hann,
        }
    }

    fn process(&mut self, input: f32, ratio: f32) -> f32 {
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

fn read_delay(buffer: &[f32], write: usize, delay: f32) -> f32 {
    let len = buffer.len() as f32;
    let pos = (write as f32 - delay + len * 4.0) % len;
    read_cubic(buffer, pos)
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
    fn empty() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn bandpass(sample_rate: f32, cutoff: f32, q: f32) -> Self {
        let mut filter = Self::empty();
        filter.set_bandpass(sample_rate, cutoff, q);
        filter
    }

    fn lowpass(sample_rate: f32, cutoff: f32, q: f32) -> Self {
        let mut filter = Self::empty();
        filter.set_lowpass(sample_rate, cutoff, q);
        filter
    }

    fn allpass(sample_rate: f32, cutoff: f32, q: f32) -> Self {
        let mut filter = Self::empty();
        filter.set_allpass(sample_rate, cutoff, q);
        filter
    }

    fn set_bandpass(&mut self, sample_rate: f32, cutoff: f32, q: f32) {
        let w0 = 2.0 * std::f32::consts::PI * cutoff / sample_rate;
        let cos = w0.cos();
        let sin = w0.sin();
        let alpha = sin / (2.0 * q.max(0.2));
        let a0 = 1.0 + alpha;
        self.b0 = alpha / a0;
        self.b1 = 0.0;
        self.b2 = -alpha / a0;
        self.a1 = -2.0 * cos / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    fn set_lowpass(&mut self, sample_rate: f32, cutoff: f32, q: f32) {
        let w0 = 2.0 * std::f32::consts::PI * cutoff.max(40.0) / sample_rate;
        let cos = w0.cos();
        let sin = w0.sin();
        let alpha = sin / (2.0 * q.max(0.2));
        let a0 = 1.0 + alpha;
        self.b0 = ((1.0 - cos) * 0.5) / a0;
        self.b1 = (1.0 - cos) / a0;
        self.b2 = ((1.0 - cos) * 0.5) / a0;
        self.a1 = -2.0 * cos / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    fn set_allpass(&mut self, sample_rate: f32, cutoff: f32, q: f32) {
        let w0 = 2.0 * std::f32::consts::PI * cutoff.max(40.0) / sample_rate;
        let cos = w0.cos();
        let sin = w0.sin();
        let alpha = sin / (2.0 * q.max(0.2));
        let a0 = 1.0 + alpha;
        self.b0 = (1.0 - alpha) / a0;
        self.b1 = -2.0 * cos / a0;
        self.b2 = (1.0 + alpha) / a0;
        self.a1 = -2.0 * cos / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}
