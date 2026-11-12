use std::mem::size_of;
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use wasapi::{initialize_mta, DeviceEnumerator};
use windows::Win32::Foundation::{CloseHandle, FALSE, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    eConsole, eRender, AudioCategory_Media, AudioClientProperties, IAudioCaptureClient,
    IAudioClient, IAudioClient3, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_E_ENGINE_PERIODICITY_LOCKED,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMOPTIONS_NONE,
    AUDCLNT_STREAMOPTIONS_RAW, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventA, SetEvent,
    WaitForMultipleObjects,
};
use windows_core::{HSTRING, PCSTR};

use crate::engine::pick_capture_device;
use crate::state::Shared;

const DRY_RING_CAPACITY: usize = 4096;
const RAMP_STEP: f32 = 1.0 / 64.0;
const WAVE_FORMAT_PCM_TAG: u16 = 1;

#[derive(Clone, Debug)]
pub struct DryMonitorMetrics {
    pub mic_name: String,
    pub capture_rate: u32,
    pub capture_period_frames: u32,
    pub capture_min_period_frames: u32,
    pub capture_default_period_frames: u32,
    pub capture_buffer_frames: u32,
    pub capture_raw_mode: bool,
    pub capture_low_period_api: bool,
    pub render_rate: u32,
    pub render_period_frames: u32,
    pub render_min_period_frames: u32,
    pub render_default_period_frames: u32,
    pub render_buffer_frames: u32,
    pub render_low_period_api: bool,
    pub initial_latency_us: u32,
}

impl DryMonitorMetrics {
    pub fn capture_period_ms(&self) -> f32 {
        frames_to_ms(self.capture_period_frames, self.capture_rate)
    }

    pub fn render_period_ms(&self) -> f32 {
        frames_to_ms(self.render_period_frames, self.render_rate)
    }

    pub fn summary(&self) -> String {
        format!(
            "mic '{}' ({} Hz, period {}/{}..{} frames, buf {}, raw={}, iaudioclient3={}) -> render ({} Hz, period {}/{}..{} frames, buf {}, iaudioclient3={})",
            self.mic_name,
            self.capture_rate,
            self.capture_period_frames,
            self.capture_min_period_frames,
            self.capture_default_period_frames,
            self.capture_buffer_frames,
            self.capture_raw_mode,
            self.capture_low_period_api,
            self.render_rate,
            self.render_period_frames,
            self.render_min_period_frames,
            self.render_default_period_frames,
            self.render_buffer_frames,
            self.render_low_period_api,
        )
    }
}

fn frames_to_ms(frames: u32, rate: u32) -> f32 {
    if rate == 0 {
        0.0
    } else {
        frames as f32 * 1000.0 / rate as f32
    }
}

pub struct DryMonitor {
    stop: Arc<AtomicBool>,
    stop_signal: Arc<StopEvent>,
    thread: Option<JoinHandle<()>>,
    metrics: DryMonitorMetrics,
}

impl DryMonitor {
    pub fn start(shared: Arc<Shared>) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_signal = Arc::new(StopEvent::new()?);
        let (init_tx, init_rx) = mpsc::sync_channel::<Result<DryMonitorMetrics, String>>(1);

        let thread_stop = Arc::clone(&stop);
        let thread_signal = Arc::clone(&stop_signal);
        let thread_shared = Arc::clone(&shared);
        let handle = thread::Builder::new()
            .name("karaoke-dry".into())
            .spawn(move || {
                let _ = initialize_mta();
                let _ = run_dry_monitor(thread_shared, thread_stop, thread_signal, init_tx);
            })
            .map_err(|err| err.to_string())?;

