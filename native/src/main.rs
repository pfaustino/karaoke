#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod dsp;
mod engine;
mod fx;
mod pitch;
mod settings;
mod state;
mod track;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use eframe::egui::{self, Color32, Pos2, RichText, Sense, Stroke, Vec2};
use engine::{list_playback_devices, stereo_mix_device_name, Engine, PlaybackDevice};
use pitch::{cents_off, NOTE_NAMES};
use settings::{Autosave, SavedSettings};
use state::{load_f32, store_f32, Shared};
use track::Track;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 900.0])
            .with_min_inner_size([720.0, 640.0])
            .with_title("Karaoke Booth"),
        ..Default::default()
    };
    eframe::run_native(
        "Karaoke Booth",
        options,
        Box::new(|cc| Ok(Box::new(BoothApp::new(cc)))),
    )
}

struct BoothApp {
    shared: Arc<Shared>,
    engine: Option<Engine>,
    voice_gain: f32,
    master_gain: f32,
    tune_on: bool,
    tune_amount: f32,
    tune_retune: f32,
    key: i32,
    scale: usize,
    reverb_on: bool,
    reverb_mix: f32,
    reverb_size: f32,
    echo_on: bool,
    echo_mix: f32,
    echo_time: f32,
    track_gain: f32,
    alien_on: bool,
    alien: f32,
    alien_rate: f32,
    chipmunk_on: bool,
    chipmunk: f32,
    chipmunk_height: f32,
    demon_on: bool,
    demon: f32,
    demon_depth: f32,
    robot_on: bool,
    robot: f32,
    robot_crunch: f32,
    telephone_on: bool,
    telephone: f32,
    telephone_tone: f32,
    chorus_on: bool,
    chorus: f32,
    chorus_rate: f32,
    radio_on: bool,
    radio: f32,
    radio_static: f32,
    vader_on: bool,
    vader: f32,
    vader_dark: f32,
    flange_on: bool,
    flange: f32,
    flange_rate: f32,
    phaser_on: bool,
    phaser: f32,
    phaser_rate: f32,
    vibrato_on: bool,
    vibrato: f32,
    vibrato_rate: f32,
    overdrive_on: bool,
    overdrive: f32,
    overdrive_drive: f32,
    underwater_on: bool,
    underwater: f32,
    underwater_depth: f32,
    send_discord: bool,
    discord_device_id: String,
    playback_devices: Vec<PlaybackDevice>,
    stereo_mix_name: Option<String>,
    autosave: Autosave,
}

