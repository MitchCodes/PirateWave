use crate::Frame;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug)]
pub struct ParameterDescriptor {
    pub id: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: &'static str,
}

/// Implement this trait and register an EffectFactory to add a processor.
/// The engine owns each instance. `process` must not block, allocate, or do I/O.
pub trait AudioEffect: Send {
    fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), String>;
    fn process(&mut self, frames: &mut [Frame]);
    fn reset(&mut self);
    /// A faulted processor passes through until it is reset or replaced.
    fn faulted(&self) -> bool {
        false
    }
}
struct GuardedEffect {
    inner: Box<dyn AudioEffect>,
    failed: bool,
}
impl AudioEffect for GuardedEffect {
    fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), String> {
        self.inner.set_parameter(id, value)
    }
    fn process(&mut self, frames: &mut [Frame]) {
        if self.failed {
            return;
        }
        for block in frames.chunks_mut(crate::BLOCK_SIZE) {
            let mut dry = [[0.; 2]; crate::BLOCK_SIZE];
            dry[..block.len()].copy_from_slice(block);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.inner.process(block)
            }));
            if result.is_err() || block.iter().flatten().any(|v| !v.is_finite()) {
                block.copy_from_slice(&dry[..block.len()]);
                self.failed = true;
                break;
            }
        }
    }
    fn reset(&mut self) {
        self.failed =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.inner.reset())).is_err();
    }
    fn faulted(&self) -> bool {
        self.failed
    }
}

pub struct EffectFactory {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: &'static [ParameterDescriptor],
    pub create: fn(f32) -> Box<dyn AudioEffect>,
}

