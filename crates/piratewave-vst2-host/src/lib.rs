//! VST 2.4 host adapter. Instances stay on the engine's owning thread.
#![allow(deprecated)] // VST2 compatibility is intentionally confined to this adapter.
use anyhow::{Context, Result};
use piratewave_core::{
    plugins::{PluginDescriptor, PluginFormat, PluginParameter, PluginRole},
    Frame, BLOCK_SIZE, SAMPLE_RATE,
};
use std::path::Path;
#[cfg(windows)]
mod editor_worker;
pub mod pe;
pub mod protocol;
pub type StateChunk = (Vec<u8>, bool, Option<u32>);
#[cfg(windows)]
pub mod window;
pub trait Hosted {
    fn descriptor(&self) -> &PluginDescriptor;
    fn set_parameter(&mut self, id: u32, value: f64);
    fn note(&mut self, note: u8, velocity: f32, on: bool);
    fn process(&mut self, frames: &mut [Frame]) -> Result<()>;
}
#[cfg(windows)]
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    OnceLock,
};
use std::sync::{Arc, Mutex};
use vst::{
    buffer::SendEventBuffer,
    event::MidiEvent,
    host::{Host, HostBuffer, PluginInstance, PluginLoader},
    plugin::{Category, Plugin, PluginParameters},
};
struct PirateWaveHost {
    #[cfg(windows)]
    resize: Arc<Mutex<Option<(i32, i32)>>>,
    #[cfg(windows)]
    edits: Arc<crossbeam_queue::ArrayQueue<(u32, f64, u64)>>,
    #[cfg(windows)]
    revisions: Arc<ParameterRevisions>,
}
#[cfg(windows)]
#[derive(Default)]
struct ParameterRevisions {
    sequence: AtomicU64,
    slots: OnceLock<Vec<ParameterRevision>>,
}
#[cfg(windows)]
struct ParameterRevision {
    latest: AtomicU64,
    host_pending: AtomicBool,
    vendor_value: AtomicU64,
}
#[cfg(windows)]
impl ParameterRevisions {
    fn initialize(&self, count: usize) {
        let _ = self.slots.set(
            (0..count)
                .map(|_| ParameterRevision {
                    latest: AtomicU64::new(0),
                    host_pending: AtomicBool::new(false),
                    vendor_value: AtomicU64::new(f64::NAN.to_bits()),
                })
                .collect(),
        );
    }
    fn slot(&self, id: u32) -> Option<&ParameterRevision> {
        self.slots.get().and_then(|slots| slots.get(id as usize))
    }
    fn mark(&self, id: u32) -> Option<u64> {
        let slot = self.slot(id)?;
        let revision = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        slot.latest.fetch_max(revision, Ordering::AcqRel);
        Some(revision)
    }
    fn begin_host(&self, id: u32) -> Option<u64> {
        if let Some(slot) = self.slot(id) {
            slot.host_pending.store(true, Ordering::Release);
            self.mark(id)
        } else {
            None
        }
    }
    fn mark_vendor(&self, id: u32, value: f64) -> Option<u64> {
        let slot = self.slot(id)?;
        if slot.vendor_value.swap(value.to_bits(), Ordering::AcqRel) == value.to_bits() {
            return None;
        }
        self.mark(id)
    }
    fn retry_vendor(&self, id: u32, value: f64) {
        if let Some(slot) = self.slot(id) {
            let _ = slot.vendor_value.compare_exchange(
                value.to_bits(),
                f64::NAN.to_bits(),
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
    }
    fn end_host(&self, id: u32, sequence: Option<u64>, value: f64) {
        if let Some(slot) = self.slot(id) {
            if sequence.is_some_and(|sequence| slot.latest.load(Ordering::Acquire) == sequence) {
                slot.vendor_value.store(value.to_bits(), Ordering::Release);
            }
            slot.host_pending.store(false, Ordering::Release);
        }
    }
    fn observe(&self, id: u32) -> Option<u64> {
        let slot = self.slot(id)?;
        if slot.host_pending.load(Ordering::Acquire) {
            return None;
        }
        Some(slot.latest.load(Ordering::Acquire))
    }
    fn mark_polled(&self, id: u32, previous: u64) -> Option<u64> {
        let slot = self.slot(id)?;
        if slot.host_pending.load(Ordering::Acquire) {
            return None;
        }
        let revision = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        slot.latest
            .compare_exchange(previous, revision, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        if slot.host_pending.load(Ordering::Acquire) {
            return None;
        }
        Some(revision)
    }
    fn current(&self, id: u32, revision: u64) -> bool {
        self.slot(id)
            .is_some_and(|slot| slot.latest.load(Ordering::Acquire) == revision)
    }
}
impl Host for PirateWaveHost {
    #[cfg(windows)]
    fn automate(&self, index: i32, value: f32) {
        if index >= 0 && value.is_finite() && (0.0..=1.0).contains(&value) {
            if let Some(revision) = self.revisions.mark_vendor(index as u32, value as f64) {
                if self
                    .edits
                    .push((index as u32, value as f64, revision))
                    .is_err()
                {
                    self.revisions.retry_vendor(index as u32, value as f64);
                }
            }
        }
    }
    #[cfg(windows)]
    fn size_window(&self, width: i32, height: i32) -> bool {
        if !(1..=8192).contains(&width) || !(1..=8192).contains(&height) {
            return false;
        }
        if let Ok(mut request) = self.resize.lock() {
            *request = Some((width, height));
            true
        } else {
            false
        }
    }
    fn get_info(&self) -> (isize, String, String) {
        (1, "PirateWave".into(), "PirateWave".into())
    }
    fn get_block_size(&self) -> isize {
        BLOCK_SIZE as isize
    }
}
pub struct Vst2 {
    instance: Option<PluginInstance>,
    parameters: Arc<dyn PluginParameters>,
    buffer: HostBuffer<f32>,
    input: Vec<[f32; BLOCK_SIZE]>,
    output: Vec<[f32; BLOCK_SIZE]>,
    events: SendEventBuffer,
    pending: Vec<MidiEvent>,
    descriptor: PluginDescriptor,
    preset_chunks: bool,
    program_count: i32,
    #[cfg(windows)]
    editor: Option<editor_worker::Worker>,
    #[cfg(windows)]
    editor_values: Arc<Mutex<std::collections::BTreeMap<u32, f64>>>,
    #[cfg(windows)]
    editor_edits: Arc<crossbeam_queue::ArrayQueue<(u32, f64, u64)>>,
    #[cfg(windows)]
    delivered_values: std::collections::BTreeMap<u32, f64>,
    #[cfg(windows)]
    revisions: Arc<ParameterRevisions>,
    #[cfg(windows)]
    editor_used: bool,
    #[cfg(windows)]
    resize: Arc<Mutex<Option<(i32, i32)>>>,
    #[cfg(windows)]
    pending_capture: Option<std::sync::mpsc::Receiver<Result<Option<StateChunk>, String>>>,
    #[cfg(windows)]
    capture_started: Option<std::time::Instant>,
    #[cfg(windows)]
    editor_started: Option<std::time::Instant>,
    #[cfg(windows)]
    editor_error: Option<String>,
}
impl Vst2 {
    /// Control-plane text readback for compatibility characterization.
    pub fn parameter_text(&self, id: u32) -> String {
        self.parameters.get_parameter_text(id as i32)
    }
    pub fn load(path: &Path) -> Result<Self> {
        // Preserve the OS loader's reason instead of the vst crate's generic InvalidPath.
        let _library=unsafe{libloading::Library::new(path)}.with_context(||format!("Windows could not load VST2 {} ({}-bit host). A required runtime/DLL may be missing",path.display(),usize::BITS))?;
        #[cfg(windows)]
        let resize = Arc::new(Mutex::new(None));
        #[cfg(windows)]
        let edits = Arc::new(crossbeam_queue::ArrayQueue::new(4096));
        #[cfg(windows)]
        let revisions = Arc::new(ParameterRevisions::default());
        let host = PirateWaveHost {
            #[cfg(windows)]
            resize: resize.clone(),
            #[cfg(windows)]
            edits: edits.clone(),
            #[cfg(windows)]
            revisions: revisions.clone(),
        };
        let mut loader = PluginLoader::load(path, Arc::new(Mutex::new(host)))?;
        let mut instance = loader.instance()?;
        instance.init();
        instance.set_sample_rate(SAMPLE_RATE as f32);
        instance.set_block_size(BLOCK_SIZE as i64);
        let info = instance.get_info();
        anyhow::ensure!(
            !matches!(info.category, Category::Shell),
            "VST2 shell plugins are unsupported; use their individual VST3 versions"
        );
        anyhow::ensure!(
            (0..=2).contains(&info.inputs) && (1..=2).contains(&info.outputs),
            "Only mono/stereo VST2 plugins are supported ({} inputs, {} outputs)",
            info.inputs,
            info.outputs
        );
        anyhow::ensure!(
            (0..=4096).contains(&info.parameters),
            "Too many plugin parameters"
        );
        #[cfg(windows)]
        revisions.initialize(info.parameters as usize);
        let parameters = instance.get_parameter_object();
        let descriptor = PluginDescriptor {
            path: path.to_string_lossy().into(),
            class_id: info.unique_id.to_string(),
            format: PluginFormat::Vst2,
            role: if matches!(info.category, Category::Synth) {
                PluginRole::Instrument
            } else {
                PluginRole::Effect
            },
            name: info.name,
            vendor: info.vendor,
            parameters: (0..info.parameters)
                .map(|i| PluginParameter {
                    id: i as u32,
                    name: parameters.get_parameter_name(i),
                    unit: parameters.get_parameter_label(i),
                    default: (parameters.get_parameter(i) as f64).clamp(0.0, 1.0),
                    steps: 0,
                })
                .collect(),
        };
        descriptor.validate().map_err(anyhow::Error::msg)?;
        #[cfg(windows)]
        let delivered_values = descriptor
            .parameters
            .iter()
            .map(|parameter| (parameter.id, parameter.default))
            .collect();
        instance.resume();
        instance.start_process();
        Ok(Self {
            instance: Some(instance),
            parameters,
            buffer: HostBuffer::new(info.inputs as usize, info.outputs as usize),
            input: vec![[0.0; BLOCK_SIZE]; info.inputs as usize],
            output: vec![[0.0; BLOCK_SIZE]; info.outputs as usize],
            events: SendEventBuffer::new(256),
            pending: Vec::with_capacity(256),
            descriptor,
            preset_chunks: info.preset_chunks,
            program_count: info.presets,
            #[cfg(windows)]
            editor: None,
            #[cfg(windows)]
            editor_values: Arc::new(Mutex::new(Default::default())),
            #[cfg(windows)]
            editor_edits: edits,
            #[cfg(windows)]
            delivered_values,
            #[cfg(windows)]
            revisions,
            #[cfg(windows)]
            editor_used: false,
            #[cfg(windows)]
            resize,
            #[cfg(windows)]
            pending_capture: None,
            #[cfg(windows)]
            capture_started: None,
            #[cfg(windows)]
            editor_started: None,
            #[cfg(windows)]
            editor_error: None,
        })
    }
    #[cfg(windows)]
    fn ensure_editor_worker(&mut self) -> Result<()> {
        if self.editor.is_none() {
            let instance = self.instance.as_mut().unwrap();
            let state_handle = instance.state_handle();
            let editor = instance.take_editor_for_ui_thread();
            self.editor = Some(editor_worker::Worker::start(
                editor,
                self.descriptor.name.clone(),
                self.resize.clone(),
                state_handle,
                self.descriptor.parameters.iter().map(|p| p.id).collect(),
                self.editor_values.clone(),
                self.editor_edits.clone(),
                self.revisions.clone(),
                self.program_count,
            )?);
        }
        Ok(())
    }
    #[cfg(windows)]
    pub fn open_editor(&mut self) -> Result<()> {
        self.ensure_editor_worker()?;
        if !self.editor_used {
            while self.editor_edits.pop().is_some() {}
        }
        self.editor_used = true;
        self.editor.as_ref().unwrap().open()?;
        self.editor_started = Some(std::time::Instant::now());
        Ok(())
    }
    #[cfg(windows)]
    pub fn editor_idle(&mut self) {
        if let Some(editor) = &self.editor {
            if editor.size().0 > 0 || !editor.is_open() {
                self.editor_started = None;
            } else if self
                .editor_started
                .is_some_and(|start| start.elapsed() >= std::time::Duration::from_secs(5))
            {
                editor.mark_open_timed_out();
                self.editor_started = None;
                self.editor_error = Some("Plugin editor did not open within five seconds".into());
            }
        }
    }
    #[cfg(windows)]
    pub fn take_editor_error(&mut self) -> Option<String> {
        self.editor_error
            .take()
            .or_else(|| self.editor.as_ref().and_then(|worker| worker.take_error()))
    }
    #[cfg(windows)]
    pub fn invalidate_capture(&mut self) {
        self.pending_capture = None;
        self.capture_started = None;
    }
    #[cfg(windows)]
    pub fn close_editor(&mut self) {
        self.editor_started = None;
        self.pending_capture = None;
        self.capture_started = None;
        if let Some(editor) = &self.editor {
            editor.close();
        }
    }
    #[cfg(windows)]
    pub fn editor_open(&self) -> bool {
        self.editor.as_ref().is_some_and(|editor| editor.is_open())
    }
    #[cfg(windows)]
    pub fn editor_size(&self) -> (i32, i32) {
        self.editor.as_ref().map_or((0, 0), |worker| worker.size())
    }
    #[cfg(all(windows, debug_assertions))]
    pub fn test_editor_key(&self, key: u32) -> Result<()> {
        self.editor
            .as_ref()
            .context("Editor worker is not running")?
            .test_key(key)
    }
    #[cfg(all(windows, debug_assertions))]
    pub fn test_editor_timeout(&mut self) -> Result<()> {
        self.editor
            .as_ref()
            .context("Editor worker is not running")?
            .simulate_slow_open();
        self.editor_started =
            std::time::Instant::now().checked_sub(std::time::Duration::from_secs(6));
        self.editor_idle();
        Ok(())
    }
    #[cfg(windows)]
    pub fn take_editor_edits(&mut self) -> Vec<(u32, f64)> {
        let mut edits = Vec::new();
        while let Some((id, value, revision)) = self.editor_edits.pop() {
            if self.editor_used
                && self.revisions.current(id, revision)
                && self.delivered_values.insert(id, value) != Some(value)
            {
                edits.push((id, value));
            }
        }
        if !edits.is_empty() {
            self.pending_capture = None;
            self.capture_started = None;
        }
        edits
    }
    #[cfg(not(windows))]
    pub fn capture_state(&mut self) -> Result<Option<StateChunk>> {
        if !self.preset_chunks {
            return Ok(None);
        }
        let instance = self.instance.as_ref().unwrap();
        if let Some(data) = instance
            .read_state_chunk(true, 64 * 1024 * 1024)
            .map_err(anyhow::Error::msg)?
        {
            let current = self.parameters.get_preset_num();
            let program = (0..self.program_count)
                .contains(&current)
                .then_some(current as u32);
            return Ok(Some((data, true, program)));
        }
        let current = self.parameters.get_preset_num();
        let program = (0..self.program_count)
            .contains(&current)
            .then_some(current as u32);
        Ok(instance
            .read_state_chunk(false, 64 * 1024 * 1024)
            .map_err(anyhow::Error::msg)?
            .map(|data| (data, false, program)))
    }
    #[cfg(windows)]
    fn request_capture(&mut self) -> Result<()> {
        if self.pending_capture.is_none() {
            self.ensure_editor_worker()?;
            self.pending_capture = Some(self.editor.as_ref().unwrap().request_capture()?);
            self.capture_started = Some(std::time::Instant::now());
        }
        Ok(())
    }
    #[cfg(windows)]
    pub fn capture_state(&mut self) -> Result<Option<StateChunk>> {
        if !self.preset_chunks {
            return Ok(None);
        }
        self.request_capture()?;
        match self.pending_capture.as_ref().unwrap().try_recv() {
            Ok(result) => {
                self.pending_capture = None;
                self.capture_started = None;
                result.map_err(anyhow::Error::msg)
            }
            Err(std::sync::mpsc::TryRecvError::Empty)
                if self
                    .capture_started
                    .is_some_and(|start| start.elapsed() >= std::time::Duration::from_secs(5)) =>
            {
                self.pending_capture = None;
                self.capture_started = None;
                anyhow::bail!("VST2 state capture timed out")
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                anyhow::bail!("VST2 state capture is pending")
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending_capture = None;
                self.capture_started = None;
                anyhow::bail!("VST2 state capture failed because the control thread stopped")
            }
        }
    }
    #[cfg(windows)]
    pub fn capture_state_for_shutdown(&mut self) -> Result<Option<StateChunk>> {
        if !self.preset_chunks {
            return Ok(None);
        }
        self.pending_capture = None;
        self.capture_started = None;
        self.request_capture()?;
        let receiver = self.pending_capture.take().unwrap();
        self.capture_started = None;
        let started = std::time::Instant::now();
        loop {
            match receiver.recv_timeout(std::time::Duration::from_millis(10)) {
                Ok(result) => return result.map_err(anyhow::Error::msg),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("VST2 control thread stopped during state capture")
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    unsafe {
                        use windows_sys::Win32::UI::WindowsAndMessaging::*;
                        let mut message: MSG = std::mem::zeroed();
                        for _ in 0..64 {
                            if PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE)
                                == 0
                            {
                                break;
                            }
                            TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                    }
                    anyhow::ensure!(
                        started.elapsed() < std::time::Duration::from_secs(5),
                        "VST2 state capture did not finish before shutdown"
                    );
                }
            }
        }
    }
    #[cfg(not(windows))]
    pub fn capture_state_for_shutdown(&mut self) -> Result<Option<StateChunk>> {
        self.capture_state()
    }
    pub fn restore_state(&self, data: &[u8], bank: bool, program: Option<u32>) -> Result<()> {
        anyhow::ensure!(self.preset_chunks, "VST2 plugin does not support chunks");
        anyhow::ensure!(data.len() <= 64 * 1024 * 1024, "VST2 state exceeds 64 MiB");
        if let Some(program) = program {
            anyhow::ensure!(
                program < self.program_count.max(0) as u32,
                "Invalid VST2 program"
            );
            if !bank {
                self.parameters.change_preset(program as i32);
            }
        }
        self.instance
            .as_ref()
            .unwrap()
            .write_state_chunk(bank, data);
        if bank {
            if let Some(program) = program {
                self.parameters.change_preset(program as i32);
            }
        }
        Ok(())
    }
}
impl Hosted for Vst2 {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }
    fn set_parameter(&mut self, id: u32, value: f64) {
        #[cfg(windows)]
        {
            self.pending_capture = None;
            self.capture_started = None;
        }
        #[cfg(windows)]
        let host_revision = self.revisions.begin_host(id);
        #[cfg(windows)]
        self.delivered_values.insert(id, value as f32 as f64);
        self.parameters.set_parameter(id as i32, value as f32);
        #[cfg(windows)]
        if let Ok(mut values) = self.editor_values.try_lock() {
            values.insert(id, value as f32 as f64);
        }
        #[cfg(windows)]
        self.revisions
            .end_host(id, host_revision, value as f32 as f64);
    }
    fn note(&mut self, note: u8, velocity: f32, on: bool) {
        if self.pending.len() < 256 {
            self.pending.push(MidiEvent {
                data: [
                    if on { 0x90 } else { 0x80 },
                    note,
                    (velocity.clamp(0.0, 1.0) * 127.0) as u8,
                ],
                delta_frames: 0,
                live: true,
                note_length: None,
                note_offset: None,
                detune: 0,
                note_off_velocity: 0,
            });
        }
    }
    fn process(&mut self, frames: &mut [Frame]) -> Result<()> {
        anyhow::ensure!(frames.len() == BLOCK_SIZE, "Unexpected VST block size");
        if !self.pending.is_empty() {
            self.events.store_events(&self.pending);
            self.instance
                .as_mut()
                .unwrap()
                .process_events(self.events.events());
            self.pending.clear();
        }
        let mono = self.input.len() == 1;
        for (channel, buffer) in self.input.iter_mut().enumerate() {
            for (i, frame) in frames.iter().enumerate() {
                buffer[i] = if mono {
                    (frame[0] + frame[1]) * 0.5
                } else {
                    frame[channel]
                };
            }
        }
        for buffer in &mut self.output {
            buffer.fill(0.0);
        }
        self.instance
            .as_mut()
            .unwrap()
            .process(&mut self.buffer.bind(&self.input, &mut self.output));
        for (i, frame) in frames.iter_mut().enumerate() {
            *frame = [self.output[0][i], self.output[self.output.len() - 1][i]];
        }
        Ok(())
    }
}
impl Drop for Vst2 {
    fn drop(&mut self) {
        #[cfg(windows)]
        self.close_editor();
        if let Some(mut instance) = self.instance.take() {
            #[cfg(windows)]
            if let Some(editor) = self.editor.take() {
                editor.shutdown(instance);
                return;
            }
            instance.stop_process();
            instance.suspend();
            drop(instance);
        }
    }
}

#[cfg(all(test, windows))]
mod revision_tests {
    use super::*;
    #[test]
    fn newer_host_and_vendor_edits_supersede_stale_events_per_parameter() {
        let revisions = ParameterRevisions::default();
        revisions.initialize(2);
        let old_vendor = revisions.mark_vendor(0, 0.2).unwrap();
        let old_poll = revisions.observe(1).unwrap();
        let host0 = revisions.begin_host(0);
        let host1 = revisions.begin_host(1);
        assert!(!revisions.current(0, old_vendor));
        assert!(revisions.mark_polled(1, old_poll).is_none());
        assert!(revisions.mark_vendor(0, 0.2).is_none());
        revisions.end_host(0, host0, 0.6);
        revisions.end_host(1, host1, 0.5);
        let new_vendor = revisions.mark_vendor(0, 0.2).unwrap();
        assert!(revisions.current(0, new_vendor));
        assert!(!revisions.current(0, old_vendor));
        assert!(revisions.mark_polled(1, old_poll).is_none());
    }
}