impl BoothApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_theme(&cc.egui_ctx);
        let mut app = Self {
            shared: Arc::new(Shared::new()),
            engine: None,
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
            send_discord: false,
            discord_device_id: String::new(),
            playback_devices: Vec::new(),
            stereo_mix_name: None,
            autosave: Autosave::new(SavedSettings::default()),
        };
        if let Some(saved) = SavedSettings::load() {
            app.apply_settings(&saved);
        }
        app.autosave = Autosave::new(app.snapshot());
        app.refresh_devices();
        app
    }

    fn refresh_devices(&mut self) {
        self.playback_devices = list_playback_devices();
        self.stereo_mix_name = stereo_mix_device_name();
        if self.discord_device_id.is_empty() {
            if let Some(device) = self
                .playback_devices
                .iter()
                .find(|device| device.looks_virtual)
            {
                self.discord_device_id = device.id.clone();
            }
        }
    }

    fn push_params(&self) {
        store_f32(&self.shared.voice_gain, self.voice_gain);
        store_f32(&self.shared.master_gain, self.master_gain);
        self.shared.tune_on.store(self.tune_on, Ordering::Relaxed);
        store_f32(&self.shared.tune_amount, self.tune_amount);
        store_f32(&self.shared.tune_retune, self.tune_retune);
        self.shared.key.store(self.key as u32, Ordering::Relaxed);
        self.shared.scale.store(self.scale as u32, Ordering::Relaxed);
        store_f32(&self.shared.reverb_mix, gated(self.reverb_on, self.reverb_mix));
        store_f32(&self.shared.reverb_size, self.reverb_size);
        store_f32(&self.shared.echo_mix, gated(self.echo_on, self.echo_mix));
        store_f32(&self.shared.echo_time, self.echo_time);
        store_f32(&self.shared.track_gain, self.track_gain);
        store_f32(&self.shared.alien, gated(self.alien_on, self.alien));
        store_f32(&self.shared.alien_rate, self.alien_rate);
        store_f32(&self.shared.chipmunk, gated(self.chipmunk_on, self.chipmunk));
        store_f32(&self.shared.chipmunk_height, self.chipmunk_height);
        store_f32(&self.shared.demon, gated(self.demon_on, self.demon));
        store_f32(&self.shared.demon_depth, self.demon_depth);
        store_f32(&self.shared.robot, gated(self.robot_on, self.robot));
        store_f32(&self.shared.robot_crunch, self.robot_crunch);
        store_f32(
            &self.shared.telephone,
            gated(self.telephone_on, self.telephone),
        );
        store_f32(&self.shared.telephone_tone, self.telephone_tone);
        store_f32(&self.shared.chorus, gated(self.chorus_on, self.chorus));
        store_f32(&self.shared.chorus_rate, self.chorus_rate);
        store_f32(&self.shared.radio, gated(self.radio_on, self.radio));
        store_f32(&self.shared.radio_static, self.radio_static);
        store_f32(&self.shared.vader, gated(self.vader_on, self.vader));
        store_f32(&self.shared.vader_dark, self.vader_dark);
        store_f32(&self.shared.flange, gated(self.flange_on, self.flange));
        store_f32(&self.shared.flange_rate, self.flange_rate);
        store_f32(&self.shared.phaser, gated(self.phaser_on, self.phaser));
        store_f32(&self.shared.phaser_rate, self.phaser_rate);
        store_f32(&self.shared.vibrato, gated(self.vibrato_on, self.vibrato));
        store_f32(&self.shared.vibrato_rate, self.vibrato_rate);
        store_f32(
            &self.shared.overdrive,
            gated(self.overdrive_on, self.overdrive),
        );
        store_f32(&self.shared.overdrive_drive, self.overdrive_drive);
        store_f32(
            &self.shared.underwater,
            gated(self.underwater_on, self.underwater),
        );
        store_f32(&self.shared.underwater_depth, self.underwater_depth);
        self.shared
            .send_discord
            .store(self.send_discord, Ordering::Relaxed);
        if let Ok(mut slot) = self.shared.discord_device.lock() {
            let virtual_id = self
                .playback_devices
                .iter()
                .find(|device| device.id == self.discord_device_id && device.looks_virtual)
                .map(|device| device.id.clone())
                .unwrap_or_default();
            *slot = virtual_id;
        }
    }

    fn toggle_mic(&mut self) {
        if self.engine.is_some() {
            if let Some(engine) = self.engine.take() {
                engine.stop();
            }
            self.shared.live.store(false, Ordering::Relaxed);
            self.shared.set_status("Mic is off");
            return;
        }
        match Engine::start(Arc::clone(&self.shared)) {
            Ok(engine) => {
                self.shared.live.store(true, Ordering::Relaxed);
                self.engine = Some(engine);
            }
            Err(err) => {
                self.shared.live.store(false, Ordering::Relaxed);
                self.shared.set_status(err);
            }
        }
    }

    fn pick_track(&self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Audio", &["mp3", "wav", "flac", "ogg", "m4a"])
            .pick_file()
        else {
            return;
        };
        match Track::load(&path) {
            Ok(track) => {
                let name = track.name.clone();
                if let Ok(mut slot) = self.shared.track.lock() {
                    *slot = Some(Arc::new(track));
                }
                self.shared.track_frame.store(0, Ordering::Relaxed);
                self.shared.track_playing.store(true, Ordering::Relaxed);
                self.shared.set_status(format!("Playing {name}"));
            }
            Err(err) => self.shared.set_status(err),
        }
    }

    fn snapshot(&self) -> SavedSettings {
        SavedSettings {
            voice_gain: self.voice_gain,
            master_gain: self.master_gain,
            tune_on: self.tune_on,
            tune_amount: self.tune_amount,
            tune_retune: self.tune_retune,
            key: self.key,
            scale: self.scale,
            reverb_on: self.reverb_on,
            reverb_mix: self.reverb_mix,
            reverb_size: self.reverb_size,
            echo_on: self.echo_on,
            echo_mix: self.echo_mix,
            echo_time: self.echo_time,
            track_gain: self.track_gain,
            send_discord: self.send_discord,
            alien_on: self.alien_on,
            alien: self.alien,
            alien_rate: self.alien_rate,
            chipmunk_on: self.chipmunk_on,
            chipmunk: self.chipmunk,
            chipmunk_height: self.chipmunk_height,
            demon_on: self.demon_on,
            demon: self.demon,
            demon_depth: self.demon_depth,
            robot_on: self.robot_on,
            robot: self.robot,
            robot_crunch: self.robot_crunch,
            telephone_on: self.telephone_on,
            telephone: self.telephone,
            telephone_tone: self.telephone_tone,
            chorus_on: self.chorus_on,
            chorus: self.chorus,
            chorus_rate: self.chorus_rate,
            radio_on: self.radio_on,
            radio: self.radio,
            radio_static: self.radio_static,
            vader_on: self.vader_on,
            vader: self.vader,
            vader_dark: self.vader_dark,
            flange_on: self.flange_on,
            flange: self.flange,
            flange_rate: self.flange_rate,
            phaser_on: self.phaser_on,
            phaser: self.phaser,
            phaser_rate: self.phaser_rate,
            vibrato_on: self.vibrato_on,
            vibrato: self.vibrato,
            vibrato_rate: self.vibrato_rate,
            overdrive_on: self.overdrive_on,
            overdrive: self.overdrive,
            overdrive_drive: self.overdrive_drive,
            underwater_on: self.underwater_on,
            underwater: self.underwater,
            underwater_depth: self.underwater_depth,
        }
    }

    fn apply_settings(&mut self, saved: &SavedSettings) {
        self.voice_gain = saved.voice_gain;
        self.master_gain = saved.master_gain;
        self.tune_on = saved.tune_on;
        self.tune_amount = saved.tune_amount;
        self.tune_retune = saved.tune_retune;
        self.key = saved.key.clamp(0, 11);
        self.scale = saved.scale.min(2);
        self.reverb_on = saved.reverb_on;
        self.reverb_mix = saved.reverb_mix;
        self.reverb_size = saved.reverb_size;
        self.echo_on = saved.echo_on;
        self.echo_mix = saved.echo_mix;
        self.echo_time = saved.echo_time;
        self.track_gain = saved.track_gain;
        self.send_discord = saved.send_discord;
        self.alien_on = saved.alien_on;
        self.alien = saved.alien;
        self.alien_rate = saved.alien_rate;
        self.chipmunk_on = saved.chipmunk_on;
        self.chipmunk = saved.chipmunk;
        self.chipmunk_height = saved.chipmunk_height;
        self.demon_on = saved.demon_on;
        self.demon = saved.demon;
        self.demon_depth = saved.demon_depth;
        self.robot_on = saved.robot_on;
        self.robot = saved.robot;
        self.robot_crunch = saved.robot_crunch;
        self.telephone_on = saved.telephone_on;
        self.telephone = saved.telephone;
        self.telephone_tone = saved.telephone_tone;
        self.chorus_on = saved.chorus_on;
        self.chorus = saved.chorus;
        self.chorus_rate = saved.chorus_rate;
        self.radio_on = saved.radio_on;
        self.radio = saved.radio;
        self.radio_static = saved.radio_static;
        self.vader_on = saved.vader_on;
        self.vader = saved.vader;
        self.vader_dark = saved.vader_dark;
        self.flange_on = saved.flange_on;
        self.flange = saved.flange;
        self.flange_rate = saved.flange_rate;
        self.phaser_on = saved.phaser_on;
        self.phaser = saved.phaser;
        self.phaser_rate = saved.phaser_rate;
        self.vibrato_on = saved.vibrato_on;
        self.vibrato = saved.vibrato;
        self.vibrato_rate = saved.vibrato_rate;
        self.overdrive_on = saved.overdrive_on;
        self.overdrive = saved.overdrive;
        self.overdrive_drive = saved.overdrive_drive;
        self.underwater_on = saved.underwater_on;
        self.underwater = saved.underwater;
        self.underwater_depth = saved.underwater_depth;
    }
}

