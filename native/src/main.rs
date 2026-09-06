#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod dsp;
mod engine;
mod fx;
mod pitch;
mod state;
mod track;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use engine::{list_playback_devices, stereo_mix_device_name, Engine, PlaybackDevice};
use pitch::{cents_off, NOTE_NAMES};
use state::{load_f32, store_f32, Shared};
use track::Track;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 1180.0])
            .with_min_inner_size([880.0, 900.0])
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
    reverb_mix: f32,
    reverb_size: f32,
    echo_mix: f32,
    echo_time: f32,
    track_gain: f32,
    alien: f32,
    alien_rate: f32,
    chipmunk: f32,
    chipmunk_height: f32,
    demon: f32,
    demon_depth: f32,
    robot: f32,
    robot_crunch: f32,
    telephone: f32,
    telephone_tone: f32,
    chorus: f32,
    chorus_rate: f32,
    radio: f32,
    radio_static: f32,
    vader: f32,
    vader_dark: f32,
    flange: f32,
    flange_rate: f32,
    phaser: f32,
    phaser_rate: f32,
    vibrato: f32,
    vibrato_rate: f32,
    overdrive: f32,
    overdrive_drive: f32,
    underwater: f32,
    underwater_depth: f32,
    send_discord: bool,
    discord_device_id: String,
    playback_devices: Vec<PlaybackDevice>,
    stereo_mix_name: Option<String>,
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
            reverb_mix: 0.33,
            reverb_size: 0.55,
            echo_mix: 0.22,
            echo_time: 0.26,
            track_gain: 0.7,
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
            send_discord: false,
            discord_device_id: String::new(),
            playback_devices: Vec::new(),
            stereo_mix_name: None,
        };
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
        store_f32(&self.shared.reverb_mix, self.reverb_mix);
        store_f32(&self.shared.reverb_size, self.reverb_size);
        store_f32(&self.shared.echo_mix, self.echo_mix);
        store_f32(&self.shared.echo_time, self.echo_time);
        store_f32(&self.shared.track_gain, self.track_gain);
        store_f32(&self.shared.alien, self.alien);
        store_f32(&self.shared.alien_rate, self.alien_rate);
        store_f32(&self.shared.chipmunk, self.chipmunk);
        store_f32(&self.shared.chipmunk_height, self.chipmunk_height);
        store_f32(&self.shared.demon, self.demon);
        store_f32(&self.shared.demon_depth, self.demon_depth);
        store_f32(&self.shared.robot, self.robot);
        store_f32(&self.shared.robot_crunch, self.robot_crunch);
        store_f32(&self.shared.telephone, self.telephone);
        store_f32(&self.shared.telephone_tone, self.telephone_tone);
        store_f32(&self.shared.chorus, self.chorus);
        store_f32(&self.shared.chorus_rate, self.chorus_rate);
        store_f32(&self.shared.radio, self.radio);
        store_f32(&self.shared.radio_static, self.radio_static);
        store_f32(&self.shared.vader, self.vader);
        store_f32(&self.shared.vader_dark, self.vader_dark);
        store_f32(&self.shared.flange, self.flange);
        store_f32(&self.shared.flange_rate, self.flange_rate);
        store_f32(&self.shared.phaser, self.phaser);
        store_f32(&self.shared.phaser_rate, self.phaser_rate);
        store_f32(&self.shared.vibrato, self.vibrato);
        store_f32(&self.shared.vibrato_rate, self.vibrato_rate);
        store_f32(&self.shared.overdrive, self.overdrive);
        store_f32(&self.shared.overdrive_drive, self.overdrive_drive);
        store_f32(&self.shared.underwater, self.underwater);
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

            ui.add_space(14.0);
            ui.columns(4, |cols| {
                panel(&mut cols[0], "VOICE", |ui| {
                    slider(ui, "Amplify", &mut self.voice_gain, 0.0..=3.0);
                    slider(ui, "Master", &mut self.master_gain, 0.0..=1.5);
                });
                panel(&mut cols[1], "AUTOTUNE", |ui| {
                    ui.checkbox(&mut self.tune_on, "Enable");
                    slider(ui, "Amount", &mut self.tune_amount, 0.0..=1.0);
                    slider(ui, "Snap speed", &mut self.tune_retune, 0.05..=1.0);
                    ui.label("Key");
                    egui::ComboBox::from_id_salt("key")
                        .selected_text(NOTE_NAMES[self.key as usize])
                        .show_ui(ui, |ui| {
                            for (i, name) in NOTE_NAMES.iter().enumerate() {
                                ui.selectable_value(&mut self.key, i as i32, *name);
                            }
                        });
                    ui.label("Scale");
                    egui::ComboBox::from_id_salt("scale")
                        .selected_text(["Chromatic", "Major", "Minor"][self.scale])
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.scale, 0, "Chromatic");
                            ui.selectable_value(&mut self.scale, 1, "Major");
                            ui.selectable_value(&mut self.scale, 2, "Minor");
                        });
                });
                panel(&mut cols[2], "SPACE", |ui| {
                    slider(ui, "Reverb", &mut self.reverb_mix, 0.0..=1.0);
                    slider(ui, "Room size", &mut self.reverb_size, 0.0..=1.0);
                    slider(ui, "Echo", &mut self.echo_mix, 0.0..=1.0);
                    slider(ui, "Echo time", &mut self.echo_time, 0.08..=0.7);
                });
                panel(&mut cols[3], "TRACK", |ui| {
                    ui.label(track_name);
                    if ui.button("Choose file").clicked() {
                        self.pick_track();
                    }
                    slider(ui, "Track level", &mut self.track_gain, 0.0..=1.2);
                    let toggle = if playing { "Pause track" } else { "Play track" };
                    if ui.button(toggle).clicked() {
                        let next = !playing;
                        self.shared.track_playing.store(next, Ordering::Relaxed);
                    }
                    ui.separator();
                    ui.checkbox(&mut self.send_discord, "Discord mode");
                    ui.label(
                        RichText::new("Free path: Windows Stereo Mix. No paid cable.")
                            .color(MUTED)
                            .small(),
                    );
                    if let Some(name) = &self.stereo_mix_name {
                        ui.label(
                            RichText::new(format!("Found {name}. In Discord, set Input Device to that."))
                                .color(CYAN)
                                .small(),
                        );
                    } else {
                        ui.label(
                            RichText::new("Enable Stereo Mix: Recording tab → right-click empty area → Show Disabled Devices → enable Stereo Mix.")
                                .color(MUTED)
                                .small(),
                        );
                    }
                    if ui.button("Open sound recording settings").clicked() {
                        let _ = std::process::Command::new("control")
                            .args(["mmsys.cpl,,1"])
                            .spawn();
                    }
                    ui.label(
                        RichText::new("Then Discord → Voice & Video → Input = Stereo Mix. Turn off Discord noise suppression. Headphones on. Restart the booth mic after checking Discord mode.")
                            .color(MUTED)
                            .small(),
                    );
                    let has_virtual = self.playback_devices.iter().any(|device| device.looks_virtual);
                    if has_virtual {
                        if ui.button("Refresh devices").clicked() {
                            self.refresh_devices();
                        }
                        let selected_name = self
                            .playback_devices
                            .iter()
                            .find(|device| device.id == self.discord_device_id)
                            .map(|device| device.name.as_str())
                            .unwrap_or("Optional virtual cable");
                        egui::ComboBox::from_id_salt("discord-device")
                            .selected_text(selected_name)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.discord_device_id,
                                    String::new(),
                                    "None (use Stereo Mix)",
                                );
                                for device in &self.playback_devices {
                                    if !device.looks_virtual {
                                        continue;
                                    }
                                    ui.selectable_value(
                                        &mut self.discord_device_id,
                                        device.id.clone(),
                                        device.name.clone(),
                                    );
                                }
                            });
                    }
                });
            });

            ui.add_space(12.0);
            ui.label(RichText::new("VOICE FX").color(GOLD).small().strong());
            ui.add_space(6.0);
            ui.columns(4, |cols| {
                panel(&mut cols[0], "ALIEN", |ui| {
                    slider(ui, "Amount", &mut self.alien, 0.0..=1.0);
                    slider(ui, "Mod rate", &mut self.alien_rate, 0.05..=1.0);
                });
                panel(&mut cols[1], "CHIPMUNK", |ui| {
                    slider(ui, "Amount", &mut self.chipmunk, 0.0..=1.0);
                    slider(ui, "Height", &mut self.chipmunk_height, 0.0..=1.0);
                });
                panel(&mut cols[2], "DEMON", |ui| {
                    slider(ui, "Amount", &mut self.demon, 0.0..=1.0);
                    slider(ui, "Depth", &mut self.demon_depth, 0.0..=1.0);
                });
                panel(&mut cols[3], "ROBOT", |ui| {
                    slider(ui, "Amount", &mut self.robot, 0.0..=1.0);
                    slider(ui, "Crunch", &mut self.robot_crunch, 0.0..=1.0);
                });
            });
            ui.add_space(8.0);
            ui.columns(3, |cols| {
                panel(&mut cols[0], "TELEPHONE", |ui| {
                    slider(ui, "Amount", &mut self.telephone, 0.0..=1.0);
                    slider(ui, "Tone", &mut self.telephone_tone, 0.0..=1.0);
                });
                panel(&mut cols[1], "CHORUS", |ui| {
                    slider(ui, "Mix", &mut self.chorus, 0.0..=1.0);
                    slider(ui, "Rate", &mut self.chorus_rate, 0.0..=1.0);
                });
                panel(&mut cols[2], "RADIO", |ui| {
                    slider(ui, "Amount", &mut self.radio, 0.0..=1.0);
                    slider(ui, "Static", &mut self.radio_static, 0.0..=1.0);
                });
            });
            ui.add_space(8.0);
            ui.columns(3, |cols| {
                panel(&mut cols[0], "VADER", |ui| {
                    slider(ui, "Amount", &mut self.vader, 0.0..=1.0);
                    slider(ui, "Dark", &mut self.vader_dark, 0.0..=1.0);
                });
                panel(&mut cols[1], "FLANGE", |ui| {
                    slider(ui, "Mix", &mut self.flange, 0.0..=1.0);
                    slider(ui, "Rate", &mut self.flange_rate, 0.0..=1.0);
                });
                panel(&mut cols[2], "PHASER", |ui| {
                    slider(ui, "Mix", &mut self.phaser, 0.0..=1.0);
                    slider(ui, "Rate", &mut self.phaser_rate, 0.0..=1.0);
                });
            });
            ui.add_space(8.0);
            ui.columns(3, |cols| {
                panel(&mut cols[0], "VIBRATO", |ui| {
                    slider(ui, "Amount", &mut self.vibrato, 0.0..=1.0);
                    slider(ui, "Rate", &mut self.vibrato_rate, 0.0..=1.0);
                });
                panel(&mut cols[1], "OVERDRIVE", |ui| {
                    slider(ui, "Amount", &mut self.overdrive, 0.0..=1.0);
                    slider(ui, "Drive", &mut self.overdrive_drive, 0.0..=1.0);
                });
                panel(&mut cols[2], "UNDERWATER", |ui| {
                    slider(ui, "Amount", &mut self.underwater, 0.0..=1.0);
                    slider(ui, "Depth", &mut self.underwater_depth, 0.0..=1.0);
                });
            });
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
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
        .corner_radius(16.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.label(RichText::new(title).color(GOLD).small().strong());
            ui.add_space(6.0);
            add(ui);
        });
}

fn slider(ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(format!("{value:.2}"));
        });
    });
    ui.add(egui::Slider::new(value, range).show_value(false));
}