pub struct EffectRegistry {
    factories: Vec<EffectFactory>,
}
impl Default for EffectRegistry {
    fn default() -> Self {
        let mut registry = Self { factories: vec![] };
        registry.register(crate::automatic_gain::factory()).unwrap();
        registry.register(crate::reverb::factory()).unwrap();
        for factory in crate::rack_processors::factories() {
            registry.register(factory).unwrap();
        }
        for factory in crate::voice_effects::factories() {
            registry.register(factory).unwrap();
        }
        registry
            .register(crate::voice_correction::factory())
            .unwrap();
        registry
            .register(EffectFactory {
                id: "equalizer",
                name: "Parametric EQ",
                description: "Shape one band with frequency, gain, and width",
                parameters: &EQ_PARAMS,
                create: |rate| {
                    Box::new(Equalizer {
                        rate,
                        frequency: 1000.0,
                        gain: 0.0,
                        q: 0.707,
                        z1: [0.0; 2],
                        z2: [0.0; 2],
                    })
                },
            })
            .unwrap();
        registry
            .register(EffectFactory {
                id: "delay",
                name: "Delay",
                description: "A tempo-free echo with feedback and wet/dry blend",
                parameters: &DELAY_PARAMS,
                create: |rate| {
                    Box::new(Delay {
                        rate,
                        time: 250.0,
                        feedback: 0.25,
                        mix: 0.25,
                        buffer: vec![[0.0; 2]; (rate * 2.0) as usize],
                        position: 0,
                    })
                },
            })
            .unwrap();
        registry
            .register(EffectFactory {
                id: "gain",
                name: "Gain",
                description: "Transparent level adjustment",
                parameters: &GAIN_PARAMS,
                create: |_| Box::new(Gain { db: 0.0 }),
            })
            .unwrap();
        registry
            .register(EffectFactory {
                id: "high_pass",
                name: "High-pass filter",
                description: "Remove rumble and low-frequency noise",
                parameters: &FILTER_PARAMS,
                create: |rate| {
                    Box::new(HighPass {
                        rate,
                        cutoff: 80.0,
                        previous_input: [0.0; 2],
                        previous_output: [0.0; 2],
                    })
                },
            })
            .unwrap();
        registry
            .register(EffectFactory {
                id: "gate",
                name: "Noise gate",
                description: "Suppress background noise between phrases",
                parameters: &GATE_PARAMS,
                create: |rate| {
                    Box::new(Gate {
                        rate,
                        threshold: -42.0,
                        release: 100.0,
                        envelope: 0.0,
                        gain: 0.0,
                    })
                },
            })
            .unwrap();
        registry
            .register(EffectFactory {
                id: "compressor",
                name: "Compressor",
                description: "Control dynamics with a linked stereo envelope",
                parameters: &COMP_PARAMS,
                create: |rate| {
                    Box::new(Compressor {
                        rate,
                        threshold: -18.0,
                        ratio: 4.0,
                        makeup: 0.0,
                        envelope: 0.0,
                    })
                },
            })
            .unwrap();
        for e in crate::extensions::catalog()
            .iter()
            .filter(|e| e.descriptor.kind == crate::extensions::TypeKind::Processor)
        {
            registry
                .register(EffectFactory {
                    id: e.factory.id,
                    name: e.factory.name,
                    description: e.factory.description,
                    parameters: e.factory.parameters,
                    create: e.factory.create,
                })
                .unwrap();
        }
        registry
    }
}
impl EffectRegistry {
    pub fn validate(&self, spec: &EffectSpec) -> Result<(), String> {
        if spec.kind.contains('/') {
            if !piratewave_extension_sdk::valid_id(&spec.kind)
                || spec.parameters.len() > 64
                || spec
                    .parameters
                    .iter()
                    .any(|(k, v)| !piratewave_extension_sdk::valid_id(k) || !v.is_finite())
            {
                return Err("Invalid extension effect".into());
            }
            if let Some(state) = &spec.extension {
                state.validate()?;
            }
            if let Some(d) = &spec.extension_descriptor {
                d.validate()?;
                if d.id != spec.kind || d.kind != crate::extensions::TypeKind::Processor {
                    return Err("Extension descriptor identity mismatch".into());
                }
                crate::extensions::validate_values(
                    d,
                    &spec.parameters,
                    &spec.extension.clone().unwrap_or(crate::extensions::State {
                        schema: d.state_schema,
                        bytes: d.default_state.clone(),
                    }),
                )?;
            }
            if spec.extension_keybinds.iter().any(|(id, key)| {
                key.len() > 256
                    || !spec
                        .extension_descriptor
                        .as_ref()
                        .is_some_and(|d| d.actions.iter().any(|a| &a.id == id))
            }) {
                return Err("Invalid extension effect action binding".into());
            }
            return Ok(());
        }
        let factory = self
            .get(&spec.kind)
            .ok_or_else(|| format!("Unknown effect: {}", spec.kind))?;
        for (id, value) in &spec.parameters {
            let p = factory
                .parameters
                .iter()
                .find(|p| p.id == id)
                .ok_or_else(|| format!("Unknown parameter {id}"))?;
            if !value.is_finite() || *value < p.min || *value > p.max {
                return Err(format!("Invalid {id}: {value}"));
            }
        }
        Ok(())
    }
    pub fn configure(&self, effect: &mut dyn AudioEffect, spec: &EffectSpec) -> Result<(), String> {
        self.validate(spec)?;
        let Some(factory) = self.get(&spec.kind) else {
            return Ok(());
        };
        for p in factory.parameters {
            effect.set_parameter(p.id, *spec.parameters.get(p.id).unwrap_or(&p.default))?;
        }
        Ok(())
    }
    pub fn register(&mut self, factory: EffectFactory) -> Result<(), String> {
        if factory.id.is_empty() {
            return Err("An effect needs a stable nonempty ID".into());
        }
        let mut ids = std::collections::HashSet::new();
        for p in factory.parameters {
            if p.id.is_empty()
                || !ids.insert(p.id)
                || !p.min.is_finite()
                || !p.max.is_finite()
                || !p.default.is_finite()
                || p.min > p.default
                || p.default > p.max
            {
                return Err(format!("Invalid parameter descriptor: {}", p.id));
            }
        }
        if self.factories.iter().any(|f| f.id == factory.id) {
            return Err(format!("Effect '{}' already registered", factory.id));
        }
        self.factories.push(factory);
        Ok(())
    }
    pub fn factories(&self) -> &[EffectFactory] {
        &self.factories
    }
    pub fn get(&self, id: &str) -> Option<&EffectFactory> {
        self.factories.iter().find(|f| f.id == id)
    }
    pub fn create(&self, spec: &EffectSpec, rate: f32) -> Result<Box<dyn AudioEffect>, String> {
        if !rate.is_finite() || !(8000.0..=384000.0).contains(&rate) {
            return Err("Unsupported processor sample rate".into());
        }
        self.validate(spec)?;
        if spec.kind.contains('/') {
            let Some(e) = crate::extensions::get(&spec.kind) else {
                return Ok(Box::new(crate::extensions::Unavailable));
            };
            if spec
                .extension_descriptor
                .as_ref()
                .is_some_and(|d| d != &e.descriptor)
            {
                return Ok(Box::new(crate::extensions::Unavailable));
            }
            let state = spec.extension.clone().unwrap_or(crate::extensions::State {
                schema: e.descriptor.state_schema,
                bytes: e.descriptor.default_state.clone(),
            });
            if state.schema != e.descriptor.state_schema {
                return Ok(Box::new(crate::extensions::Unavailable));
            }
            let Ok(mut effect) = crate::extensions::Effect::new(e, rate, &state) else {
                return Ok(Box::new(crate::extensions::Unavailable));
            };
            if self.configure(&mut effect, spec).is_err() {
                return Ok(Box::new(crate::extensions::Unavailable));
            }
            return Ok(Box::new(effect));
        }
        let factory = self
            .get(&spec.kind)
            .ok_or_else(|| format!("Unknown effect: {}", spec.kind))?;
        let mut effect = (factory.create)(rate);
        self.configure(effect.as_mut(), spec)?;
        Ok(Box::new(GuardedEffect {
            inner: effect,
            failed: false,
        }))
    }
}