        match init_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(metrics)) => {
                shared
                    .dry_latency_us
                    .store(metrics.initial_latency_us, Ordering::Relaxed);
                Ok(Self {
                    stop,
                    stop_signal,
                    thread: Some(handle),
                    metrics,
                })
            }
            Ok(Err(err)) => {
                stop.store(true, Ordering::Relaxed);
                stop_signal.signal();
                let _ = handle.join();
                Err(err)
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                stop_signal.signal();
                let _ = handle.join();
                Err("Dry monitor initialization timed out".into())
            }
        }
    }

    pub fn metrics(&self) -> &DryMonitorMetrics {
        &self.metrics
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.stop_signal.signal();
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

pub(crate) struct StopEvent(isize);

unsafe impl Send for StopEvent {}
unsafe impl Sync for StopEvent {}

impl StopEvent {
    fn new() -> Result<Self, String> {
        let handle = unsafe { CreateEventA(None, true, false, PCSTR::null()) }
            .map_err(|err| err.to_string())?;
        Ok(Self(handle.0 as isize))
    }

    fn raw(&self) -> HANDLE {
        HANDLE(self.0 as *mut core::ffi::c_void)
    }

    fn signal(&self) {
        unsafe {
            let _ = SetEvent(self.raw());
        }
    }
}

impl Drop for StopEvent {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.raw());
        }
    }
}

struct LocalEvent(HANDLE);

impl LocalEvent {
    fn new() -> Result<Self, String> {
        let handle = unsafe { CreateEventA(None, false, false, PCSTR::null()) }
            .map_err(|err| err.to_string())?;
        Ok(Self(handle))
    }

    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for LocalEvent {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct MmcssGuard(Option<HANDLE>);

impl MmcssGuard {
    fn elevate_pro_audio() -> Self {
        let mut task_index = 0u32;
        let task = HSTRING::from("Pro Audio");
        let handle = unsafe { AvSetMmThreadCharacteristicsW(&task, &mut task_index) }.ok();
        Self(handle)
    }
}

impl Drop for MmcssGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            unsafe {
                let _ = AvRevertMmThreadCharacteristics(handle);
            }
        }
    }
}

struct MixFormatGuard(*mut WAVEFORMATEX);

impl Drop for MixFormatGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CoTaskMemFree(Some(self.0.cast())) };
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SampleEncoding {
    Float32,
    Pcm16,
    Pcm24,
    Pcm32,
}

#[derive(Clone, Copy, Debug)]
struct EndpointFormat {
    sample_rate: u32,
    channels: usize,
    block_align: usize,
    encoding: SampleEncoding,
}

struct LowLatencyEndpoint {
    client: IAudioClient,
    event: LocalEvent,
    format: EndpointFormat,
    period_frames: u32,
    min_period_frames: u32,
    default_period_frames: u32,
    buffer_frames: u32,
    raw_mode: bool,
    low_period_api: bool,
}

