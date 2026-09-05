use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rtrb::{Consumer, Producer, RingBuffer};
use wasapi::{
    Direction, DeviceEnumerator, SampleType, StreamMode, WaveFormat, initialize_mta,
};

use crate::dsp::VoiceChain;
use crate::state::{load_f32, Shared};

const RING_FRAMES: usize = 1024;
const CABLE_RING_FRAMES: usize = 4096;
const TARGET_RATE: usize = 48_000;

#[derive(Clone)]
pub struct PlaybackDevice {
    pub id: String,
    pub name: String,
    pub looks_virtual: bool,
}

pub fn list_playback_devices() -> Vec<PlaybackDevice> {
    let Ok(enumerator) = DeviceEnumerator::new() else {
        return Vec::new();
    };
    let Ok(collection) = enumerator.get_device_collection(&Direction::Render) else {
        return Vec::new();
    };
    let count = collection.get_nbr_devices().unwrap_or(0);
    let mut devices = Vec::new();
    for index in 0..count {
        let Ok(device) = collection.get_device_at_index(index) else {
            continue;
        };
        let Ok(name) = device.get_friendlyname() else {
            continue;
        };
        let Ok(id) = device.get_id() else {
            continue;
        };
        let looks_virtual = is_virtual_cable_name(&name);
        devices.push(PlaybackDevice {
            id,
            name,
            looks_virtual,
        });
    }
    devices.sort_by(|a, b| b.looks_virtual.cmp(&a.looks_virtual).then(a.name.cmp(&b.name)));
    devices
}

pub fn is_virtual_cable_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("cable input")
        || lower.contains("vb-audio")
        || lower.contains("voicemeeter")
        || lower.contains("virtual cable")
        || lower.contains("virtual audio")
        || lower.contains("line 1")
}

pub fn is_stereo_mix_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("stereo mix")
        || lower.contains("what u hear")
        || lower.contains("wave out mix")
        || lower.contains("waveoutmix")
        || lower.contains("rec. playback")
        || lower.contains("loopback")
}

pub fn stereo_mix_device_name() -> Option<String> {
    let enumerator = DeviceEnumerator::new().ok()?;
    let collection = enumerator.get_device_collection(&Direction::Capture).ok()?;
    let count = collection.get_nbr_devices().unwrap_or(0);
    for index in 0..count {
        let Ok(device) = collection.get_device_at_index(index) else {
            continue;
        };
        let Ok(name) = device.get_friendlyname() else {
            continue;
        };
        if is_stereo_mix_name(&name) {
            return Some(name);
        }
    }
    None
}

pub struct Engine {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Engine {
    pub fn start(shared: Arc<Shared>) -> Result<Self, String> {
        let _ = initialize_mta();
        shared.sample_rate.store(0, Ordering::Relaxed);
        shared.output_ready.store(false, Ordering::Relaxed);
        let stop = Arc::new(AtomicBool::new(false));
        let send_discord = shared.send_discord.load(Ordering::Relaxed);
        let discord_device = shared
            .discord_device
            .lock()
            .ok()
            .map(|id| id.clone())
            .unwrap_or_default();
        let (producer, consumer) = RingBuffer::<f32>::new(RING_FRAMES);
        let mut cable_pair = None;
        if send_discord && !discord_device.is_empty() {
            cable_pair = Some(RingBuffer::<f32>::new(CABLE_RING_FRAMES));
        }

        let cap_shared = Arc::clone(&shared);
        let cap_stop = Arc::clone(&stop);
        let capture = thread::Builder::new()
            .name("karaoke-cap".into())
            .spawn(move || {
                let _ = initialize_mta();
                if let Err(err) = capture_thread(producer, cap_stop, cap_shared.clone()) {
                    cap_shared.set_status(format!("Mic error: {err}"));
                    cap_shared.live.store(false, Ordering::Relaxed);
                }
            })
            .map_err(|err| err.to_string())?;

        let out_shared = Arc::clone(&shared);
        let out_stop = Arc::clone(&stop);
        let (cable_out, cable_cons) = match cable_pair {
            Some((prod, cons)) => (Some(prod), Some(cons)),
            None => (None, None),
        };
        let render = thread::Builder::new()
            .name("karaoke-out".into())
            .spawn(move || {
                let _ = initialize_mta();
                if let Err(err) = render_thread(
                    consumer,
                    cable_out,
                    out_stop,
                    out_shared.clone(),
                    !send_discord,
                ) {
                    out_shared.set_status(format!("Output error: {err}"));
                    out_shared.live.store(false, Ordering::Relaxed);
                }
            })
            .map_err(|err| err.to_string())?;

        let mut threads = vec![capture, render];
        if let Some(cable_cons) = cable_cons {
            let cable_shared = Arc::clone(&shared);
            let cable_stop = Arc::clone(&stop);
            let cable = thread::Builder::new()
                .name("karaoke-discord".into())
                .spawn(move || {
                    let _ = initialize_mta();
                    if let Err(err) =
                        cable_thread(cable_cons, cable_stop, cable_shared.clone(), discord_device)
                    {
                        cable_shared.set_status(format!("Discord cable: {err}"));
                    }
                })
                .map_err(|err| err.to_string())?;
            threads.push(cable);
        }

        Ok(Self {
            stop,
            threads,
        })
    }

    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        for handle in self.threads {
            let _ = handle.join();
        }
    }
}