pub(crate) fn new_effect_id() -> u64 {
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    };
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    NEXT.get_or_init(|| {
        AtomicU64::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64,
        )
    })
    .fetch_add(1, Ordering::Relaxed)
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EffectSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_descriptor: Option<crate::extensions::TypeDescriptor>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub extension_keybinds: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension: Option<crate::extensions::State>,
    #[serde(default = "new_effect_id")]
    pub id: u64,
    #[serde(default)]
    pub keybinds: std::collections::BTreeMap<crate::shortcuts::EffectAction, String>,
    pub kind: String,
    pub enabled: bool,
    pub parameters: std::collections::BTreeMap<String, f32>,
}
impl EffectSpec {
    pub fn new(factory: &EffectFactory) -> Self {
        Self {
            extension_descriptor: crate::extensions::get(factory.id).map(|e| e.descriptor.clone()),
            extension_keybinds: Default::default(),
            extension: crate::extensions::get(factory.id).map(|e| crate::extensions::State {
                schema: e.descriptor.state_schema,
                bytes: e.descriptor.default_state.clone(),
            }),
            id: new_effect_id(),
            keybinds: Default::default(),
            kind: factory.id.into(),
            enabled: true,
            parameters: factory
                .parameters
                .iter()
                .map(|p| (p.id.into(), p.default))
                .collect(),
        }
    }
}
const GAIN_PARAMS: [ParameterDescriptor; 1] = [ParameterDescriptor {
    id: "gain",
    label: "Gain",
    min: -60.0,
    max: 24.0,
    default: 0.0,
    unit: "dB",
}];
const FILTER_PARAMS: [ParameterDescriptor; 1] = [ParameterDescriptor {
    id: "cutoff",
    label: "Cutoff",
    min: 20.0,
    max: 2000.0,
    default: 80.0,
    unit: "Hz",
}];
const GATE_PARAMS: [ParameterDescriptor; 2] = [
    ParameterDescriptor {
        id: "threshold",
        label: "Threshold",
        min: -80.0,
        max: 0.0,
        default: -42.0,
        unit: "dB",
    },
    ParameterDescriptor {
        id: "release",
        label: "Release",
        min: 10.0,
        max: 1000.0,
        default: 100.0,
        unit: "ms",
    },
];
const COMP_PARAMS: [ParameterDescriptor; 3] = [
    ParameterDescriptor {
        id: "threshold",
        label: "Threshold",
        min: -60.0,
        max: 0.0,
        default: -18.0,
        unit: "dB",
    },
    ParameterDescriptor {
        id: "ratio",
        label: "Ratio",
        min: 1.0,
        max: 20.0,
        default: 4.0,
        unit: ":1",
    },
    ParameterDescriptor {
        id: "makeup",
        label: "Makeup",
        min: 0.0,
        max: 24.0,
        default: 0.0,
        unit: "dB",
    },
];
const EQ_PARAMS: [ParameterDescriptor; 3] = [
    ParameterDescriptor {
        id: "frequency",
        label: "Frequency",
        min: 30.0,
        max: 18000.0,
        default: 1000.0,
        unit: "Hz",
    },
    ParameterDescriptor {
        id: "gain",
        label: "Gain",
        min: -18.0,
        max: 18.0,
        default: 0.0,
        unit: "dB",
    },
    ParameterDescriptor {
        id: "q",
        label: "Width (Q)",
        min: 0.1,
        max: 10.0,
        default: 0.707,
        unit: "",
    },
];
const DELAY_PARAMS: [ParameterDescriptor; 3] = [
    ParameterDescriptor {
        id: "time",
        label: "Time",
        min: 1.0,
        max: 1000.0,
        default: 250.0,
        unit: "ms",
    },
    ParameterDescriptor {
        id: "feedback",
        label: "Feedback",
        min: 0.0,
        max: 0.85,
        default: 0.25,
        unit: "",
    },
    ParameterDescriptor {
        id: "mix",
        label: "Wet / dry",
        min: 0.0,
        max: 1.0,
        default: 0.25,
        unit: "",
    },
];
pub fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}
struct Gain {
    db: f32,
}
impl AudioEffect for Gain {
    fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), String> {
        if id != "gain" {
            return Err(id.into());
        }
        self.db = value;
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        let gain = db_to_gain(self.db);
        for f in frames {
            f[0] *= gain;
            f[1] *= gain;
        }
    }
    fn reset(&mut self) {}
}
struct HighPass {
    rate: f32,
    cutoff: f32,
    previous_input: Frame,
    previous_output: Frame,
}
impl AudioEffect for HighPass {
    fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), String> {
        if id != "cutoff" {
            return Err(id.into());
        }
        self.cutoff = value;
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        let rc = 1.0 / (std::f32::consts::TAU * self.cutoff);
        let a = rc / (rc + 1.0 / self.rate);
        for frame in frames {
            for (c, sample) in frame.iter_mut().enumerate() {
                let input = *sample;
                *sample = a * (self.previous_output[c] + input - self.previous_input[c]);
                self.previous_input[c] = input;
                self.previous_output[c] = *sample;
            }
        }
    }
    fn reset(&mut self) {
        self.previous_input = [0.0; 2];
        self.previous_output = [0.0; 2];
    }
}
struct Gate {
    rate: f32,
    threshold: f32,
    release: f32,
    envelope: f32,
    gain: f32,
}
impl AudioEffect for Gate {
    fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), String> {
        match id {
            "threshold" => self.threshold = value,
            "release" => self.release = value,
            _ => return Err(id.into()),
        }
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        let threshold = db_to_gain(self.threshold);
        let release = (-1.0 / (self.rate * self.release * 0.001)).exp();
        let attack = (-1.0 / (self.rate * 0.002)).exp();
        for frame in frames {
            let peak = frame[0].abs().max(frame[1].abs());
            self.envelope = peak.max(self.envelope * release);
            let target = if self.envelope > threshold { 1.0 } else { 0.0 };
            let coeff = if target > self.gain { attack } else { release };
            self.gain = target + (self.gain - target) * coeff;
            frame[0] *= self.gain;
            frame[1] *= self.gain;
        }
    }
    fn reset(&mut self) {
        self.envelope = 0.0;
        self.gain = 0.0;
    }
}
struct Compressor {
    rate: f32,
    threshold: f32,
    ratio: f32,
    makeup: f32,
    envelope: f32,
}
impl AudioEffect for Compressor {
    fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), String> {
        match id {
            "threshold" => self.threshold = value,
            "ratio" => self.ratio = value,
            "makeup" => self.makeup = value,
            _ => return Err(id.into()),
        }
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        let attack = (-1.0 / (self.rate * 0.005)).exp();
        let release = (-1.0 / (self.rate * 0.100)).exp();
        for frame in frames {
            let peak = frame[0].abs().max(frame[1].abs());
            let coeff = if peak > self.envelope {
                attack
            } else {
                release
            };
            self.envelope = peak + (self.envelope - peak) * coeff;
            let db = 20.0 * self.envelope.max(1e-9).log10();
            let reduction = if db > self.threshold {
                (self.threshold + (db - self.threshold) / self.ratio) - db
            } else {
                0.0
            };
            let gain = db_to_gain(reduction + self.makeup);
            frame[0] *= gain;
            frame[1] *= gain;
        }
    }
    fn reset(&mut self) {
        self.envelope = 0.0;
    }
}