/// Runs the dedicated low-latency dry monitoring loop on its own real-time MMCSS thread.
/// Reads unprocessed microphone packets directly from WASAPI capture and writes them
/// straight to the default render endpoint with a pre-allocated lock-free ring buffer.
pub(crate) fn run_dry_monitor(
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    stop_signal: Arc<StopEvent>,
    init_tx: mpsc::SyncSender<Result<DryMonitorMetrics, String>>,
) -> Result<(), String> {
    let _mmcss = MmcssGuard::elevate_pro_audio();

    let setup = (|| -> Result<(LowLatencyEndpoint, LowLatencyEndpoint, String), String> {
        let wasapi_enum = DeviceEnumerator::new().map_err(|err| err.to_string())?;
        let (mic_dev, mic_name) = pick_capture_device(&wasapi_enum)?;
        let mic_id = mic_dev.get_id().map_err(|err| err.to_string())?;

        let com_enum: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                .map_err(|err| err.to_string())?;
        let wide_mic_id = HSTRING::from(&mic_id);
        let cap_device =
            unsafe { com_enum.GetDevice(&wide_mic_id) }.map_err(|err| err.to_string())?;
        let out_device = unsafe { com_enum.GetDefaultAudioEndpoint(eRender, eConsole) }
            .map_err(|err| err.to_string())?;

        let cap = open_low_latency_endpoint(&cap_device, true)?;
        let out = open_low_latency_endpoint(&out_device, false)?;
        Ok((cap, out, mic_name))
    })();

    let (cap, out, mic_name) = match setup {
        Ok(ready) => ready,
        Err(err) => {
            let _ = init_tx.send(Err(err.clone()));
            return Err(err);
        }
    };

    let capture_client: IAudioCaptureClient =
        unsafe { cap.client.GetService() }.map_err(|err| {
            let msg = err.to_string();
            let _ = init_tx.send(Err(msg.clone()));
            msg
        })?;
    let render_client: IAudioRenderClient =
        unsafe { out.client.GetService() }.map_err(|err| {
            let msg = err.to_string();
            let _ = init_tx.send(Err(msg.clone()));
            msg
        })?;

    let cap_in_out_frames = ((cap.period_frames as u64 * out.format.sample_rate as u64)
        .div_ceil(cap.format.sample_rate.max(1) as u64)) as usize;
    let out_period = out.period_frames.max(1) as usize;
    let out_buffer = out.buffer_frames.max(1) as usize;
    let prime_frames = out_period.min(out_buffer);

    if let Err(err) = prime_silent(&render_client, prime_frames) {
        let _ = init_tx.send(Err(err.clone()));
        return Err(err);
    }

    if let Err(err) = unsafe { out.client.Start() }.map_err(|e| e.to_string()) {
        let _ = init_tx.send(Err(err.clone()));
        return Err(err);
    }
    if let Err(err) = unsafe { cap.client.Start() }.map_err(|e| e.to_string()) {
        unsafe {
            let _ = out.client.Stop();
        }
        let _ = init_tx.send(Err(err.clone()));
        return Err(err);
    }

    let initial_total_frames = (cap_in_out_frames + prime_frames) as u64;
    let initial_latency_us =
        ((initial_total_frames * 1_000_000) / out.format.sample_rate.max(1) as u64) as u32;

    let metrics = DryMonitorMetrics {
        mic_name,
        capture_rate: cap.format.sample_rate,
        capture_period_frames: cap.period_frames,
        capture_min_period_frames: cap.min_period_frames,
        capture_default_period_frames: cap.default_period_frames,
        capture_buffer_frames: cap.buffer_frames,
        capture_raw_mode: cap.raw_mode,
        capture_low_period_api: cap.low_period_api,
        render_rate: out.format.sample_rate,
        render_period_frames: out.period_frames,
        render_min_period_frames: out.min_period_frames,
        render_default_period_frames: out.default_period_frames,
        render_buffer_frames: out.buffer_frames,
        render_low_period_api: out.low_period_api,
        initial_latency_us,
    };
    let _ = init_tx.send(Ok(metrics));

    let mut ring = DryRing::new();
    let mut resampler = StreamingResampler::new(cap.format.sample_rate, out.format.sample_rate);
    let max_ring_frames = (cap_in_out_frames + out_period).max(out_period * 2);
    let trim_target_frames = cap_in_out_frames.max(out_period);
    let target_render_padding = out_period.min(out_buffer);

    let mut gain = 0.0f32;
    let mut last_sample = 0.0f32;
    let mut was_enabled = false;
    let wait_handles = [stop_signal.raw(), cap.event.raw(), out.event.raw()];

    while !stop.load(Ordering::Relaxed) {
        let wait = unsafe { WaitForMultipleObjects(&wait_handles, false, 50) };
        if stop.load(Ordering::Relaxed) || wait == WAIT_OBJECT_0 {
            break;
        }

        drain_capture_packets(&capture_client, &cap.format, &mut resampler, &mut ring);
        if ring.len() > max_ring_frames {
            ring.trim_to(trim_target_frames);
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
            continue;
        }

        let enabled = shared.dry_monitor.load(Ordering::Relaxed);
        if enabled && !was_enabled {
            ring.trim_to(out_period);
        }
        was_enabled = enabled;

        if !enabled && gain <= 1e-5 {
            gain = 0.0;
            last_sample = 0.0;
            ring.clear();
            let _ = prime_silent(&render_client, to_write);
            continue;
        }

        let target_gain = if enabled { 1.0 } else { 0.0 };
        if write_dry_frames(
            &render_client,
            to_write,
            &out.format,
            &mut ring,
            &mut gain,
            target_gain,
            &mut last_sample,
        )
        .is_err()
        {
            break;
        }

        let post_padding = padding + to_write;
        let in_flight_frames = (cap_in_out_frames + ring.len() + post_padding) as u64;
        let latency_us =
            ((in_flight_frames * 1_000_000) / out.format.sample_rate.max(1) as u64) as u32;
        shared.dry_latency_us.store(latency_us, Ordering::Relaxed);
    }

    shared.dry_latency_us.store(0, Ordering::Relaxed);
    unsafe {
        let _ = cap.client.Stop();
        let _ = out.client.Stop();
    }
    Ok(())
}

