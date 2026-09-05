use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::dsp::Params;
use crate::fx::FxKnob;
use crate::track::Track;

pub struct Shared {
    pub live: AtomicBool,
    pub voice_gain: AtomicU32,
    pub master_gain: AtomicU32,
    pub tune_on: AtomicBool,
    pub tune_amount: AtomicU32,
    pub tune_retune: AtomicU32,
    pub key: AtomicU32,
    pub scale: AtomicU32,
    pub reverb_mix: AtomicU32,
    pub reverb_size: AtomicU32,
    pub echo_mix: AtomicU32,
    pub echo_time: AtomicU32,
    pub alien: AtomicU32,
    pub alien_rate: AtomicU32,
    pub chipmunk: AtomicU32,
    pub chipmunk_height: AtomicU32,
    pub demon: AtomicU32,
    pub demon_depth: AtomicU32,
    pub robot: AtomicU32,
    pub robot_crunch: AtomicU32,
    pub telephone: AtomicU32,
    pub telephone_tone: AtomicU32,
    pub chorus: AtomicU32,
    pub chorus_rate: AtomicU32,
    pub radio: AtomicU32,
    pub radio_static: AtomicU32,
    pub track_gain: AtomicU32,
    pub send_discord: AtomicBool,
    pub discord_device: Mutex<String>,
    pub track_playing: AtomicBool,
    pub track_frame: AtomicUsize,
    pub freq: AtomicU32,
    pub target_midi: AtomicU32,
    pub rms: AtomicU32,
    pub exclusive: AtomicBool,
    pub output_ready: AtomicBool,
    pub sample_rate: AtomicU32,
    pub latency_us: AtomicU32,
    pub status: Mutex<String>,
    pub track: Mutex<Option<Arc<Track>>>,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            live: AtomicBool::new(false),
            voice_gain: AtomicU32::new(1.4f32.to_bits()),
            master_gain: AtomicU32::new(0.95f32.to_bits()),
            tune_on: AtomicBool::new(true),
            tune_amount: AtomicU32::new(0.85f32.to_bits()),
            tune_retune: AtomicU32::new(0.55f32.to_bits()),
            key: AtomicU32::new(0),
            scale: AtomicU32::new(1),
            reverb_mix: AtomicU32::new(0.33f32.to_bits()),
            reverb_size: AtomicU32::new(0.55f32.to_bits()),
            echo_mix: AtomicU32::new(0.22f32.to_bits()),
            echo_time: AtomicU32::new(0.26f32.to_bits()),
            alien: AtomicU32::new(0f32.to_bits()),
            alien_rate: AtomicU32::new(0.45f32.to_bits()),
            chipmunk: AtomicU32::new(0f32.to_bits()),
            chipmunk_height: AtomicU32::new(0.7f32.to_bits()),
            demon: AtomicU32::new(0f32.to_bits()),
            demon_depth: AtomicU32::new(0.7f32.to_bits()),
            robot: AtomicU32::new(0f32.to_bits()),
            robot_crunch: AtomicU32::new(0.55f32.to_bits()),
            telephone: AtomicU32::new(0f32.to_bits()),
            telephone_tone: AtomicU32::new(0.45f32.to_bits()),
            chorus: AtomicU32::new(0f32.to_bits()),
            chorus_rate: AtomicU32::new(0.35f32.to_bits()),
            radio: AtomicU32::new(0f32.to_bits()),
            radio_static: AtomicU32::new(0.3f32.to_bits()),
            track_gain: AtomicU32::new(0.7f32.to_bits()),
            send_discord: AtomicBool::new(false),
            discord_device: Mutex::new(String::new()),
            track_playing: AtomicBool::new(false),
            track_frame: AtomicUsize::new(0),
            freq: AtomicU32::new(0),
            target_midi: AtomicU32::new(0),
            rms: AtomicU32::new(0),
            exclusive: AtomicBool::new(false),
            output_ready: AtomicBool::new(false),
            sample_rate: AtomicU32::new(0),
            latency_us: AtomicU32::new(0),
            status: Mutex::new("Mic is off".to_string()),
            track: Mutex::new(None),
        }
    }

    pub fn params(&self) -> Params {
        Params {
            voice_gain: load_f32(&self.voice_gain),
            master_gain: load_f32(&self.master_gain),
            tune_on: self.tune_on.load(Ordering::Relaxed),
            tune_amount: load_f32(&self.tune_amount),
            tune_retune: load_f32(&self.tune_retune),
            key: self.key.load(Ordering::Relaxed) as i32,
            scale: self.scale.load(Ordering::Relaxed) as usize,
            reverb_mix: load_f32(&self.reverb_mix),
            reverb_size: load_f32(&self.reverb_size),
            echo_mix: load_f32(&self.echo_mix),
            echo_time: load_f32(&self.echo_time),
            fx: FxKnob {
                alien: load_f32(&self.alien),
                alien_rate: load_f32(&self.alien_rate),
                chipmunk: load_f32(&self.chipmunk),
                chipmunk_height: load_f32(&self.chipmunk_height),
                demon: load_f32(&self.demon),
                demon_depth: load_f32(&self.demon_depth),
                robot: load_f32(&self.robot),
                robot_crunch: load_f32(&self.robot_crunch),
                telephone: load_f32(&self.telephone),
                telephone_tone: load_f32(&self.telephone_tone),
                chorus: load_f32(&self.chorus),
                chorus_rate: load_f32(&self.chorus_rate),
                radio: load_f32(&self.radio),
                radio_static: load_f32(&self.radio_static),
            },
        }
    }

    pub fn set_status(&self, text: impl Into<String>) {
        if let Ok(mut status) = self.status.lock() {
            *status = text.into();
        }
    }
}

pub fn store_f32(slot: &AtomicU32, value: f32) {
    slot.store(value.to_bits(), Ordering::Relaxed);
}

pub fn load_f32(slot: &AtomicU32) -> f32 {
    f32::from_bits(slot.load(Ordering::Relaxed))
}