impl eframe::App for BoothApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.push_params();
        ctx.request_repaint();

        let live = self.shared.live.load(Ordering::Relaxed);
        let freq = load_f32(&self.shared.freq);
        let target = load_f32(&self.shared.target_midi);
        let rms = load_f32(&self.shared.rms);
        let status = self
            .shared
            .status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();
        let track_name = self
            .shared
            .track
            .lock()
            .ok()
            .and_then(|t| t.as_ref().map(|track| track.name.clone()))
            .unwrap_or_else(|| "No file chosen".to_string());
        let playing = self.shared.track_playing.load(Ordering::Relaxed);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("LIVE VOICE BOOTH")
                        .color(GOLD)
                        .small()
                        .strong(),
                );
                ui.label(RichText::new("KARAOKE").color(PINK).size(64.0).strong());
                ui.label(
                    RichText::new("Native WASAPI path — amplify, snap to pitch, soak in reverb.")
                        .color(MUTED),
                );
            });
            ui.add_space(12.0);

            egui::Frame::new()
                .fill(PANEL)
                .stroke(Stroke::new(1.0_f32, LINE))
                .corner_radius(18.0)
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        let note = if freq > 0.0 && target > 0.0 {
                            let rounded = target.round() as i32;
                            let pc = ((rounded % 12) + 12) % 12;
                            let octave = rounded.div_euclid(12) - 1;
                            format!("{}{octave}", NOTE_NAMES[pc as usize])
                        } else {
                            "—".to_string()
                        };
                        ui.label(RichText::new(note).color(Color32::WHITE).size(56.0).strong());
                        let meta = if freq > 0.0 {
                            let cents = cents_off(freq, target).round();
                            format!("{freq:.0} Hz · {cents:+.0}¢")
                        } else if live {
                            "listening".to_string()
                        } else {
                            "waiting".to_string()
                        };
                        ui.label(RichText::new(meta).color(CYAN));
                        ui.add_space(8.0);
                        let meter = (rms * 2.8).clamp(0.0, 1.0);
                        let mut dummy = meter;
                        ui.add(
                            egui::ProgressBar::new(dummy)
                                .desired_width(360.0)
                                .fill(PINK),
                        );
                        dummy = meter;
                        let _ = dummy;
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("Use headphones so the speakers do not feed back.")
                                .color(MUTED),
                        );
                        ui.add_space(10.0);
                        let label = if live {
                            "Stop microphone"
                        } else {
                            "Start microphone"
                        };
                        if ui
                            .add_sized(Vec2::new(220.0, 40.0), egui::Button::new(label))
                            .clicked()
                        {
                            self.toggle_mic();
                        }
                        ui.label(RichText::new(status).color(if live { CYAN } else { MUTED }));
                    });
                });

            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                compact_panel(ui, "VOICE", |ui| {
                    ui.horizontal(|ui| {
                        knob(ui, "Amplify", &mut self.voice_gain, 0.0..=3.0);
                        knob(ui, "Master", &mut self.master_gain, 0.0..=1.5);
                    });
                });
                compact_panel(ui, "AUTOTUNE", |ui| {
                    ui.checkbox(&mut self.tune_on, "Enable");
                    egui::ComboBox::from_id_salt("key")
                        .width(CARD_INNER - 4.0)
                        .selected_text(format!("Key {}", NOTE_NAMES[self.key as usize]))
                        .show_ui(ui, |ui| {
                            for (i, name) in NOTE_NAMES.iter().enumerate() {
                                ui.selectable_value(&mut self.key, i as i32, *name);
                            }
                        });
                    egui::ComboBox::from_id_salt("scale")
                        .width(CARD_INNER - 4.0)
                        .selected_text(["Chromatic", "Major", "Minor"][self.scale])
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.scale, 0, "Chromatic");
                            ui.selectable_value(&mut self.scale, 1, "Major");
                            ui.selectable_value(&mut self.scale, 2, "Minor");
                        });
                    ui.horizontal(|ui| {
                        knob(ui, "Amount", &mut self.tune_amount, 0.0..=1.0);
                        knob(ui, "Snap", &mut self.tune_retune, 0.05..=1.0);
                    });
                });
                compact_panel(ui, "SPACE", |ui| {
                    on_off_row(ui, "Reverb", &mut self.reverb_on);
                    ui.horizontal(|ui| {
                        knob(ui, "Reverb", &mut self.reverb_mix, 0.0..=1.0);
                        knob(ui, "Room", &mut self.reverb_size, 0.0..=1.0);
                    });
                    on_off_row(ui, "Echo", &mut self.echo_on);
                    ui.horizontal(|ui| {
                        knob(ui, "Echo", &mut self.echo_mix, 0.0..=1.0);
                        knob(ui, "Time", &mut self.echo_time, 0.08..=0.7);
                    });
                });
                fx_panel(ui, "ALIEN", &mut self.alien_on, |ui| {
                    ui.horizontal(|ui| {
                        knob(ui, "Amount", &mut self.alien, 0.0..=1.0);
                        knob(ui, "Rate", &mut self.alien_rate, 0.05..=1.0);
                    });
                });
                fx_panel(ui, "CHIPMUNK", &mut self.chipmunk_on, |ui| {
                    ui.horizontal(|ui| {
                        knob(ui, "Amount", &mut self.chipmunk, 0.0..=1.0);
                        knob(ui, "Height", &mut self.chipmunk_height, 0.0..=1.0);
                    });
                });
                fx_panel(ui, "DEMON", &mut self.demon_on, |ui| {
                    ui.horizontal(|ui| {
                        knob(ui, "Amount", &mut self.demon, 0.0..=1.0);
                        knob(ui, "Depth", &mut self.demon_depth, 0.0..=1.0);
                    });
                });
                fx_panel(ui, "ROBOT", &mut self.robot_on, |ui| {
                    ui.horizontal(|ui| {
                        knob(ui, "Amount", &mut self.robot, 0.0..=1.0);
                        knob(ui, "Crunch", &mut self.robot_crunch, 0.0..=1.0);
                    });
                });
                fx_panel(ui, "TELEPHONE", &mut self.telephone_on, |ui| {
                    ui.horizontal(|ui| {
                        knob(ui, "Amount", &mut self.telephone, 0.0..=1.0);
                        knob(ui, "Tone", &mut self.telephone_tone, 0.0..=1.0);
                    });
                });
                fx_panel(ui, "CHORUS", &mut self.chorus_on, |ui| {
                    ui.horizontal(|ui| {
                        knob(ui, "Mix", &mut self.chorus, 0.0..=1.0);
                        knob(ui, "Rate", &mut self.chorus_rate, 0.0..=1.0);
                    });
                });
                fx_panel(ui, "RADIO", &mut self.radio_on, |ui| {
                    ui.horizontal(|ui| {
                        knob(ui, "Amount", &mut self.radio, 0.0..=1.0);
                        knob(ui, "Static", &mut self.radio_static, 0.0..=1.0);
                    });
                });
                fx_panel(ui, "VADER", &mut self.vader_on, |ui| {
                    ui.horizontal(|ui| {
                        knob(ui, "Amount", &mut self.vader, 0.0..=1.0);
                        knob(ui, "Dark", &mut self.vader_dark, 0.0..=1.0);
                    });
                });
                fx_panel(ui, "FLANGE", &mut self.flange_on, |ui| {
                    ui.horizontal(|ui| {
                        knob(ui, "Mix", &mut self.flange, 0.0..=1.0);
                        knob(ui, "Rate", &mut self.flange_rate, 0.0..=1.0);
                    });
                });
                fx_panel(ui, "PHASER", &mut self.phaser_on, |ui| {
                    ui.horizontal(|ui| {
                        knob(ui, "Mix", &mut self.phaser, 0.0..=1.0);
                        knob(ui, "Rate", &mut self.phaser_rate, 0.0..=1.0);
                    });
                });
                fx_panel(ui, "VIBRATO", &mut self.vibrato_on, |ui| {
                    ui.horizontal(|ui| {
                        knob(ui, "Amount", &mut self.vibrato, 0.0..=1.0);
                        knob(ui, "Rate", &mut self.vibrato_rate, 0.0..=1.0);
                    });
                });
                fx_panel(ui, "OVERDRIVE", &mut self.overdrive_on, |ui| {
                    ui.horizontal(|ui| {
                        knob(ui, "Amount", &mut self.overdrive, 0.0..=1.0);
                        knob(ui, "Drive", &mut self.overdrive_drive, 0.0..=1.0);
                    });
                });
                fx_panel(ui, "UNDERWATER", &mut self.underwater_on, |ui| {
                    ui.horizontal(|ui| {
                        knob(ui, "Amount", &mut self.underwater, 0.0..=1.0);
                        knob(ui, "Depth", &mut self.underwater_depth, 0.0..=1.0);
                    });
                });
            });

            ui.add_space(8.0);
            panel(ui, "TRACK", |ui| {
                ui.horizontal(|ui| {
                    ui.label(track_name);
                    if ui.button("Choose file").clicked() {
                        self.pick_track();
                    }
                    let toggle = if playing { "Pause" } else { "Play" };
                    if ui.button(toggle).clicked() {
                        self.shared.track_playing.store(!playing, Ordering::Relaxed);
                    }
                    knob(ui, "Level", &mut self.track_gain, 0.0..=1.2);
                    ui.separator();
                    ui.checkbox(&mut self.send_discord, "Discord mode");
                    if ui.button("Recording settings").clicked() {
                        let _ = std::process::Command::new("control")
                            .args(["mmsys.cpl,,1"])
                            .spawn();
                    }
                    if let Some(name) = &self.stereo_mix_name {
                        ui.label(RichText::new(format!("Use {name} in Discord")).color(CYAN).small());
                    } else {
                        ui.label(
                            RichText::new("Enable Stereo Mix, then set Discord input to it.")
                                .color(MUTED)
                                .small(),
                        );
                    }
                    let has_virtual = self.playback_devices.iter().any(|device| device.looks_virtual);
                    if has_virtual {
                        if ui.button("Refresh").clicked() {
                            self.refresh_devices();
                        }
                        let selected_name = self
                            .playback_devices
                            .iter()
                            .find(|device| device.id == self.discord_device_id)
                            .map(|device| device.name.as_str())
                            .unwrap_or("Cable");
                        egui::ComboBox::from_id_salt("discord-device")
                            .selected_text(selected_name)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.discord_device_id,
                                    String::new(),
                                    "None (Stereo Mix)",
                                );
                                for device in &self.playback_devices {
                                    if device.looks_virtual {
                                        ui.selectable_value(
                                            &mut self.discord_device_id,
                                            device.id.clone(),
                                            device.name.clone(),
                                        );
                                    }
                                }
                            });
                    }
                });
            });
        });
        self.autosave.tick(self.snapshot());
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.autosave.flush(self.snapshot());
        if let Some(engine) = self.engine.take() {
            engine.stop();
        }
    }
}

