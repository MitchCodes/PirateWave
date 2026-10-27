//! CPAL callbacks only convert samples and move frames through bounded queues.
//! Device streams remain on the engine thread; no UI locks are taken by callbacks.
use anyhow::{anyhow, Context, Result};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    FromSample, Sample, SampleFormat, SizedSample,
};
use crossbeam_queue::ArrayQueue;
use piratewave_core::{Frame, SAMPLE_RATE};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

pub const QUEUE_CAPACITY: usize = 8192;
// The worker follows output callbacks in bursts, not one sample at a time.
// Keep ~43 ms on each side to cover 10–20 ms WASAPI packets plus worker jitter.
pub const TARGET_FRAMES: usize = 2048;
fn input_target(rate: u32) -> usize {
    (TARGET_FRAMES as u64 * rate as u64).div_ceil(SAMPLE_RATE as u64) as usize
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Devices {
    pub inputs: Vec<DeviceInfo>,
    pub outputs: Vec<DeviceInfo>,
}
fn enumerate(output: bool) -> Result<Vec<(DeviceInfo, cpal::Device)>> {
    let host = cpal::default_host();
    let devices = if output {
        host.output_devices()?
    } else {
        host.input_devices()?
    };
    let mut counts = std::collections::HashMap::<String, usize>::new();
    Ok(devices
        .filter_map(|device| {
            let name = device.name().ok()?;
            let index = counts.entry(name.clone()).or_default();
            *index += 1;
            let id = format!("{name}::{}", *index);
            Some((DeviceInfo { id, name }, device))
        })
        .collect())
}
pub fn devices() -> Result<Devices> {
    Ok(Devices {
        inputs: enumerate(false)?.into_iter().map(|(d, _)| d).collect(),
        outputs: enumerate(true)?.into_iter().map(|(d, _)| d).collect(),
    })
}
/// Device enumeration may synchronously probe every Windows audio driver. Never
/// do that repeatedly on the render worker while waiting for a missing endpoint.
#[derive(Clone, Debug)]
pub struct DeviceSnapshot {
    pub devices: Devices,
    pub default_output: String,
}
impl DeviceSnapshot {
    pub fn output_needs_reopen(&self, id: &Option<String>, previous_default: &str) -> bool {
        !self.available(id, true) || (id.is_none() && self.default_output != previous_default)
    }
    pub fn available(&self, id: &Option<String>, output: bool) -> bool {
        let devices = if output {
            &self.devices.outputs
        } else {
            &self.devices.inputs
        };
        match id {
            Some(id) => devices.iter().any(|d| d.id == *id),
            None => !devices.is_empty(),
        }
    }
}
pub struct DeviceWatcher {
    receiver: std::sync::mpsc::Receiver<DeviceSnapshot>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    worker: std::thread::JoinHandle<()>,
    snapshot: Option<DeviceSnapshot>,
    pub published_devices: Option<Arc<Devices>>,
}
impl DeviceWatcher {
    pub fn new() -> std::io::Result<Self> {
        Self::with_scan(|| {
            Ok(DeviceSnapshot {
                devices: devices()?,
                default_output: default_output_identity(),
            })
        })
    }
    fn with_scan(
        mut scan: impl FnMut() -> Result<DeviceSnapshot> + Send + 'static,
    ) -> std::io::Result<Self> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = std::thread::Builder::new()
            .name("piratewave-device-discovery".into())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    if let Ok(snapshot) = scan() {
                        if let Err(std::sync::mpsc::TrySendError::Disconnected(_)) =
                            sender.try_send(snapshot)
                        {
                            break;
                        }
                    }
                    if worker_stop.load(Ordering::Acquire) {
                        break;
                    }
                    std::thread::park_timeout(std::time::Duration::from_millis(500));
                }
            })?;
        Ok(Self {
            receiver,
            stop,
            worker,
            snapshot: None,
            published_devices: None,
        })
    }
    pub fn latest(&mut self) -> Option<&DeviceSnapshot> {
        while let Ok(snapshot) = self.receiver.try_recv() {
            if self.published_devices.as_deref() != Some(&snapshot.devices) {
                self.published_devices = Some(Arc::new(snapshot.devices.clone()));
            }
            self.snapshot = Some(snapshot);
        }
        self.snapshot.as_ref()
    }
}
impl Drop for DeviceWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.thread().unpark();
        // A slow driver may still be enumerating. Do not join it on the audio worker.
        // It owns no streams and exits when the in-flight enumeration completes.
    }
}

