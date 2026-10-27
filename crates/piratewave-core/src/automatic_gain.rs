//! Linked stereo RMS automatic gain control. No allocations or I/O during process.
use crate::{
    effects::{db_to_gain, AudioEffect, EffectFactory, ParameterDescriptor},
    Frame,
};
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
const PARAMS: &[ParameterDescriptor] = &[
    p("target", "Target RMS", -36., -6., -18., "dB"),
    p("boost", "Maximum boost", 0., 30., 18., "dB"),
    p("reduction", "Maximum reduction", 0., 48., 30., "dB"),
    p("floor", "Noise floor", -80., -30., -55., "dB"),
    p("attack", "Reduce gain time", 1., 500., 20., "ms"),
    p("release", "Raise gain time", 50., 3000., 500., "ms"),
];
pub fn factory() -> EffectFactory {
    EffectFactory{id:"automatic_gain_control",name:"Automatic Gain Control",description:"Keep speech near a target RMS level with bounded boost, attenuation and a noise-floor guard",parameters:PARAMS,create:|rate|Box::new(Agc{rate,values:[-18.,18.,30.,-55.,20.,500.],power:0.,gain_db:0.})}
}
struct Agc {
    rate: f32,
    values: [f32; 6],
    power: f32,
    gain_db: f32,
}
impl AudioEffect for Agc {
    fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), String> {
        let i = PARAMS
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| format!("Unknown parameter {id}"))?;
        if !value.is_finite() || !(PARAMS[i].min..=PARAMS[i].max).contains(&value) {
            return Err(format!("Invalid {id}: {value}"));
        }
        self.values[i] = value;
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        let envelope = (-1. / (self.rate * 0.02)).exp();
        let down = (-1. / (self.rate * self.values[4] * 0.001)).exp();
        let up = (-1. / (self.rate * self.values[5] * 0.001)).exp();
        for frame in frames {
            let input_power = frame[0].powi(2).max(frame[1].powi(2));
            self.power = envelope * self.power + (1. - envelope) * input_power;
            let level = 10. * self.power.max(1e-16).log10();
            let desired = if level > self.values[3] {
                (self.values[0] - level).clamp(-self.values[2], self.values[1])
            } else {
                0.
            };
            let coeff = if desired < self.gain_db { down } else { up };
            self.gain_db = desired + (self.gain_db - desired) * coeff;
            let gain = db_to_gain(self.gain_db);
            for sample in frame {
                *sample *= gain;
            }
        }
    }
    fn reset(&mut self) {
        self.power = 0.;
        self.gain_db = 0.;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn rms(frames: &[Frame]) -> f32 {
        (frames.iter().map(|f| f[0] * f[0]).sum::<f32>() / frames.len() as f32).sqrt()
    }
    #[test]
    fn controls_quiet_and_loud_speech_without_boosting_silence() {
        for amplitude in [0.03, 0.8] {
            let mut effect = (factory().create)(48000.);
            let mut audio: Vec<_> = (0..144000)
                .map(|i| {
                    let x = amplitude * (std::f32::consts::TAU * 200. * i as f32 / 48000.).sin();
                    [x, x * 0.5]
                })
                .collect();
            effect.process(&mut audio);
            let level = 20. * rms(&audio[120000..]).log10();
            assert!((level + 18.).abs() < 0.7, "{amplitude}: {level}");
            assert!(audio.iter().all(|f| (f[0] * 0.5 - f[1]).abs() < 1e-6));
        }
        let mut effect = (factory().create)(48000.);
        let mut noise = vec![[0.00001; 2]; 144000];
        effect.process(&mut noise);
        assert!(noise.iter().all(|f| f[0] <= 0.000011));
        effect.reset();
        let mut silence = [[0.; 2]; 128];
        effect.process(&mut silence);
        assert_eq!(silence, [[0.; 2]; 128]);
    }
    #[test]
    fn rejects_invalid_parameters_and_is_partition_independent() {
        let mut a = (factory().create)(48000.);
        let mut b = (factory().create)(48000.);
        let mut whole = vec![[0.1; 2]; 10000];
        let mut split = whole.clone();
        a.process(&mut whole);
        for block in split.chunks_mut(97) {
            b.process(block);
        }
        assert_eq!(whole, split);
        assert!(a.set_parameter("target", f32::NAN).is_err());
    }
}