const BG: Color32 = Color32::from_rgb(7, 6, 12);
const PANEL: Color32 = Color32::from_rgb(22, 16, 34);
const LINE: Color32 = Color32::from_rgb(70, 56, 32);
const PINK: Color32 = Color32::from_rgb(255, 45, 149);
const GOLD: Color32 = Color32::from_rgb(255, 209, 102);
const CYAN: Color32 = Color32::from_rgb(92, 225, 230);
const MUTED: Color32 = Color32::from_rgb(183, 173, 200);
const KNOB_COL: f32 = 58.0;
const CARD_INNER: f32 = KNOB_COL * 2.0 + 6.0;

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = Color32::from_rgb(16, 12, 24);
    visuals.override_text_color = Some(Color32::from_rgb(246, 241, 255));
    let rail = Color32::from_rgb(132, 112, 168);
    visuals.widgets.inactive.bg_fill = rail;
    visuals.widgets.inactive.weak_bg_fill = rail;
    visuals.widgets.noninteractive.bg_fill = rail;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(168, 142, 204);
    visuals.widgets.active.bg_fill = PINK;
    visuals.selection.bg_fill = PINK;
    visuals.slider_trailing_fill = true;
    ctx.set_visuals(visuals);
}

fn panel(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0_f32, LINE))
        .corner_radius(12.0)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.label(RichText::new(title).color(GOLD).small().strong());
            ui.add_space(4.0);
            add(ui);
        });
}

