//! Preallocated, band-limited rate conversion and slowly varying clock correction.
//! All coefficient construction and buffer allocation happens in `new`.
use super::*;
use rubato::{
    Resampler, SincFixedOut, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
const CHUNK: usize = 128;
pub struct QueueReader {
    queue: Arc<ArrayQueue<Frame>>,
    converter: SincFixedOut<f32>,
    input: Vec<Vec<f32>>,
    output: Vec<Vec<f32>>,
    cursor: usize,
    target: usize,
    primed: bool,
    underruns: u64,
    same_rate: bool,
    correct_clock: bool,
    correction: f64,
}
impl QueueReader {
    pub fn new(
        queue: Arc<ArrayQueue<Frame>>,
        input_rate: u32,
        output_rate: u32,
        target: usize,
    ) -> Self {
        let converter = SincFixedOut::<f32>::new(
            output_rate as f64 / input_rate as f64,
            1.01,
            SincInterpolationParameters {
                sinc_len: 128,
                f_cutoff: 0.95,
                oversampling_factor: 128,
                interpolation: SincInterpolationType::Linear,
                window: WindowFunction::BlackmanHarris2,
            },
            CHUNK,
            2,
        )
        .expect("positive device rates and fixed resampler parameters");
        let input = converter.input_buffer_allocate(true);
        let output = converter.output_buffer_allocate(true);
        Self {
            queue,
            converter,
            input,
            output,
            cursor: CHUNK,
            target,
            primed: false,
            underruns: 0,
            same_rate: input_rate == output_rate,
            correct_clock: true,
            correction: 0.,
        }
    }
    /// Only the engine's pacing output may bypass independent-clock correction.
    pub fn set_clock_correction(&mut self, enabled: bool) {
        if self.correct_clock != enabled {
            self.correct_clock = enabled;
            self.converter.reset();
            self.cursor = CHUNK;
            self.primed = false;
            self.correction = 0.;
        }
    }
    pub fn underruns(&self) -> u64 {
        self.underruns
    }
    pub fn next_frame(&mut self) -> Option<Frame> {
        let direct = self.same_rate && !self.correct_clock;
        if direct {
            if !self.primed {
                if self.queue.len() < self.target.max(1) {
                    return None;
                }
                self.primed = true;
            }
            return self.queue.pop().or_else(|| {
                self.primed = false;
                self.underruns += 1;
                None
            });
        }
        if self.cursor == CHUNK {
            if !self.primed {
                if self.queue.len() < self.target.max(self.converter.input_frames_next()) {
                    return None;
                }
                self.primed = true;
            }
            // Once per block, with a slow low-pass to reject normal WASAPI packet bursts.
            let error = (self.queue.len() as f64 - self.target as f64) / self.target.max(1) as f64;
            let desired = if self.correct_clock {
                (error * 0.0005).clamp(-0.002, 0.002)
            } else {
                0.
            };
            self.correction += 0.01 * (desired - self.correction);
            if self
                .converter
                .set_resample_ratio_relative(1. / (1. + self.correction), true)
                .is_err()
            {
                return None;
            }
            let needed = self.converter.input_frames_next();
            if self.queue.len() < needed {
                self.underruns += 1;
                self.primed = false;
                self.converter.reset();
                self.correction = 0.;
                return None;
            }
            for i in 0..needed {
                let frame = self.queue.pop()?;
                self.input[0][i] = frame[0];
                self.input[1][i] = frame[1];
            }
            if self
                .converter
                .process_into_buffer(&self.input, &mut self.output, None)
                .is_err()
            {
                self.primed = false;
                self.underruns += 1;
                self.converter.reset();
                return None;
            }
            self.cursor = 0;
        }
        let value = [self.output[0][self.cursor], self.output[1][self.cursor]];
        self.cursor += 1;
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synchronized_equal_rate_is_exact() {
        let q = Arc::new(ArrayQueue::new(1024));
        let data: Vec<_> = (0..512)
            .map(|i| [i as f32 / 512., -(i as f32) / 512.])
            .collect();
        for f in &data {
            q.push(*f).unwrap();
        }
        let mut reader = QueueReader::new(q, 48000, 48000, 128);
        reader.set_clock_correction(false);
        for f in data {
            assert_eq!(reader.next_frame(), Some(f));
        }
    }
    fn tone(input_rate: u32, hz: f32) -> f32 {
        let q = Arc::new(ArrayQueue::new(input_rate as usize));
        for i in 0..input_rate / 2 {
            let x = ((std::f64::consts::TAU * hz as f64 * i as f64 / input_rate as f64).sin() * 0.5)
                as f32;
            q.push([x, -x]).unwrap();
        }
        let mut reader = QueueReader::new(q, input_rate, 48000, 128);
        reader.set_clock_correction(false);
        let mut power = 0.;
        for i in 0..20000 {
            let f = reader.next_frame().unwrap();
            assert!((f[0] + f[1]).abs() < 1e-6);
            if i >= 1000 {
                power += f[0] * f[0];
            }
        }
        (power / 19000.).sqrt()
    }
    #[test]
    fn passband_and_alias_rejection_preserve_stereo() {
        for rate in [44100, 96000, 192000] {
            assert!((tone(rate, 1000.) - 0.35355).abs() < 0.002);
        }
        assert!(tone(96000, 30000.) < 0.0001);
    }
}