struct ClientInfo {
    exclusive: bool,
    float: bool,
    channels: usize,
    sample_rate: u32,
    block_align: usize,
}

fn open_client(
    direction: Direction,
    channels: usize,
    prefer_exclusive: bool,
) -> Result<(wasapi::AudioClient, wasapi::Handle, ClientInfo), String> {
    let enumerator = DeviceEnumerator::new().map_err(|err| err.to_string())?;
    let device = enumerator
        .get_default_device(&direction)
        .map_err(|err| err.to_string())?;
    let desired = WaveFormat::new(32, 32, &SampleType::Float, TARGET_RATE as usize, channels, None);

    if prefer_exclusive {
        if let Ok(opened) = try_exclusive(&device, &direction, &desired) {
            return Ok(opened);
        }
    }

    let mut client = device.get_iaudioclient().map_err(|err| err.to_string())?;
    let (_def, min_time) = client.get_device_period().map_err(|err| err.to_string())?;
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: min_time,
    };
    client
        .initialize_client(&desired, &direction, &mode)
        .map_err(|err| err.to_string())?;
    let event = client.set_get_eventhandle().map_err(|err| err.to_string())?;
    Ok((
        client,
        event,
        ClientInfo {
            exclusive: false,
            float: true,
            channels,
            sample_rate: TARGET_RATE as u32,
            block_align: desired.get_blockalign() as usize,
        },
    ))
}

fn open_shared_capture(sample_rate: usize) -> Result<(wasapi::AudioClient, wasapi::Handle, ClientInfo), String> {
    let enumerator = DeviceEnumerator::new().map_err(|err| err.to_string())?;
    let device = enumerator
        .get_default_device(&Direction::Capture)
        .map_err(|err| err.to_string())?;
    let desired = WaveFormat::new(32, 32, &SampleType::Float, sample_rate, 1, None);
    let mut client = device.get_iaudioclient().map_err(|err| err.to_string())?;
    let (_def, min_time) = client.get_device_period().map_err(|err| err.to_string())?;
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: min_time,
    };
    client
        .initialize_client(&desired, &Direction::Capture, &mode)
        .map_err(|err| err.to_string())?;
    let event = client.set_get_eventhandle().map_err(|err| err.to_string())?;
    Ok((
        client,
        event,
        ClientInfo {
            exclusive: false,
            float: true,
            channels: 1,
            sample_rate: sample_rate as u32,
            block_align: desired.get_blockalign() as usize,
        },
    ))
}

fn try_exclusive(
    device: &wasapi::Device,
    direction: &Direction,
    desired: &WaveFormat,
) -> Result<(wasapi::AudioClient, wasapi::Handle, ClientInfo), String> {
    let mut client = device.get_iaudioclient().map_err(|err| err.to_string())?;
    let fmt = client
        .is_supported_exclusive_with_quirks(desired)
        .map_err(|err| err.to_string())?;
    let bits = fmt.get_bitspersample();
    if bits != 16 && bits != 32 {
        return Err("exclusive format is not 16 or 32 bit".into());
    }
    let (_def, min_time) = client.get_device_period().map_err(|err| err.to_string())?;
    let period = client
        .calculate_aligned_period_near(min_time, Some(128), &fmt)
        .unwrap_or(min_time);
    let mode = StreamMode::EventsExclusive { period_hns: period };
    client
        .initialize_client(&fmt, direction, &mode)
        .map_err(|err| err.to_string())?;
    let event = client.set_get_eventhandle().map_err(|err| err.to_string())?;
    Ok((
        client,
        event,
        ClientInfo {
            exclusive: true,
            float: bits == 32,
            channels: fmt.get_nchannels() as usize,
            sample_rate: fmt.get_samplespersec(),
            block_align: fmt.get_blockalign() as usize,
        },
    ))
}

