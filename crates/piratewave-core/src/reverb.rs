//! Stereo algorithmic room reverb: damped parallel combs followed by diffusion.
//! All delay storage is allocated at construction, never in the sample path.
use crate::{
    effects::{AudioEffect, EffectFactory, ParameterDescriptor},
    Frame,
};

const PARAMS: &[ParameterDescriptor] = &[
    ParameterDescriptor {
        id: "mix",
        label: "Wet mix",
        min: 0.0,
        max: 100.0,
        default: 20.0,
        unit: "%",
    },
    ParameterDescriptor {
        id: "decay",
        label: "Decay",
        min: 0.1,
        max: 10.0,
        default: 1.5,
        unit: "s",
    },
    ParameterDescriptor {
        id: "damping",
        label: "Damping",
        min: 0.0,
        max: 100.0,
        default: 50.0,
        unit: "%",
    },
    ParameterDescriptor {
        id: "predelay",
        label: "Pre-delay",
        min: 0.0,
        max: 200.0,
        default: 20.0,
        unit: "ms",
    },
    ParameterDescriptor {
        id: "width",
        label: "Stereo width",
        min: 0.0,
        max: 100.0,
        default: 100.0,
        unit: "%",
    },
];

pub fn factory() -> EffectFactory {
    EffectFactory {
        id: "reverb",
        name: "Reverb",
        description: "Add room ambience with a diffuse stereo tail",
        parameters: PARAMS,
        create: |rate| Box::new(Reverb::new(rate)),
    }
}

