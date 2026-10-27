//! Native monophonic pitch correction. See docs/AUTOMATIC_VOICE_CORRECTION.md
//! for binary evidence, recovered DSP, validation, and remaining limitations.
use crate::{
    effects::{AudioEffect, EffectFactory, ParameterDescriptor},
    Frame,
};
#[cfg(test)]
use std::f32::consts::TAU;
mod detector;
mod shifter;
use detector::{Biquad, Detector};
use shifter::{RecoveredShifter, RING_SIZE};

pub const EFFECT_ID: &str = "automatic_voice_correction";
pub const LATENCY_FRAMES: usize = 1390;
pub const LATENCY_MS: f32 = LATENCY_FRAMES as f32 / 48.0;
const PREPARATION_DELAY: usize = 1280;
pub const KEYS: &[&str] = &[
    "C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
];
pub const SCALES: &[&str] = &["Major", "Minor", "Chromatic"];
pub const VOICES: &[&str] = &[
    "Soprano",
    "Alto / tenor",
    "Low male",
    "Instrument",
    "Bass instrument",
];
pub fn choices(id: &str) -> Option<&'static [&'static str]> {
    match id {
        "key" => Some(KEYS),
        "scale" => Some(SCALES),
        "voice" => Some(VOICES),
        _ => None,
    }
}
const PARAMS: &[ParameterDescriptor] = &[
    ParameterDescriptor {
        id: "key",
        label: "Key",
        min: 0.0,
        max: 11.0,
        default: 0.0,
        unit: "",
    },
    ParameterDescriptor {
        id: "scale",
        label: "Scale",
        min: 0.0,
        max: 2.0,
        default: 2.0,
        unit: "",
    },
    ParameterDescriptor {
        id: "voice",
        label: "Voice range",
        min: 0.0,
        max: 4.0,
        default: 1.0,
        unit: "",
    },
    ParameterDescriptor {
        id: "retune",
        label: "Retune time",
        min: 0.0,
        max: 400.0,
        default: 20.0,
        unit: "ms",
    },
    ParameterDescriptor {
        id: "humanize",
        label: "Humanize",
        min: 0.0,
        max: 100.0,
        default: 0.0,
        unit: "%",
    },
    ParameterDescriptor {
        id: "detune",
        label: "Reference detune",
        min: -100.0,
        max: 100.0,
        default: 0.0,
        unit: "cents",
    },
    ParameterDescriptor {
        id: "transpose",
        label: "Transpose",
        min: -12.0,
        max: 12.0,
        default: 0.0,
        unit: "st",
    },
    ParameterDescriptor {
        id: "amount",
        label: "Correction amount",
        min: 0.0,
        max: 100.0,
        default: 100.0,
        unit: "%",
    },
    ParameterDescriptor {
        id: "mix",
        label: "Wet mix",
        min: 0.0,
        max: 100.0,
        default: 100.0,
        unit: "%",
    },
];
pub fn factory() -> EffectFactory {
    EffectFactory {
        id: EFFECT_ID,
        name: "Automatic Voice Correction",
        description:
            "Tune a single voice to a key and scale. About 29 ms latency at 48 kHz; use on a vocal route.",
        parameters: PARAMS,
        create: |rate| Box::new(VoiceCorrection::new(rate)),
    }
}

/// Fixed transposition shares the tested period detector, antialias filters and
/// phase-aligned shifter, with note snapping disabled.
pub(crate) struct PitchShift {
    inner: VoiceCorrection,
}
impl PitchShift {
    pub(crate) fn new(rate: f32) -> Self {
        let mut inner = VoiceCorrection::new(rate);
        inner.amount = 0.;
        inner.mix = 0.5;
        inner.voice = 3;
        inner.detector.set_voice(3);
        Self { inner }
    }
}
impl AudioEffect for PitchShift {
    fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), String> {
        let valid = value.is_finite()
            && match id {
                "semitones" => (-24. ..=24.).contains(&value),
                "mix" => (0. ..=1.).contains(&value),
                _ => false,
            };
        if !valid {
            return Err(format!("Invalid pitch parameter {id}: {value}"));
        }
        match id {
            "semitones" => self.inner.transpose = value,
            "mix" => self.inner.mix = value,
            _ => unreachable!(),
        }
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        self.inner.process(frames);
    }
    fn reset(&mut self) {
        self.inner.reset();
    }
}