pub fn default_output_identity() -> String {
    cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
        .map(|name| format!("{name}::1"))
        .unwrap_or_else(|| "<system-default>".into())
}
fn find_device(id: &Option<String>, output: bool) -> Result<cpal::Device> {
    if let Some(id) = id {
        return enumerate(output)?
            .into_iter()
            .find(|(d, _)| &d.id == id)
            .map(|(_, d)| d)
            .ok_or_else(|| anyhow!("Audio device is unavailable: {id}"));
    }
    let host = cpal::default_host();
    if output {
        host.default_output_device()
    } else {
        host.default_input_device()
    }
    .ok_or_else(|| {
        anyhow!(
            "No default {} device",
            if output { "output" } else { "input" }
        )
    })
}
#[derive(Default)]
pub struct Counters {
    pub captured: AtomicU64,
    pub rendered: AtomicU64,
    pub underflows: AtomicU64,
    pub overflows: AtomicU64,
}
pub struct StreamHealth {
    ticks: Arc<AtomicU64>,
    errors: Arc<ArrayQueue<String>>,
    observed: u64,
    last_callback: std::time::Instant,
}
impl StreamHealth {
    fn new(errors: Arc<ArrayQueue<String>>) -> Self {
        Self {
            ticks: Arc::new(AtomicU64::new(0)),
            errors,
            observed: 0,
            last_callback: std::time::Instant::now(),
        }
    }
    #[cfg(test)]
    pub(crate) fn inject_error(&self, message: &str) {
        self.errors.push(message.into()).unwrap();
    }
    pub fn failure(&mut self) -> Option<String> {
        self.check(
            self.ticks.load(Ordering::Relaxed),
            std::time::Instant::now(),
        )
    }
    fn check(&mut self, ticks: u64, now: std::time::Instant) -> Option<String> {
        if ticks != self.observed {
            self.observed = ticks;
            self.last_callback = now;
        }
        self.errors.pop().or_else(|| {
            (now.duration_since(self.last_callback) > std::time::Duration::from_secs(2))
                .then(|| "Audio callbacks stalled".into())
        })
    }
}
pub struct InputStream {
    pub health: StreamHealth,
    pub _stream: cpal::Stream,
    pub resampler: QueueReader,
}
pub struct OutputStream {
    pub health: StreamHealth,
    pub _stream: cpal::Stream,
    pub queue: Arc<ArrayQueue<Frame>>,
    pub node_id: u64,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Dedicated optional workbench capture. It reuses the sample conversion callback
/// but deliberately has no mixer/resampler reserve before measurement.
pub struct ProbeInput {
    pub health: StreamHealth,
    pub queue: Arc<ArrayQueue<Frame>>,
    pub sample_rate: u32,
    _stream: cpal::Stream,
}
pub fn open_probe_input(
    id: &Option<String>,
    counters: Arc<Counters>,
    errors: Arc<ArrayQueue<String>>,
) -> Result<ProbeInput> {
    let device = find_device(id, false)?;
    let supported = device.default_input_config()?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    anyhow::ensure!(
        (8000..=384000).contains(&config.sample_rate.0),
        "Unsupported diagnostic input rate"
    );
    let queue = Arc::new(ArrayQueue::new(config.sample_rate.0 as usize * 2));
    let health = StreamHealth::new(errors.clone());
    macro_rules! build {
        ($t:ty) => {
            build_input::<$t>(
                &device,
                &config,
                queue.clone(),
                counters,
                errors,
                health.ticks.clone(),
            )
        };
    }
    let stream = match format {
        SampleFormat::F32 => build!(f32),
        SampleFormat::F64 => build!(f64),
        SampleFormat::I8 => build!(i8),
        SampleFormat::I16 => build!(i16),
        SampleFormat::I32 => build!(i32),
        SampleFormat::I64 => build!(i64),
        SampleFormat::U8 => build!(u8),
        SampleFormat::U16 => build!(u16),
        SampleFormat::U32 => build!(u32),
        SampleFormat::U64 => build!(u64),
        _ => Err(anyhow!("Unsupported diagnostic input format {format:?}")),
    }?;
    stream.play().context("Starting diagnostic input")?;
    Ok(ProbeInput {
        queue,
        health,
        sample_rate: config.sample_rate.0,
        _stream: stream,
    })
}

#[derive(Clone, Debug)]
pub struct EndpointReport {
    pub id: String,
    pub name: String,
    pub output: bool,
    pub default_name_match: bool,
    pub rate: u32,
    pub channels: u16,
    pub format: String,
    pub buffer: String,
    pub supported: Vec<String>,
    pub error: String,
}
/// Metadata only: no stream is opened and no device configuration is changed.
pub fn inspect_devices() -> Vec<EndpointReport> {
    let host = cpal::default_host();
    let mut result = Vec::new();
    for output in [false, true] {
        let default = if output {
            host.default_output_device()
        } else {
            host.default_input_device()
        }
        .and_then(|d| d.name().ok());
        let devices = match enumerate(output) {
            Ok(d) => d,
            Err(error) => {
                result.push(EndpointReport {
                    id: String::new(),
                    name: if output { "Outputs" } else { "Inputs" }.into(),
                    output,
                    default_name_match: false,
                    rate: 0,
                    channels: 0,
                    format: String::new(),
                    buffer: String::new(),
                    supported: vec![],
                    error: error.to_string(),
                });
                continue;
            }
        };
        for (info, device) in devices {
            let config = if output {
                device.default_output_config()
            } else {
                device.default_input_config()
            };
            let mut report = EndpointReport {
                default_name_match: default.as_ref() == Some(&info.name),
                id: info.id,
                name: info.name,
                output,
                rate: 0,
                channels: 0,
                format: String::new(),
                buffer: String::new(),
                supported: vec![],
                error: String::new(),
            };
            match config {
                Ok(c) => {
                    report.rate = c.sample_rate().0;
                    report.channels = c.channels();
                    report.format = format!("{:?}", c.sample_format());
                    report.buffer = match c.buffer_size() {
                        cpal::SupportedBufferSize::Range { min, max } => {
                            format!("{min}–{max} frames supported")
                        }
                        cpal::SupportedBufferSize::Unknown => {
                            "Driver-managed buffer; size not exposed".into()
                        }
                    };
                }
                Err(e) => report.error = e.to_string(),
            }
            let ranges = if output {
                device
                    .supported_output_configs()
                    .map(|i| i.collect::<Vec<_>>())
            } else {
                device
                    .supported_input_configs()
                    .map(|i| i.collect::<Vec<_>>())
            };
            if let Ok(ranges) = ranges {
                report.supported = ranges
                    .into_iter()
                    .take(64)
                    .map(|r| {
                        format!(
                            "{}–{} Hz · {} channels · {:?}",
                            r.min_sample_rate().0,
                            r.max_sample_rate().0,
                            r.channels(),
                            r.sample_format()
                        )
                    })
                    .collect();
            }
            result.push(report);
        }
    }
    result
}

/// Stateful linear sample-rate conversion plus slow occupancy-based clock correction.
/// Source and sink clocks are independent; the correction avoids unbounded drift.
pub struct QueueReader {
    queue: Arc<ArrayQueue<Frame>>,
    ratio: f64,
    phase: f64,
    a: Frame,
    b: Frame,
    primed: bool,
    target: usize,
    underruns: u64,
}
impl QueueReader {
    pub fn new(
        queue: Arc<ArrayQueue<Frame>>,
        input_rate: u32,
        output_rate: u32,
        target: usize,
    ) -> Self {
        Self {
            queue,
            ratio: input_rate as f64 / output_rate as f64,
            phase: 0.0,
            a: [0.0; 2],
            b: [0.0; 2],
            primed: false,
            target,
            underruns: 0,
        }
    }
    /// Actual starvation after playback began; initial buffering is not an underrun.
    pub fn underruns(&self) -> u64 {
        self.underruns
    }
    pub fn next_frame(&mut self) -> Option<Frame> {
        if !self.primed {
            if self.queue.len() < self.target.max(2) {
                return None;
            }
            self.a = self.queue.pop()?;
            self.b = self.queue.pop()?;
            self.phase = 0.0;
            self.primed = true;
        }
        let result = [
            self.a[0] + (self.b[0] - self.a[0]) * self.phase as f32,
            self.a[1] + (self.b[1] - self.a[1]) * self.phase as f32,
        ];
        let error = (self.queue.len() as f64 - self.target as f64) / self.target.max(1) as f64;
        self.phase += self.ratio * (1.0 + (error * 0.0005).clamp(-0.002, 0.002));
        while self.phase >= 1.0 {
            self.phase -= 1.0;
            self.a = self.b;
            match self.queue.pop() {
                Some(frame) => self.b = frame,
                None => {
                    self.underruns += 1;
                    self.primed = false;
                    return None;
                }
            }
        }
        Some(result)
    }
}
fn build_input<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    queue: Arc<ArrayQueue<Frame>>,
    counters: Arc<Counters>,
    errors: Arc<ArrayQueue<String>>,
    ticks: Arc<AtomicU64>,
) -> Result<cpal::Stream>
where
    T: SizedSample + Copy,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    Ok(device.build_input_stream(
        config,
        move |data: &[T], _| {
            ticks.fetch_add(1, Ordering::Relaxed);
            for frame in data.chunks_exact(channels) {
                let left = f32::from_sample(frame[0]);
                let right = if channels > 1 {
                    f32::from_sample(frame[1])
                } else {
                    left
                };
                let value = [
                    if left.is_finite() { left } else { 0.0 },
                    if right.is_finite() { right } else { 0.0 },
                ];
                if queue.push(value).is_err() {
                    counters.overflows.fetch_add(1, Ordering::Relaxed);
                }
            }
            counters
                .captured
                .fetch_add((data.len() / channels) as u64, Ordering::Relaxed);
        },
        move |error| {
            let _ = errors.push(format!("Input stream: {error}"));
        },
        None,
    )?)
}
fn build_output<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    queue: Arc<ArrayQueue<Frame>>,
    counters: Arc<Counters>,
    errors: Arc<ArrayQueue<String>>,
    ticks: Arc<AtomicU64>,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut reader = QueueReader::new(queue, SAMPLE_RATE, config.sample_rate.0, TARGET_FRAMES);
    Ok(device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            ticks.fetch_add(1, Ordering::Relaxed);
            let mut underflow = false;
            for frame in data.chunks_exact_mut(channels) {
                let value = reader.next_frame().unwrap_or_else(|| {
                    underflow = true;
                    [0.0; 2]
                });
                if channels == 1 {
                    frame[0] = T::from_sample((value[0] + value[1]) * 0.5);
                } else {
                    frame[0] = T::from_sample(value[0]);
                    frame[1] = T::from_sample(value[1]);
                    for sample in &mut frame[2..] {
                        *sample = T::from_sample(0.0);
                    }
                }
            }
            counters
                .rendered
                .fetch_add((data.len() / channels) as u64, Ordering::Relaxed);
            if underflow {
                counters.underflows.fetch_add(1, Ordering::Relaxed);
            }
        },
        move |error| {
            let _ = errors.push(format!("Output stream: {error}"));
        },
        None,
    )?)
}
pub fn open_input(
    id: &Option<String>,
    loopback: bool,
    counters: Arc<Counters>,
    errors: Arc<ArrayQueue<String>>,
) -> Result<InputStream> {
    #[cfg(not(windows))]
    if loopback {
        return Err(anyhow!(
            "Desktop loopback capture is currently Windows-only"
        ));
    }
    let device = find_device(id, loopback)?;
    let supported = if loopback {
        device.default_output_config()?
    } else {
        device.default_input_config()?
    };
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    // Preserve the same time reserve/capacity at 44.1, 48, 96 and 192 kHz.
    let queue = Arc::new(ArrayQueue::new(input_target(config.sample_rate.0) * 4));
    let health = StreamHealth::new(errors.clone());
    macro_rules! build {
        ($t:ty) => {
            build_input::<$t>(
                &device,
                &config,
                queue.clone(),
                counters,
                errors,
                health.ticks.clone(),
            )
        };
    }
    let stream = match format {
        SampleFormat::F32 => build!(f32),
        SampleFormat::I16 => build!(i16),
        SampleFormat::U16 => build!(u16),
        SampleFormat::I32 => build!(i32),
        SampleFormat::U32 => build!(u32),
        SampleFormat::F64 => build!(f64),
        SampleFormat::I8 => build!(i8),
        SampleFormat::U8 => build!(u8),
        SampleFormat::I64 => build!(i64),
        SampleFormat::U64 => build!(u64),
        _ => Err(anyhow!("Unsupported input format {format:?}")),
    }?;
    stream.play().context("Starting input stream")?;
    Ok(InputStream {
        health,
        _stream: stream,
        resampler: QueueReader::new(
            queue,
            config.sample_rate.0,
            SAMPLE_RATE,
            input_target(config.sample_rate.0),
        ),
    })
}
pub fn open_output(
    node_id: u64,
    id: &Option<String>,
    counters: Arc<Counters>,
    errors: Arc<ArrayQueue<String>>,
) -> Result<OutputStream> {
    let device = find_device(id, true)?;
    let supported = device.default_output_config()?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let queue = Arc::new(ArrayQueue::new(QUEUE_CAPACITY));
    for _ in 0..TARGET_FRAMES {
        let _ = queue.push([0.0; 2]);
    }
    let health = StreamHealth::new(errors.clone());
    macro_rules! build {
        ($t:ty) => {
            build_output::<$t>(
                &device,
                &config,
                queue.clone(),
                counters,
                errors,
                health.ticks.clone(),
            )
        };
    }
    let stream = match format {
        SampleFormat::F32 => build!(f32),
        SampleFormat::I16 => build!(i16),
        SampleFormat::U16 => build!(u16),
        SampleFormat::I32 => build!(i32),
        SampleFormat::U32 => build!(u32),
        SampleFormat::F64 => build!(f64),
        SampleFormat::I8 => build!(i8),
        SampleFormat::U8 => build!(u8),
        SampleFormat::I64 => build!(i64),
        SampleFormat::U64 => build!(u64),
        _ => Err(anyhow!("Unsupported output format {format:?}")),
    }?;
    stream.play().context("Starting output stream")?;
    Ok(OutputStream {
        health,
        _stream: stream,
        queue,
        node_id,
        sample_rate: config.sample_rate.0,
        channels: config.channels,
    })
}
#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    #[ignore = "native device discovery soak; queries installed Windows audio endpoints"]
    fn native_device_discovery_memory_soak() {
        #[repr(C)]
        #[derive(Default)]
        struct Memory {
            cb: u32,
            faults: u32,
            peak_ws: usize,
            ws: usize,
            peak_paged: usize,
            paged: usize,
            peak_nonpaged: usize,
            nonpaged: usize,
            pagefile: usize,
            peak_pagefile: usize,
            private: usize,
        }
        #[link(name = "psapi")]
        unsafe extern "system" {
            fn GetProcessMemoryInfo(
                process: *mut std::ffi::c_void,
                info: *mut Memory,
                size: u32,
            ) -> i32;
        }
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};
        let mut baseline = None;
        for i in 0..=200 {
            let _ = super::devices().unwrap();
            let _ = super::default_output_identity();
            if i % 25 == 0 {
                let mut m = Memory {
                    cb: std::mem::size_of::<Memory>() as u32,
                    ..Default::default()
                };
                let mut handles = 0;
                unsafe {
                    assert_ne!(
                        GetProcessMemoryInfo(
                            GetCurrentProcess(),
                            &mut m,
                            std::mem::size_of::<Memory>() as u32
                        ),
                        0
                    );
                    assert_ne!(GetProcessHandleCount(GetCurrentProcess(), &mut handles), 0);
                }
                println!(
                    "discovery iteration={i} private={} working_set={} handles={handles}",
                    m.private, m.ws
                );
                if i == 25 {
                    baseline = Some((m.private, handles));
                }
                if i == 200 {
                    let (private, original_handles) = baseline.unwrap();
                    assert!(
                        m.private <= private + 2 * 1024 * 1024,
                        "Device discovery retained more than 2 MiB after warmup: {} -> {}",
                        private,
                        m.private
                    );
                    assert!(
                        handles <= original_handles + 32,
                        "Device discovery retained handles: {original_handles} -> {handles}"
                    );
                }
            }
        }
    }
    use super::*;
    #[test]
    fn device_snapshot_checks_direction_and_does_not_retry_missing_endpoints() {
        let snapshot = DeviceSnapshot {
            devices: Devices {
                inputs: vec![DeviceInfo {
                    id: "mic::1".into(),
                    name: "mic".into(),
                }],
                outputs: vec![DeviceInfo {
                    id: "line::1".into(),
                    name: "line".into(),
                }],
            },
            default_output: "line::1".into(),
        };
        assert!(snapshot.available(&Some("mic::1".into()), false));
        assert!(!snapshot.available(&Some("mic::1".into()), true));
        assert!(snapshot.available(&Some("line::1".into()), true));
        assert!(!snapshot.available(&Some("missing-headphones::1".into()), true));
        assert!(!snapshot.available(&Some(String::new()), true));
        assert!(snapshot.available(&None, false));
    }
    #[test]
    fn output_recovery_preserves_explicit_selection_and_tracks_default_changes() {
        let selected = Some("Headphones::1".to_string());
        let mut snapshot = DeviceSnapshot {
            devices: Devices::default(),
            default_output: "Speakers::1".into(),
        };
        assert!(snapshot.output_needs_reopen(&selected, "Speakers::1"));
        assert_eq!(selected.as_deref(), Some("Headphones::1"));
        snapshot.devices.outputs.push(DeviceInfo {
            id: "Headphones::1".into(),
            name: "Headphones".into(),
        });
        assert!(!snapshot.output_needs_reopen(&selected, "Other::1"));
        assert!(snapshot.output_needs_reopen(&None, "Other::1"));
        assert!(!snapshot.output_needs_reopen(&None, "Speakers::1"));
        snapshot.devices.outputs.clear();
        assert!(snapshot.output_needs_reopen(&selected, "Speakers::1"));
    }
    #[test]
    fn stalled_device_discovery_does_not_block_poll_or_shutdown() {
        use std::sync::{atomic::AtomicBool, mpsc};
        let (started, began) = mpsc::channel();
        let (release, waiting) = mpsc::channel();
        let finished = Arc::new(AtomicBool::new(false));
        let worker_finished = finished.clone();
        let mut watcher = DeviceWatcher::with_scan(move || {
            let _ = started.send(());
            let _ = waiting.recv_timeout(std::time::Duration::from_secs(2));
            worker_finished.store(true, Ordering::Release);
            Ok(DeviceSnapshot {
                devices: Devices::default(),
                default_output: String::new(),
            })
        })
        .unwrap();
        began
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(watcher.latest().is_none());
        drop(watcher);
        assert!(
            !finished.load(Ordering::Acquire),
            "discovery blocked the audio worker"
        );
        let _ = release.send(());
    }
    #[test]
    fn endpoint_errors_are_isolated_and_missing_callbacks_are_detected() {
        let errors = Arc::new(ArrayQueue::new(4));
        let mut mic = StreamHealth::new(errors.clone());
        let mut output = StreamHealth::new(Arc::new(ArrayQueue::new(4)));
        let now = std::time::Instant::now();
        errors.push("Device unplugged".into()).unwrap();
        assert_eq!(mic.check(1, now).as_deref(), Some("Device unplugged"));
        assert!(output.check(1, now).is_none());
        assert!(output
            .check(2, now + std::time::Duration::from_secs(3))
            .is_none());
        assert_eq!(
            mic.check(1, now + std::time::Duration::from_secs(3))
                .as_deref(),
            Some("Audio callbacks stalled")
        );
        // A newly opened endpoint gets a fresh grace period and independent error queue.
        let mut reconnected = StreamHealth::new(Arc::new(ArrayQueue::new(4)));
        assert!(reconnected.failure().is_none());
        assert!(reconnected
            .check(1, now + std::time::Duration::from_secs(4))
            .is_none());
    }
    // A direct source -> bus route with independent capture/render clocks.
    // Windows delivers packets, and the soft-real-time worker wakes in bursts.
    // Constant nonzero audio makes even a one-frame inserted silence observable.
    fn simulate_route(
        rate: u32,
        drift: f64,
        input_reserve: usize,
        output_reserve: usize,
    ) -> (usize, usize) {
        let source = Arc::new(ArrayQueue::new(
            QUEUE_CAPACITY * rate as usize / SAMPLE_RATE as usize,
        ));
        let sink = Arc::new(ArrayQueue::new(QUEUE_CAPACITY));
        let mut input = QueueReader::new(source.clone(), rate, SAMPLE_RATE, input_reserve);
        let mut output = QueueReader::new(sink.clone(), SAMPLE_RATE, 48000, output_reserve);
        for _ in 0..output_reserve {
            sink.push([0.0; 2]).unwrap();
        }
        let mut capture_time = 0.0;
        let mut next_worker = 0;
        let mut wake = 0;
        let mut gaps = 0;
        let mut drops = 0;
        for ms in 0..30_000 {
            capture_time += 1.0 + drift;
            if capture_time >= 10.0 {
                capture_time -= 10.0;
                for _ in 0..rate / 100 {
                    drops += usize::from(source.push([0.25; 2]).is_err());
                }
            }
            if ms % 10 == 3 {
                for _ in 0..480 {
                    let frame = output.next_frame().unwrap_or([0.0; 2]);
                    if ms > 1000 && frame[0] < 0.24 {
                        gaps += 1;
                    }
                }
            }
            if ms >= next_worker {
                while sink.len() < output_reserve {
                    for _ in 0..piratewave_core::BLOCK_SIZE {
                        let frame = input.next_frame().unwrap_or([0.0; 2]);
                        drops += usize::from(sink.push(frame).is_err());
                    }
                }
                next_worker = ms + [1, 1, 15, 1, 7][wake % 5];
                wake += 1;
            }
        }
        (gaps, drops)
    }
    #[test]
    fn direct_route_stays_continuous_with_packet_delivery_and_worker_jitter() {
        for (rate, drift) in [
            (48000, 0.0),
            (44100, 0.0003),
            (96000, -0.0003),
            (192000, 0.0003),
        ] {
            assert_eq!(
                simulate_route(rate, drift, input_target(rate), TARGET_FRAMES),
                (0, 0),
                "rate {rate}, clock drift {drift}"
            );
        }
    }
    #[test]
    fn regression_fixture_detects_the_previous_short_reserve() {
        assert!(simulate_route(48000, 0.0, 256, 1024).0 > 0);
    }
    #[test]
    fn reader_silences_underflow_and_recovers() {
        let queue = Arc::new(ArrayQueue::new(100));
        let mut reader = QueueReader::new(queue.clone(), 48000, 48000, 2);
        assert!(reader.next_frame().is_none());
        assert_eq!(reader.underruns(), 0);
        for _ in 0..10 {
            queue.push([0.25; 2]).unwrap();
        }
        assert_eq!(reader.next_frame(), Some([0.25; 2]));
        for _ in 0..30 {
            reader.next_frame();
        }
        assert!(reader.next_frame().is_none());
        assert_eq!(reader.underruns(), 1);
        for _ in 0..5 {
            queue.push([0.5; 2]).unwrap();
        }
        assert_eq!(reader.next_frame(), Some([0.5; 2]));
    }
    #[test]
    fn resamples_without_non_finite_values() {
        let queue = Arc::new(ArrayQueue::new(1000));
        for i in 0..900 {
            queue.push([(i as f32 * 0.1).sin(); 2]).unwrap();
        }
        let mut reader = QueueReader::new(queue, 44100, 48000, 2);
        for _ in 0..800 {
            let frame = reader.next_frame().unwrap();
            assert!(frame[0].is_finite() && frame[0].abs() <= 1.0);
        }
    }
}
