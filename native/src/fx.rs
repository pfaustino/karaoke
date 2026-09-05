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
    fn bandpass(sample_rate: f32, cutoff: f32, q: f32) -> Self {
        let mut filter = Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        };
        filter.set_bandpass(sample_rate, cutoff, q);
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

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}