// Standard major/minor interval arrays implement the observed scale/key menus.
// Hysteresis is our own implementation, preventing chatter at note boundaries.
fn target_note(note: f32, key: i32, scale: usize, previous: Option<i32>) -> i32 {
    let mask: &[i32] = match scale {
        0 => &[0, 2, 4, 5, 7, 9, 11],
        1 => &[0, 2, 3, 5, 7, 8, 10],
        _ => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
    };
    let allowed = |n: i32| mask.contains(&(n - key).rem_euclid(12));
    let center = note.round() as i32;
    let nearest = (center - 2..=center + 2)
        .filter(|n| allowed(*n))
        .min_by(|a, b| {
            (2.0_f32.powf((note - *a as f32) / 12.0) - 1.0)
                .abs()
                .total_cmp(&(2.0_f32.powf((note - *b as f32) / 12.0) - 1.0).abs())
        })
        .unwrap_or(center);
    if let Some(old) = previous {
        if allowed(old) && (old as f32 - note).abs() <= (nearest as f32 - note).abs() + 0.06 {
            return old;
        }
    }
    nearest
}

// See the address/evidence ledger in the accompanying documentation.
fn smoothing_alpha(rate: f32, retune: f32, humanize: f32, held_ms: f32, samples: usize) -> f32 {
    let coefficient = |ms: f32| {
        if ms <= 0.0 {
            0.0
        } else {
            0.5_f32.powf(5000.0 / (rate * ms))
        }
    };
    let initial = coefficient(retune);
    let final_alpha = coefficient(retune + humanize);
    (initial + (final_alpha - initial) * (held_ms / 400.0).clamp(0.0, 1.0))
        .powf(samples as f32 / 5.0)
}

#[derive(Clone, Copy, Default)]
struct Control {
    period: f64,
    last_period: f64,
    ratio: f64,
}

