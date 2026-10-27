//! VST 2.4 host adapter. Instances stay on the engine's owning thread.
#![allow(deprecated)] // VST2 compatibility is intentionally confined to this adapter.
use anyhow::{Context, Result};
use piratewave_core::{
    plugins::{PluginDescriptor, PluginFormat, PluginParameter, PluginRole},
    Frame, BLOCK_SIZE, SAMPLE_RATE,
};
use std::path::Path;
pub mod pe;
pub mod protocol;
pub trait Hosted {
    fn descriptor(&self) -> &PluginDescriptor;
    fn set_parameter(&mut self, id: u32, value: f64);
    fn note(&mut self, note: u8, velocity: f32, on: bool);
    fn process(&mut self, frames: &mut [Frame]) -> Result<()>;
}
use std::sync::{Arc, Mutex};
use vst::{
    buffer::SendEventBuffer,
    event::MidiEvent,
    host::{Host, HostBuffer, PluginInstance, PluginLoader},
    plugin::{Category, Plugin, PluginParameters},
};
struct PirateWaveHost;
impl Host for PirateWaveHost {
    fn get_info(&self) -> (isize, String, String) {
        (1, "PirateWave".into(), "PirateWave".into())
    }
    fn get_block_size(&self) -> isize {
        BLOCK_SIZE as isize
    }
}
pub struct Vst2 {
    instance: PluginInstance,
    parameters: Arc<dyn PluginParameters>,
    buffer: HostBuffer<f32>,
    input: Vec<[f32; BLOCK_SIZE]>,
    output: Vec<[f32; BLOCK_SIZE]>,
    events: SendEventBuffer,
    pending: Vec<MidiEvent>,
    descriptor: PluginDescriptor,
}
impl Vst2 {
    /// Control-plane text readback for compatibility characterization.
    pub fn parameter_text(&self, id: u32) -> String {
        self.parameters.get_parameter_text(id as i32)
    }
    pub fn load(path: &Path) -> Result<Self> {
        // Preserve the OS loader's reason instead of the vst crate's generic InvalidPath.
        let _library=unsafe{libloading::Library::new(path)}.with_context(||format!("Windows could not load VST2 {} ({}-bit host). A required runtime/DLL may be missing",path.display(),usize::BITS))?;
        let mut loader = PluginLoader::load(path, Arc::new(Mutex::new(PirateWaveHost)))?;
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
        instance.resume();
        instance.start_process();
        Ok(Self {
            instance,
            parameters,
            buffer: HostBuffer::new(info.inputs as usize, info.outputs as usize),
            input: vec![[0.0; BLOCK_SIZE]; info.inputs as usize],
            output: vec![[0.0; BLOCK_SIZE]; info.outputs as usize],
            events: SendEventBuffer::new(256),
            pending: Vec::with_capacity(256),
            descriptor,
        })
    }
}
impl Hosted for Vst2 {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }
    fn set_parameter(&mut self, id: u32, value: f64) {
        self.parameters.set_parameter(id as i32, value as f32);
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
            self.instance.process_events(self.events.events());
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
            .process(&mut self.buffer.bind(&self.input, &mut self.output));
        for (i, frame) in frames.iter_mut().enumerate() {
            *frame = [self.output[0][i], self.output[self.output.len() - 1][i]];
        }
        Ok(())
    }
}
impl Drop for Vst2 {
    fn drop(&mut self) {
        self.instance.stop_process();
        self.instance.suspend();
    }
}
