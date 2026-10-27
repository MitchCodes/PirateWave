use super::db;
use piratewave_core::Frame;
use rustfft::{num_complex::Complex, Fft, FftPlanner};
use std::{collections::VecDeque, sync::Arc};

#[derive(Clone, Debug, Default)]
pub struct Reading {
    pub frames: u64,
    pub peak: [f32; 2],
    pub rms: [f32; 2],
    pub peak_db: f32,
    pub floor_db: f32,
    pub separation_db: f32,
    pub clipped: u64,
    pub bands: [f32; 3],
}
pub struct Analyzer {
    rate: u32,
    frames: u64,
    peak: [f32; 2],
    power: [f64; 2],
    clipped: u64,
    window_power: [f64; 2],
    window_frames: usize,
    windows: VecDeque<f64>,
    recent_rms: [f32; 2],
    history: VecDeque<Frame>,
    fft: Arc<dyn Fft<f32>>,
    fft_buffer: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    spectrum: Vec<f32>,
}
impl Analyzer {
    pub fn new(rate: u32, size: usize) -> Self {
        let size = if size == 8192 { 8192 } else { 2048 };
        let fft = FftPlanner::new().plan_fft_forward(size);
        Self {
            rate,
            frames: 0,
            peak: [0.; 2],
            power: [0.; 2],
            clipped: 0,
            window_power: [0.; 2],
            window_frames: 0,
            windows: VecDeque::with_capacity(1500),
            recent_rms: [0.; 2],
            history: VecDeque::with_capacity(size),
            scratch: vec![Complex::default(); fft.get_inplace_scratch_len()],
            fft,
            fft_buffer: vec![Complex::default(); size],
            spectrum: vec![-120.; size / 2 + 1],
        }
    }
    pub fn push(&mut self, frame: Frame) {
        let frame = frame.map(|v| if v.is_finite() { v } else { 0. });
        for (ch, sample) in frame.into_iter().enumerate() {
            self.peak[ch] = self.peak[ch].max(sample.abs());
            let power = sample as f64 * sample as f64;
            self.power[ch] += power;
            self.window_power[ch] += power;
        }
        self.frames += 1;
        self.clipped += u64::from(frame.iter().any(|x| x.abs() >= 0.999));
        self.window_frames += 1;
        if self.window_frames >= (self.rate as usize / 50).max(1) {
            self.recent_rms = self
                .window_power
                .map(|p| (p / self.window_frames as f64).sqrt() as f32);
            if self.windows.len() == 1500 {
                self.windows.pop_front();
            }
            self.windows.push_back(
                self.window_power[0].max(self.window_power[1]) / self.window_frames as f64,
            );
            self.window_power = [0.; 2];
            self.window_frames = 0;
        }
        if self.history.len() == self.fft_buffer.len() {
            self.history.pop_front();
        }
        self.history.push_back(frame);
    }
    pub fn reading(&self) -> Reading {
        let mut windows: Vec<f64> = self.windows.iter().copied().collect();
        windows.sort_by(f64::total_cmp);
        let quiet = (windows.len() / 4).max(1).min(windows.len());
        let floor = if quiet > 0 {
            (windows[..quiet].iter().sum::<f64>() / quiet as f64).sqrt()
        } else {
            0.
        };
        let peak_db = db(self.peak[0].max(self.peak[1]) as f64);
        Reading {
            frames: self.frames,
            peak: self.peak,
            rms: self.recent_rms,
            peak_db,
            floor_db: db(floor),
            separation_db: (peak_db - db(floor)).max(0.),
            clipped: self.clipped,
            bands: self.band_balance(),
        }
    }
    pub fn spectrum(&mut self) -> Vec<f32> {
        let size = self.fft_buffer.len();
        self.spectrum.fill(-120.);
        let window_sum = (size - 1) as f32 * 0.5;
        for ch in 0..2 {
            for (i, bin) in self.fft_buffer.iter_mut().enumerate() {
                let sample = self.history.get(i).map_or(0., |f| f[ch]);
                let hann = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (size - 1) as f32).cos();
                *bin = Complex::new(sample * hann, 0.);
            }
            self.fft
                .process_with_scratch(&mut self.fft_buffer, &mut self.scratch);
            for (i, value) in self.spectrum.iter_mut().enumerate() {
                let factor = if i == 0 || i == size / 2 { 1. } else { 2. };
                *value = value.max(db((self.fft_buffer[i].norm() * factor / window_sum) as f64));
            }
        }
        self.spectrum.clone()
    }
    pub fn waveform(&self) -> Vec<f32> {
        let step = (self.history.len() / 256).max(1);
        self.history
            .iter()
            .step_by(step)
            .take(256)
            .map(|f| if f[0].abs() >= f[1].abs() { f[0] } else { f[1] })
            .collect()
    }
    fn band_balance(&self) -> [f32; 3] {
        let mut power = [0.; 3];
        for (i, db) in self.spectrum.iter().enumerate().skip(1) {
            let frequency = i as f32 * self.rate as f32 / self.fft_buffer.len() as f32;
            let band = if frequency < 250. {
                0
            } else if frequency < 4000. {
                1
            } else {
                2
            };
            power[band] += 10_f32.powf(*db / 10.);
        }
        let sum = power.iter().sum::<f32>().max(1e-12);
        power.map(|p| p / sum)
    }
}

