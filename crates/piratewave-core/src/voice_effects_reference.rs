// Frozen pre-optimization DSP, compiled only for sample-equivalence tests.
use super::*;
#[derive(Clone, Copy, Default)]
struct Band {
    z1: Frame,
    z2: Frame,
}
impl Band {
    fn run(&mut self, x: Frame, hz: f32, q: f32, rate: f32) -> Frame {
        let w = TAU * hz.min(rate * 0.45) / rate;
        let a = w.sin() / (2. * q);
        let b = a / (1. + a);
        let a1 = -2. * w.cos() / (1. + a);
        let a2 = (1. - a) / (1. + a);
        let mut y = [0.; 2];
        for c in 0..2 {
            y[c] = b * x[c] + self.z1[c];
            self.z1[c] = -a1 * y[c] + self.z2[c];
            self.z2[c] = -b * x[c] - a2 * y[c];
        }
        y
    }
}
struct LegacyVoiceEffect {
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
    envelopes: [Frame; 12],
    damp: Frame,
}
impl LegacyVoiceEffect {
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
impl AudioEffect for LegacyVoiceEffect {
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
        let mix = self.value("mix");
        for frame in frames {
            let dry = frame.map(|x| if x.is_finite() { x } else { 0. });
            let mut wet = dry;
            let lfo = (TAU * self.phase).sin();
            let mut stored = dry;
            match self.kind {
                Kind::Distortion => {
                    let drive = self.value("drive");
                    let gain = 10_f32.powf(self.value("output") / 20.);
                    for c in 0..2 {
                        let x = dry[c] * drive;
                        wet[c] = gain
                            * match self.value("mode").round() as u8 {
                                1 => x.clamp(-1., 1.),
                                2 => ((x + 1.).rem_euclid(4.) - 2.).abs() - 1.,
                                _ => x.tanh(),
                            };
                    }
                }
                Kind::Tremolo => {
                    let wave = if self.value("mode").round() == 1. {
                        if lfo >= 0. {
                            1.
                        } else {
                            0.
                        }
                    } else {
                        0.5 + 0.5 * lfo
                    };
                    let g = 1. - self.value("depth") * (1. - wave);
                    wet = dry.map(|x| x * g);
                }
                Kind::Stereo => {
                    let mid = (dry[0] + dry[1]) * 0.5;
                    let side = (dry[0] - dry[1]) * 0.5 * self.value("width");
                    stored = [mid + side, mid - side];
                    let right = if self.value("delay") == 0. {
                        stored[1]
                    } else {
                        self.tap(self.value("delay") * self.rate / 1000., 1)
                    };
                    let balance = self.value("balance");
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
                        let d = (base + self.value("depth") * (0.5 + 0.5 * modulation)) * self.rate
                            / 1000.;
                        wet[c] = self.tap(d, c);
                        if chorus {
                            wet[c] = (wet[c]
                                + self.tap(
                                    (25. + self.value("depth") * (0.5 - 0.5 * modulation))
                                        * self.rate
                                        / 1000.,
                                    c,
                                ))
                                * 0.5;
                        }
                        stored[c] = dry[c] + wet[c] * self.value("feedback");
                    }
                }
                Kind::Resonator => {
                    for c in 0..2 {
                        let v = self.tap(self.rate / self.value("frequency"), c);
                        self.damp[c] =
                            v * (1. - self.value("damping")) + self.damp[c] * self.value("damping");
                        stored[c] = dry[c] + self.damp[c] * self.value("feedback");
                        wet[c] = self.damp[c] * (1. - self.value("feedback"));
                    }
                }
                Kind::Wah => {
                    let hz = self.value("frequency")
                        * 2_f32.powf(self.value("depth") * (0.5 + 0.5 * lfo));
                    wet = self.bands[0].run(dry, hz, self.value("q"), self.rate);
                }
                Kind::Vocoder => {
                    let mode = self.value("mode").round() as u8;
                    let mut carrier = 0.;
                    for i in 0..3 {
                        let ratio = if mode == 2 {
                            [1., 1.259921, 1.498307][i]
                        } else {
                            1.
                        };
                        self.carrier_phase[i] = (self.carrier_phase[i]
                            + self.value("carrier") * ratio / self.rate)
                            .fract();
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
                    let release = (-1. / (self.rate * self.value("release") * 0.001)).exp();
                    let attack = (-1. / (self.rate * 0.002)).exp();
                    for i in 0..12 {
                        let hz = 100. * (60_f32).powf(i as f32 / 11.);
                        let speech = self.bands[i].run(dry, hz, 2.2, self.rate);
                        let synth = self.carrier_bands[i].run([carrier; 2], hz, 2.2, self.rate);
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
            self.phase = (self.phase + self.value("rate") / self.rate).fract();
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

#[test]
fn optimized_effects_match_original_samples_through_edits_and_reset() {
    for (kind, descriptors) in [
        (Kind::Vocoder, VOCODER),
        (Kind::Resonator, RESONATOR),
        (Kind::Distortion, DISTORTION),
        (Kind::Flanger, FLANGER),
        (Kind::Chorus, CHORUS),
        (Kind::Tremolo, TREMOLO),
        (Kind::Stereo, STEREO),
        (Kind::Wah, WAH),
    ] {
        for rate in [8000., 44100., 48000., 96000., 384000.] {
            let mut original = LegacyVoiceEffect::new(rate, kind, descriptors);
            let mut optimized = VoiceEffect::new(rate, kind, descriptors);
            let mut seed = 42_u32;
            for edit in 0..8 {
                for p in descriptors {
                    let value = match edit {
                        0 | 7 => p.default,
                        1 => p.min,
                        2 => p.max,
                        _ => p.min + (p.max - p.min) * (edit as f32 / 8.),
                    };
                    original.set_parameter(p.id, value).unwrap();
                    optimized.set_parameter(p.id, value).unwrap();
                }
                if edit == 7 {
                    original.reset();
                    optimized.reset();
                }
                for block in 0..40 {
                    let mut expected: Vec<Frame> = (0..[1, 97, 128, 257][block % 4])
                        .map(|_| {
                            std::array::from_fn(|_| {
                                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                                if block > 20 {
                                    0.
                                } else {
                                    (seed as f32 / u32::MAX as f32 - 0.5) * 2.
                                }
                            })
                        })
                        .collect();
                    if block == 10 {
                        expected[0] = [f32::NAN, f32::INFINITY];
                    }
                    let mut actual = expected.clone();
                    original.process(&mut expected);
                    optimized.process(&mut actual);
                    for (a, b) in actual.iter().flatten().zip(expected.iter().flatten()) {
                        assert_eq!(
                            a.to_bits(),
                            b.to_bits(),
                            "{} rate={rate} edit={edit} block={block}",
                            descriptors[0].id
                        );
                    }
                }
            }
        }
    }
}