fn compact_panel(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    sized_card(ui, |ui| {
        ui.label(RichText::new(title).color(GOLD).small().strong());
        ui.add_space(4.0);
        add(ui);
    });
}

fn fx_panel(ui: &mut egui::Ui, title: &str, on: &mut bool, add: impl FnOnce(&mut egui::Ui)) {
    sized_card(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(title).color(GOLD).small().strong());
            ui.add_space(4.0);
            toggle_switch(ui, on);
            ui.add_space(6.0);
        });
        add(ui);
    });
}

fn sized_card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let outer = CARD_INNER + 16.0;
    ui.scope(|ui| {
        ui.set_min_width(outer);
        ui.set_max_width(outer);
        egui::Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0_f32, LINE))
            .corner_radius(12.0)
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.set_width(CARD_INNER);
                ui.set_max_width(CARD_INNER);
                add(ui);
            });
    });
}

fn on_off_row(ui: &mut egui::Ui, label: &str, on: &mut bool) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(GOLD).small().strong());
        ui.add_space(6.0);
        toggle_switch(ui, on);
    });
}

fn toggle_switch(ui: &mut egui::Ui, on: &mut bool) {
    let size = Vec2::new(36.0, 18.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
    }
    let fill = if *on {
        PINK
    } else {
        Color32::from_rgb(70, 56, 80)
    };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 9.0, fill);
    let knob_x = if *on {
        rect.right() - 9.0
    } else {
        rect.left() + 9.0
    };
    painter.circle_filled(Pos2::new(knob_x, rect.center().y), 6.5, Color32::WHITE);
}