fn open_low_latency_endpoint(
    device: &IMMDevice,
    request_raw: bool,
) -> Result<LowLatencyEndpoint, String> {
    if let Ok(opened) = try_open_audioclient3(device, request_raw) {
        return Ok(opened);
    }
    open_fallback_audioclient(device)
}

fn try_open_audioclient3(
    device: &IMMDevice,
    request_raw: bool,
) -> Result<LowLatencyEndpoint, String> {
    let mut client3: IAudioClient3 =
        unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(|err| err.to_string())?;
    let mut raw_mode = false;

    if request_raw {
        let props = AudioClientProperties {
            cbSize: size_of::<AudioClientProperties>() as u32,
            bIsOffload: FALSE,
            eCategory: AudioCategory_Media,
            Options: AUDCLNT_STREAMOPTIONS_RAW,
        };
        if unsafe { client3.SetClientProperties(&props) }.is_ok() {
            raw_mode = true;
        } else {
            client3 =
                unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(|err| err.to_string())?;
            let fallback_props = AudioClientProperties {
                cbSize: size_of::<AudioClientProperties>() as u32,
                bIsOffload: FALSE,
                eCategory: AudioCategory_Media,
                Options: AUDCLNT_STREAMOPTIONS_NONE,
            };
            let _ = unsafe { client3.SetClientProperties(&fallback_props) };
        }
    }

    let fmt_ptr = unsafe { client3.GetMixFormat() }.map_err(|err| err.to_string())?;
    let _fmt_guard = MixFormatGuard(fmt_ptr);
    let format = unsafe { parse_waveformat(fmt_ptr) }?;

    let mut default_period = 0u32;
    let mut fundamental_period = 0u32;
    let mut min_period = 0u32;
    let mut max_period = 0u32;
    unsafe {
        client3.GetSharedModeEnginePeriod(
            fmt_ptr,
            &mut default_period,
            &mut fundamental_period,
            &mut min_period,
            &mut max_period,
        )
    }
    .map_err(|err| err.to_string())?;

    let desired_period = min_period.max(1);
    let init_res = unsafe {
        client3.InitializeSharedAudioStream(
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            desired_period,
            fmt_ptr,
            None,
        )
    };

    let mut chosen_period = desired_period;
    if let Err(err) = init_res {
        if err.code() == AUDCLNT_E_ENGINE_PERIODICITY_LOCKED {
            let fresh: IAudioClient3 =
                unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(|e| e.to_string())?;
            if raw_mode {
                let props = AudioClientProperties {
                    cbSize: size_of::<AudioClientProperties>() as u32,
                    bIsOffload: FALSE,
                    eCategory: AudioCategory_Media,
                    Options: AUDCLNT_STREAMOPTIONS_RAW,
                };
                let _ = unsafe { fresh.SetClientProperties(&props) };
            }
            let mut cur_fmt: *mut WAVEFORMATEX = core::ptr::null_mut();
            let mut cur_period = default_period.max(1);
            unsafe { fresh.GetCurrentSharedModeEnginePeriod(&mut cur_fmt, &mut cur_period) }
                .map_err(|e| e.to_string())?;
            let _cur_guard = MixFormatGuard(cur_fmt);
            let use_fmt = if cur_fmt.is_null() { fmt_ptr } else { cur_fmt };
            let parsed = unsafe { parse_waveformat(use_fmt) }?;
            unsafe {
                fresh.InitializeSharedAudioStream(
                    AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                    cur_period,
                    use_fmt,
                    None,
                )
            }
            .map_err(|e| e.to_string())?;
            let event = LocalEvent::new()?;
            unsafe { fresh.SetEventHandle(event.raw()) }.map_err(|e| e.to_string())?;
            let buffer_frames = unsafe { fresh.GetBufferSize() }.map_err(|e| e.to_string())?;
            let base: IAudioClient = fresh.into();
            return Ok(LowLatencyEndpoint {
                client: base,
                event,
                format: parsed,
                period_frames: cur_period,
                min_period_frames: min_period,
                default_period_frames: default_period,
                buffer_frames,
                raw_mode,
                low_period_api: true,
            });
        }
        return Err(err.to_string());
    }

    let mut cur_fmt: *mut WAVEFORMATEX = core::ptr::null_mut();
    let mut actual_period = 0u32;
    if unsafe { client3.GetCurrentSharedModeEnginePeriod(&mut cur_fmt, &mut actual_period) }.is_ok()
    {
        let _cur_guard = MixFormatGuard(cur_fmt);
        if actual_period > 0 {
            chosen_period = actual_period;
        }
    }

    let event = LocalEvent::new()?;
    unsafe { client3.SetEventHandle(event.raw()) }.map_err(|err| err.to_string())?;
    let buffer_frames = unsafe { client3.GetBufferSize() }.map_err(|err| err.to_string())?;
    let client: IAudioClient = client3.into();

    Ok(LowLatencyEndpoint {
        client,
        event,
        format,
        period_frames: chosen_period,
        min_period_frames: min_period,
        default_period_frames: default_period,
        buffer_frames,
        raw_mode,
        low_period_api: true,
    })
}

