//! Final device quantization only; effects and recordings remain floating point.
use cpal::{FromSample, SampleFormat, SizedSample};
pub struct Encoder {
    random: u32,
    lsb: f32,
}
impl Encoder {
    pub fn new(format: SampleFormat) -> Self {
        Self {
            random: 0x7a31_89d5,
            lsb: match format {
                SampleFormat::I8 | SampleFormat::U8 => 1. / 128.,
                SampleFormat::I16 | SampleFormat::U16 => 1. / 32768.,
                _ => 0.,
            },
        }
    }
    fn uniform(&mut self) -> f32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 17;
        self.random ^= self.random << 5;
        (self.random >> 8) as f32 / 16777216.
    }
    pub fn sample<T: SizedSample + FromSample<f32>>(&mut self, sample: f32) -> T {
        let mut value = if sample.is_finite() { sample } else { 0. };
        // Preserve actual silence, including missing-device/underrun silence.
        if self.lsb != 0. && value != 0. {
            value += (self.uniform() - self.uniform()) * self.lsb;
            value = (value / self.lsb).round() * self.lsb;
        }
        T::from_sample(value.clamp(-1., 1.))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encoding_bounds_silence_and_unbiased_dither() {
        let mut e = Encoder::new(SampleFormat::I16);
        assert_eq!(e.sample::<i16>(0.), 0);
        assert_eq!(e.sample::<i16>(f32::NAN), 0);
        assert_eq!(e.sample::<i16>(2.), i16::MAX);
        assert_eq!(e.sample::<i16>(-2.), i16::MIN);
        let mut error = 0_f64;
        for _ in 0..100000 {
            error += e.sample::<i16>(0.125) as f64 - 4096.;
        }
        assert!(error.abs() / 100000. < 0.1);
        let mut e = Encoder::new(SampleFormat::F32);
        assert_eq!(e.sample::<f32>(0.12345), 0.12345);
    }
}
