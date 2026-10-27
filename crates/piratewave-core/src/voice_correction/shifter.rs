//! Period-synchronous read-head shifting recovered from DLL VA 0x10030598–0x10030aba.
//! The period detector, control timing, and optional antialias/formant filters live
//! outside this component. See the research ledger for evidence and limitations.
use crate::Frame;

pub(super) const RING_SIZE: usize = 0x1640;

#[derive(Clone, Copy)]
struct Head {
    integer: i32,
    fraction: f64,
}
impl Head {
    fn jump(&mut self, samples: f64) {
        let position = self.integer as f64 + self.fraction + samples;
        self.integer = (position + 0.5).floor() as i32;
        self.fraction = position - self.integer as f64;
    }
}

/// The DLL's two cursors and 50-sample linear fade, with linked stereo phase.
pub(super) struct RecoveredShifter {
    audio: Vec<[f64; 2]>,
    write: i32,
    active: Head,
    previous: Head,
    fade: f64,
}
impl RecoveredShifter {
    pub fn new() -> Self {
        Self {
            audio: vec![[0.0; 2]; RING_SIZE],
            write: 0,
            active: Head {
                integer: -110,
                fraction: 0.0,
            },
            previous: Head {
                integer: -110,
                fraction: 0.0,
            },
            fade: 1.0,
        }
    }

    pub fn reset(&mut self) {
        self.audio.fill([0.0; 2]);
        self.write = 0;
        self.active = Head {
            integer: -110,
            fraction: 0.0,
        };
        self.previous = self.active;
        self.fade = 1.0;
    }

    /// `input` is the already aligned sample stored at the DSP write position;
    /// `period` and `ratio` are the control values chosen for that write position.
    pub fn process_sample(&mut self, input: Frame, period: f64, ratio: f64) -> Frame {
        self.write += 1;
        if self.write == RING_SIZE as i32 {
            self.write = 0;
            self.active.integer -= RING_SIZE as i32;
            self.previous.integer -= RING_SIZE as i32;
        }
        self.audio[self.write as usize] = [input[0] as f64, input[1] as f64];
        // Invalid controls are a host safety boundary, not a recovered DLL branch.
        let period = if period.is_finite() {
            period.clamp(0.0, 2400.0)
        } else {
            0.0
        };
        let ratio = if period > 0.0 && ratio.is_finite() {
            ratio.clamp(0.125, 8.0)
        } else {
            1.0
        };
        if ratio >= 1.0 {
            let whole = ratio.trunc() as i32;
            let fraction = ratio - whole as f64;
            for head in [&mut self.active, &mut self.previous] {
                head.integer += whole;
                head.fraction += fraction;
                if head.fraction >= 0.5 {
                    head.integer += 1;
                    head.fraction -= 1.0;
                }
            }
            if self.previous.integer >= self.write {
                self.fade = 1.0;
            }
            if self.active.integer >= self.write - 100 {
                self.previous = self.active;
                self.fade = 0.0;
                self.active.jump(-period);
            }
        } else {
            // The slow-reading branch deliberately checks its lower boundary
            // only when the active fractional cursor borrows an integer sample.
            self.active.integer += 1;
            self.previous.integer += 1;
            self.active.fraction += ratio - 1.0;
            self.previous.fraction += ratio - 1.0;
            if self.previous.fraction < -0.5 {
                self.previous.integer -= 1;
                self.previous.fraction += 1.0;
            }
            if self.active.fraction < -0.5 {
                self.active.integer -= 1;
                self.active.fraction += 1.0;
                if self.active.integer as f64 <= self.write as f64 - period * 1.2 - 100.0 {
                    self.previous = self.active;
                    self.fade = 0.0;
                    self.active.jump(period);
                }
            }
        }
        let active = self.read(self.active);
        if self.fade >= 1.0 {
            return [active[0] as f32, active[1] as f32];
        }
        let previous = self.read(self.previous);
        let output = [
            (active[0] * self.fade + previous[0] * (1.0 - self.fade)) as f32,
            (active[1] * self.fade + previous[1] * (1.0 - self.fade)) as f32,
        ];
        self.fade = (self.fade + 0.02).min(1.0);
        output
    }

    fn read(&self, head: Head) -> [f64; 2] {
        let center = head.integer.rem_euclid(RING_SIZE as i32) as usize;
        let before = (center + RING_SIZE - 1) % RING_SIZE;
        let after = (center + 1) % RING_SIZE;
        let mut output = [0.0; 2];
        for (channel, value) in output.iter_mut().enumerate() {
            let left = self.audio[before][channel];
            let middle = self.audio[center][channel];
            let right = self.audio[after][channel];
            *value = middle
                + (0.5 * (right - left) + (0.5 * (right + left) - middle) * head.fraction)
                    * head.fraction;
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quadratic_interpolation_reconstructs_a_parabola() {
        let mut shifter = RecoveredShifter::new();
        shifter.audio[19] = [1.0, -1.0];
        shifter.audio[20] = [0.0, 0.0];
        shifter.audio[21] = [1.0, -1.0];
        assert_eq!(
            shifter.read(Head {
                integer: 20,
                fraction: 0.25
            }),
            [0.0625, -0.0625]
        );
    }

    #[test]
    fn upward_jump_preserves_previous_head_and_starts_linear_fade() {
        let mut shifter = RecoveredShifter::new();
        shifter.active.integer = -100;
        shifter.process_sample([0.0; 2], 200.25, 1.0);
        assert_eq!(shifter.previous.integer, -99);
        assert_eq!(shifter.active.integer, -299);
        assert_eq!(shifter.active.fraction, -0.25);
        assert_eq!(shifter.fade, 0.02);
    }

    #[test]
    fn negative_cursor_wrap_and_stereo_are_stable() {
        let mut shifter = RecoveredShifter::new();
        for i in 0..RING_SIZE * 3 {
            let sample = (i as f64 * std::f64::consts::TAU / 200.0).sin() as f32;
            let out = shifter.process_sample([sample, -sample], 200.0, 0.8);
            assert!(out[0].is_finite());
            assert_eq!(out[0], -out[1]);
            assert!(out[0].abs() < 1.1);
        }
    }

    #[test]
    fn period_jumps_keep_the_shifted_tone_phase_continuous() {
        for ratio in [0.8, 1.5] {
            let mut shifter = RecoveredShifter::new();
            for sample in 0..RING_SIZE * 3 {
                let input = (sample as f64 * std::f64::consts::TAU / 200.0).sin() as f32;
                let output = shifter.process_sample([input; 2], 200.0, ratio)[0];
                if sample > 400 {
                    // Removing/adding a full period must not alter phase, even
                    // through overlapping heads and the physical ring wrap.
                    let expected = ((ratio * (sample + 1) as f64 - 111.0) * std::f64::consts::TAU
                        / 200.0)
                        .sin();
                    assert!((output as f64 - expected).abs() < 1e-5);
                }
            }
        }
    }
}