fn open_fallback_audioclient(device: &IMMDevice) -> Result<LowLatencyEndpoint, String> {
    let client: IAudioClient =
        unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(|err| err.to_string())?;
    let fmt_ptr = unsafe { client.GetMixFormat() }.map_err(|err| err.to_string())?;
    let _fmt_guard = MixFormatGuard(fmt_ptr);
    let format = unsafe { parse_waveformat(fmt_ptr) }?;

    let mut def_hns = 0i64;
    let mut min_hns = 0i64;
    unsafe { client.GetDevicePeriod(Some(&mut def_hns), Some(&mut min_hns)) }
        .map_err(|err| err.to_string())?;

    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            min_hns,
            0,
            fmt_ptr,
            None,
        )
    }
    .map_err(|err| err.to_string())?;

    let event = LocalEvent::new()?;
    unsafe { client.SetEventHandle(event.raw()) }.map_err(|err| err.to_string())?;
    let buffer_frames = unsafe { client.GetBufferSize() }.map_err(|err| err.to_string())?;

    let default_period_frames =
        ((def_hns.max(1) as u64 * format.sample_rate as u64) / 10_000_000).max(1) as u32;
    let min_period_frames =
        ((min_hns.max(1) as u64 * format.sample_rate as u64) / 10_000_000).max(1) as u32;

    Ok(LowLatencyEndpoint {
        client,
        event,
        format,
        period_frames: default_period_frames,
        min_period_frames,
        default_period_frames,
        buffer_frames,
        raw_mode: false,
        low_period_api: false,
    })
}

unsafe fn parse_waveformat(fmt_ptr: *const WAVEFORMATEX) -> Result<EndpointFormat, String> {
    if fmt_ptr.is_null() {
        return Err("WASAPI returned a null mix format".into());
    }
    let wf = unsafe { fmt_ptr.read_unaligned() };
    let channels = wf.nChannels as usize;
    let sample_rate = wf.nSamplesPerSec;
    let block_align = wf.nBlockAlign as usize;
    let bits = wf.wBitsPerSample;
    let tag = wf.wFormatTag as u32;

    if channels == 0 || sample_rate == 0 || block_align == 0 {
        return Err("Invalid endpoint mix format".into());
    }

    let is_float = if tag == WAVE_FORMAT_IEEE_FLOAT {
        true
    } else if tag == u32::from(WAVE_FORMAT_PCM_TAG) {
        false
    } else if tag == WAVE_FORMAT_EXTENSIBLE {
        let ext = unsafe { fmt_ptr.cast::<WAVEFORMATEXTENSIBLE>().read_unaligned() };
        let sub = ext.SubFormat;
        if sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
            true
        } else if sub == KSDATAFORMAT_SUBTYPE_PCM {
            false
        } else {
            return Err("Unsupported WAVEFORMATEXTENSIBLE subtype".into());
        }
    } else {
        return Err(format!("Unsupported WAVEFORMATEX tag {tag}"));
    };

    let encoding = match (is_float, bits) {
        (true, 32) => SampleEncoding::Float32,
        (false, 16) => SampleEncoding::Pcm16,
        (false, 24) => SampleEncoding::Pcm24,
        (false, 32) => SampleEncoding::Pcm32,
        _ => return Err(format!("Unsupported bit depth {bits} (float={is_float})")),
    };

    Ok(EndpointFormat {
        sample_rate,
        channels,
        block_align,
        encoding,
    })
}

