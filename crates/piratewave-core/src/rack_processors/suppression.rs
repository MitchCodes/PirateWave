//! RNNoise speech suppression with streaming linear conversion to/from 48 kHz.
use super::*;
use nnnoiseless::DenoiseState;
const N: usize = 480;
const RING: usize = 2048;
const PARAMS: &[ParameterDescriptor] = &[p("strength", "Strength", 0., 100., 100., "%")];
pub(super) fn factory() -> EffectFactory {
    EffectFactory {
        id: "noise_suppression",
        name: "Noise Suppression",
        description: "RNNoise speech denoising, including noise during speech; about 20 ms latency",
        parameters: PARAMS,
        create: |rate| Box::new(Suppression::new(rate)),
    }
}
struct Suppression {
    rate: f64,
    strength: f32,
    states: [Box<DenoiseState<'static>>; 2],
    pristine: Box<DenoiseState<'static>>,
    input: [[f32; N]; 2],
    output: [[f32; N]; 2],
    position: usize,
    count: usize,
    rendered: Vec<Frame>,
    dry: Vec<Frame>,
    native: usize,
    next: f64,
    previous: Frame,
}
impl Suppression {
    fn new(rate: f32) -> Self {
        let mut states = std::array::from_fn(|_| DenoiseState::new());
        let mut scratch = [0.; N];
        for state in &mut states {
            state.process_frame(&mut scratch, &[0.; N]);
        }
        let pristine = Box::new((*states[0]).clone());
        Self {
            rate: rate as f64,
            strength: 1.,
            states,
            pristine,
            input: [[0.; N]; 2],
            output: [[0.; N]; 2],
            position: 0,
            count: 0,
            rendered: vec![[0.; 2]; RING],
            dry: vec![[0.; 2]; (rate as f64 * 0.02).ceil() as usize + 3],
            native: 0,
            next: 0.,
            previous: [0.; 2],
        }
    }
    fn push(&mut self, frame: Frame) {
        for (ch, &sample) in frame.iter().enumerate() {
            self.input[ch][self.position] = sample.clamp(-1., 1.) * 32768.;
        }
        self.position += 1;
        self.count += 1;
        if self.position == N {
            for ch in 0..2 {
                self.states[ch].process_frame(&mut self.output[ch], &self.input[ch]);
            }
            for i in 0..N {
                self.rendered[(self.count - N + i) % RING] =
                    [self.output[0][i] / 32768., self.output[1][i] / 32768.];
            }
            self.position = 0;
        }
    }
}
impl AudioEffect for Suppression {
    fn set_parameter(&mut self, id: &str, v: f32) -> Result<(), String> {
        validate(PARAMS, id, v)?;
        self.strength = v / 100.;
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        let step = self.rate / 48000.;
        let delay = (self.rate * 0.02).round() as usize;
        for frame in frames {
            let current = frame.map(|v| if v.is_finite() { v } else { 0. });
            let dry_index = self.native % self.dry.len();
            self.dry[dry_index] = current;
            while self.next <= self.native as f64 {
                let t = (self.next - (self.native as f64 - 1.)).clamp(0., 1.) as f32;
                self.push(std::array::from_fn(|ch| {
                    self.previous[ch] + t * (current[ch] - self.previous[ch])
                }));
                self.next += step;
            }
            let cursor = self.native as f64 * 48000. / self.rate - N as f64;
            let wet = if cursor >= 0. {
                let i = cursor.floor() as usize;
                let t = (cursor - i as f64) as f32;
                std::array::from_fn(|ch| {
                    self.rendered[i % RING][ch]
                        + t * (self.rendered[(i + 1) % RING][ch] - self.rendered[i % RING][ch])
                })
            } else {
                [0.; 2]
            };
            let dry = if self.native >= delay {
                self.dry[(self.native - delay) % self.dry.len()]
            } else {
                [0.; 2]
            };
            for ch in 0..2 {
                frame[ch] = dry[ch] + self.strength * (wet[ch] - dry[ch]);
            }
            self.previous = current;
            self.native += 1;
        }
    }
    // Reset is a control-plane operation, called during graph reconfiguration.
    fn reset(&mut self) {
        for state in &mut self.states {
            **state = (*self.pristine).clone();
        }
        self.input.fill([0.; N]);
        self.output.fill([0.; N]);
        self.rendered.fill([0.; 2]);
        self.dry.fill([0.; 2]);
        self.position = 0;
        self.count = 0;
        self.native = 0;
        self.next = 0.;
        self.previous = [0.; 2];
    }
}
