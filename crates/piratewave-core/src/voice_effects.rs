//! Native creative voice processors. All delay/filter storage is allocated at creation.
use crate::{
    effects::{AudioEffect, EffectFactory, ParameterDescriptor},
    Frame,
};
use std::f32::consts::TAU;
const fn p(
    id: &'static str,
    label: &'static str,
    min: f32,
    max: f32,
    default: f32,
    unit: &'static str,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id,
        label,
        min,
        max,
        default,
        unit,
    }
}
const MIX: ParameterDescriptor = p("mix", "Wet / dry", 0., 1., 0.5, "");
const VOCODER: &[ParameterDescriptor] = &[
    p("mode", "Character", 0., 3., 0., ""),
    p("carrier", "Carrier pitch", 40., 400., 110., "Hz"),
    p("release", "Envelope release", 10., 300., 70., "ms"),
    MIX,
];
const RESONATOR: &[ParameterDescriptor] = &[
    p("frequency", "Resonance pitch", 40., 1600., 220., "Hz"),
    p("feedback", "Resonance", 0., 0.95, 0.75, ""),
    p("damping", "Damping", 0., 0.95, 0.4, ""),
    MIX,
];
const DISTORTION: &[ParameterDescriptor] = &[
    p("drive", "Drive", 1., 40., 5., ""),
    p("mode", "Style", 0., 2., 0., ""),
    p("output", "Output", -30., 0., -9., "dB"),
    MIX,
];
const PITCH: &[ParameterDescriptor] = &[p("semitones", "Pitch shift", -24., 24., 0., "st"), MIX];
const FLANGER: &[ParameterDescriptor] = &[
    p("rate", "Sweep rate", 0.05, 5., 0.3, "Hz"),
    p("depth", "Sweep depth", 0., 5., 2., "ms"),
    p("feedback", "Feedback", -0.9, 0.9, 0.4, ""),
    MIX,
];
const CHORUS: &[ParameterDescriptor] = &[
    p("rate", "Modulation rate", 0.05, 5., 0.8, "Hz"),
    p("depth", "Depth", 0., 10., 4., "ms"),
    MIX,
];
const TREMOLO: &[ParameterDescriptor] = &[
    p("rate", "Pulse rate", 0.1, 20., 4., "Hz"),
    p("depth", "Depth", 0., 1., 0.7, ""),
    p("mode", "Waveform", 0., 1., 0., ""),
];
const STEREO: &[ParameterDescriptor] = &[
    p("width", "Stereo width", 0., 2., 1., ""),
    p("delay", "Right channel delay", 0., 30., 8., "ms"),
    p("balance", "Balance", -1., 1., 0., ""),
];
const WAH: &[ParameterDescriptor] = &[
    p("rate", "Sweep rate", 0.1, 8., 1.5, "Hz"),
    p("frequency", "Base frequency", 100., 1000., 300., "Hz"),
    p("depth", "Sweep octaves", 0., 4., 2., ""),
    p("q", "Resonance", 0.5, 8., 3., ""),
    MIX,
];
#[derive(Clone, Copy)]
enum Kind {
    Vocoder,
    Resonator,
    Distortion,
    Flanger,
    Chorus,
    Tremolo,
    Stereo,
    Wah,
}
pub fn is_voice_effect(id: &str) -> bool {
    matches!(
        id,
        "automatic_voice_correction"
            | "delay"
            | "vocoder"
            | "resonator"
            | "distortion"
            | "pitch"
            | "flanger"
            | "chorus"
            | "tremolo"
            | "stereo"
            | "wahwah"
    )
}
pub fn choices(effect: &str, id: &str) -> Option<&'static [&'static str]> {
    match (effect, id) {
        ("vocoder", "mode") => Some(&["Classic saw", "Robot", "Chord", "Pulse"]),
        ("distortion", "mode") => Some(&["Warm saturation", "Hard clip", "Foldback"]),
        ("tremolo", "mode") => Some(&["Sine", "Square"]),
        _ => None,
    }
}
pub fn factories() -> Vec<EffectFactory> {
    vec![
EffectFactory { id:"vocoder", name:"Vocoder", description:"Twelve-band speech envelopes shape a synthesized carrier; choose Classic saw, Robot, Chord or Pulse", parameters:VOCODER, create:|r| Box::new(VoiceEffect::new(r, Kind::Vocoder, VOCODER)) },
EffectFactory { id:"resonator", name:"Resonator", description:"A damped tuned feedback comb adds a ringing vocal resonance", parameters:RESONATOR, create:|r| Box::new(VoiceEffect::new(r, Kind::Resonator, RESONATOR)) },
EffectFactory { id:"distortion", name:"Distortion", description:"Warm saturation, hard clipping or foldback with output trim", parameters:DISTORTION, create:|r| Box::new(VoiceEffect::new(r, Kind::Distortion, DISTORTION)) },
EffectFactory { id:"pitch", name:"Pitch", description:"Fixed voice transposition with phase-aligned delay heads; about 29 ms latency, no formant preservation", parameters:PITCH, create:|r| Box::new(crate::voice_correction::PitchShift::new(r)) },
EffectFactory { id:"flanger", name:"Flanger", description:"Short swept stereo delay with positive or negative feedback", parameters:FLANGER, create:|r| Box::new(VoiceEffect::new(r, Kind::Flanger, FLANGER)) },
EffectFactory { id:"chorus", name:"Chorus", description:"Two detuned stereo delay voices add width and doubling", parameters:CHORUS, create:|r| Box::new(VoiceEffect::new(r, Kind::Chorus, CHORUS)) },
EffectFactory { id:"tremolo", name:"Tremolo", description:"Rhythmic amplitude modulation with sine or square waveform", parameters:TREMOLO, create:|r| Box::new(VoiceEffect::new(r, Kind::Tremolo, TREMOLO)) },
EffectFactory { id:"stereo", name:"Stereo", description:"Mid/side width, right-channel Haas delay and balance", parameters:STEREO, create:|r| Box::new(VoiceEffect::new(r, Kind::Stereo, STEREO)) },
EffectFactory { id:"wahwah", name:"WahWah", description:"Sweeping resonant band-pass filter for talking vowel tones", parameters:WAH, create:|r| Box::new(VoiceEffect::new(r, Kind::Wah, WAH)) },
    ]
}
#[derive(Clone, Copy, Default)]
struct Band {
    z1: Frame,
    z2: Frame,
}
impl Band {
    fn run(&mut self, x: Frame, hz: f32, q: f32, rate: f32) -> Frame {
        self.run_coefficients(x, BandCoefficients::new(hz, q, rate))
    }
    fn run_coefficients(&mut self, x: Frame, coefficients: BandCoefficients) -> Frame {
        let BandCoefficients { b, a1, a2 } = coefficients;
        let mut y = [0.; 2];
        for c in 0..2 {
            y[c] = b * x[c] + self.z1[c];
            self.z1[c] = -a1 * y[c] + self.z2[c];
            self.z2[c] = -b * x[c] - a2 * y[c];
        }
        y
    }
}
#[derive(Clone, Copy)]
struct BandCoefficients {
    b: f32,
    a1: f32,
    a2: f32,
}
impl BandCoefficients {
    fn new(hz: f32, q: f32, rate: f32) -> Self {
        let w = TAU * hz.min(rate * 0.45) / rate;
        let a = w.sin() / (2. * q);
        let b = a / (1. + a);
        let a1 = -2. * w.cos() / (1. + a);
        let a2 = (1. - a) / (1. + a);
        Self { b, a1, a2 }
    }
}
struct VoiceEffect {
    rate: f32,
    kind: Kind,
    descriptors: &'static [ParameterDescriptor],
    values: [f32; 6],
    buffer: Vec<Frame>,
    write: usize,
    phase: f32,
    carrier_phase: [f32; 3],
    bands: [Band; 12],
    carrier_bands: [Band; 12],
    vocoder_coefficients: [BandCoefficients; 12],
    envelopes: [Frame; 12],
    damp: Frame,
}
impl VoiceEffect {
    fn new(rate: f32, kind: Kind, descriptors: &'static [ParameterDescriptor]) -> Self {
        let mut values = [0.; 6];
        for (i, p) in descriptors.iter().enumerate() {
            values[i] = p.default;
        }
        Self {
            rate,
            kind,
            descriptors,
            values,
            buffer: vec![[0.; 2]; (rate * 0.15).ceil() as usize + 4],
            write: 0,
            phase: 0.,
            carrier_phase: [0.; 3],
            bands: [Band::default(); 12],
            carrier_bands: [Band::default(); 12],
            vocoder_coefficients: std::array::from_fn(|i| {
                BandCoefficients::new(100. * (60_f32).powf(i as f32 / 11.), 2.2, rate)
            }),
            envelopes: [[0.; 2]; 12],
            damp: [0.; 2],
        }
    }
    fn value(&self, id: &str) -> f32 {
        self.descriptors
            .iter()
            .position(|p| p.id == id)
            .map_or(0., |i| self.values[i])
    }
    fn tap(&self, delay: f32, channel: usize) -> f32 {
        let pos = (self.write as f32 - delay.clamp(1., self.buffer.len() as f32 - 2.))
            .rem_euclid(self.buffer.len() as f32);
        let i = pos.floor() as usize;
        let frac = pos - i as f32;
        self.buffer[i][channel] * (1. - frac)
            + self.buffer[(i + 1) % self.buffer.len()][channel] * frac
    }
}
impl AudioEffect for VoiceEffect {
    fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), String> {
        let i = self
            .descriptors
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| format!("Unknown parameter {id}"))?;
        let p = &self.descriptors[i];
        if !value.is_finite() || value < p.min || value > p.max {
            return Err(format!("Invalid {id}"));
        }
        self.values[i] = value;
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        // Parameters are immutable during a block. Preserve the original equations.
        let parameter_mix = self.value("mix");
        let parameter_drive = self.value("drive");
        let parameter_output = self.value("output");
        let parameter_mode = self.value("mode");
        let parameter_depth = self.value("depth");
        let parameter_width = self.value("width");
        let parameter_delay = self.value("delay");
        let parameter_balance = self.value("balance");
        let parameter_feedback = self.value("feedback");
        let parameter_frequency = self.value("frequency");
        let parameter_damping = self.value("damping");
        let parameter_q = self.value("q");
        let parameter_carrier = self.value("carrier");
        let parameter_release = self.value("release");
        let parameter_rate = self.value("rate");
        let mix = parameter_mix;
        let distortion_gain = if matches!(self.kind, Kind::Distortion) {
            10_f32.powf(parameter_output / 20.)
        } else {
            0.
        };
        let (release, attack) = if matches!(self.kind, Kind::Vocoder) {
            (
                (-1. / (self.rate * parameter_release * 0.001)).exp(),
                (-1. / (self.rate * 0.002)).exp(),
            )
        } else {
            (0., 0.)
        };
        for frame in frames {
            let dry = frame.map(|x| if x.is_finite() { x } else { 0. });
            let mut wet = dry;
            let lfo = (TAU * self.phase).sin();
            let mut stored = dry;
            match self.kind {
                Kind::Distortion => {
                    let drive = parameter_drive;
                    let gain = distortion_gain;
                    for c in 0..2 {
                        let x = dry[c] * drive;
                        wet[c] = gain
                            * match parameter_mode.round() as u8 {
                                1 => x.clamp(-1., 1.),
                                2 => ((x + 1.).rem_euclid(4.) - 2.).abs() - 1.,
                                _ => x.tanh(),
                            };
                    }
                }
                Kind::Tremolo => {
                    let wave = if parameter_mode.round() == 1. {
                        if lfo >= 0. {
                            1.
                        } else {
                            0.
                        }
                    } else {
                        0.5 + 0.5 * lfo
                    };
                    let g = 1. - parameter_depth * (1. - wave);
                    wet = dry.map(|x| x * g);
                }
                Kind::Stereo => {
                    let mid = (dry[0] + dry[1]) * 0.5;
                    let side = (dry[0] - dry[1]) * 0.5 * parameter_width;
                    stored = [mid + side, mid - side];
                    let right = if parameter_delay == 0. {
                        stored[1]
                    } else {
                        self.tap(parameter_delay * self.rate / 1000., 1)
                    };
                    let balance = parameter_balance;
                    wet = [
                        stored[0] * (1. - balance.max(0.)),
                        right * (1. + balance.min(0.)),
                    ];
                }
                Kind::Flanger | Kind::Chorus => {
                    let chorus = matches!(self.kind, Kind::Chorus);
                    let base = if chorus { 18. } else { 1. };
                    for c in 0..2 {
                        let modulation = (TAU * (self.phase + c as f32 * 0.25)).sin();
                        let d =
                            (base + parameter_depth * (0.5 + 0.5 * modulation)) * self.rate / 1000.;
                        wet[c] = self.tap(d, c);
                        if chorus {
                            wet[c] = (wet[c]
                                + self.tap(
                                    (25. + parameter_depth * (0.5 - 0.5 * modulation)) * self.rate
                                        / 1000.,
                                    c,
                                ))
                                * 0.5;
                        }
                        stored[c] = dry[c] + wet[c] * parameter_feedback;
                    }
                }
                Kind::Resonator => {
                    for c in 0..2 {
                        let v = self.tap(self.rate / parameter_frequency, c);
                        self.damp[c] =
                            v * (1. - parameter_damping) + self.damp[c] * parameter_damping;
                        stored[c] = dry[c] + self.damp[c] * parameter_feedback;
                        wet[c] = self.damp[c] * (1. - parameter_feedback);
                    }
                }
                Kind::Wah => {
                    let hz = parameter_frequency * 2_f32.powf(parameter_depth * (0.5 + 0.5 * lfo));
                    wet = self.bands[0].run(dry, hz, parameter_q, self.rate);
                }
                Kind::Vocoder => {
                    let mode = parameter_mode.round() as u8;
                    let mut carrier = 0.;
                    for i in 0..3 {
                        let ratio = if mode == 2 {
                            [1., 1.259921, 1.498307][i]
                        } else {
                            1.
                        };
                        self.carrier_phase[i] =
                            (self.carrier_phase[i] + parameter_carrier * ratio / self.rate).fract();
                        let ph = self.carrier_phase[i];
                        carrier += match mode {
                            1 => (TAU * ph).sin(),
                            3 => {
                                if ph < 0.3 {
                                    0.7
                                } else {
                                    -0.3
                                }
                            }
                            _ => 2. * ph - 1.,
                        };
                        if mode != 2 {
                            break;
                        }
                    }
                    if mode == 2 {
                        carrier /= 3.;
                    }
                    wet = [0.; 2];
                    for i in 0..12 {
                        let coefficients = self.vocoder_coefficients[i];
                        let speech = self.bands[i].run_coefficients(dry, coefficients);
                        let synth =
                            self.carrier_bands[i].run_coefficients([carrier; 2], coefficients);
                        for c in 0..2 {
                            let level = speech[c].abs();
                            let a = if level > self.envelopes[i][c] {
                                attack
                            } else {
                                release
                            };
                            self.envelopes[i][c] = level + (self.envelopes[i][c] - level) * a;
                            wet[c] += synth[c] * self.envelopes[i][c] * 10.;
                        }
                    }
                }
            }
            self.buffer[self.write] = stored.map(|x| {
                if x.is_finite() {
                    x.clamp(-16., 16.)
                } else {
                    0.
                }
            });
            self.write = (self.write + 1) % self.buffer.len();
            self.phase = (self.phase + parameter_rate / self.rate).fract();
            let blend = if matches!(self.kind, Kind::Tremolo | Kind::Stereo) {
                1.
            } else {
                mix
            };
            for c in 0..2 {
                frame[c] = dry[c] * (1. - blend) + wet[c] * blend;
            }
        }
    }
    fn reset(&mut self) {
        self.buffer.fill([0.; 2]);
        self.write = 0;
        self.phase = 0.;
        self.carrier_phase = [0.; 3];
        self.bands = [Band::default(); 12];
        self.carrier_bands = [Band::default(); 12];
        self.envelopes = [[0.; 2]; 12];
        self.damp = [0.; 2];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::{EffectRegistry, EffectSpec};
    fn signal(n: usize, rate: f32) -> Vec<Frame> {
        (0..n)
            .map(|i| [(TAU * 220. * i as f32 / rate).sin() * 0.2; 2])
            .collect()
    }
    #[test]
    fn all_effects_are_finite_at_extremes_reset_and_keep_block_history() {
        let registry = EffectRegistry::default();
        for factory in factories() {
            for rate in [8000., 44100., 48000., 96000., 384000.] {
                for extreme in [false, true] {
                    let mut spec = EffectSpec::new(&factory);
                    for p in factory.parameters {
                        spec.parameters
                            .insert(p.id.into(), if extreme { p.max } else { p.min });
                    }
                    let mut fx = registry.create(&spec, rate).unwrap();
                    let mut audio = signal(4096, rate);
                    fx.process(&mut audio);
                    assert!(
                        audio.iter().flatten().all(|v| v.is_finite()),
                        "{} {rate}",
                        factory.id
                    );
                    assert!(!fx.faulted(), "{} faulted at {rate}", factory.id);
                    fx.reset();
                    let mut silence = vec![[0.; 2]; 4096];
                    fx.process(&mut silence);
                    assert!(
                        silence.iter().flatten().all(|v| v.abs() < 1e-7),
                        "{} reset",
                        factory.id
                    );
                }
            }
            let spec = EffectSpec::new(&factory);
            let mut a = registry.create(&spec, 48000.).unwrap();
            let mut b = registry.create(&spec, 48000.).unwrap();
            let mut whole = signal(10001, 48000.);
            let mut split = whole.clone();
            a.process(&mut whole);
            for block in split.chunks_mut(97) {
                b.process(block);
            }
            assert_eq!(whole, split, "{} depends on block boundaries", factory.id);
            for p in factory.parameters {
                assert!(a.set_parameter(p.id, f32::NAN).is_err());
            }
        }
    }
    fn tone_energy(frames: &[Frame], hz: f32) -> f32 {
        let (mut re, mut im) = (0., 0.);
        for (i, f) in frames.iter().enumerate() {
            let phase = TAU * hz * i as f32 / 48000.;
            re += f[0] * phase.cos();
            im += f[0] * phase.sin();
        }
        (re * re + im * im).sqrt() / frames.len() as f32
    }
    #[test]
    fn pitch_transposes_an_octave_in_both_directions() {
        let reg = EffectRegistry::default();
        for (shift, target) in [(12., 440.), (-12., 110.)] {
            let mut spec = EffectSpec::new(reg.get("pitch").unwrap());
            spec.parameters.insert("semitones".into(), shift);
            spec.parameters.insert("mix".into(), 1.);
            let mut fx = reg.create(&spec, 48000.).unwrap();
            let mut frames = signal(48000, 48000.);
            fx.process(&mut frames);
            let tail = &frames[12000..];
            let wanted = tone_energy(tail, target);
            let original = tone_energy(tail, 220.);
            assert!(
                wanted > 0.04 && wanted > original * 5.,
                "{shift}: wanted={wanted}, original={original}"
            );
        }
    }
    #[test]
    fn vocoder_characters_are_distinct_and_follow_speech_envelopes() {
        let reg = EffectRegistry::default();
        let mut results = vec![];
        for mode in 0..4 {
            let mut spec = EffectSpec::new(reg.get("vocoder").unwrap());
            spec.parameters.insert("mode".into(), mode as f32);
            spec.parameters.insert("mix".into(), 1.);
            let mut fx = reg.create(&spec, 48000.).unwrap();
            let mut frames = signal(12000, 48000.);
            fx.process(&mut frames);
            assert!(frames.iter().any(|f| f[0].abs() > 0.001));
            results.push(frames);
            let mut silence = vec![[0.; 2]; 48000];
            fx.process(&mut silence);
            assert!(silence[47000..].iter().all(|f| f[0].abs() < 0.0001));
        }
        for i in 0..4 {
            for j in i + 1..4 {
                assert!(results[i]
                    .iter()
                    .zip(&results[j])
                    .any(|(a, b)| (a[0] - b[0]).abs() > 0.001));
            }
        }
    }
    #[test]
    fn resonator_echoes_at_tuned_period_and_tremolo_changes_amplitude() {
        let reg = EffectRegistry::default();
        let mut spec = EffectSpec::new(reg.get("resonator").unwrap());
        spec.parameters.insert("frequency".into(), 240.);
        spec.parameters.insert("damping".into(), 0.);
        spec.parameters.insert("mix".into(), 1.);
        let mut fx = reg.create(&spec, 48000.).unwrap();
        let mut frames = vec![[0.; 2]; 1000];
        frames[0] = [1.; 2];
        fx.process(&mut frames);
        assert!(frames[200][0] > 0.2);
        assert!(frames[400][0] > 0.1);
        assert_eq!(frames[199], [0.; 2]);
        let mut fx = reg
            .create(&EffectSpec::new(reg.get("tremolo").unwrap()), 48000.)
            .unwrap();
        let mut frames = vec![[0.5; 2]; 48000];
        fx.process(&mut frames);
        let min = frames.iter().map(|f| f[0]).fold(f32::INFINITY, f32::min);
        let max = frames.iter().map(|f| f[0]).fold(0., f32::max);
        assert!(min < 0.16 && max > 0.49);
    }
}

#[cfg(test)]
#[path = "voice_effects_reference.rs"]
mod reference;