fn prime_silent(render: &IAudioRenderClient, frames: usize) -> Result<(), String> {
    if frames == 0 {
        return Ok(());
    }
    unsafe {
        let _ = render
            .GetBuffer(frames as u32)
            .map_err(|err| err.to_string())?;
        render
            .ReleaseBuffer(frames as u32, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn drain_capture_packets(
    capture: &IAudioCaptureClient,
    format: &EndpointFormat,
    resampler: &mut StreamingResampler,
    ring: &mut DryRing,
) {
    loop {
        let Ok(packet_frames) = (unsafe { capture.GetNextPacketSize() }) else {
            break;
        };
        if packet_frames == 0 {
            break;
        }
        let mut buf_ptr: *mut u8 = core::ptr::null_mut();
        let mut frames_read = 0u32;
        let mut flags = 0u32;
        if unsafe {
            capture.GetBuffer(&mut buf_ptr, &mut frames_read, &mut flags, None, None)
        }
        .is_err()
        {
            break;
        }
        let count = frames_read as usize;
        let is_silent = (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || buf_ptr.is_null();
        if is_silent {
            for _ in 0..count {
                resampler.push(0.0, ring);
            }
        } else {
            let byte_len = count * format.block_align;
            let bytes = unsafe { slice::from_raw_parts(buf_ptr, byte_len) };
            for frame in bytes.chunks_exact(format.block_align) {
                let mono = decode_frame_to_mono(frame, format);
                resampler.push(mono, ring);
            }
        }
        let _ = unsafe { capture.ReleaseBuffer(frames_read) };
    }
}

fn write_dry_frames(
    render: &IAudioRenderClient,
    frames: usize,
    format: &EndpointFormat,
    ring: &mut DryRing,
    gain: &mut f32,
    target_gain: f32,
    last_sample: &mut f32,
) -> Result<(), String> {
    if frames == 0 {
        return Ok(());
    }
    let buf_ptr = unsafe { render.GetBuffer(frames as u32) }.map_err(|err| err.to_string())?;
    let byte_len = frames * format.block_align;
    let out_bytes = unsafe { slice::from_raw_parts_mut(buf_ptr, byte_len) };

    for frame in out_bytes.chunks_exact_mut(format.block_align) {
        if (*gain - target_gain).abs() <= RAMP_STEP {
            *gain = target_gain;
        } else if *gain < target_gain {
            *gain += RAMP_STEP;
        } else {
            *gain -= RAMP_STEP;
        }
        let raw_sample = match ring.pop() {
            Some(s) => {
                *last_sample = s;
                s
            }
            None => {
                *last_sample *= 0.985;
                *last_sample
            }
        };
        let sample = raw_sample * *gain;
        encode_mono_to_frame(sample, frame, format);
    }

    unsafe { render.ReleaseBuffer(frames as u32, 0) }.map_err(|err| err.to_string())
}

fn decode_frame_to_mono(frame: &[u8], format: &EndpointFormat) -> f32 {
    let ch_count = format.channels.max(1);
    let mut sum = 0.0f32;
    let mut read_channels = 0usize;
    let step = match format.encoding {
        SampleEncoding::Float32 | SampleEncoding::Pcm32 => 4,
        SampleEncoding::Pcm24 => 3,
        SampleEncoding::Pcm16 => 2,
    };

    for ch in 0..ch_count {
        let offset = ch * step;
        if offset + step > frame.len() {
            break;
        }
        let sample = match format.encoding {
            SampleEncoding::Float32 => f32::from_le_bytes([
                frame[offset],
                frame[offset + 1],
                frame[offset + 2],
                frame[offset + 3],
            ]),
            SampleEncoding::Pcm16 => {
                let v = i16::from_le_bytes([frame[offset], frame[offset + 1]]);
                f32::from(v) / 32768.0
            }
            SampleEncoding::Pcm24 => {
                let raw = ((frame[offset + 2] as i32) << 24)
                    | ((frame[offset + 1] as i32) << 16)
                    | ((frame[offset] as i32) << 8);
                (raw >> 8) as f32 / 8_388_608.0
            }
            SampleEncoding::Pcm32 => {
                let v = i32::from_le_bytes([
                    frame[offset],
                    frame[offset + 1],
                    frame[offset + 2],
                    frame[offset + 3],
                ]);
                v as f32 / 2_147_483_648.0
            }
        };
        sum += sample;
        read_channels += 1;
    }

    if read_channels == 0 {
        0.0
    } else {
        (sum / read_channels as f32).clamp(-1.0, 1.0)
    }
}

fn encode_mono_to_frame(sample: f32, frame: &mut [u8], format: &EndpointFormat) {
    let clamped = sample.clamp(-1.0, 1.0);
    let step = match format.encoding {
        SampleEncoding::Float32 | SampleEncoding::Pcm32 => 4,
        SampleEncoding::Pcm24 => 3,
        SampleEncoding::Pcm16 => 2,
    };
    let active_channels = format.channels.min(2);
    frame.fill(0);

    for ch in 0..active_channels {
        let offset = ch * step;
        if offset + step > frame.len() {
            break;
        }
        match format.encoding {
            SampleEncoding::Float32 => {
                frame[offset..offset + 4].copy_from_slice(&clamped.to_le_bytes());
            }
            SampleEncoding::Pcm16 => {
                let v = (clamped * 32767.0) as i16;
                frame[offset..offset + 2].copy_from_slice(&v.to_le_bytes());
            }
            SampleEncoding::Pcm24 => {
                let v = (clamped * 8_388_607.0) as i32;
                let bytes = v.to_le_bytes();
                frame[offset] = bytes[0];
                frame[offset + 1] = bytes[1];
                frame[offset + 2] = bytes[2];
            }
            SampleEncoding::Pcm32 => {
                let v = (f64::from(clamped) * 2_147_483_647.0) as i32;
                frame[offset..offset + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
    }
}

struct DryRing {
    buf: Box<[f32; DRY_RING_CAPACITY]>,
    read: usize,
    write: usize,
    len: usize,
}

impl DryRing {
    fn new() -> Self {
        Self {
            buf: Box::new([0.0; DRY_RING_CAPACITY]),
            read: 0,
            write: 0,
            len: 0,
        }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn clear(&mut self) {
        self.read = 0;
        self.write = 0;
        self.len = 0;
    }

    fn push(&mut self, sample: f32) {
        self.buf[self.write] = sample;
        self.write = (self.write + 1) & (DRY_RING_CAPACITY - 1);
        if self.len == DRY_RING_CAPACITY {
            self.read = (self.read + 1) & (DRY_RING_CAPACITY - 1);
        } else {
            self.len += 1;
        }
    }

    fn pop(&mut self) -> Option<f32> {
        if self.len == 0 {
            return None;
        }
        let sample = self.buf[self.read];
        self.read = (self.read + 1) & (DRY_RING_CAPACITY - 1);
        self.len -= 1;
        Some(sample)
    }

    fn trim_to(&mut self, keep_latest: usize) {
        if self.len > keep_latest {
            let drop_count = self.len - keep_latest;
            self.read = (self.read + drop_count) & (DRY_RING_CAPACITY - 1);
            self.len = keep_latest;
        }
    }
}

struct StreamingResampler {
    passthrough: bool,
    step_in_per_out: f64,
    pos: f64,
    prev: f32,
    initialized: bool,
}

impl StreamingResampler {
    fn new(in_rate: u32, out_rate: u32) -> Self {
        let in_r = in_rate.max(1);
        let out_r = out_rate.max(1);
        Self {
            passthrough: in_r == out_r,
            step_in_per_out: f64::from(in_r) / f64::from(out_r),
            pos: 0.0,
            prev: 0.0,
            initialized: false,
        }
    }

    fn push(&mut self, input: f32, ring: &mut DryRing) {
        if self.passthrough {
            ring.push(input);
            return;
        }
        if !self.initialized {
            self.prev = input;
            self.initialized = true;
            ring.push(input);
            self.pos = self.step_in_per_out;
            return;
        }
        while self.pos <= 1.0 {
            let frac = self.pos as f32;
            let interp = self.prev + (input - self.prev) * frac;
            ring.push(interp);
            self.pos += self.step_in_per_out;
        }
        self.pos -= 1.0;
        self.prev = input;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;

    #[test]
    fn dry_ring_wraps_and_trims_oldest_without_allocating() {
        let mut ring = DryRing::new();
        for i in 0..(DRY_RING_CAPACITY + 10) {
            ring.push(i as f32);
        }
        assert_eq!(ring.len(), DRY_RING_CAPACITY);
        assert_eq!(ring.pop(), Some(10.0));

        ring.trim_to(4);
        assert_eq!(ring.len(), 4);
        let tail: Vec<f32> = (0..4).filter_map(|_| ring.pop()).collect();
        let expected: Vec<f32> = ((DRY_RING_CAPACITY + 6)..(DRY_RING_CAPACITY + 10))
            .map(|v| v as f32)
            .collect();
        assert_eq!(tail, expected);
    }

    #[test]
    fn decode_and_encode_formats_preserve_signal() {
        for encoding in [
            SampleEncoding::Float32,
            SampleEncoding::Pcm16,
            SampleEncoding::Pcm24,
            SampleEncoding::Pcm32,
        ] {
            let step = match encoding {
                SampleEncoding::Float32 | SampleEncoding::Pcm32 => 4,
                SampleEncoding::Pcm24 => 3,
                SampleEncoding::Pcm16 => 2,
            };
            let fmt = EndpointFormat {
                sample_rate: 48_000,
                channels: 2,
                block_align: step * 2,
                encoding,
            };
            let mut frame = vec![0u8; fmt.block_align];
            encode_mono_to_frame(0.5, &mut frame, &fmt);
            let decoded = decode_frame_to_mono(&frame, &fmt);
            assert!(
                (decoded - 0.5).abs() < 1e-3,
                "encoding {encoding:?} decoded {decoded}"
            );
        }
    }

    #[test]
    fn streaming_resampler_converts_44100_to_48000() {
        let mut ring = DryRing::new();
        let mut resampler = StreamingResampler::new(44_100, 48_000);
        for i in 0..441 {
            let phase = 2.0 * std::f32::consts::PI * 440.0 * (i as f32 / 44_100.0);
            resampler.push(phase.sin(), &mut ring);
        }
        assert!(
            (ring.len() as isize - 480).abs() <= 1,
            "expected ~480 samples, got {}",
            ring.len()
        );
    }

    #[test]
    fn dry_monitor_coexists_with_engine_and_measures_latency() {
        let _ = initialize_mta();
        let Ok(enumerator) = DeviceEnumerator::new() else {
            return;
        };
        let Ok(out_wdev) = enumerator.get_default_device(&wasapi::Direction::Render) else {
            return;
        };
        let Ok((cap_wdev, _)) = pick_capture_device(&enumerator) else {
            return;
        };
        if let (Ok(cap_c), Ok(out_c)) = (cap_wdev.get_iaudioclient(), out_wdev.get_iaudioclient())
        {
            println!(
                "[LATENCY] driver_periods_hns: cap={:?} out={:?} out_name={:?}",
                cap_c.get_device_period().ok(),
                out_c.get_device_period().ok(),
                out_wdev.get_friendlyname().ok()
            );
        }

        let shared = Arc::new(Shared::new());
        shared.dry_monitor.store(true, Ordering::Relaxed);
        let engine = Engine::start(Arc::clone(&shared))
            .expect("Engine + DryMonitor should start concurrently without device conflict");

        let mut dry_samples_us = Vec::new();
        for step in 0..20 {
            if step == 10 {
                shared.dry_monitor.store(false, Ordering::Relaxed);
            } else if step == 14 {
                shared.dry_monitor.store(true, Ordering::Relaxed);
            }
            thread::sleep(Duration::from_millis(15));
            let us = shared.dry_latency_us.load(Ordering::Relaxed);
            if us > 0 && shared.dry_monitor.load(Ordering::Relaxed) {
                dry_samples_us.push(us);
            }
        }

        let wet_render_us = shared.latency_us.load(Ordering::Relaxed);
        let wet_rate = shared.sample_rate.load(Ordering::Relaxed);
        let metrics = engine.dry_monitor_metrics().cloned();
        println!(
            "[LATENCY] dry_metrics={metrics:?}\n[LATENCY] dry_monitor_samples_us={dry_samples_us:?} wet_render_us={wet_render_us} wet_rate={wet_rate} status={:?}",
            shared.status.lock().ok().map(|s| s.clone())
        );

        assert!(
            !dry_samples_us.is_empty(),
            "Dry monitor should publish non-zero live latency measurements when enabled"
        );
        engine.stop();
    }
}