pub struct VoiceCorrection {
    rate: f64,
    key: i32,
    scale: usize,
    voice: usize,
    retune: f32,
    humanize: f32,
    detune: f32,
    transpose: f32,
    amount: f32,
    mix: f32,
    detector: Detector,
    shifter: RecoveredShifter,
    audio: Vec<Frame>,
    controls: Vec<Control>,
    write: usize,
    energies: [f64; 2],
    channel: usize,
    note: Option<i32>,
    held_ms: f64,
    ratio: f64,
    last_period: f64,
    antialias: [[Biquad; 2]; 2],
    filter_ratio: f64,
}
impl VoiceCorrection {
    pub fn new(rate: f32) -> Self {
        let rate = rate as f64;
        Self {
            rate,
            key: 0,
            scale: 2,
            voice: 1,
            retune: 20.0,
            humanize: 0.0,
            detune: 0.0,
            transpose: 0.0,
            amount: 1.0,
            mix: 1.0,
            detector: Detector::new(rate, 1),
            shifter: RecoveredShifter::new(),
            audio: vec![[0.0; 2]; RING_SIZE],
            controls: vec![Control::default(); RING_SIZE],
            write: 0,
            energies: [0.0; 2],
            channel: 0,
            note: None,
            held_ms: 0.0,
            ratio: 1.0,
            last_period: rate / 440.0,
            antialias: [[Biquad::identity(); 2]; 2],
            filter_ratio: 1.0,
        }
    }
    fn update_pitch(&mut self, period: f64) {
        if period <= 0.0 {
            self.note = None;
            self.held_ms = 0.0;
            self.ratio = 1.0;
            return;
        }
        self.last_period = period;
        let frequency = self.rate / period;
        let detected = 69.0 + 12.0 * (frequency / 440.0).log2() - self.detune as f64 / 100.0;
        let target = target_note(detected as f32, self.key, self.scale, self.note);
        if self.note == Some(target) {
            self.held_ms += 5000.0 / self.rate;
        } else {
            self.held_ms = 0.0;
        }
        self.note = Some(target);
        let desired = 2.0_f64.powf((target as f64 - detected) * self.amount as f64 / 12.0);
        let alpha = smoothing_alpha(
            self.rate as f32,
            self.retune,
            self.humanize,
            self.held_ms as f32,
            5,
        ) as f64;
        self.ratio = alpha * self.ratio + (1.0 - alpha) * desired;
    }
}
impl AudioEffect for VoiceCorrection {
    fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), String> {
        let p = PARAMS
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| format!("Unknown voice correction parameter: {id}"))?;
        if !value.is_finite() || !(p.min..=p.max).contains(&value) {
            return Err(format!("Invalid {id}: {value}"));
        }
        match id {
            "key" => self.key = value.round() as i32,
            "scale" => self.scale = value.round() as usize,
            "voice" => {
                self.voice = value.round() as usize;
                self.detector.set_voice(self.voice);
            }
            "retune" => self.retune = value,
            "humanize" => self.humanize = value,
            "detune" => self.detune = value,
            "transpose" => self.transpose = value.round(),
            "amount" => self.amount = value / 100.0,
            "mix" => self.mix = value / 100.0,
            _ => unreachable!(),
        }
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        for frame in frames {
            let input = frame.map(|v| if v.is_finite() { v } else { 0.0 });
            self.write = (self.write + 1) % RING_SIZE;
            self.audio[self.write] = input;
            for (ch, &sample) in input.iter().enumerate() {
                self.energies[ch] += 0.001 * ((sample as f64).powi(2) - self.energies[ch]);
            }
            // Stereo channel selection is a host extension; the detector itself
            // follows the DLL's acquisition and adaptive five-sample updates.
            if self.detector.acquiring() {
                self.channel = usize::from(self.energies[1] > self.energies[0] * 1.5);
            }
            if let Some(period) = self.detector.process_sample(input[self.channel] as f64) {
                self.update_pitch(period);
            }
            let current = Control {
                period: self.detector.period(),
                last_period: self.last_period,
                ratio: self.ratio * 2.0_f64.powf(self.transpose as f64 / 12.0),
            };
            self.controls[self.write] = current;
            let delayed = (self.write + RING_SIZE - PREPARATION_DELAY) % RING_SIZE;
            let old = self.controls[delayed];
            // These are the DLL's delayed-period / current-period / held-period
            // fallbacks before cursor stepping (VA0x10030514..0x100305b3).
            let (period, ratio) = if old.period > 0.0 {
                (old.period, old.ratio)
            } else if current.period > 0.0 {
                (current.period, current.ratio)
            } else {
                (old.last_period, old.ratio)
            };
            // Recovered four input low-pass filters (two per channel), updated
            // when upward resampling changes by more than ten percent.
            let filter_ratio = current.ratio;
            if filter_ratio < self.filter_ratio * 0.9 || filter_ratio > self.filter_ratio * 1.1 {
                self.filter_ratio = filter_ratio;
                if filter_ratio > 1.0 {
                    for channel in &mut self.antialias {
                        for filter in channel {
                            filter.set_lowpass(self.rate, self.rate * 0.49 / filter_ratio);
                        }
                    }
                } else {
                    for channel in &mut self.antialias {
                        for filter in channel {
                            filter.set_identity();
                        }
                    }
                }
            }
            let mut prepared = self.audio[delayed];
            {
                for (ch, sample) in prepared.iter_mut().enumerate() {
                    let mut v = *sample as f64;
                    for filter in &mut self.antialias[ch] {
                        v = filter.process(v);
                    }
                    *sample = v as f32;
                }
            }
            let shifted = self.shifter.process_sample(prepared, period, ratio);
            let dry = self.audio[(self.write + RING_SIZE - LATENCY_FRAMES) % RING_SIZE];
            for ch in 0..2 {
                frame[ch] = dry[ch] + self.mix * (shifted[ch] - dry[ch]);
            }
        }
    }
    fn reset(&mut self) {
        self.detector.reset();
        self.shifter.reset();
        self.audio.fill([0.0; 2]);
        self.controls.fill(Control::default());
        self.write = 0;
        self.energies = [0.0; 2];
        self.channel = 0;
        self.note = None;
        self.held_ms = 0.0;
        self.ratio = 1.0;
        self.last_period = self.rate / 440.0;
        self.filter_ratio = 1.0;
        for channel in &mut self.antialias {
            for filter in channel {
                *filter = Biquad::identity();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tone(rate: f32, hz: f32, seconds: f32) -> Vec<Frame> {
        (0..(seconds * rate) as usize)
            .map(|i| {
                let phase = TAU * hz * i as f32 / rate;
                let v = 0.2 * phase.sin() + 0.08 * (2.0 * phase).sin() + 0.04 * (3.0 * phase).sin();
                [v, -v]
            })
            .collect()
    }
    fn frequency(frames: &[Frame], rate: f32) -> f32 {
        let crossings: Vec<f32> = frames
            .windows(2)
            .enumerate()
            .filter(|(_, w)| w[0][0] <= 0.0 && w[1][0] > 0.0)
            .map(|(i, w)| i as f32 - w[0][0] / (w[1][0] - w[0][0]))
            .collect();
        rate * (crossings.len() - 1) as f32 / (crossings.last().unwrap() - crossings[0])
    }
    #[test]
    #[ignore = "requires a locally generated read-only DLL oracle trace; set PIRATEWAVE_VOICE_TRACE"]
    fn recovered_components_match_dll_trace() {
        let path = std::env::var("PIRATEWAVE_VOICE_TRACE")
            .expect("Set PIRATEWAVE_VOICE_TRACE to trace.csv");
        let text = std::fs::read_to_string(path).unwrap();
        let mut lines = text.lines();
        let header: Vec<_> = lines.next().unwrap().split(',').collect();
        let mut detector = Detector::new(48000.0, 0);
        let mut shifter = RecoveredShifter::new();
        let (mut max_period, mut max_audio) = (0.0_f64, 0.0_f64);
        let mut frames = 0;
        for line in lines {
            let row: Vec<_> = line.split(',').collect();
            let get = |name: &str| -> f64 {
                row[header.iter().position(|h| *h == name).unwrap()]
                    .parse()
                    .unwrap()
            };
            detector.process_sample(get("input") as f32 as f64);
            max_period = max_period.max((detector.period() - get("period_640")).abs());
            assert_eq!(
                detector.acquiring(),
                get("mode_488") != 0.0,
                "mode at {frames}"
            );
            let (period, ratio) = if get("delayed_period") > 0.0 {
                (get("delayed_period"), get("delayed_ratio"))
            } else if get("period_640") > 0.0 {
                (get("period_640"), get("current_ratio"))
            } else {
                (get("delayed_last_period"), get("delayed_ratio"))
            };
            let actual = shifter.process_sample(
                [get("prepared_l") as f32, get("prepared_r") as f32],
                period,
                ratio,
            );
            max_audio = max_audio.max((actual[0] as f64 - get("output")).abs());
            frames += 1;
        }
        assert!(max_period < 1e-8, "period error {max_period}");
        assert!(max_audio < 1e-7, "audio error {max_audio}");
        println!("{frames} DLL samples: period error {max_period}, output error {max_audio}");
    }
    #[test]
    fn formant_shaped_voice_and_note_transition() {
        // Synthetic glottal harmonics weighted around two vowel resonances,
        // followed by a new note. This catches fundamental/octave tracking and
        // stale control history that clean sine-only fixtures would miss.
        let rate = 48000.0;
        let mut audio: Vec<Frame> = (0..96000)
            .map(|i| {
                let hz = if i < 48000 { 215.0 } else { 250.0 };
                let phase = TAU * hz * i as f32 / rate;
                let value = (1..=16)
                    .map(|h| {
                        let frequency = hz * h as f32;
                        let envelope = 0.4
                            + 3.0 * (-((frequency - 700.0) / 200.0).powi(2)).exp()
                            + 2.0 * (-((frequency - 1200.0) / 250.0).powi(2)).exp();
                        0.1 * envelope / (h as f32) * (phase * h as f32).sin()
                    })
                    .sum::<f32>();
                [value; 2]
            })
            .collect();
        let mut effect = VoiceCorrection::new(rate);
        effect.process(&mut audio);
        fn power(audio: &[Frame], hz: f32) -> f32 {
            let (mut real, mut imaginary) = (0.0, 0.0);
            for (i, frame) in audio.iter().enumerate() {
                let phase = TAU * hz * i as f32 / 48000.0;
                real += frame[0] * phase.cos();
                imaginary += frame[0] * phase.sin();
            }
            real * real + imaginary * imaginary
        }
        for (slice, target, input) in [
            (&audio[24000..48000], 220.0, 215.0),
            (&audio[72000..96000], 246.94165, 250.0),
        ] {
            assert!(
                power(slice, target) > power(slice, input) * 10.0,
                "voiced target must dominate untuned pitch"
            );
        }
    }
    #[test]
    fn recovered_retune_and_humanize_laws() {
        let alpha = smoothing_alpha(48000.0, 20.0, 0.0, 0.0, 960);
        assert!(
            (alpha - 0.5).abs() < 0.00001,
            "20 ms must halve ratio error: {alpha}"
        );
        assert_eq!(smoothing_alpha(48000.0, 0.0, 0.0, 1000.0, 256), 0.0);
        let initial = smoothing_alpha(48000.0, 20.0, 100.0, 0.0, 256);
        let middle = smoothing_alpha(48000.0, 20.0, 100.0, 200.0, 256);
        let held = smoothing_alpha(48000.0, 20.0, 100.0, 400.0, 256);
        assert!(initial < middle && middle < held);
        assert!((held - smoothing_alpha(48000.0, 120.0, 0.0, 0.0, 256)).abs() < 1e-6);
    }
    #[test]
    fn low_high_voices_and_other_sample_rates_tune() {
        for (rate, hz, voice, target) in [
            (48000.0, 65.0, 2.0, 65.4064),
            (48000.0, 1080.0, 0.0, 1108.7305),
            (8000.0, 215.0, 1.0, 220.0),
            (44100.0, 430.0, 1.0, 440.0),
            (96000.0, 430.0, 1.0, 440.0),
        ] {
            let mut effect = VoiceCorrection::new(rate);
            effect.set_parameter("voice", voice).unwrap();
            effect.set_parameter("retune", 0.0).unwrap();
            let mut audio = tone(rate, hz, 1.0);
            effect.process(&mut audio);
            let measured = frequency(&audio[(rate * 0.5) as usize..], rate);
            assert!(
                (1200.0 * (measured / target).log2()).abs() < 8.0,
                "{rate}: {hz} became {measured}, wanted {target}"
            );
        }
    }
    #[test]
    fn routing_and_processor_bypass_shortcuts() {
        use crate::{
            effects::{EffectRegistry, EffectSpec},
            graph::Graph,
            project::{Connection, Node, NodeKind, Project},
            shortcuts::NodeAction,
            BLOCK_SIZE,
        };
        let registry = EffectRegistry::default();
        let source = Node::new(
            1,
            "Voice",
            NodeKind::Input {
                device: None,
                loopback: false,
            },
            [0.0; 2],
        );
        let mut processor = Node::new(2, "Correction", NodeKind::Processor, [0.0; 2]);
        processor
            .effects
            .push(EffectSpec::new(registry.get(EFFECT_ID).unwrap()));
        let output = Node::new(3, "Out", NodeKind::Output { device: None }, [0.0; 2]);
        let mut project = Project {
            version: 1,
            name: "Voice route".into(),
            nodes: vec![source, processor, output],
            connections: vec![
                Connection {
                    input: Default::default(),
                    port: Default::default(),
                    from: 1,
                    to: 2,
                },
                Connection {
                    input: Default::default(),
                    port: Default::default(),
                    from: 2,
                    to: 3,
                },
            ],
            sounds: vec![],
        };
        let mut graph = Graph::new(&project, &registry).unwrap();
        let input = tone(48000.0, 430.0, 1.0);
        let mut rendered = vec![];
        for block in input.as_chunks::<BLOCK_SIZE>().0 {
            graph.process(&[(1, *block)]);
            rendered.extend_from_slice(&graph.nodes[2].frames);
        }
        assert!((frequency(&rendered[24000..], 48000.0) - 440.0).abs() < 1.0);
        NodeAction::Disable.apply(&mut project.nodes[1]).unwrap();
        graph.update(&project, &registry).unwrap();
        let block: [[f32; 2]; BLOCK_SIZE] = input[..BLOCK_SIZE].try_into().unwrap();
        graph.process(&[(1, block)]);
        assert_eq!(graph.nodes[2].frames, block);
        NodeAction::Enable.apply(&mut project.nodes[1]).unwrap();
        graph.update(&project, &registry).unwrap();
        graph.process(&[(1, [[0.0; 2]; BLOCK_SIZE])]);
        assert!(graph.nodes[2].frames.iter().all(|f| *f == [0.0; 2]));
    }
    #[test]
    fn chromatic_tuning_and_transpose() {
        for (input, transpose, expected) in [
            (430.0, 0.0, 440.0),
            (250.0, 0.0, 246.94165),
            (430.0, 7.0, 659.2551),
            (220.0, -12.0, 110.0),
        ] {
            let mut effect = VoiceCorrection::new(48000.0);
            effect.set_parameter("retune", 0.0).unwrap();
            effect.set_parameter("transpose", transpose).unwrap();
            let mut audio = tone(48000.0, input, 1.0);
            for block in audio.chunks_mut(128) {
                effect.process(block);
            }
            let measured = frequency(&audio[24000..], 48000.0);
            assert!(
                (1200.0 * (measured / expected).log2()).abs() < 5.0,
                "{input} -> {measured}, wanted {expected}"
            );
            assert!(audio.iter().all(|f| (f[0] + f[1]).abs() < 1e-6));
        }
    }
    #[test]
    fn scale_key_and_hysteresis() {
        assert_eq!(target_note(62.9, 0, 0, None), 62);
        assert_eq!(target_note(63.0, 0, 1, None), 63);
        assert_eq!(target_note(61.0, 1, 0, None), 61);
        assert_eq!(target_note(60.52, 0, 2, Some(60)), 60);
        assert_eq!(target_note(60.6, 0, 2, Some(60)), 61);
    }
    #[test]
    fn dry_mix_latency_reset_and_block_independence() {
        let mut a = VoiceCorrection::new(48000.0);
        a.set_parameter("mix", 0.0).unwrap();
        let mut impulse = vec![[0.0; 2]; 4096];
        impulse[0] = [1.0, -1.0];
        a.process(&mut impulse);
        assert_eq!(impulse[LATENCY_FRAMES], [1.0, -1.0]);
        assert_eq!(impulse.iter().filter(|f| f[0] != 0.0).count(), 1);
        a.reset();
        let mut silence = vec![[0.0; 2]; 8192];
        a.process(&mut silence);
        assert!(silence.iter().all(|f| *f == [0.0; 2]));
        let mut a = VoiceCorrection::new(48000.0);
        let mut b = VoiceCorrection::new(48000.0);
        let mut x = tone(48000.0, 430.0, 0.3);
        let mut y = x.clone();
        a.process(&mut x);
        for block in y.chunks_mut(127) {
            b.set_parameter("retune", 20.0).unwrap();
            b.process(block);
        }
        assert_eq!(x, y);
    }
    #[test]
    fn noise_silence_and_supported_rates_are_safe() {
        for rate in [8000.0, 44100.0, 48000.0, 96000.0, 384000.0] {
            let mut effect = VoiceCorrection::new(rate);
            let mut seed = 12345u32;
            let mut noise: Vec<Frame> = (0..(rate * 0.15) as usize)
                .map(|_| {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    [(seed as i32 as f32 / i32::MAX as f32) * 0.1; 2]
                })
                .collect();
            effect.process(&mut noise);
            assert!(noise
                .iter()
                .flatten()
                .all(|v| v.is_finite() && v.abs() < 0.2));
            assert!(
                effect.detector.acquiring(),
                "noise detected as voice at {rate}"
            );
            effect.reset();
            let mut silence = vec![[0.0; 2]; 4096];
            effect.process(&mut silence);
            assert!(silence.iter().all(|f| *f == [0.0; 2]));
        }
    }
    #[test]
    fn factory_validation_and_project_roundtrip() {
        let registry = crate::effects::EffectRegistry::default();
        let mut spec = crate::effects::EffectSpec::new(registry.get(EFFECT_ID).unwrap());
        spec.parameters.insert("transpose".into(), 7.0);
        let json = serde_json::to_string(&spec).unwrap();
        let restored = serde_json::from_str(&json).unwrap();
        assert!(registry.create(&restored, 48000.0).is_ok());
        spec.parameters.insert("retune".into(), f32::NAN);
        assert!(registry.validate(&spec).is_err());
    }
}
