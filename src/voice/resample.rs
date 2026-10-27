//! Fixed-ratio, continuous windowed-sinc conversion for PirateWave's 48 kHz engine.
//! Construction is off the render thread. Streaming operations allocate nothing.
const TAPS: usize = 255;
pub struct Resampler {
    kernel: [f32; TAPS],
    down: [f32; TAPS],
    up: [f32; TAPS],
    down_at: usize,
    up_at: usize,
    phase: usize,
}
impl Default for Resampler {
    fn default() -> Self {
        let mut kernel = [0.0; TAPS];
        let fc = 6800.0 / 48000.0;
        for (i, v) in kernel.iter_mut().enumerate() {
            let x = i as f64 - (TAPS - 1) as f64 / 2.0;
            let sinc = if x == 0.0 {
                2.0 * fc
            } else {
                (2.0 * std::f64::consts::PI * fc * x).sin() / (std::f64::consts::PI * x)
            };
            let window =
                0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (TAPS - 1) as f64).cos();
            *v = (sinc * window) as f32;
        }
        let sum: f32 = kernel.iter().sum();
        for v in &mut kernel {
            *v /= sum;
        }
        Self {
            kernel,
            down: [0.0; TAPS],
            up: [0.0; TAPS],
            down_at: 0,
            up_at: 0,
            phase: 0,
        }
    }
}
impl Resampler {
    pub fn down(&mut self, sample: f32) -> Option<f32> {
        self.down[self.down_at] = sample;
        self.down_at = (self.down_at + 1) % TAPS;
        self.phase = (self.phase + 1) % 3;
        if self.phase != 0 {
            return None;
        }
        Some(
            self.kernel
                .iter()
                .enumerate()
                .map(|(i, k)| self.down[(self.down_at + TAPS - 1 - i) % TAPS] * k)
                .sum(),
        )
    }
    pub fn up(&mut self, sample: f32) -> [f32; 3] {
        self.up[self.up_at] = sample;
        self.up_at = (self.up_at + 1) % TAPS;
        std::array::from_fn(|phase| {
            self.kernel
                .iter()
                .enumerate()
                .skip(phase)
                .step_by(3)
                .map(|(i, k)| self.up[(self.up_at + TAPS - 1 - (i - phase) / 3) % TAPS] * k * 3.0)
                .sum()
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn continuous_ratio_dc_and_alias_rejection() {
        let mut r = Resampler::default();
        let down: Vec<_> = (0..48000).filter_map(|_| r.down(0.25)).collect();
        assert_eq!(down.len(), 16000);
        assert!((down[1000] - 0.25).abs() < 1e-6);
        let up: Vec<_> = down.into_iter().flat_map(|v| r.up(v)).collect();
        assert_eq!(up.len(), 48000);
        assert!((up[3000] - 0.25).abs() < 1e-5);
        let mut alias = Resampler::default();
        let samples: Vec<_> = (0..48000)
            .filter_map(|i| {
                alias.down((std::f32::consts::TAU * 12000.0 * i as f32 / 48000.0).sin())
            })
            .skip(500)
            .collect();
        let rms = (samples.iter().map(|v| v * v).sum::<f32>() / samples.len() as f32).sqrt();
        assert!(rms < 0.003, "12kHz alias RMS {rms}");
    }
}