fn gated(on: bool, value: f32) -> f32 {
    if on {
        value
    } else {
        0.0
    }
}

fn knob(ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>) {
    ui.allocate_ui_with_layout(
        Vec2::new(KNOB_COL, 86.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::click_and_drag());
        let min = *range.start();
        let max = *range.end();
        let span = (max - min).max(1e-6);
        if response.dragged() {
            let dy = ui.input(|input| input.pointer.delta().y);
            *value = (*value - dy * span / 130.0).clamp(min, max);
        }
        if response.hovered() {
            let scroll = ui.input(|input| input.raw_scroll_delta.y);
            if scroll.abs() > 0.0 {
                *value = (*value + scroll.signum() * span * 0.04).clamp(min, max);
            }
        }
        let t = ((*value - min) / span).clamp(0.0, 1.0);
        let center = rect.center();
        let radius = 17.0;
        let painter = ui.painter_at(rect);
        painter.circle_filled(center, radius, Color32::from_rgb(16, 12, 24));
        painter.circle_stroke(center, radius, Stroke::new(2.0_f32, LINE));
        let start = std::f32::consts::PI * 0.75;
        let sweep = std::f32::consts::PI * 1.5;
        let steps = 28;
        let mut prev = None;
        for i in 0..=steps {
            let unit = i as f32 / steps as f32;
            let angle = start + sweep * unit;
            let point = Pos2::new(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            );
            if let Some(last) = prev {
                let color = if unit <= t {
                    PINK
                } else {
                    Color32::from_rgb(72, 58, 88)
                };
                painter.line_segment([last, point], Stroke::new(3.0_f32, color));
            }
            prev = Some(point);
        }
        let angle = start + sweep * t;
        let tip = Pos2::new(
            center.x + (radius - 5.0) * angle.cos(),
            center.y + (radius - 5.0) * angle.sin(),
        );
        painter.line_segment([center, tip], Stroke::new(2.0_f32, GOLD));
        let cap = if response.hovered() || response.dragged() {
            PINK
        } else {
            GOLD
        };
        painter.circle_filled(center, 3.4, cap);
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.label(RichText::new(label).small().color(MUTED));
            ui.label(RichText::new(format!("{value:.2}")).small().color(CYAN));
        });
    });
}