/// Match a short chirp without assuming it must have arrived. Normalized
/// correlation rejects unrelated speech/noise; the worker also checks SNR.
pub fn correlate(signal: &[f32], reference: &[f32]) -> Option<(usize, f32)> {
    if signal.len() < reference.len() || reference.is_empty() {
        return None;
    }
    let energy = reference.iter().map(|x| x * x).sum::<f32>();
    if energy <= 1e-12 {
        return None;
    }
    let mut best = (0, 0_f32);
    for offset in 0..=signal.len() - reference.len() {
        let window = &signal[offset..offset + reference.len()];
        let power = window.iter().map(|x| x * x).sum::<f32>();
        if power < 1e-10 {
            continue;
        }
        let score = window
            .iter()
            .zip(reference)
            .map(|(a, b)| a * b)
            .sum::<f32>()
            .abs()
            / (power * energy).sqrt();
        if score > best.1 {
            best = (offset, score);
        }
    }
    (best.1 > 0.65).then_some(best)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metrics_calibrate_rms_noise_clipping_and_stereo_without_phase_cancellation() {
        let mut meter = Analyzer::new(48000, 2048);
        for _ in 0..48000 {
            meter.push([0.001, -0.001]);
        }
        assert!((meter.reading().floor_db + 60.).abs() < 0.01);
        for _ in 0..4800 {
            meter.push([0.5, -0.5]);
        }
        let reading = meter.reading();
        assert!((reading.peak_db + 6.0206).abs() < 0.01);
        assert!(reading.separation_db > 53.);
        meter.push([1., f32::NAN]);
        assert_eq!(meter.reading().clipped, 1);
    }
    #[test]
    fn fft_preserves_antiphase_stereo_and_calibrates_bin_centered_sine() {
        let mut analyzer = Analyzer::new(48000, 8192);
        for i in 0..8192 {
            let x = 0.5 * (std::f32::consts::TAU * 128. * i as f32 / 8192.).sin();
            analyzer.push([x, -x]);
        }
        let bins = analyzer.spectrum();
        assert!((bins[128] + 6.0206).abs() < 0.05);
        assert_eq!(
            bins.iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap()
                .0,
            128
        );
        assert!(analyzer.reading().bands[1] > 0.99);
    }
    #[test]
    fn chirp_detection_never_invents_an_echo() {
        let reference: Vec<f32> = (0..100)
            .map(|i| (i as f32 * i as f32 * 0.013).sin())
            .collect();
        let mut signal = vec![0.; 1000];
        signal[317..417].copy_from_slice(&reference);
        let (index, score) = correlate(&signal, &reference).unwrap();
        assert_eq!(index, 317);
        assert!(score > 0.99);
        assert!(correlate(&vec![0.; 1000], &reference).is_none());
    }
}
