//! Native rack processors. Control changes preserve history; processing is allocation-free.
use crate::{
    effects::{AudioEffect, EffectFactory, ParameterDescriptor},
    Frame,
};
mod suppression;
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
const DYNAMICS: &[ParameterDescriptor] = &[
    p("mode", "Direction", 0., 1., 0., ""),
    p("threshold", "Threshold", -80., 0., -35., "dB"),
    p("ratio", "Ratio", 1., 10., 2., ":1"),
    p("attack", "Attack", 0.1, 100., 5., "ms"),
    p("release", "Release", 10., 1000., 120., "ms"),
    p("range", "Maximum change", 0., 60., 24., "dB"),
    p("detection", "Detection", 0., 1., 0., ""),
    p("output", "Output gain", -24., 24., 0., "dB"),
];
const UPWARD: &[ParameterDescriptor] = &[
    p("threshold", "Threshold", -80., 0., -30., "dB"),
    p("ratio", "Ratio", 1., 10., 2., ":1"),
    p("attack", "Attack", 0.1, 100., 10., "ms"),
    p("release", "Release", 10., 1000., 150., "ms"),
    p("range", "Maximum boost", 0., 36., 12., "dB"),
    p("detection", "Detection", 0., 1., 0., ""),
    p("output", "Output gain", -24., 24., 0., "dB"),
];
const DEESS: &[ParameterDescriptor] = &[
    p("frequency", "Sibilance cutoff", 2000., 12000., 4500., "Hz"),
    p("threshold", "Threshold", -60., 0., -30., "dB"),
    p("ratio", "Ratio", 1., 10., 4., ":1"),
    p("range", "Maximum reduction", 0., 24., 12., "dB"),
    p("attack", "Attack", 0.1, 50., 2., "ms"),
    p("release", "Release", 10., 500., 70., "ms"),
    p("mode", "Processing", 0., 1., 0., ""),
];
const LIMITER: &[ParameterDescriptor] = &[
    p("ceiling", "Ceiling", -30., 0., -1., "dB"),
    p("release", "Release", 10., 1000., 60., "ms"),
];
const EQ: &[ParameterDescriptor] = &[
    p("low", "Low", -24., 24., 0., "dB"),
    p("mid", "Mid", -24., 24., 0., "dB"),
    p("high", "High", -24., 24., 0., "dB"),
];
const POLARITY: &[ParameterDescriptor] = &[p("channels", "Channels", 0., 2., 0., "")];
pub fn choices(effect: &str, id: &str) -> Option<&'static [&'static str]> {
    match (effect, id) {
        ("expander", "mode") => Some(&["Downward", "Upward"]),
        ("expander" | "upward_compressor", "detection") => Some(&["RMS", "Peak"]),
        ("de_esser", "mode") => Some(&["Split band", "Broadband"]),
        ("invert_polarity", "channels") => Some(&["Both", "Left", "Right"]),
        _ => crate::voice_effects::choices(effect, id),
    }
}
pub fn factories() -> Vec<EffectFactory> {
    vec![
 EffectFactory{id:"expander",name:"Expander",description:"Expand dynamics downward below threshold or upward above it; linked stereo",parameters:DYNAMICS,create:|r|Box::new(Dynamics::new(r,false))},
 EffectFactory{id:"upward_compressor",name:"Upward Compressor",description:"Raise quieter speech below threshold with a bounded boost; may also raise room noise",parameters:UPWARD,create:|r|Box::new(Dynamics::new(r,true))},
 EffectFactory{id:"de_esser",name:"De-esser",description:"Tame sibilance using a high-frequency detector and split-band or broadband reduction",parameters:DEESS,create:|r|Box::new(Deesser::new(r))},
 EffectFactory{id:"limiter",name:"Limiter",description:"Instant sample-peak ceiling with linked stereo and release smoothing; place last",parameters:LIMITER,create:|r|Box::new(Limiter{rate:r,ceiling:-1.,release:60.,gain:1.})},
 EffectFactory{id:"invert_polarity",name:"Invert Polarity",description:"Reverse the sign of both channels or one channel to correct polarity cancellation",parameters:POLARITY,create:|_|Box::new(Polarity{channels:0})},
 EffectFactory{id:"three_band_equalizer",name:"3 Band Equalizer",description:"Low shelf at 200 Hz, mid bell at 1 kHz, and high shelf at 5 kHz",parameters:EQ,create:|r|Box::new(ThreeBand{rate:r,gains:[0.;3],filters:[Biquad::identity();3]})},
 suppression::factory(),
]
}
fn validate(parameters: &[ParameterDescriptor], id: &str, value: f32) -> Result<(), String> {
    let p = parameters
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("Unknown parameter {id}"))?;
    if !value.is_finite() || value < p.min || value > p.max {
        return Err(format!("Invalid {id}: {value}"));
    }
    Ok(())
}
fn amplitude(db: f32) -> f32 {
    10_f32.powf(db / 20.)
}
fn db(value: f32) -> f32 {
    20. * value.max(1e-12).log10()
}
fn coefficient(rate: f32, ms: f32) -> f32 {
    (-1. / (rate * ms * 0.001)).exp()
}
struct Dynamics {
    rate: f32,
    up_compress: bool,
    mode: usize,
    threshold: f32,
    ratio: f32,
    attack: f32,
    release: f32,
    range: f32,
    detection: usize,
    output: f32,
    power: f32,
    gain_db: f32,
}
impl Dynamics {
    fn new(rate: f32, up_compress: bool) -> Self {
        Self {
            rate,
            up_compress,
            mode: 0,
            threshold: if up_compress { -30. } else { -35. },
            ratio: 2.,
            attack: if up_compress { 10. } else { 5. },
            release: if up_compress { 150. } else { 120. },
            range: if up_compress { 12. } else { 24. },
            detection: 0,
            output: 0.,
            power: 0.,
            gain_db: 0.,
        }
    }
}
impl AudioEffect for Dynamics {
    fn set_parameter(&mut self, id: &str, v: f32) -> Result<(), String> {
        validate(if self.up_compress { UPWARD } else { DYNAMICS }, id, v)?;
        match id {
            "mode" => self.mode = v.round() as usize,
            "threshold" => self.threshold = v,
            "ratio" => self.ratio = v,
            "attack" => self.attack = v,
            "release" => self.release = v,
            "range" => self.range = v,
            "detection" => self.detection = v.round() as usize,
            "output" => self.output = v,
            _ => unreachable!(),
        }
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        let attack = coefficient(self.rate, self.attack);
        let release = coefficient(self.rate, self.release);
        let rms = coefficient(self.rate, 10.);
        for frame in frames {
            let peak = frame[0].abs().max(frame[1].abs());
            self.power = rms * self.power + (1. - rms) * peak * peak;
            let level = db(if self.detection == 0 {
                self.power.sqrt()
            } else {
                peak
            });
            let desired = if self.up_compress {
                (self.threshold - level).max(0.) * (1. - 1. / self.ratio)
            } else if self.mode == 0 {
                (level - self.threshold).min(0.) * (self.ratio - 1.)
            } else {
                (level - self.threshold).max(0.) * (self.ratio - 1.)
            };
            let desired = desired.clamp(-self.range, self.range);
            let a = if desired.abs() > self.gain_db.abs() {
                attack
            } else {
                release
            };
            self.gain_db = a * self.gain_db + (1. - a) * desired;
            let gain = amplitude(self.gain_db + self.output);
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
#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [[f64; 2]; 2],
}
impl Biquad {
    fn identity() -> Self {
        Self {
            b: [1., 0., 0.],
            a: [0.; 2],
            z: [[0.; 2]; 2],
        }
    }
    fn configure(&mut self, rate: f32, hz: f32, gain: f32, kind: usize) {
        let w = std::f64::consts::TAU * hz.min(rate * 0.45) as f64 / rate as f64;
        let c = w.cos();
        let a = 10_f64.powf(gain as f64 / 40.);
        let alpha = w.sin() / 2_f64.sqrt();
        let beta = 2. * a.sqrt() * alpha;
        let (b0, b1, b2, a0, a1, a2) = match kind {
            0 => (
                a * ((a + 1.) - (a - 1.) * c + beta),
                2. * a * ((a - 1.) - (a + 1.) * c),
                a * ((a + 1.) - (a - 1.) * c - beta),
                (a + 1.) + (a - 1.) * c + beta,
                -2. * ((a - 1.) + (a + 1.) * c),
                (a + 1.) + (a - 1.) * c - beta,
            ),
            2 => (
                a * ((a + 1.) + (a - 1.) * c + beta),
                -2. * a * ((a - 1.) + (a + 1.) * c),
                a * ((a + 1.) + (a - 1.) * c - beta),
                (a + 1.) - (a - 1.) * c + beta,
                2. * ((a - 1.) - (a + 1.) * c),
                (a + 1.) - (a - 1.) * c - beta,
            ),
            3 => (
                (1. + c) / 2.,
                -(1. + c),
                (1. + c) / 2.,
                1. + alpha,
                -2. * c,
                1. - alpha,
            ),
            _ => (
                1. + alpha * a,
                -2. * c,
                1. - alpha * a,
                1. + alpha / a,
                -2. * c,
                1. - alpha / a,
            ),
        };
        self.b = [b0 / a0, b1 / a0, b2 / a0];
        self.a = [a1 / a0, a2 / a0];
    }
    fn process(&mut self, frame: Frame) -> Frame {
        std::array::from_fn(|ch| {
            let x = frame[ch] as f64;
            let y = self.b[0] * x + self.z[ch][0];
            self.z[ch][0] = self.b[1] * x - self.a[0] * y + self.z[ch][1];
            self.z[ch][1] = self.b[2] * x - self.a[1] * y;
            y as f32
        })
    }
    fn reset(&mut self) {
        self.z = [[0.; 2]; 2];
    }
}
struct ThreeBand {
    rate: f32,
    gains: [f32; 3],
    filters: [Biquad; 3],
}
impl AudioEffect for ThreeBand {
    fn set_parameter(&mut self, id: &str, v: f32) -> Result<(), String> {
        validate(EQ, id, v)?;
        let index = match id {
            "low" => 0,
            "mid" => 1,
            _ => 2,
        };
        if self.gains[index] != v {
            self.gains[index] = v;
            self.filters[index].configure(self.rate, [200., 1000., 5000.][index], v, index);
        }
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        for frame in frames {
            for filter in &mut self.filters {
                *frame = filter.process(*frame);
            }
        }
    }
    fn reset(&mut self) {
        for filter in &mut self.filters {
            filter.reset();
        }
    }
}
struct Deesser {
    rate: f32,
    frequency: f32,
    threshold: f32,
    ratio: f32,
    range: f32,
    attack: f32,
    release: f32,
    mode: usize,
    filter: Biquad,
    shelf: Biquad,
    cadence: usize,
    gain: f32,
}
impl Deesser {
    fn new(rate: f32) -> Self {
        let mut filter = Biquad::identity();
        filter.configure(rate, 4500., 0., 3);
        Self {
            rate,
            frequency: 4500.,
            threshold: -30.,
            ratio: 4.,
            range: 12.,
            attack: 2.,
            release: 70.,
            mode: 0,
            filter,
            shelf: Biquad::identity(),
            cadence: 0,
            gain: 0.,
        }
    }
}
impl AudioEffect for Deesser {
    fn set_parameter(&mut self, id: &str, v: f32) -> Result<(), String> {
        validate(DEESS, id, v)?;
        match id {
            "frequency" => {
                if self.frequency != v {
                    self.frequency = v;
                    self.filter.configure(self.rate, v, 0., 3);
                }
            }
            "threshold" => self.threshold = v,
            "ratio" => self.ratio = v,
            "range" => self.range = v,
            "attack" => self.attack = v,
            "release" => self.release = v,
            "mode" => self.mode = v.round() as usize,
            _ => unreachable!(),
        }
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        let attack = coefficient(self.rate, self.attack);
        let release = coefficient(self.rate, self.release);
        for frame in frames {
            let high = self.filter.process(*frame);
            let level = db(high[0].abs().max(high[1].abs()));
            let desired =
                -((level - self.threshold).max(0.) * (1. - 1. / self.ratio)).min(self.range);
            let a = if desired < self.gain { attack } else { release };
            self.gain = a * self.gain + (1. - a) * desired;
            if self.cadence == 0 {
                self.shelf
                    .configure(self.rate, self.frequency, self.gain, 2);
            }
            self.cadence = (self.cadence + 1) % 16;
            let shaped = self.shelf.process(*frame);
            let gain = amplitude(self.gain);
            for ch in 0..2 {
                frame[ch] = if self.mode == 0 {
                    shaped[ch]
                } else {
                    frame[ch] * gain
                };
            }
        }
    }
    fn reset(&mut self) {
        self.filter.reset();
        self.shelf = Biquad::identity();
        self.cadence = 0;
        self.gain = 0.;
    }
}
struct Limiter {
    rate: f32,
    ceiling: f32,
    release: f32,
    gain: f32,
}
impl AudioEffect for Limiter {
    fn set_parameter(&mut self, id: &str, v: f32) -> Result<(), String> {
        validate(LIMITER, id, v)?;
        match id {
            "ceiling" => self.ceiling = v,
            "release" => self.release = v,
            _ => unreachable!(),
        }
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        let ceiling = amplitude(self.ceiling);
        let release = coefficient(self.rate, self.release);
        for frame in frames {
            let peak = frame[0].abs().max(frame[1].abs());
            let needed = if peak > ceiling { ceiling / peak } else { 1. };
            self.gain = if needed < self.gain {
                needed
            } else {
                (release * self.gain + (1. - release)).min(needed)
            };
            for sample in frame {
                *sample = (*sample * self.gain).clamp(-ceiling, ceiling);
            }
        }
    }
    fn reset(&mut self) {
        self.gain = 1.;
    }
}
struct Polarity {
    channels: usize,
}
impl AudioEffect for Polarity {
    fn set_parameter(&mut self, id: &str, v: f32) -> Result<(), String> {
        validate(POLARITY, id, v)?;
        self.channels = v.round() as usize;
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        for frame in frames {
            if self.channels != 2 {
                frame[0] = -frame[0];
            }
            if self.channels != 1 {
                frame[1] = -frame[1];
            }
        }
    }
    fn reset(&mut self) {}
}
#[cfg(test)]
mod tests;