struct Equalizer {
    rate: f32,
    frequency: f32,
    gain: f32,
    q: f32,
    z1: Frame,
    z2: Frame,
}
impl AudioEffect for Equalizer {
    fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), String> {
        match id {
            "frequency" => self.frequency = value,
            "gain" => self.gain = value,
            "q" => self.q = value,
            _ => return Err(id.into()),
        }
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        let a = 10.0_f32.powf(self.gain / 40.0);
        let w = std::f32::consts::TAU * self.frequency.min(self.rate * 0.49) / self.rate;
        let alpha = w.sin() / (2.0 * self.q);
        let a0 = 1.0 + alpha / a;
        let b0 = (1.0 + alpha * a) / a0;
        let b1 = -2.0 * w.cos() / a0;
        let b2 = (1.0 - alpha * a) / a0;
        let a1 = b1;
        let a2 = (1.0 - alpha / a) / a0;
        for f in frames {
            for (c, s) in f.iter_mut().enumerate() {
                let x = *s;
                let y = b0 * x + self.z1[c];
                self.z1[c] = b1 * x - a1 * y + self.z2[c];
                self.z2[c] = b2 * x - a2 * y;
                *s = y;
            }
        }
    }
    fn reset(&mut self) {
        self.z1 = [0.0; 2];
        self.z2 = [0.0; 2];
    }
}
struct Delay {
    rate: f32,
    time: f32,
    feedback: f32,
    mix: f32,
    buffer: Vec<Frame>,
    position: usize,
}
impl AudioEffect for Delay {
    fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), String> {
        match id {
            "time" => self.time = value,
            "feedback" => self.feedback = value,
            "mix" => self.mix = value,
            _ => return Err(id.into()),
        }
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        let offset = ((self.time * 0.001 * self.rate) as usize).clamp(1, self.buffer.len() - 1);
        for f in frames {
            let read = (self.position + self.buffer.len() - offset) % self.buffer.len();
            let wet = self.buffer[read];
            self.buffer[self.position] =
                [f[0] + wet[0] * self.feedback, f[1] + wet[1] * self.feedback];
            f[0] = f[0] * (1.0 - self.mix) + wet[0] * self.mix;
            f[1] = f[1] * (1.0 - self.mix) + wet[1] * self.mix;
            self.position = (self.position + 1) % self.buffer.len();
        }
    }
    fn reset(&mut self) {
        self.buffer.fill([0.0; 2]);
        self.position = 0;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Broken {
        panic: bool,
        recovered: bool,
    }
    impl AudioEffect for Broken {
        fn set_parameter(&mut self, _: &str, _: f32) -> Result<(), String> {
            Ok(())
        }
        fn process(&mut self, frames: &mut [Frame]) {
            if self.recovered {
                for f in frames {
                    *f = [0.25; 2];
                }
                return;
            }
            frames[0] = [f32::NAN; 2];
            assert!(!self.panic, "injected processor failure");
        }
        fn reset(&mut self) {
            self.recovered = true;
        }
    }
    #[test]
    fn effect_panic_and_nan_preserve_dry_audio_and_recover_on_reset() {
        for panic in [false, true] {
            let mut fx = GuardedEffect {
                inner: Box::new(Broken {
                    panic,
                    recovered: false,
                }),
                failed: false,
            };
            let mut frames = [[0.5; 2]; 512];
            fx.process(&mut frames);
            assert_eq!(frames, [[0.5; 2]; 512]);
            assert!(fx.faulted());
            fx.process(&mut frames);
            assert_eq!(frames, [[0.5; 2]; 512]);
            fx.reset();
            fx.process(&mut frames);
            assert!(!fx.faulted());
            assert_eq!(frames, [[0.25; 2]; 512]);
        }
    }
    #[test]
    fn equalizer_is_unity_at_zero_gain() {
        let reg = EffectRegistry::default();
        let mut fx = reg
            .create(&EffectSpec::new(reg.get("equalizer").unwrap()), 48000.0)
            .unwrap();
        let mut frames: Vec<_> = (0..4000).map(|i| [(i as f32 * 0.12).sin(); 2]).collect();
        let original = frames.clone();
        fx.process(&mut frames);
        assert!(frames
            .iter()
            .zip(&original)
            .all(|(a, b)| (a[0] - b[0]).abs() < 0.001));
    }
    #[test]
    fn delay_emits_at_requested_time() {
        let reg = EffectRegistry::default();
        let mut spec = EffectSpec::new(reg.get("delay").unwrap());
        spec.parameters.insert("time".into(), 10.0);
        spec.parameters.insert("feedback".into(), 0.0);
        spec.parameters.insert("mix".into(), 1.0);
        let mut fx = reg.create(&spec, 48000.0).unwrap();
        let mut frames = vec![[0.0; 2]; 1000];
        frames[0] = [1.0; 2];
        fx.process(&mut frames);
        assert_eq!(frames[0], [0.0; 2]);
        assert_eq!(frames[480], [1.0; 2]);
        fx.reset();
        frames.fill([0.0; 2]);
        fx.process(&mut frames);
        assert!(frames.iter().all(|f| *f == [0.0; 2]));
    }
    #[test]
    fn gain_is_correct() {
        let reg = EffectRegistry::default();
        let mut spec = EffectSpec::new(reg.get("gain").unwrap());
        spec.parameters.insert("gain".into(), -6.0206);
        let mut fx = reg.create(&spec, 48000.0).unwrap();
        let mut frames = [[1.0, -1.0]; 32];
        fx.process(&mut frames);
        assert!((frames[0][0] - 0.5).abs() < 1e-5);
        assert!((frames[0][1] + 0.5).abs() < 1e-5);
    }
    #[test]
    fn rejects_non_finite_parameters() {
        let reg = EffectRegistry::default();
        let mut spec = EffectSpec::new(reg.get("gain").unwrap());
        spec.parameters.insert("gain".into(), f32::NAN);
        assert!(reg.create(&spec, 48000.0).is_err());
    }
    #[test]
    fn high_pass_removes_dc_and_resets() {
        let reg = EffectRegistry::default();
        let spec = EffectSpec::new(reg.get("high_pass").unwrap());
        let mut fx = reg.create(&spec, 48000.0).unwrap();
        let mut frames = vec![[0.5; 2]; 48000];
        fx.process(&mut frames);
        assert!(frames.last().unwrap()[0].abs() < 1e-5);
        fx.reset();
        let mut silence = [[0.0; 2]; 32];
        fx.process(&mut silence);
        assert_eq!(silence, [[0.0; 2]; 32]);
    }
    #[test]
    fn compressor_reduces_loud_signal() {
        let reg = EffectRegistry::default();
        let mut fx = reg
            .create(&EffectSpec::new(reg.get("compressor").unwrap()), 48000.0)
            .unwrap();
        let mut frames = vec![[1.0; 2]; 48000];
        fx.process(&mut frames);
        let last = frames.last().unwrap()[0];
        assert!(last > 0.1 && last < 0.3, "{last}");
    }
    #[test]
    fn gate_attenuates_quiet_signal() {
        let reg = EffectRegistry::default();
        let mut fx = reg
            .create(&EffectSpec::new(reg.get("gate").unwrap()), 48000.0)
            .unwrap();
        let mut frames = [[0.001; 2]; 128];
        fx.process(&mut frames);
        assert!(frames.iter().all(|f| f[0].abs() < 1e-8));
    }
}
