//! DLL-derived acquisition and adaptive lag tracker. The equations and branch
//! constants are recorded in dist/voice-correction-research/v2-detector.
//! No original source was available; this is a Rust translation of static DSP
//! observations, checked against the DLL's read-only runtime trace.
const RING: usize = 0x1640;
const BINS: usize = 250;

#[derive(Clone, Copy)]
pub(super) struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}
impl Biquad {
    fn new(rate: f64, hz: f64, high_pass: bool) -> Self {
        let w = std::f64::consts::TAU * hz / rate;
        let c = w.cos();
        let alpha = w.sin() / (2.0 * f64::from_bits(0x3fe69fbe76c8b439));
        let a0 = 1.0 + alpha;
        let k = if high_pass {
            (1.0 + c) * 0.5
        } else {
            (1.0 - c) * 0.5
        };
        Self {
            b: [
                k / a0,
                if high_pass {
                    -2.0 * k / a0
                } else {
                    2.0 * k / a0
                },
                k / a0,
            ],
            a: [-2.0 * c / a0, (1.0 - alpha) / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }
    pub(super) fn lowpass(rate: f64, hz: f64) -> Self {
        Self::new(rate, hz, false)
    }
    pub(super) fn identity() -> Self {
        Self {
            b: [1.0, 0.0, 0.0],
            a: [0.0; 2],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }
    pub(super) fn set_lowpass(&mut self, rate: f64, hz: f64) {
        let updated = Self::lowpass(rate, hz);
        self.b = updated.b;
        self.a = updated.a;
    }
    pub(super) fn set_identity(&mut self) {
        self.b = [1.0, 0.0, 0.0];
        self.a = [0.0; 2];
    }
    pub(super) fn process(&mut self, x: f64) -> f64 {
        let y = ((self.b[2] * self.x[1] + self.b[1] * self.x[0] + self.b[0] * x)
            - self.a[0] * self.y[0])
            - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
    pub(super) fn reset(&mut self) {
        self.x = [0.0; 2];
        self.y = [0.0; 2];
    }
}

pub(super) struct Detector {
    rate: f64,
    voice: usize,
    filters: [Biquad; 3],
    raw: Box<[f64]>,
    write: usize,
    acquisition_ring: [f64; 500],
    acquisition_write: usize,
    acquisition_count: usize,
    acquisition_energy: [f64; BINS],
    acquisition_cross: [f64; BINS],
    energy: [f64; 60],
    cross: [f64; 60],
    lag: usize,
    margin: usize,
    cadence: usize,
    acquiring: bool,
    fresh_acquisition: bool,
    period: f64,
}
impl Detector {
    pub(super) fn new(rate: f64, voice: usize) -> Self {
        let mut result = Self {
            rate,
            voice: voice.min(4),
            filters: [Biquad::new(rate, 600.0, false); 3],
            raw: vec![0.0; RING].into_boxed_slice(),
            write: 0,
            acquisition_ring: [0.0; 500],
            acquisition_write: 0,
            acquisition_count: 0,
            acquisition_energy: [0.0; BINS],
            acquisition_cross: [0.0; BINS],
            energy: [0.0; 60],
            cross: [0.0; 60],
            lag: 0,
            margin: 6,
            cadence: 0,
            acquiring: true,
            fresh_acquisition: true,
            period: 0.0,
        };
        result.configure_voice();
        result
    }
    fn configure_voice(&mut self) {
        let cutoff = [600.0, 400.0, 300.0, 1000.0, 200.0][self.voice];
        self.margin = [6, 15, 30, 6, 30][self.voice];
        self.filters = [
            Biquad::new(self.rate, cutoff, false),
            Biquad::new(self.rate, cutoff * 2.0, false),
            Biquad::new(self.rate, 30.0, true),
        ];
    }
    pub(super) fn set_voice(&mut self, voice: usize) {
        if voice.min(4) != self.voice {
            self.voice = voice.min(4);
            self.configure_voice();
            self.lose_lock();
        }
    }
    pub(super) fn reset(&mut self) {
        self.raw.fill(0.0);
        self.write = 0;
        self.lag = 0;
        self.cadence = 0;
        for filter in &mut self.filters {
            filter.reset();
        }
        self.lose_lock();
    }
    fn lose_lock(&mut self) {
        self.acquiring = true;
        self.fresh_acquisition = true;
        self.period = 0.0;
    }
    pub(super) fn period(&self) -> f64 {
        self.period
    }
    pub(super) fn acquiring(&self) -> bool {
        self.acquiring
    }
    fn back(&self, delay: usize) -> f64 {
        self.raw[(self.write + RING - delay % RING) % RING]
    }
    fn recompute(&mut self, index: usize) {
        let lag = self.lag + index;
        let mut energy = 0.0;
        let mut cross = 0.0;
        for k in 0..2 * lag {
            let x = self.back(k);
            energy += x * x;
        }
        for k in 0..lag {
            cross += self.back(k) * self.back(k + lag);
        }
        self.energy[index] = energy;
        self.cross[index] = cross;
    }
    fn acquire(&mut self, input: f64) -> Option<usize> {
        if self.fresh_acquisition {
            self.acquisition_ring.fill(0.0);
            self.acquisition_energy.fill(0.0);
            self.acquisition_cross.fill(0.0);
            self.acquisition_write = 0;
            self.acquisition_count = 0;
            self.fresh_acquisition = false;
        }
        let t = self.acquisition_write;
        self.acquisition_ring[t] = input;
        self.acquisition_write = (t + 1) % 500;
        for lag in 0..BINS {
            let mid = self.acquisition_ring[(t + 500 - lag) % 500];
            let old = self.acquisition_ring[(t + 500 - 2 * lag) % 500];
            self.acquisition_cross[lag] += mid * input - old * mid;
            self.acquisition_energy[lag] += input * input - old * old;
        }
        self.acquisition_count += 1;
        if self.acquisition_count <= 10 {
            return None;
        }
        let n = ((self.acquisition_count - 6) / 2).min(249);
        let mut difference = [0.0; BINS];
        for (i, value) in difference.iter_mut().enumerate().take(n).skip(1) {
            *value = if self.acquisition_energy[i] > 0.0 {
                (0.5 * self.acquisition_energy[i] - self.acquisition_cross[i])
                    / self.acquisition_energy[i]
            } else {
                1.0
            };
        }
        let mut start = 2;
        while start < n.saturating_sub(2) && difference[start] <= 0.6 {
            start += 1;
        }
        let mut candidates = [0.0; 3];
        let mut count = 0;
        for i in start + 1..n.saturating_sub(1) {
            let l = difference[i - 1];
            let m = difference[i];
            let r = difference[i + 1];
            if m >= l || m >= r {
                continue;
            }
            let offset = (l - r) * 0.5 / (l - 2.0 * m + r);
            let minimum = r * 0.5 * offset * (offset + 1.0)
                + (l * 0.5 * offset * (offset - 1.0) - (offset + 1.0) * m * (offset - 1.0));
            if minimum < 0.05 {
                candidates[count] = (i as f64 + offset) * 8.0;
                count += 1;
                if count == 3 {
                    break;
                }
            }
        }
        let p = candidates[0];
        let estimate = match count {
            1 if self.voice == 4 => p,
            2 if (p - (candidates[1] - p)).abs() < p * 0.1 => candidates[1] * 0.5,
            3 => {
                let d1 = candidates[1] - p;
                let d2 = candidates[2] - candidates[1];
                if (p - d1).abs() < p * 0.1 && (d1 - d2).abs() < p * 0.1 {
                    candidates[2] / 3.0
                } else if (d1 - d2).abs() < d1 * 0.1 {
                    (candidates[2] - p) * 0.5
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        let lag = (estimate + 0.5) as usize;
        lag.checked_sub(self.margin)
            .filter(|lag| *lag > 0 && 2 * (lag + 2 * self.margin) < RING)
    }
    pub(super) fn process_sample(&mut self, input: f64) -> Option<f64> {
        self.write = (self.write + 1) % RING;
        self.raw[self.write] = input;
        let mut filtered = input;
        for filter in &mut self.filters {
            filtered = filter.process(filtered);
        }
        if self.acquiring {
            if self.write & 7 == 0 {
                if let Some(lag) = self.acquire(filtered) {
                    self.lag = lag;
                    self.recompute(0);
                    if self.energy[0] >= 1.65e-5 {
                        for i in 1..self.margin * 2 {
                            self.recompute(i);
                        }
                        self.acquiring = false;
                        self.cadence = 4;
                    }
                }
            }
            return None;
        }
        for i in 0..self.margin * 2 {
            let lag = self.lag + i;
            let mid = self.back(lag);
            let old = self.back(2 * lag);
            self.energy[i] += input * input - old * old;
            self.cross[i] += input * mid - old * mid;
        }
        self.cadence += 1;
        if self.cadence != 5 {
            return None;
        }
        self.cadence = 0;
        let n = self.margin * 2;
        let mut best = 0;
        for i in 1..n {
            if self.energy[i] * 0.5 - self.cross[i] < self.energy[best] * 0.5 - self.cross[best] {
                best = i;
            }
        }
        if best == 0 || best == n - 1 || self.energy[best] < 1.5e-5 {
            self.lose_lock();
            return Some(0.0);
        }
        if best < self.margin - 1 {
            self.lag -= 1;
            best += 1;
            self.energy.copy_within(0..n - 1, 1);
            self.cross.copy_within(0..n - 1, 1);
            self.recompute(0);
        } else if best > self.margin {
            self.lag += 1;
            best -= 1;
            self.energy.copy_within(1..n, 0);
            self.cross.copy_within(1..n, 0);
            self.recompute(n - 1);
        }
        let left = self.energy[best - 1] * 0.5 - self.cross[best - 1];
        let center = self.energy[best] * 0.5 - self.cross[best];
        let right = self.energy[best + 1] * 0.5 - self.cross[best + 1];
        self.period =
            (self.lag + best) as f64 + (left - right) * 0.5 / (left - 2.0 * center + right);
        Some(self.period)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dll_trace_matches_acquisition_cadence_and_refined_periods() {
        // Values observed through the DLL's VST processReplacing callback;
        // captured before changing the implementation, not self-generated.
        let checkpoints = [
            (526, 0.0),
            (527, 0.0),
            (528, 111.62685448130311),
            (532, 111.62685448130311),
            (533, 111.6260870302723),
            (1000, 111.62009012300086),
            (10000, 111.62724018850331),
            (47999, 111.61503401618361),
        ];
        let mut detector = Detector::new(48000.0, 0);
        for sample in 0..48000 {
            let phase = std::f32::consts::TAU * 430.0 * sample as f32 / 48000.0;
            let input = 0.2 * phase.sin() + 0.08 * (2.0 * phase).sin() + 0.04 * (3.0 * phase).sin();
            detector.process_sample(input as f64);
            assert_eq!(detector.acquiring(), sample < 527);
            if let Some((_, expected)) = checkpoints.iter().find(|(index, _)| *index == sample) {
                assert!(
                    (detector.period() - expected).abs() < 1e-7,
                    "sample{sample}:{} expected{expected}",
                    detector.period()
                );
            }
        }
    }
    #[test]
    fn recovered_detector_acquires_harmonic_voice() {
        for voice in 0..5 {
            let mut detector = Detector::new(48000.0, voice);
            for t in 0..48000 {
                let p = std::f64::consts::TAU * 430.0 * t as f64 / 48000.0;
                detector.process_sample(
                    0.2 * p.sin() + 0.08 * (p * 2.0).sin() + 0.04 * (p * 3.0).sin(),
                );
            }
            assert!(!detector.acquiring());
            assert!(
                (detector.period() - 48000.0 / 430.0).abs() < 0.02,
                "voice{voice}:{}",
                detector.period()
            );
        }
    }
    #[test]
    fn reset_and_silence_clear_history() {
        let mut detector = Detector::new(48000.0, 0);
        for t in 0..2000 {
            detector.process_sample((t as f64 * 0.05).sin());
        }
        detector.reset();
        detector.set_voice(1);
        for _ in 0..10000 {
            detector.process_sample(0.0);
        }
        assert_eq!(detector.period(), 0.0);
        assert!(detector.acquiring());
    }
}
