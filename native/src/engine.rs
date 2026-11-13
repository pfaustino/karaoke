use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use wasapi::{initialize_mta, DeviceEnumerator, Direction};
use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::Media::Audio::{
    IAudioCaptureClient, IAudioRenderClient, IMMDeviceEnumerator, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
use windows::Win32::System::Threading::WaitForMultipleObjects;
use windows_core::HSTRING;

use crate::dry_monitor::{
    drain_capture_packets, encode_mono_to_frame, frames_to_ms, open_low_latency_endpoint,
    prime_silent, DryMonitor, DryMonitorMetrics, DryRing, EndpointFormat, LowLatencyEndpoint,
    MmcssGuard, StopEvent, StreamingResampler,
};
use crate::dsp::VoiceChain;
use crate::state::{load_f32, Shared};

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
    devices.sort_by(|a, b| {
        b.looks_virtual
            .cmp(&a.looks_virtual)
            .then(a.name.cmp(&b.name))
    });
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

pub fn is_virtual_mic_render_name(name: &str) -> bool {
    is_virtual_cable_name(name) || name.to_ascii_lowercase().contains("streaming microphone")
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

pub(crate) fn adapter_key(interface_name: &str, friendly_name: &str) -> String {
    let trimmed = interface_name.trim();
    if !trimmed.is_empty() {
        return trimmed.to_ascii_lowercase();
    }
    if let (Some(open), Some(close)) = (friendly_name.find('('), friendly_name.rfind(')')) {
        if open + 1 < close {
            return friendly_name[open + 1..close].trim().to_ascii_lowercase();
        }
    }
    String::new()
}

#[derive(Clone, Debug)]
struct WetStaticMetrics {
    target_name: String,
    rate: u32,
    period_frames: u32,
    min_period_frames: u32,
    buffer_frames: u32,
    low_period_api: bool,
    periodicity_locked: bool,
}

pub struct Engine {
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    wet_stop_signal: Arc<StopEvent>,
    wet_thread: Option<JoinHandle<()>>,
    wet_static: WetStaticMetrics,
    dry_monitor: Option<DryMonitor>,
}

impl Engine {
    pub fn start(shared: Arc<Shared>) -> Result<Self, String> {
        let _ = initialize_mta();
        shared.reset_stream_metrics();

        let dry_monitor = match DryMonitor::start(Arc::clone(&shared)) {
            Ok(dm) => Some(dm),
            Err(err) => {
                eprintln!("[audio] dry-monitor failed to start: {err}");
                None
            }
        };

        let stop = Arc::new(AtomicBool::new(false));
        let wet_stop_signal = Arc::new(StopEvent::new()?);
        let discord_device = shared
            .discord_device
            .lock()
            .ok()
            .map(|id| id.clone())
            .unwrap_or_default();

        let (init_tx, init_rx) = mpsc::sync_channel::<Result<WetStaticMetrics, String>>(1);
        let wet_shared = Arc::clone(&shared);
        let wet_stop = Arc::clone(&stop);
        let wet_signal = Arc::clone(&wet_stop_signal);

        let wet_handle = thread::Builder::new()
            .name("karaoke-wet".into())
            .spawn(move || {
                let _ = initialize_mta();
                if let Err(err) =
                    run_wet_pipeline(wet_shared.clone(), wet_stop, wet_signal, discord_device, init_tx)
                {
                    wet_shared.set_status(format!("Wet pipeline error: {err}"));
                    wet_shared.live.store(false, Ordering::Relaxed);
                }
            })
            .map_err(|err| err.to_string())?;

        let wet_static = match init_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(info)) => info,
            Ok(Err(err)) => {
                stop.store(true, Ordering::Relaxed);
                wet_stop_signal.signal();
                let _ = wet_handle.join();
                if let Some(dry) = dry_monitor {
                    dry.stop();
                }
                return Err(err);
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                wet_stop_signal.signal();
                let _ = wet_handle.join();
                if let Some(dry) = dry_monitor {
                    dry.stop();
                }
                return Err("Wet pipeline initialization timed out".into());
            }
        };

        Ok(Self {
            shared,
            stop,
            wet_stop_signal,
            wet_thread: Some(wet_handle),
            wet_static,
            dry_monitor,
        })
    }

    pub fn dry_monitor_metrics(&self) -> Option<DryMonitorMetrics> {
        let mut m = if let Some(ref dry) = self.dry_monitor {
            dry.base_metrics().clone()
        } else {
            DryMonitorMetrics {
                mic_name: String::new(),
                capture_rate: 0,
                capture_period_frames: 0,
                capture_min_period_frames: 0,
                capture_default_period_frames: 0,
                capture_buffer_frames: 0,
                capture_raw_mode: false,
                capture_low_period_api: false,
                capture_periodicity_locked: false,
                render_rate: 0,
                render_period_frames: 0,
                render_min_period_frames: 0,
                render_default_period_frames: 0,
                render_buffer_frames: 0,
                render_low_period_api: false,
                render_periodicity_locked: false,
                initial_latency_us: 0,
                dry_ring_fill_frames: 0,
                dry_max_ring_frames: 0,
                dry_underruns: 0,
                dry_overruns: 0,
                wet_target_name: String::new(),
                wet_rate: 0,
                wet_period_frames: 0,
                wet_min_period_frames: 0,
                wet_buffer_frames: 0,
                wet_low_period_api: false,
                wet_periodicity_locked: false,
                wet_ring_fill_frames: 0,
                wet_max_ring_frames: 0,
                wet_underruns: 0,
                wet_overruns: 0,
            }
        };

        m.dry_ring_fill_frames = self.shared.dry_ring_fill_frames.load(Ordering::Relaxed);
        m.dry_max_ring_frames = self.shared.dry_max_ring_frames.load(Ordering::Relaxed);
        m.dry_underruns = self.shared.dry_underruns.load(Ordering::Relaxed);
        m.dry_overruns = self.shared.dry_overruns.load(Ordering::Relaxed);

        m.wet_target_name = self.wet_static.target_name.clone();
        m.wet_rate = self.wet_static.rate;
        m.wet_period_frames = self.wet_static.period_frames;
        m.wet_min_period_frames = self.wet_static.min_period_frames;
        m.wet_buffer_frames = self.wet_static.buffer_frames;
        m.wet_low_period_api = self.wet_static.low_period_api;
        m.wet_periodicity_locked = self.wet_static.periodicity_locked;
        m.wet_ring_fill_frames = self.shared.wet_ring_fill_frames.load(Ordering::Relaxed);
        m.wet_max_ring_frames = self.shared.wet_max_ring_frames.load(Ordering::Relaxed);
        m.wet_underruns = self.shared.wet_underruns.load(Ordering::Relaxed);
        m.wet_overruns = self.shared.wet_overruns.load(Ordering::Relaxed);

        Some(m)
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.wet_stop_signal.signal();
        if let Some(dry) = self.dry_monitor.take() {
            dry.stop();
        }
        if let Some(handle) = self.wet_thread.take() {
            let _ = handle.join();
        }
        self.shared.reclaim_old_tracks();
    }
}

