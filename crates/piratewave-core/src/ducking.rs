//! Priority-driven stereo ducking. All buffers/model state are prepared on the
//! control plane; processing uses bounded, allocation-free 48 kHz blocks.
use crate::{effects::db_to_gain, Frame, SAMPLE_RATE};
use nnnoiseless::DenoiseState;
use serde::{Deserialize, Serialize};
#[cfg(test)]
mod tests;
const WINDOW: usize = 480;
const DELAY: usize = 2401;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Detection {
    #[default]
    Level,
    Speech,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DuckingConfig {
    pub detection: Detection,
    pub threshold: f32,
    pub sensitivity: f32,
    pub amount: f32,
    pub attack: f32,
    pub hold: f32,
    pub release: f32,
    pub lookahead_enabled: bool,
    pub lookahead: f32,
}
impl Default for DuckingConfig {
    fn default() -> Self {
        Self {
            detection: Detection::Level,
            threshold: -35.,
            sensitivity: 60.,
            amount: -18.,
            attack: 10.,
            hold: 150.,
            release: 400.,
            lookahead_enabled: false,
            lookahead: 10.,
        }
    }
}
pub struct Parameter {
    pub id: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub step: f32,
    pub unit: &'static str,
    pub choices: &'static [&'static str],
}
pub const PARAMETERS: &[Parameter] = &[
    Parameter {
        id: "detection",
        label: "Detection",
        min: 0.,
        max: 1.,
        default: 0.,
        step: 1.,
        unit: "",
        choices: &["Level", "Speech"],
    },
    Parameter {
        id: "threshold",
        label: "Level threshold",
        min: -80.,
        max: 0.,
        default: -35.,
        step: 1.,
        unit: "dBFS",
        choices: &[],
    },
    Parameter {
        id: "sensitivity",
        label: "Speech sensitivity",
        min: 10.,
        max: 95.,
        default: 60.,
        step: 5.,
        unit: "% probability",
        choices: &[],
    },
    Parameter {
        id: "amount",
        label: "Duck amount",
        min: -60.,
        max: 0.,
        default: -18.,
        step: 1.,
        unit: "dB",
        choices: &[],
    },
    Parameter {
        id: "attack",
        label: "Attack",
        min: 1.,
        max: 500.,
        default: 10.,
        step: 1.,
        unit: "ms",
        choices: &[],
    },
    Parameter {
        id: "hold",
        label: "Hold",
        min: 0.,
        max: 2000.,
        default: 150.,
        step: 10.,
        unit: "ms",
        choices: &[],
    },
    Parameter {
        id: "release",
        label: "Release",
        min: 10.,
        max: 5000.,
        default: 400.,
        step: 10.,
        unit: "ms",
        choices: &[],
    },
    Parameter {
        id: "lookahead_enabled",
        label: "Lookahead",
        min: 0.,
        max: 1.,
        default: 0.,
        step: 1.,
        unit: "",
        choices: &["Off", "On"],
    },
    Parameter {
        id: "lookahead",
        label: "Lookahead time",
        min: 0.,
        max: 50.,
        default: 10.,
        step: 1.,
        unit: "ms",
        choices: &[],
    },
];
impl DuckingConfig {
    pub fn value(&self, id: &str) -> f32 {
        match id {
            "detection" => {
                if self.detection == Detection::Speech {
                    1.
                } else {
                    0.
                }
            }
            "threshold" => self.threshold,
            "sensitivity" => self.sensitivity,
            "amount" => self.amount,
            "attack" => self.attack,
            "hold" => self.hold,
            "release" => self.release,
            "lookahead_enabled" => u8::from(self.lookahead_enabled) as f32,
            "lookahead" => self.lookahead,
            _ => 0.,
        }
    }
    pub fn set(&mut self, id: &str, v: f32) -> Result<(), String> {
        let p = PARAMETERS
            .iter()
            .find(|p| p.id == id)
            .ok_or("Unknown ducking parameter")?;
        if !v.is_finite()
            || !(p.min..=p.max).contains(&v)
            || (!p.choices.is_empty() && v.fract() != 0.)
        {
            return Err("Invalid ducking parameter".into());
        }
        match id {
            "detection" => {
                self.detection = if v == 0. {
                    Detection::Level
                } else {
                    Detection::Speech
                }
            }
            "threshold" => self.threshold = v,
            "sensitivity" => self.sensitivity = v,
            "amount" => self.amount = v,
            "attack" => self.attack = v,
            "hold" => self.hold = v,
            "release" => self.release = v,
            "lookahead_enabled" => self.lookahead_enabled = v != 0.,
            "lookahead" => self.lookahead = v,
            _ => unreachable!(),
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut copy = self.clone();
        for p in PARAMETERS {
            copy.set(p.id, self.value(p.id))?;
        }
        Ok(())
    }
    fn delay(&self) -> usize {
        if self.lookahead_enabled {
            (self.lookahead * 48.).round() as usize
        } else {
            0
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct DuckingStatus {
    pub priority_peak: f32,
    pub ducked_peak: f32,
    pub active: bool,
    pub reduction_db: f32,
    pub speech_probability: f32,
    pub fallback: bool,
    /// Detector work was skipped for this block because one signal was absent.
    pub fast_path: bool,
}
pub struct AutoDucking {
    config: DuckingConfig,
    rms: Box<[Frame; WINDOW]>,
    sum: [f64; 2],
    cursor: usize,
    models: [Box<DenoiseState<'static>>; 2],
    pristine: Box<DenoiseState<'static>>,
    speech_input: Box<[[f32; WINDOW]; 2]>,
    speech_output: Box<[[f32; WINDOW]; 2]>,
    speech_cursor: usize,
    priority_delay: Box<[Frame; DELAY]>,
    ducked_delay: Box<[Frame; DELAY]>,
    delay_cursor: usize,
    filled: usize,
    fade: usize,
    hold: usize,
    gain_db: f32,
    attack: f32,
    release: f32,
    idle: bool,
    models_idle: bool,
    ducked_pending: usize,
    #[cfg(test)]
    detector_samples: usize,
    #[cfg(test)]
    inference_frames: usize,
    status: DuckingStatus,
}
impl AutoDucking {
    pub fn new(config: &DuckingConfig) -> Self {
        let mut models = std::array::from_fn(|_| DenoiseState::new());
        for model in &mut models {
            model.process_frame(&mut [0.; WINDOW], &[0.; WINDOW]);
        }
        let pristine = Box::new((*models[0]).clone());
        let mut s = Self {
            config: config.clone(),
            rms: Box::new([[0.; 2]; WINDOW]),
            sum: [0.; 2],
            cursor: 0,
            models,
            pristine,
            speech_input: Box::new([[0.; WINDOW]; 2]),
            speech_output: Box::new([[0.; WINDOW]; 2]),
            speech_cursor: 0,
            priority_delay: Box::new([[0.; 2]; DELAY]),
            ducked_delay: Box::new([[0.; 2]; DELAY]),
            delay_cursor: 0,
            filled: 0,
            fade: 240,
            hold: 0,
            gain_db: 0.,
            attack: 0.,
            release: 0.,
            idle: false,
            models_idle: false,
            ducked_pending: 0,
            #[cfg(test)]
            detector_samples: 0,
            #[cfg(test)]
            inference_frames: 0,
            status: DuckingStatus::default(),
        };
        s.coefficients();
        s
    }
    fn coefficients(&mut self) {
        self.attack = (-1. / (self.config.attack * 48.)).exp();
        self.release = (-1. / (self.config.release * 48.)).exp();
    }
    pub fn configure(&mut self, config: &DuckingConfig) {
        if self.config.detection != config.detection {
            self.reset_models();
            self.rms.fill([0.; 2]);
            self.sum = [0.; 2];
            self.cursor = 0;
            self.status.active = false;
            self.hold = 0;
        }
        if self.config.delay() != config.delay() {
            self.priority_delay.fill([0.; 2]);
            self.ducked_delay.fill([0.; 2]);
            self.delay_cursor = 0;
            self.filled = 0;
            self.ducked_pending = 0;
            self.fade = 0;
        }
        self.config = config.clone();
        self.coefficients();
    }
    fn reset_models(&mut self) {
        for model in &mut self.models {
            **model = (*self.pristine).clone();
        }
        self.speech_input.fill([0.; WINDOW]);
        self.speech_output.fill([0.; WINDOW]);
        self.speech_cursor = 0;
        self.status.speech_probability = 0.;
        self.status.fallback = false;
    }
    /// Control-plane retry; neither model reinitialization nor logging occurs in process.
    pub fn recover(&mut self) {
        if self.status.fallback || self.models_idle {
            self.reset_models();
            self.models_idle = false;
        }
    }
    pub fn status(&self) -> DuckingStatus {
        self.status
    }
    fn detect(&mut self, level: f32, probability: f32) {
        let threshold =
            db_to_gain(self.config.threshold - if self.status.active { 6. } else { 0. });
        let speech_threshold =
            (self.config.sensitivity / 100. - if self.status.active { 0.15 } else { 0. }).max(0.01);
        self.status.active = level >= threshold
            && (self.config.detection == Detection::Level
                || self.status.fallback
                || probability >= speech_threshold);
        if self.status.active {
            self.hold = (self.config.hold * SAMPLE_RATE as f32 / 1000.).round() as usize;
        }
    }
    /// Standalone callers provide both buffers. Graph callers also supply cached
    /// port presence so disconnected inputs require no signal scan.
    pub fn process(
        &mut self,
        priority: &mut [Frame],
        ducked: &mut [Frame],
        mixed: &mut [Frame],
        bypass: bool,
    ) {
        self.process_connected(priority, ducked, mixed, bypass, [true, true]);
    }
    fn idle_detector(&mut self) {
        if !self.idle {
            self.rms.fill([0.; 2]);
            self.sum = [0.; 2];
            self.cursor = 0;
            self.speech_input.fill([0.; WINDOW]);
            self.speech_cursor = 0;
            self.models_idle = self.config.detection == Detection::Speech;
        }
        self.idle = true;
        self.status.active = false;
        self.status.speech_probability = 0.;
    }
    fn envelope(&mut self, target: f32) -> f32 {
        let coefficient = if target < self.gain_db {
            self.attack
        } else {
            self.release
        };
        self.gain_db = target + (self.gain_db - target) * coefficient;
        if target == 0. && self.gain_db.abs() < 0.0001 {
            self.gain_db = 0.;
        }
        if self.gain_db == 0. {
            1.
        } else {
            db_to_gain(self.gain_db)
        }
    }
    fn route(&mut self, p: &mut Frame, d: &mut Frame, m: &mut Frame, gain: f32, delay: usize) {
        let ready = delay == 0 || self.filled >= delay;
        if delay > 0 {
            self.priority_delay[self.delay_cursor] = *p;
            self.ducked_delay[self.delay_cursor] = *d;
            if d.iter().any(|v| *v != 0.) {
                self.ducked_pending = delay;
            } else {
                self.ducked_pending = self.ducked_pending.saturating_sub(1);
            }
            let read = (self.delay_cursor + DELAY - delay) % DELAY;
            if !ready {
                *p = [0.; 2];
                *d = [0.; 2];
                self.filled += 1;
            } else {
                *p = self.priority_delay[read];
                *d = self.ducked_delay[read];
            }
            self.delay_cursor = (self.delay_cursor + 1) % DELAY;
        }
        let fade = (self.fade as f32 / 240.).min(1.);
        if ready {
            self.fade = (self.fade + 1).min(240);
        }
        if fade != 1. {
            for c in 0..2 {
                p[c] *= fade;
                d[c] *= fade;
            }
        }
        if gain != 1. {
            for sample in d.iter_mut() {
                *sample *= gain;
            }
        }
        *m = [p[0] + d[0], p[1] + d[1]];
    }
    pub fn process_connected(
        &mut self,
        priority: &mut [Frame],
        ducked: &mut [Frame],
        mixed: &mut [Frame],
        bypass: bool,
        connected: [bool; 2],
    ) {
        fn clean_peak(frames: &mut [Frame], connected: bool) -> f32 {
            if !connected {
                frames.fill([0.; 2]);
                return 0.;
            }
            let mut peak = 0_f32;
            for frame in frames {
                for sample in frame {
                    if !sample.is_finite() {
                        *sample = 0.;
                    }
                    peak = peak.max(sample.abs());
                }
            }
            peak
        }
        self.status.priority_peak = clean_peak(priority, connected[0]);
        self.status.ducked_peak = clean_peak(ducked, connected[1]);
        let priority_active = self.status.priority_peak != 0.;
        let ducked_active = self.status.ducked_peak != 0. || self.ducked_pending > 0;
        let delay = self.config.delay();
        self.status.fast_path = !priority_active || !ducked_active;
        if self.status.fast_path {
            self.idle_detector();
            // No background audio (including buffered lookahead): attenuation
            // cannot be audible, so discard old hold/envelope without a click.
            if !ducked_active {
                self.hold = 0;
                self.gain_db = 0.;
            }
            let unity =
                self.gain_db == 0. && (bypass || self.hold == 0 || self.config.amount == 0.);
            if unity && delay == 0 && self.fade == 240 {
                self.hold = self.hold.saturating_sub(mixed.len());
                if !priority_active {
                    mixed.copy_from_slice(ducked);
                } else {
                    mixed.copy_from_slice(priority);
                }
            } else {
                for ((p, d), m) in priority.iter_mut().zip(ducked).zip(mixed) {
                    let target = if !bypass && self.hold > 0 {
                        self.config.amount
                    } else {
                        0.
                    };
                    self.hold = self.hold.saturating_sub(1);
                    let gain = if self.gain_db == 0. && target == 0. {
                        1.
                    } else {
                        self.envelope(target)
                    };
                    self.route(p, d, m, gain, delay);
                }
            }
            self.status.reduction_db = self.gain_db;
            return;
        }
        // Resume on a fresh analysis window; do not splice partial frames or
        // reuse a held probability from before the signal disappeared. Frozen
        // model state can resume until control-plane maintenance resets it idle.
        self.idle = false;
        self.models_idle = false;
        for ((p, d), m) in priority.iter_mut().zip(ducked).zip(mixed) {
            #[cfg(test)]
            {
                self.detector_samples += 1;
            }
            for (c, &v) in p.iter().enumerate() {
                let sample = v.clamp(-16., 16.);
                let power = sample * sample;
                self.sum[c] += (power - self.rms[self.cursor][c]) as f64;
                self.rms[self.cursor][c] = power;
                if self.config.detection == Detection::Speech {
                    self.speech_input[c][self.speech_cursor] = v.clamp(-1., 1.) * 32768.;
                }
            }
            self.cursor = (self.cursor + 1) % WINDOW;
            self.speech_cursor += 1;
            if self.speech_cursor == WINDOW {
                self.speech_cursor = 0;
                if self.config.detection == Detection::Speech && !self.status.fallback {
                    #[cfg(test)]
                    {
                        self.inference_frames += 1;
                    }
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut probability = 0_f32;
                        for c in 0..2 {
                            let v = self.models[c]
                                .process_frame(&mut self.speech_output[c], &self.speech_input[c]);
                            if !v.is_finite() {
                                return None;
                            }
                            probability = probability.max(v);
                        }
                        Some(probability.clamp(0., 1.))
                    }));
                    if let Ok(Some(probability)) = result {
                        self.status.speech_probability = probability;
                    } else {
                        self.status.fallback = true;
                    }
                }
            }
            let level = (self.sum[0].max(self.sum[1]).max(0.) / WINDOW as f64).sqrt() as f32;
            self.detect(level, self.status.speech_probability);
            let target = if !bypass && (self.status.active || self.hold > 0) {
                self.config.amount
            } else {
                0.
            };
            self.hold = self.hold.saturating_sub(1);
            let gain = self.envelope(target);
            self.route(p, d, m, gain, delay);
        }
        self.status.reduction_db = self.gain_db;
    }
}