struct Delay {
    data: Vec<f32>,
    position: usize,
}
impl Delay {
    fn new(seconds: f32, rate: f32) -> Self {
        Self {
            data: vec![0.0; (seconds * rate).round().max(1.0) as usize],
            position: 0,
        }
    }
    fn read(&self) -> f32 {
        self.data[self.position]
    }
    fn write(&mut self, value: f32) {
        // Avoid costly subnormal arithmetic in long, inaudible tails.
        self.data[self.position] = if value.abs() < 1.0e-20 { 0.0 } else { value };
        self.position = (self.position + 1) % self.data.len();
    }
    fn reset(&mut self) {
        self.data.fill(0.0);
        self.position = 0;
    }
}
struct Comb {
    delay: Delay,
    low: f32,
    feedback: f32,
    target: f32,
}
struct Channel {
    combs: Vec<Comb>,
    diffusers: Vec<Delay>,
}
struct Reverb {
    rate: f32,
    channels: [Channel; 2],
    pre: Vec<Frame>,
    position: usize,
    values: [f32; 5],
    current: [f32; 5],
    smoothing: f32,
    initialized: bool,
}
impl Reverb {
    fn new(rate: f32) -> Self {
        let values = [20.0, 1.5, 50.0, 20.0, 100.0];
        let channels = std::array::from_fn(|channel| {
            let offset = channel as f32 * 0.00073;
            Channel {
                combs: [
                    0.0297, 0.0323, 0.0349, 0.0371, 0.0397, 0.0419, 0.0443, 0.0473,
                ]
                .into_iter()
                .map(|time| {
                    let delay = Delay::new(time + offset, rate);
                    let feedback =
                        10.0_f32.powf(-3.0 * delay.data.len() as f32 / (rate * values[1]));
                    Comb {
                        delay,
                        low: 0.0,
                        feedback,
                        target: feedback,
                    }
                })
                .collect(),
                diffusers: [0.0047, 0.0033, 0.0019, 0.0011]
                    .into_iter()
                    .map(|time| Delay::new(time + offset * 0.3, rate))
                    .collect(),
            }
        });
        Self {
            rate,
            channels,
            pre: vec![[0.0; 2]; (rate * 0.2).ceil() as usize + 2],
            position: 0,
            values,
            current: values,
            smoothing: 1.0 - (-1.0 / (0.02 * rate)).exp(),
            initialized: false,
        }
    }
}
impl AudioEffect for Reverb {
    fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), String> {
        let index = PARAMS
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| format!("Unknown reverb parameter: {id}"))?;
        let p = &PARAMS[index];
        if !value.is_finite() || value < p.min || value > p.max {
            return Err(format!("Invalid {id}: {value}"));
        }
        self.values[index] = value;
        if id == "decay" {
            for channel in &mut self.channels {
                for comb in &mut channel.combs {
                    // Undamped feedback reaches -60 dB after the requested time.
                    comb.target =
                        10.0_f32.powf(-3.0 * comb.delay.data.len() as f32 / (self.rate * value));
                }
            }
        }
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        if !self.initialized {
            self.reset();
        }
        for frame in frames {
            for (current, target) in self.current.iter_mut().zip(self.values) {
                *current += self.smoothing * (target - *current);
            }
            self.pre[self.position] = *frame;
            let tap = self.current[3] * self.rate * 0.001;
            let whole = tap.floor() as usize;
            let fraction = tap - whole as f32;
            let index = (self.position + self.pre.len() - whole) % self.pre.len();
            let previous = (index + self.pre.len() - 1) % self.pre.len();
            let input: Frame = std::array::from_fn(|c| {
                self.pre[index][c] * (1.0 - fraction) + self.pre[previous][c] * fraction
            });
            self.position = (self.position + 1) % self.pre.len();
            let mut wet = [0.0; 2];
            let damping = self.current[2] * 0.009;
            for (c, channel) in self.channels.iter_mut().enumerate() {
                let excitation = input[c] * 0.8 + input[1 - c] * 0.2;
                for comb in &mut channel.combs {
                    let delayed = comb.delay.read();
                    comb.low = delayed * (1.0 - damping) + comb.low * damping;
                    if comb.low.abs() < 1.0e-20 {
                        comb.low = 0.0;
                    }
                    comb.feedback += self.smoothing * (comb.target - comb.feedback);
                    comb.delay.write(excitation + comb.low * comb.feedback);
                    wet[c] += delayed * 0.125;
                }
                for diffuser in &mut channel.diffusers {
                    let delayed = diffuser.read();
                    let output = delayed - 0.5 * wet[c];
                    diffuser.write(wet[c] + 0.5 * output);
                    wet[c] = output;
                }
            }
            let mid = (wet[0] + wet[1]) * 0.5;
            let side = (wet[0] - wet[1]) * 0.5 * self.current[4] * 0.01;
            let mix = self.current[0] * 0.01;
            if mix != 0.0 {
                frame[0] = frame[0] * (1.0 - mix) + (mid + side) * mix;
                frame[1] = frame[1] * (1.0 - mix) + (mid - side) * mix;
            }
        }
    }
    fn reset(&mut self) {
        self.initialized = true;
        self.pre.fill([0.0; 2]);
        self.position = 0;
        self.current = self.values;
        for channel in &mut self.channels {
            for comb in &mut channel.combs {
                comb.delay.reset();
                comb.low = 0.0;
                comb.feedback = comb.target;
            }
            for diffuser in &mut channel.diffusers {
                diffuser.reset();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::{EffectRegistry, EffectSpec};
    fn instance(rate: f32, changes: &[(&str, f32)]) -> Box<dyn AudioEffect> {
        let registry = EffectRegistry::default();
        let mut spec = EffectSpec::new(registry.get("reverb").unwrap());
        for (id, v) in changes {
            spec.parameters.insert((*id).into(), *v);
        }
        let saved = serde_json::to_string(&spec).unwrap();
        let mut effect = registry
            .create(&serde_json::from_str(&saved).unwrap(), rate)
            .unwrap();
        effect.reset();
        effect
    }
    fn impulse(decay: f32) -> Vec<Frame> {
        let mut effect = instance(
            48000.0,
            &[("mix", 100.0), ("decay", decay), ("damping", 0.0)],
        );
        let mut frames = vec![[0.0; 2]; 48000 * 3];
        frames[0] = [1.0, 0.0];
        effect.process(&mut frames);
        frames
    }
    #[test]
    fn impulse_has_predelay_diffuse_stereo_tail_and_adjustable_decay() {
        let short = impulse(0.3);
        let long = impulse(3.0);
        assert!(long[..2300].iter().flatten().all(|v| *v == 0.0));
        assert!(long[2400..4800].iter().flatten().any(|v| v.abs() > 0.001));
        assert!(long.iter().any(|f| (f[0] - f[1]).abs() > 0.001));
        let energy = |audio: &[Frame]| {
            audio[48000..96000]
                .iter()
                .flatten()
                .map(|v| v * v)
                .sum::<f32>()
        };
        assert!(energy(&long) > 100.0 * energy(&short));
    }
    #[test]
    fn dry_is_exact_reset_clears_tails_and_partitioning_is_identical() {
        let mut dry = instance(48000.0, &[("mix", 0.0)]);
        let mut signal: Vec<Frame> = (0..9600)
            .map(|i| [(i as f32 * 0.1).sin() * 0.2, 0.1])
            .collect();
        let original = signal.clone();
        dry.process(&mut signal);
        assert_eq!(signal, original);
        let mut a = instance(48000.0, &[]);
        let mut b = instance(48000.0, &[]);
        a.set_parameter("decay", 5.0).unwrap();
        b.set_parameter("decay", 5.0).unwrap();
        a.process(&mut signal);
        let mut partitioned = original;
        for block in partitioned.chunks_mut(127) {
            b.process(block);
        }
        assert_eq!(signal, partitioned);
        a.reset();
        signal.fill([0.0; 2]);
        a.process(&mut signal);
        assert!(signal.iter().flatten().all(|v| *v == 0.0));
    }
    #[test]
    fn supported_rates_extremes_and_mono_width_stay_finite() {
        for rate in [8000.0, 44100.0, 48000.0, 96000.0, 192000.0, 384000.0] {
            let mut effect = instance(
                rate,
                &[
                    ("mix", 100.0),
                    ("decay", 10.0),
                    ("predelay", 200.0),
                    ("damping", 100.0),
                    ("width", 0.0),
                ],
            );
            let mut frames = vec![[0.0; 2]; rate as usize];
            frames[0] = [1.0, -0.5];
            effect.process(&mut frames);
            assert!(frames
                .iter()
                .all(|f| f[0].is_finite() && f[0].abs() < 2.0 && f[0] == f[1]));
            assert!(effect.set_parameter("decay", f32::NAN).is_err());
            assert!(effect.set_parameter("mix", 101.0).is_err());
        }
    }
}