pub(crate) fn pick_capture_device(
    enumerator: &DeviceEnumerator,
) -> Result<(wasapi::Device, String), String> {
    let default = enumerator
        .get_default_device(&Direction::Capture)
        .map_err(|err| err.to_string())?;
    let default_name = default.get_friendlyname().unwrap_or_default();
    if !is_stereo_mix_name(&default_name) {
        return Ok((default, default_name));
    }

    let collection = enumerator
        .get_device_collection(&Direction::Capture)
        .map_err(|err| err.to_string())?;
    let count = collection.get_nbr_devices().unwrap_or(0);
    for index in 0..count {
        let Ok(device) = collection.get_device_at_index(index) else {
            continue;
        };
        let Ok(name) = device.get_friendlyname() else {
            continue;
        };
        if !is_stereo_mix_name(&name) {
            return Ok((device, name));
        }
    }
    Err(
        "Windows default input is Stereo Mix, so the booth heard silence. Keep Stereo Mix enabled for Discord, but set Sound → Recording default back to your microphone."
            .into(),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RenderEndpointCandidate {
    pub id: String,
    pub name: String,
    pub interface_name: String,
}

/// Selects ordered non-default render endpoints for the wet (effected) signal so wet output
/// NEVER shares `default_render_id` (`GetDefaultAudioEndpoint(eRender, eConsole)`) with the
/// local dry monitor.
///
/// Priority:
/// 1. User-selected `discord_device_id` (virtual cable), if non-empty and `!= default_render_id`.
/// 2. Render endpoint on the same hardware audio adapter as `Stereo Mix`, if `!= default_render_id`
///    (preferring digital/secondary buses on that adapter).
/// 3. Any active virtual cable / virtual mic render endpoint (`!= default_render_id`).
pub(crate) fn select_wet_candidates(
    default_render_id: &str,
    discord_device_id: &str,
    stereo_mix_adapter_key: Option<&str>,
    render_endpoints: &[RenderEndpointCandidate],
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut push_unique = |id: &str, label: String| {
        if id != default_render_id && !out.iter().any(|(existing, _)| existing == id) {
            out.push((id.to_string(), label));
        }
    };

    if !discord_device_id.is_empty() && discord_device_id != default_render_id {
        if let Some(ep) = render_endpoints.iter().find(|e| e.id == discord_device_id) {
            push_unique(&ep.id, format!("{} [virtual cable]", ep.name));
        } else {
            push_unique(discord_device_id, "Discord virtual cable".to_string());
        }
    }

    if let Some(sm_key) = stereo_mix_adapter_key.filter(|k| !k.is_empty()) {
        let mut adapter_matches: Vec<&RenderEndpointCandidate> = render_endpoints
            .iter()
            .filter(|ep| {
                ep.id != default_render_id
                    && adapter_key(&ep.interface_name, &ep.name) == sm_key
            })
            .collect();
        adapter_matches.sort_by_key(|ep| {
            let lower = ep.name.to_ascii_lowercase();
            if lower.contains("digital") || lower.contains("2nd") {
                0usize
            } else {
                1usize
            }
        });
        for ep in adapter_matches {
            push_unique(&ep.id, format!("{} [Stereo Mix bus]", ep.name));
        }
    }

    for ep in render_endpoints {
        if ep.id != default_render_id && is_virtual_mic_render_name(&ep.name) {
            push_unique(&ep.id, format!("{} [virtual cable]", ep.name));
        }
    }

    out
}

fn discover_wet_candidates(
    enumerator: &DeviceEnumerator,
    discord_device_id: &str,
) -> Result<Vec<(String, String)>, String> {
    let default_render = enumerator
        .get_default_device(&Direction::Render)
        .map_err(|err| err.to_string())?;
    let default_render_id = default_render.get_id().map_err(|err| err.to_string())?;

    let mut stereo_mix_key: Option<String> = None;
    if let Ok(cap_coll) = enumerator.get_device_collection(&Direction::Capture) {
        let cap_count = cap_coll.get_nbr_devices().unwrap_or(0);
        for index in 0..cap_count {
            let Ok(dev) = cap_coll.get_device_at_index(index) else {
                continue;
            };
            let Ok(name) = dev.get_friendlyname() else {
                continue;
            };
            if is_stereo_mix_name(&name) {
                let iface = dev.get_interface_friendlyname().unwrap_or_default();
                let key = adapter_key(&iface, &name);
                if !key.is_empty() {
                    stereo_mix_key = Some(key);
                    break;
                }
            }
        }
    }

    let mut render_endpoints = Vec::new();
    if let Ok(ren_coll) = enumerator.get_device_collection(&Direction::Render) {
        let ren_count = ren_coll.get_nbr_devices().unwrap_or(0);
        for index in 0..ren_count {
            let Ok(dev) = ren_coll.get_device_at_index(index) else {
                continue;
            };
            let Ok(id) = dev.get_id() else {
                continue;
            };
            let Ok(name) = dev.get_friendlyname() else {
                continue;
            };
            let interface_name = dev.get_interface_friendlyname().unwrap_or_default();
            render_endpoints.push(RenderEndpointCandidate {
                id,
                name,
                interface_name,
            });
        }
    }

    Ok(select_wet_candidates(
        &default_render_id,
        discord_device_id,
        stereo_mix_key.as_deref(),
        &render_endpoints,
    ))
}

enum WetSink {
    Endpoint {
        out: LowLatencyEndpoint,
        render_client: IAudioRenderClient,
        label: String,
    },
    Headless,
}

fn run_wet_pipeline(
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    stop_signal: Arc<StopEvent>,
    discord_device_id: String,
    init_tx: mpsc::SyncSender<Result<WetStaticMetrics, String>>,
) -> Result<(), String> {
    let _mmcss = MmcssGuard::elevate_pro_audio();

    let opened = (|| -> Result<(LowLatencyEndpoint, IAudioCaptureClient, WetSink, String), String> {
        let wasapi_enum = DeviceEnumerator::new().map_err(|err| err.to_string())?;
        let (mic_dev, mic_name) = pick_capture_device(&wasapi_enum)?;
        let mic_id = mic_dev.get_id().map_err(|err| err.to_string())?;
        let candidates = discover_wet_candidates(&wasapi_enum, &discord_device_id)?;

        let com_enum: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                .map_err(|err| err.to_string())?;
        let wide_mic_id = HSTRING::from(&mic_id);
        let cap_device =
            unsafe { com_enum.GetDevice(&wide_mic_id) }.map_err(|err| err.to_string())?;
        let cap = open_low_latency_endpoint(&cap_device, "wet-capture", true)?;
        let capture_client: IAudioCaptureClient =
            unsafe { cap.client.GetService() }.map_err(|err| err.to_string())?;

        let mut sink = WetSink::Headless;
        for (cand_id, cand_label) in candidates {
            let wide_id = HSTRING::from(&cand_id);
            let Ok(out_dev) = (unsafe { com_enum.GetDevice(&wide_id) }) else {
                continue;
            };
            match open_low_latency_endpoint(&out_dev, "wet-render", false) {
                Ok(out) => match unsafe { out.client.GetService::<IAudioRenderClient>() } {
                    Ok(render_client) => {
                        sink = WetSink::Endpoint {
                            out,
                            render_client,
                            label: cand_label,
                        };
                        break;
                    }
                    Err(err) => {
                        eprintln!(
                            "[audio] wet-render GetService failed on {cand_label}: {err}"
                        );
                    }
                },
                Err(err) => {
                    eprintln!("[audio] wet-render open failed on {cand_label}: {err}");
                }
            }
        }

        Ok((cap, capture_client, sink, mic_name))
    })();

    let (cap, capture_client, sink, mic_name) = match opened {
        Ok(ready) => ready,
        Err(err) => {
            let _ = init_tx.send(Err(err.clone()));
            return Err(err);
        }
    };

    match sink {
        WetSink::Endpoint {
            out,
            render_client,
            label,
        } => run_wet_with_endpoint(
            shared,
            stop,
            stop_signal,
            cap,
            capture_client,
            out,
            render_client,
            label,
            mic_name,
            init_tx,
        ),
        WetSink::Headless => run_wet_headless(
            shared,
            stop,
            stop_signal,
            cap,
            capture_client,
            mic_name,
            init_tx,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_wet_with_endpoint(
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    stop_signal: Arc<StopEvent>,
    cap: LowLatencyEndpoint,
    capture_client: IAudioCaptureClient,
    out: LowLatencyEndpoint,
    render_client: IAudioRenderClient,
    target_label: String,
    mic_name: String,
    init_tx: mpsc::SyncSender<Result<WetStaticMetrics, String>>,
) -> Result<(), String> {
    let out_rate = out.format.sample_rate.max(1);
    let cap_in_out_frames = ((cap.period_frames as u64 * out_rate as u64)
        .div_ceil(cap.format.sample_rate.max(1) as u64)) as usize;
    let out_period = out.period_frames.max(1) as usize;
    let out_buffer = out.buffer_frames.max(1) as usize;
    let prime_frames = out_period.min(out_buffer);
    let max_ring_frames = (cap_in_out_frames + out_period * 2).max(out_period * 3);
    let trim_target_frames = cap_in_out_frames.max(out_period);
    let target_render_padding = out_period.min(out_buffer);

    let mut chain = VoiceChain::new(out_rate as f32);
    let mut ring = DryRing::new();
    let mut resampler = StreamingResampler::new(cap.format.sample_rate, out_rate);
    let mut wet_underruns = 0u64;
    let mut wet_overruns = 0u64;
    let mut last_raw = 0.0f32;
    let mut rms_acc = 0.0f32;
    let mut rms_n = 0usize;

    // Start capture first and keep one period in the ring as jitter cushion between async USB/cable clocks.
    if let Err(err) = unsafe { cap.client.Start() }.map_err(|e| e.to_string()) {
        let _ = init_tx.send(Err(err.clone()));
        return Err(err);
    }
    let pre_handles = [stop_signal.raw(), cap.event.raw()];
    let deadline = std::time::Instant::now() + Duration::from_millis(50);
    while ring.len() < cap_in_out_frames && std::time::Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let _ = unsafe { WaitForMultipleObjects(&pre_handles, false, 15) };
        wet_overruns = wet_overruns.saturating_add(drain_capture_packets(
            &capture_client,
            &cap.format,
            &mut resampler,
            &mut ring,
        ));
    }
    ring.trim_to(trim_target_frames);

    if let Err(err) = prime_silent(&render_client, prime_frames) {
        unsafe {
            let _ = cap.client.Stop();
        }
        let _ = init_tx.send(Err(err.clone()));
        return Err(err);
    }

    if let Err(err) = unsafe { out.client.Start() }.map_err(|e| e.to_string()) {
        unsafe {
            let _ = cap.client.Stop();
        }
        let _ = init_tx.send(Err(err.clone()));
        return Err(err);
    }

    let initial_frames = (cap_in_out_frames + ring.len() + prime_frames) as u64;
    let initial_latency_us = ((initial_frames * 1_000_000) / out_rate as u64) as u32;

    shared.exclusive.store(false, Ordering::Relaxed);
    shared.sample_rate.store(out_rate, Ordering::Relaxed);
    shared
        .latency_us
        .store(initial_latency_us, Ordering::Relaxed);
    shared
        .wet_max_ring_frames
        .store(max_ring_frames as u32, Ordering::Relaxed);
    shared
        .wet_ring_fill_frames
        .store(ring.len() as u32, Ordering::Relaxed);
    shared.wet_underruns.store(wet_underruns, Ordering::Relaxed);
    shared.wet_overruns.store(wet_overruns, Ordering::Relaxed);
    shared.output_ready.store(true, Ordering::Relaxed);

    let locked_note = if cap.periodicity_locked || out.periodicity_locked {
        " (period locked)"
    } else {
        ""
    };
    shared.set_status(format!(
        "wet -> {target_label} · {out_rate} Hz · ~{:.1} ms{locked_note} · mic {mic_name}",
        frames_to_ms(initial_frames as u32, out_rate)
    ));

    let static_metrics = WetStaticMetrics {
        target_name: target_label,
        rate: out_rate,
        period_frames: out.period_frames,
        min_period_frames: out.min_period_frames,
        buffer_frames: out.buffer_frames,
        low_period_api: out.low_period_api,
        periodicity_locked: cap.periodicity_locked || out.periodicity_locked,
    };
    let _ = init_tx.send(Ok(static_metrics));

    let wait_handles = [stop_signal.raw(), cap.event.raw(), out.event.raw()];

    // Real-time audio callback loop: zero allocations, locks, logging, or blocking operations.
    while !stop.load(Ordering::Relaxed) {
        let wait = unsafe { WaitForMultipleObjects(&wait_handles, false, 50) };
        if stop.load(Ordering::Relaxed) || wait == WAIT_OBJECT_0 {
            break;
        }

        let cap_overruns =
            drain_capture_packets(&capture_client, &cap.format, &mut resampler, &mut ring);
        if cap_overruns > 0 {
            wet_overruns = wet_overruns.saturating_add(cap_overruns);
            shared.wet_overruns.store(wet_overruns, Ordering::Relaxed);
        }
        if ring.len() > max_ring_frames {
            ring.trim_to(trim_target_frames);
            wet_overruns = wet_overruns.saturating_add(1);
            shared.wet_overruns.store(wet_overruns, Ordering::Relaxed);
        }

        let Ok(padding_u32) = (unsafe { out.client.GetCurrentPadding() }) else {
            break;
        };
        let padding = padding_u32 as usize;
        let available = out_buffer.saturating_sub(padding);
        let to_write = target_render_padding
            .saturating_sub(padding)
            .min(available);
        if to_write == 0 {
            shared
                .wet_ring_fill_frames
                .store(ring.len() as u32, Ordering::Relaxed);
            continue;
        }

        let prev_underruns = wet_underruns;
        if write_wet_frames(
            &render_client,
            to_write,
            &out.format,
            &mut ring,
            &mut chain,
            &shared,
            &mut last_raw,
            &mut wet_underruns,
            &mut rms_acc,
            &mut rms_n,
        )
        .is_err()
        {
            break;
        }
        if wet_underruns != prev_underruns {
            shared.wet_underruns.store(wet_underruns, Ordering::Relaxed);
        }

        let ring_fill = ring.len();
        shared
            .wet_ring_fill_frames
            .store(ring_fill as u32, Ordering::Relaxed);
        let post_padding = padding + to_write;
        let in_flight_frames = (cap_in_out_frames + ring_fill + post_padding) as u64;
        let latency_us = ((in_flight_frames * 1_000_000) / out_rate as u64) as u32;
        shared.latency_us.store(latency_us, Ordering::Relaxed);
    }

    shared.wet_ring_fill_frames.store(0, Ordering::Relaxed);
    unsafe {
        let _ = cap.client.Stop();
        let _ = out.client.Stop();
    }
    Ok(())
}

fn run_wet_headless(
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    stop_signal: Arc<StopEvent>,
    cap: LowLatencyEndpoint,
    capture_client: IAudioCaptureClient,
    mic_name: String,
    init_tx: mpsc::SyncSender<Result<WetStaticMetrics, String>>,
) -> Result<(), String> {
    let rate = cap.format.sample_rate.max(1);
    let period = cap.period_frames.max(1) as usize;
    let max_ring_frames = (period * 2).max(2);
    let trim_target_frames = period;

    let mut chain = VoiceChain::new(rate as f32);
    let mut ring = DryRing::new();
    let mut resampler = StreamingResampler::new(rate, rate);
    let mut wet_overruns = 0u64;
    let mut rms_acc = 0.0f32;
    let mut rms_n = 0usize;

    if let Err(err) = unsafe { cap.client.Start() }.map_err(|e| e.to_string()) {
        let _ = init_tx.send(Err(err.clone()));
        return Err(err);
    }

    let latency_us = ((period as u64 * 1_000_000) / rate as u64) as u32;
    shared.exclusive.store(false, Ordering::Relaxed);
    shared.sample_rate.store(rate, Ordering::Relaxed);
    shared.latency_us.store(latency_us, Ordering::Relaxed);
    shared
        .wet_max_ring_frames
        .store(max_ring_frames as u32, Ordering::Relaxed);
    shared.output_ready.store(true, Ordering::Relaxed);
    shared.set_status(format!(
        "wet -> headless (no separate cable/Stereo Mix bus) · {rate} Hz · mic {mic_name}"
    ));

    let static_metrics = WetStaticMetrics {
        target_name: "headless (no separate wet bus)".to_string(),
        rate,
        period_frames: cap.period_frames,
        min_period_frames: cap.min_period_frames,
        buffer_frames: cap.buffer_frames,
        low_period_api: cap.low_period_api,
        periodicity_locked: cap.periodicity_locked,
    };
    let _ = init_tx.send(Ok(static_metrics));

    let wait_handles = [stop_signal.raw(), cap.event.raw()];
    while !stop.load(Ordering::Relaxed) {
        let wait = unsafe { WaitForMultipleObjects(&wait_handles, false, 50) };
        if stop.load(Ordering::Relaxed) || wait == WAIT_OBJECT_0 {
            break;
        }

        let cap_overruns =
            drain_capture_packets(&capture_client, &cap.format, &mut resampler, &mut ring);
        if cap_overruns > 0 {
            wet_overruns = wet_overruns.saturating_add(cap_overruns);
            shared.wet_overruns.store(wet_overruns, Ordering::Relaxed);
        }
        if ring.len() > max_ring_frames {
            ring.trim_to(trim_target_frames);
            wet_overruns = wet_overruns.saturating_add(1);
            shared.wet_overruns.store(wet_overruns, Ordering::Relaxed);
        }
        shared
            .wet_ring_fill_frames
            .store(ring.len() as u32, Ordering::Relaxed);

        let params = shared.params();
        while let Some(raw) = ring.pop() {
            let (_voice, pitch) = chain.process(raw, &params);
            if shared.track_playing.load(Ordering::Relaxed) {
                if let Some(track) = shared.load_track() {
                    let idx = shared.track_frame.fetch_add(1, Ordering::Relaxed);
                    if idx >= track.frame_count(rate) {
                        shared.track_playing.store(false, Ordering::Relaxed);
                        shared.track_frame.store(0, Ordering::Relaxed);
                    }
                }
            }
            update_pitch_and_rms(&shared, raw, pitch.freq, pitch.target_midi, &mut rms_acc, &mut rms_n);
        }
    }

    shared.wet_ring_fill_frames.store(0, Ordering::Relaxed);
    unsafe {
        let _ = cap.client.Stop();
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_wet_frames(
    render: &IAudioRenderClient,
    frames: usize,
    format: &EndpointFormat,
    ring: &mut DryRing,
    chain: &mut VoiceChain,
    shared: &Shared,
    last_raw: &mut f32,
    underruns: &mut u64,
    rms_acc: &mut f32,
    rms_n: &mut usize,
) -> Result<(), String> {
    if frames == 0 {
        return Ok(());
    }
    let buf_ptr = unsafe { render.GetBuffer(frames as u32) }.map_err(|err| err.to_string())?;
    let byte_len = frames * format.block_align;
    let out_bytes = unsafe { slice::from_raw_parts_mut(buf_ptr, byte_len) };

    let params = shared.params();
    let track_gain = load_f32(&shared.track_gain);
    let mut playing = shared.track_playing.load(Ordering::Relaxed);
    let track_ref = shared.load_track();

    for frame in out_bytes.chunks_exact_mut(format.block_align) {
        let raw = pop_wet_sample(ring, last_raw, underruns);
        let (voice, pitch) = chain.process(raw, &params);
        let mut mixed = voice;
        if playing {
            if let Some(track) = track_ref {
                let idx = shared.track_frame.fetch_add(1, Ordering::Relaxed);
                if idx >= track.frame_count(format.sample_rate) {
                    shared.track_playing.store(false, Ordering::Relaxed);
                    shared.track_frame.store(0, Ordering::Relaxed);
                    playing = false;
                } else {
                    mixed += track.sample_at(idx, format.sample_rate) * track_gain;
                }
            }
        }
        encode_mono_to_frame(mixed, frame, format);
        update_pitch_and_rms(
            shared,
            raw,
            pitch.freq,
            pitch.target_midi,
            rms_acc,
            rms_n,
        );
    }

    unsafe { render.ReleaseBuffer(frames as u32, 0) }.map_err(|err| err.to_string())
}

#[inline]
pub(crate) fn pop_wet_sample(ring: &mut DryRing, last_raw: &mut f32, underruns: &mut u64) -> f32 {
    match ring.pop() {
        Some(sample) => {
            *last_raw = sample;
            sample
        }
        None => {
            *underruns = underruns.saturating_add(1);
            *last_raw *= 0.985;
            *last_raw
        }
    }
}

#[inline]
fn update_pitch_and_rms(
    shared: &Shared,
    raw: f32,
    freq: f32,
    target_midi: f32,
    rms_acc: &mut f32,
    rms_n: &mut usize,
) {
    *rms_acc += raw * raw;
    *rms_n += 1;
    if freq > 0.0 {
        shared.freq.store(freq.to_bits(), Ordering::Relaxed);
        shared
            .target_midi
            .store(target_midi.to_bits(), Ordering::Relaxed);
    } else if *rms_n > 64 {
        shared.freq.store(0, Ordering::Relaxed);
    }
    if *rms_n >= 256 {
        let rms = (*rms_acc / *rms_n as f32).sqrt();
        shared.rms.store(rms.to_bits(), Ordering::Relaxed);
        *rms_acc = 0.0;
        *rms_n = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        adapter_key, is_stereo_mix_name, is_virtual_cable_name, pop_wet_sample,
        select_wet_candidates, RenderEndpointCandidate,
    };
    use crate::dry_monitor::DryRing;

    #[test]
    fn stereo_mix_is_not_a_microphone() {
        assert!(is_stereo_mix_name("Stereo Mix (Realtek(R) Audio)"));
        assert!(is_stereo_mix_name("What U Hear"));
        assert!(is_stereo_mix_name("Wave Out Mix"));
        assert!(!is_stereo_mix_name("Headset Microphone (Realtek(R) Audio)"));
        assert!(!is_stereo_mix_name("USB PnP Sound Device"));
    }

    #[test]
    fn virtual_cable_names_stay_narrow() {
        assert!(is_virtual_cable_name("CABLE Input (VB-Audio Virtual Cable)"));
        assert!(!is_virtual_cable_name("Speakers (Realtek(R) Audio)"));
    }

    #[test]
    fn wet_candidates_never_include_default_render_and_prefer_cable_then_stereo_mix_bus() {
        let endpoints = vec![
            RenderEndpointCandidate {
                id: "dev-headphones".into(),
                name: "HISENSE (NVIDIA High Definition Audio)".into(),
                interface_name: "NVIDIA High Definition Audio".into(),
            },
            RenderEndpointCandidate {
                id: "dev-realtek-spdif".into(),
                name: "Realtek Digital Output (3- Realtek High Definition Audio)".into(),
                interface_name: "Realtek High Definition Audio".into(),
            },
            RenderEndpointCandidate {
                id: "dev-steam-mic".into(),
                name: "Speakers (Steam Streaming Microphone)".into(),
                interface_name: "Steam Streaming Microphone".into(),
            },
        ];

        // When listening on HISENSE and Stereo Mix is on Realtek, wet picks Realtek Digital Output first.
        let sm_key = adapter_key(
            "Realtek High Definition Audio",
            "Stereo Mix (3- Realtek High Definition Audio)",
        );
        let ordered = select_wet_candidates("dev-headphones", "", Some(&sm_key), &endpoints);
        assert_eq!(ordered[0].0, "dev-realtek-spdif");
        assert!(ordered.iter().all(|(id, _)| id != "dev-headphones"));

        // When default render IS Realtek Digital Output, Stereo Mix shares the default endpoint,
        // so wet splits onto the virtual mic cable and NEVER includes default_render_id.
        let split = select_wet_candidates("dev-realtek-spdif", "", Some(&sm_key), &endpoints);
        assert_eq!(split.len(), 1);
        assert_eq!(split[0].0, "dev-steam-mic");
        assert!(split.iter().all(|(id, _)| id != "dev-realtek-spdif"));
    }

    #[test]
    fn wet_underrun_counts_and_decays_smoothly_without_zero_step() {
        let mut ring = DryRing::new();
        ring.push(0.8);
        let mut last_raw = 0.0;
        let mut underruns = 0u64;

        let s0 = pop_wet_sample(&mut ring, &mut last_raw, &mut underruns);
        assert!((s0 - 0.8).abs() < 1e-6);
        assert_eq!(underruns, 0);

        let s1 = pop_wet_sample(&mut ring, &mut last_raw, &mut underruns);
        let s2 = pop_wet_sample(&mut ring, &mut last_raw, &mut underruns);
        assert_eq!(underruns, 2);
        assert!(s1 > 0.7 && s1 < 0.8, "expected decayed sample, got {s1}");
        assert!(s2 > 0.7 && s2 < s1, "expected monotonic decay, got {s2}");
    }
}
