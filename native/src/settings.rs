use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedSettings {
    pub voice_gain: f32,
    pub master_gain: f32,
    pub tune_on: bool,
    pub tune_amount: f32,
    pub tune_retune: f32,
    pub key: i32,
    pub scale: usize,
    pub reverb_on: bool,
    pub reverb_mix: f32,
    pub reverb_size: f32,
    pub echo_on: bool,
    pub echo_mix: f32,
    pub echo_time: f32,
    pub track_gain: f32,
    pub send_discord: bool,
    pub alien_on: bool,
    pub alien: f32,
    pub alien_rate: f32,
    pub chipmunk_on: bool,
    pub chipmunk: f32,
    pub chipmunk_height: f32,
    pub demon_on: bool,
    pub demon: f32,
    pub demon_depth: f32,
    pub robot_on: bool,
    pub robot: f32,
    pub robot_crunch: f32,
    pub telephone_on: bool,
    pub telephone: f32,
    pub telephone_tone: f32,
    pub chorus_on: bool,
    pub chorus: f32,
    pub chorus_rate: f32,
    pub radio_on: bool,
    pub radio: f32,
    pub radio_static: f32,
    pub vader_on: bool,
    pub vader: f32,
    pub vader_dark: f32,
    pub flange_on: bool,
    pub flange: f32,
    pub flange_rate: f32,
    pub phaser_on: bool,
    pub phaser: f32,
    pub phaser_rate: f32,
    pub vibrato_on: bool,
    pub vibrato: f32,
    pub vibrato_rate: f32,
    pub overdrive_on: bool,
    pub overdrive: f32,
    pub overdrive_drive: f32,
    pub underwater_on: bool,
    pub underwater: f32,
    pub underwater_depth: f32,
}

impl Default for SavedSettings {
    fn default() -> Self {
        Self {
            voice_gain: 1.4,
            master_gain: 0.95,
            tune_on: true,
            tune_amount: 0.85,
            tune_retune: 0.55,
            key: 0,
            scale: 1,
            reverb_on: true,
            reverb_mix: 0.33,
            reverb_size: 0.55,
            echo_on: true,
            echo_mix: 0.22,
            echo_time: 0.26,
            track_gain: 0.7,
            send_discord: false,
            alien_on: true,
            alien: 0.0,
            alien_rate: 0.45,
            chipmunk_on: true,
            chipmunk: 0.0,
            chipmunk_height: 0.7,
            demon_on: true,
            demon: 0.0,
            demon_depth: 0.7,
            robot_on: true,
            robot: 0.0,
            robot_crunch: 0.55,
            telephone_on: true,
            telephone: 0.0,
            telephone_tone: 0.45,
            chorus_on: true,
            chorus: 0.0,
            chorus_rate: 0.35,
            radio_on: true,
            radio: 0.0,
            radio_static: 0.3,
            vader_on: true,
            vader: 0.0,
            vader_dark: 0.7,
            flange_on: true,
            flange: 0.0,
            flange_rate: 0.35,
            phaser_on: true,
            phaser: 0.0,
            phaser_rate: 0.4,
            vibrato_on: true,
            vibrato: 0.0,
            vibrato_rate: 0.45,
            overdrive_on: true,
            overdrive: 0.0,
            overdrive_drive: 0.55,
            underwater_on: true,
            underwater: 0.0,
            underwater_depth: 0.6,
        }
    }
}

impl SavedSettings {
    pub fn load() -> Option<Self> {
        let path = settings_path()?;
        let text = fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self) {
        let Some(path) = settings_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, text);
        }
    }
}

fn settings_path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("KaraokeBooth").join("settings.json"))
}

pub struct Autosave {
    last: SavedSettings,
    last_write: Instant,
}

impl Autosave {
    pub fn new(current: SavedSettings) -> Self {
        Self {
            last: current,
            last_write: Instant::now(),
        }
    }

    pub fn tick(&mut self, current: SavedSettings) {
        if current == self.last {
            return;
        }
        if self.last_write.elapsed() < Duration::from_millis(500) {
            return;
        }
        current.save();
        self.last = current;
        self.last_write = Instant::now();
    }

    pub fn flush(&mut self, current: SavedSettings) {
        if current == self.last {
            return;
        }
        current.save();
        self.last = current;
    }
}

#[cfg(test)]
mod tests {
    use super::SavedSettings;

    #[test]
    fn missing_fields_keep_defaults() {
        let parsed: SavedSettings =
            serde_json::from_str(r#"{"vader":0.8,"vader_on":false}"#).unwrap();
        assert!((parsed.vader - 0.8).abs() < f32::EPSILON);
        assert!(!parsed.vader_on);
        assert!(parsed.flange_on);
        assert!((parsed.voice_gain - 1.4).abs() < f32::EPSILON);
    }
}