fn wait_for_output(shared: &Shared, stop: &AtomicBool) -> Result<usize, String> {
    for _ in 0..200 {
        if stop.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        if shared.output_ready.load(Ordering::Relaxed) {
            let rate = shared.sample_rate.load(Ordering::Relaxed);
            if rate > 0 {
                return Ok(rate as usize);
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err("Output device did not start".into())
}

fn capture_thread(
    mut producer: Producer<f32>,
    stop: Arc<AtomicBool>,
    shared: Arc<Shared>,
) -> Result<(), String> {
    let rate = wait_for_output(&shared, &stop)?;
    let (client, event, info) = open_shared_capture(rate)?;
    let capture = client
        .get_audiocaptureclient()
        .map_err(|err| err.to_string())?;
    let mut queue = VecDeque::new();
    client.start_stream().map_err(|err| err.to_string())?;

    while !stop.load(Ordering::Relaxed) {
        if event.wait_for_event(200).is_err() {
            continue;
        }
        capture
            .read_from_device_to_deque(&mut queue)
            .map_err(|err| err.to_string())?;
        while queue.len() >= info.block_align {
            let mut frame = vec![0u8; info.block_align];
            for byte in &mut frame {
                *byte = queue.pop_front().unwrap_or(0);
            }
            let sample = frame_to_mono(&frame, info.channels, info.float);
            let _ = producer.push(sample);
        }
    }
    let _ = client.stop_stream();
    let _ = shared;
    Ok(())
}

fn render_thread(
    mut consumer: Consumer<f32>,
    mut cable: Option<Producer<f32>>,
    stop: Arc<AtomicBool>,
    shared: Arc<Shared>,
    prefer_exclusive: bool,
) -> Result<(), String> {
    let (client, event, info) = open_client(Direction::Render, 2, prefer_exclusive)?;
    let render = client
        .get_audiorenderclient()
        .map_err(|err| err.to_string())?;
    let mut chain = VoiceChain::new(info.sample_rate as f32);
    let mut rms_acc = 0.0;
    let mut rms_n = 0usize;

    shared.exclusive.store(info.exclusive, Ordering::Relaxed);
    shared
        .sample_rate
        .store(info.sample_rate, Ordering::Relaxed);
    let period_frames = client.get_buffer_size().unwrap_or(128);
    let latency_us = (period_frames.saturating_mul(2) * 1_000_000) / info.sample_rate.max(1);
    shared.latency_us.store(latency_us, Ordering::Relaxed);

    let available = client
        .get_available_space_in_frames()
        .map_err(|err| err.to_string())? as usize;
    write_silence(&render, available, &info)?;
    client.start_stream().map_err(|err| err.to_string())?;
    shared.output_ready.store(true, Ordering::Relaxed);

    let mode = if info.exclusive {
        "WASAPI exclusive"
    } else {
        "WASAPI shared (low period)"
    };
    let discord = if cable.is_some() {
        " · sending to Discord cable"
    } else {
        ""
    };
    shared.set_status(format!(
        "{mode} · {} Hz · ~{:.1} ms{discord}",
        info.sample_rate,
        latency_us as f32 / 1000.0
    ));

    while !stop.load(Ordering::Relaxed) {
        if event.wait_for_event(200).is_err() {
            continue;
        }
        let frames = client
            .get_available_space_in_frames()
            .map_err(|err| err.to_string())? as usize;
        if frames == 0 {
            continue;
        }
        let params = shared.params();
        let track_gain = load_f32(&shared.track_gain);
        let playing = shared.track_playing.load(Ordering::Relaxed);
        let track = shared.track.lock().ok().and_then(|g| g.clone());
        let mut mono = vec![0.0f32; frames];
        for sample in &mut mono {
            let raw = consumer.pop().unwrap_or(0.0);
            let (voice, pitch) = chain.process(raw, &params);
            let mut mixed = voice;
            if playing {
                if let Some(ref track) = track {
                    let frame = shared.track_frame.fetch_add(1, Ordering::Relaxed);
                    if frame >= track.frame_count(info.sample_rate) {
                        shared.track_playing.store(false, Ordering::Relaxed);
                        shared.track_frame.store(0, Ordering::Relaxed);
                    } else {
                        mixed += track.sample_at(frame, info.sample_rate) * track_gain;
                    }
                }
            }
            *sample = mixed.clamp(-1.0, 1.0);
            if let Some(ref mut cable) = cable {
                let _ = cable.push(*sample);
            }
            rms_acc += raw * raw;
            rms_n += 1;
            if pitch.freq > 0.0 {
                shared.freq.store(pitch.freq.to_bits(), Ordering::Relaxed);
                shared
                    .target_midi
                    .store(pitch.target_midi.to_bits(), Ordering::Relaxed);
            } else if rms_n > 64 {
                shared.freq.store(0, Ordering::Relaxed);
            }
        }
        if rms_n >= 256 {
            let rms = (rms_acc / rms_n as f32).sqrt();
            shared.rms.store(rms.to_bits(), Ordering::Relaxed);
            rms_acc = 0.0;
            rms_n = 0;
        }
        let bytes = encode_frames(&mono, &info);
        render
            .write_to_device(frames, &bytes, None)
            .map_err(|err| err.to_string())?;
    }
    let _ = client.stop_stream();
    Ok(())
}

fn open_shared_by_id(
    device_id: &str,
    sample_rate: usize,
) -> Result<(wasapi::AudioClient, wasapi::Handle, ClientInfo), String> {
    let enumerator = DeviceEnumerator::new().map_err(|err| err.to_string())?;
    let device = enumerator.get_device(device_id).map_err(|err| err.to_string())?;
    let desired = WaveFormat::new(32, 32, &SampleType::Float, sample_rate, 2, None);
    let mut client = device.get_iaudioclient().map_err(|err| err.to_string())?;
    let (_def, min_time) = client.get_device_period().map_err(|err| err.to_string())?;
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: min_time,
    };
    client
        .initialize_client(&desired, &Direction::Render, &mode)
        .map_err(|err| err.to_string())?;
    let event = client.set_get_eventhandle().map_err(|err| err.to_string())?;
    Ok((
        client,
        event,
        ClientInfo {
            exclusive: false,
            float: true,
            channels: 2,
            sample_rate: sample_rate as u32,
            block_align: desired.get_blockalign() as usize,
        },
    ))
}

fn cable_thread(
    mut consumer: Consumer<f32>,
    stop: Arc<AtomicBool>,
    shared: Arc<Shared>,
    device_id: String,
) -> Result<(), String> {
    let rate = wait_for_output(&shared, &stop)?;
    let (client, event, info) = open_shared_by_id(&device_id, rate)?;
    let render = client
        .get_audiorenderclient()
        .map_err(|err| err.to_string())?;
    let available = client
        .get_available_space_in_frames()
        .map_err(|err| err.to_string())? as usize;
    write_silence(&render, available, &info)?;
    client.start_stream().map_err(|err| err.to_string())?;

    while !stop.load(Ordering::Relaxed) {
        if event.wait_for_event(200).is_err() {
            continue;
        }
        let frames = client
            .get_available_space_in_frames()
            .map_err(|err| err.to_string())? as usize;
        if frames == 0 {
            continue;
        }
        let mut mono = vec![0.0f32; frames];
        for sample in &mut mono {
            *sample = consumer.pop().unwrap_or(0.0);
        }
        let bytes = encode_frames(&mono, &info);
        render
            .write_to_device(frames, &bytes, None)
            .map_err(|err| err.to_string())?;
    }
    let _ = client.stop_stream();
    Ok(())
}

fn write_silence(
    render: &wasapi::AudioRenderClient,
    frames: usize,
    info: &ClientInfo,
) -> Result<(), String> {
    let zeros = vec![0.0f32; frames];
    let bytes = encode_frames(&zeros, info);
    render
        .write_to_device(frames, &bytes, None)
        .map_err(|err| err.to_string())
}

fn frame_to_mono(frame: &[u8], channels: usize, float: bool) -> f32 {
    if frame.is_empty() || channels == 0 {
        return 0.0;
    }
    if float {
        let step = 4;
        let mut sum = 0.0;
        let mut count = 0usize;
        for ch in 0..channels {
            let start = ch * step;
            if start + 4 <= frame.len() {
                sum += f32::from_le_bytes([frame[start], frame[start + 1], frame[start + 2], frame[start + 3]]);
                count += 1;
            }
        }
        if count == 0 {
            0.0
        } else {
            sum / count as f32
        }
    } else {
        let step = 2;
        let mut sum = 0.0;
        let mut count = 0usize;
        for ch in 0..channels {
            let start = ch * step;
            if start + 2 <= frame.len() {
                let v = i16::from_le_bytes([frame[start], frame[start + 1]]);
                sum += f32::from(v) / 32768.0;
                count += 1;
            }
        }
        if count == 0 {
            0.0
        } else {
            sum / count as f32
        }
    }
}

fn encode_frames(mono: &[f32], info: &ClientInfo) -> Vec<u8> {
    let mut out = Vec::with_capacity(mono.len() * info.block_align);
    for &sample in mono {
        for _ in 0..info.channels {
            if info.float {
                out.extend_from_slice(&sample.to_le_bytes());
            } else {
                let v = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    out
}
