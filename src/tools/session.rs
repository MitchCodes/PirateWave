use super::analysis::{Analyzer, Reading};
use crate::audio_io;
use crossbeam_queue::ArrayQueue;
use piratewave_core::{
    effects::{AudioEffect, EffectRegistry, EffectSpec},
    Frame, SAMPLE_RATE,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Meter,
    Quiet,
    Spectrum,
    Monitor,
    Voice,
    Tone,
    Stereo,
    Latency,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    Sine,
    Square,
    Triangle,
    Saw,
}
impl Wave {
    pub const ALL: [Self; 4] = [Self::Sine, Self::Square, Self::Triangle, Self::Saw];
    pub fn name(self) -> &'static str {
        match self {
            Self::Sine => "Sine",
            Self::Square => "Square",
            Self::Triangle => "Triangle",
            Self::Saw => "Sawtooth",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Config {
    pub mode: Mode,
    pub input: Option<String>,
    pub output: Option<String>,
    pub seconds: f32,
    pub fft_size: usize,
    pub frequency: f32,
    pub level_db: f32,
    pub wave: Wave,
    /// 0 both, 1 left, 2 right, 3 inverted right.
    pub channel: usize,
    pub effects: Vec<EffectSpec>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Meter,
            input: None,
            output: None,
            seconds: 5.,
            fft_size: 2048,
            frequency: 440.,
            level_db: -24.,
            wave: Wave::Sine,
            channel: 0,
            effects: vec![],
        }
    }
}
impl Config {
    fn validate(&self) -> Result<(), String> {
        if !self.seconds.is_finite()
            || !(1.0..=600.).contains(&self.seconds)
            || !self.frequency.is_finite()
            || !(20.0..=20000.).contains(&self.frequency)
            || !self.level_db.is_finite()
            || !(-60.0..=-6.0).contains(&self.level_db)
            || self.channel > 3
            || ![2048, 8192].contains(&self.fft_size)
        {
            return Err("Invalid workbench audio configuration".into());
        }
        let registry = EffectRegistry::default();
        for e in &self.effects {
            registry.validate(e)?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub running: bool,
    pub message: String,
    pub elapsed: f32,
    pub input_rate: u32,
    pub output_rate: u32,
    pub output_channels: u16,
    pub reading: Reading,
    pub spectrum: Vec<f32>,
    pub waveform: Vec<f32>,
    pub latency_ms: Option<f32>,
    pub confidence: f32,
    pub dropped: u64,
    pub underruns: u64,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            running: false,
            message: "Ready. No tool audio is active.".into(),
            elapsed: 0.,
            input_rate: 0,
            output_rate: 0,
            output_channels: 0,
            reading: Reading::default(),
            spectrum: vec![],
            waveform: vec![],
            latency_ms: None,
            confidence: 0.,
            dropped: 0,
            underruns: 0,
        }
    }
}
pub struct Session {
    pub state: Arc<arc_swap::ArcSwap<Snapshot>>,
    stop: Arc<AtomicBool>,
    updates: mpsc::SyncSender<Config>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Session {
    pub fn start(config: Config) -> Result<Self, String> {
        config.validate()?;
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(arc_swap::ArcSwap::from_pointee(Snapshot {
            running: true,
            message: "Opening tool devices…".into(),
            ..Snapshot::default()
        }));
        let (tx, rx) = mpsc::sync_channel(8);
        let (s, t) = (state.clone(), stop.clone());
        let worker = thread::Builder::new()
            .name("piratewave-workbench".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(config, rx, &t, &s)
                }));
                let mut last = (**s.load()).clone();
                last.running = false;
                last.message = match result {
                    Ok(Ok(message)) => message,
                    Ok(Err(error)) => {
                        crate::logging::error(format_args!("Workbench: {error}"));
                        format!("Tool stopped: {error}")
                    }
                    Err(_) => {
                        "Tool stopped after an internal error. Its devices have been released."
                            .into()
                    }
                };
                s.store(Arc::new(last));
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            state,
            stop,
            updates: tx,
            worker: Some(worker),
        })
    }
    pub fn update(&self, config: Config) -> Result<(), String> {
        config.validate()?;
        self.updates
            .try_send(config)
            .map_err(|_| "Tool control queue is busy; try again".into())
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
    pub fn finished(&self) -> bool {
        self.worker.as_ref().is_none_or(|w| w.is_finished())
    }
    #[cfg(test)]
    pub(crate) fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
        // Device opening/closing stays off the UI thread. A live session is kept
        // by the UI until its worker finishes, preventing overlapping restarts.
        if self.worker.as_ref().is_some_and(|w| w.is_finished()) {
            let _ = self.worker.take().unwrap().join();
        }
    }
}

pub fn chirp(rate: u32) -> Vec<f32> {
    let length = (rate as f32 * 0.025) as usize;
    (0..length)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let envelope = (std::f32::consts::PI * i as f32 / (length - 1) as f32)
                .sin()
                .powi(2);
            (std::f32::consts::TAU * (900. * t + 22000. * t * t)).sin() * envelope * 0.125
        })
        .collect()
}
pub struct Oscillator {
    phase: f64,
    gain: f32,
    frequency: f32,
}
impl Oscillator {
    pub fn new() -> Self {
        Self {
            phase: 0.,
            gain: 0.,
            frequency: 440.,
        }
    }
    pub fn next(&mut self, c: &Config, ending: bool) -> Frame {
        self.frequency += (c.frequency - self.frequency) * 0.002;
        let target = if ending {
            0.
        } else {
            10_f32.powf(c.level_db / 20.)
        };
        self.gain += (target - self.gain) * 0.005;
        self.phase = (self.phase + self.frequency as f64 / SAMPLE_RATE as f64).fract();
        // Fourier series bounded below Nyquist avoids the obvious high-frequency
        // aliasing of discontinuous square and saw waveforms.
        let max = (SAMPLE_RATE as f32 * 0.48 / self.frequency)
            .floor()
            .clamp(1., 63.) as usize;
        let phase = self.phase as f32 * std::f32::consts::TAU;
        let value=match c.wave {
            Wave::Sine=>phase.sin(),
            Wave::Square=>(1..=max).step_by(2).map(|h|(phase*h as f32).sin()/h as f32).sum::<f32>()*4./std::f32::consts::PI,
            Wave::Saw=>(1..=max).map(|h|if h%2==0 {-1.}else{1.}*(phase*h as f32).sin()/h as f32).sum::<f32>()*2./std::f32::consts::PI,
            Wave::Triangle=>(1..=max).step_by(2).enumerate().map(|(i,h)|if i%2==0 {1.}else{-1.}*(phase*h as f32).sin()/(h*h) as f32).sum::<f32>()*8./std::f32::consts::PI.powi(2),
        }.clamp(-1.,1.)*self.gain;
        match c.channel {
            1 => [value, 0.],
            2 => [0., value],
            3 => [value, -value],
            _ => [value, value],
        }
    }
}
fn effects(config: &Config) -> Result<Vec<Box<dyn AudioEffect>>, String> {
    let registry = EffectRegistry::default();
    config
        .effects
        .iter()
        .filter(|e| e.enabled)
        .map(|s| registry.create(s, SAMPLE_RATE as f32))
        .collect()
}
fn run(
    mut config: Config,
    updates: mpsc::Receiver<Config>,
    stop: &AtomicBool,
    state: &arc_swap::ArcSwap<Snapshot>,
) -> Result<String, String> {
    let capture = matches!(
        config.mode,
        Mode::Meter | Mode::Quiet | Mode::Spectrum | Mode::Monitor | Mode::Voice | Mode::Latency
    );
    let playback = matches!(
        config.mode,
        Mode::Monitor | Mode::Voice | Mode::Tone | Mode::Stereo | Mode::Latency
    );
    let counters = Arc::new(audio_io::Counters::default());
    let mut input = if capture {
        Some(
            audio_io::open_probe_input(
                &config.input,
                counters.clone(),
                Arc::new(ArrayQueue::new(8)),
            )
            .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let mut output = if playback {
        Some(
            audio_io::open_output(
                0,
                &config.output,
                counters.clone(),
                Arc::new(ArrayQueue::new(8)),
            )
            .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let rate = input.as_ref().map_or(SAMPLE_RATE, |s| s.sample_rate);
    let mut analyzer = Analyzer::new(rate, config.fft_size);
    let mut processors = if config.mode == Mode::Voice {
        effects(&config)?
    } else {
        vec![]
    };
    let mut oscillator = Oscillator::new();
    let start = Instant::now();
    let mut published = start - Duration::from_secs(1);
    let mut previous = [0.; 2];
    let mut phase = 0_f64;
    let mut monitor = Vec::<Frame>::with_capacity(8192);
    let pulse = chirp(SAMPLE_RATE);
    let mut pulse_pos = None;
    let mut pulse_at = None;
    let mut echo = Vec::<f32>::with_capacity(rate as usize * 2);
    let mut echo_times = Vec::<f32>::with_capacity(rate as usize * 2);
    let mut snap = Snapshot {
        running: true,
        input_rate: if capture { rate } else { 0 },
        output_rate: output.as_ref().map_or(0, |o| o.sample_rate),
        output_channels: output.as_ref().map_or(0, |o| o.channels),
        ..Snapshot::default()
    };
    let mut stop_at: Option<Instant> = None;
    let mut monitor_dropped = 0_u64;
    let mut monitor_gain = 0_f32;
    loop {
        let elapsed = start.elapsed().as_secs_f32();
        if config.mode == Mode::Stereo && elapsed >= config.seconds {
            stop_at.get_or_insert_with(Instant::now);
        }
        if stop.load(Ordering::Acquire) {
            if !matches!(config.mode, Mode::Tone | Mode::Stereo) {
                return Ok("Tool stopped. Devices released.".into());
            }
            stop_at.get_or_insert_with(Instant::now);
        }
        if stop_at.is_some_and(|t| t.elapsed() > Duration::from_millis(150)) {
            return Ok("Playback stopped. Devices released.".into());
        }
        for next in updates.try_iter().take(8) {
            if next.mode != config.mode
                || next.input != config.input
                || next.output != config.output
            {
                return Err("Stop the tool before changing devices or mode".into());
            }
            if next.fft_size != config.fft_size {
                analyzer = Analyzer::new(rate, next.fft_size);
            }
            if config.mode == Mode::Voice && next.effects != config.effects {
                if next
                    .effects
                    .iter()
                    .map(|s| &s.kind)
                    .eq(config.effects.iter().map(|s| &s.kind))
                    && next.effects.len() == processors.len()
                {
                    let registry = EffectRegistry::default();
                    for (fx, spec) in processors.iter_mut().zip(&next.effects) {
                        registry.configure(fx.as_mut(), spec)?;
                    }
                } else {
                    processors = effects(&next)?;
                }
            }
            config = next;
        }
        if let Some(i) = &mut input {
            if let Some(e) = i.health.failure() {
                return Err(e);
            }
            for _ in 0..(rate as usize / 5).max(1024) {
                let Some(frame) = i.queue.pop() else { break };
                analyzer.push(frame);
                if config.mode == Mode::Latency
                    && pulse_at.is_some()
                    && echo.len() < rate as usize * 2
                {
                    // Keep channels separate until choosing the louder signal.
                    // Opposite-polarity stereo must not cancel the test chirp.
                    echo.push(if frame[0].abs() >= frame[1].abs() {
                        frame[0]
                    } else {
                        frame[1]
                    });
                    echo_times.push(start.elapsed().as_secs_f32());
                }
                if matches!(config.mode, Mode::Monitor | Mode::Voice) {
                    let ratio = SAMPLE_RATE as f64 / rate as f64;
                    phase += ratio;
                    while phase >= 1. {
                        phase -= 1.;
                        let t = (1. - phase / ratio) as f32;
                        monitor.push([
                            previous[0] + (frame[0] - previous[0]) * t,
                            previous[1] + (frame[1] - previous[1]) * t,
                        ]);
                    }
                    previous = frame;
                }
            }
        }
        if let Some(o) = &mut output {
            if let Some(e) = o.health.failure() {
                return Err(e);
            }
            if matches!(config.mode, Mode::Monitor | Mode::Voice) {
                for block in monitor.chunks_mut(piratewave_core::BLOCK_SIZE) {
                    for fx in &mut processors {
                        fx.process(block);
                        if fx.faulted() {
                            return Err("An audition effect failed; monitoring stopped".into());
                        }
                    }
                }
                let gain = 10_f32.powf(config.level_db / 20.);
                for frame in monitor.drain(..) {
                    monitor_gain += (gain - monitor_gain) * 0.005;
                    if o.queue
                        .push(frame.map(|v| (v * monitor_gain).clamp(-0.9, 0.9)))
                        .is_err()
                    {
                        monitor_dropped += 1;
                    }
                }
            } else {
                while o.queue.len() < audio_io::TARGET_FRAMES {
                    let value = if config.mode == Mode::Latency {
                        if elapsed >= 0.7 && pulse_at.is_none() {
                            pulse_at = Some(start.elapsed().as_secs_f32());
                            pulse_pos = Some(0);
                        }
                        if let Some(pos) = pulse_pos {
                            let value = pulse[pos];
                            pulse_pos = if pos + 1 < pulse.len() {
                                Some(pos + 1)
                            } else {
                                None
                            };
                            [value, value]
                        } else {
                            [0.; 2]
                        }
                    } else {
                        oscillator.next(&config, stop_at.is_some())
                    };
                    if o.queue.push(value).is_err() {
                        break;
                    }
                }
            }
        }
        if published.elapsed() >= Duration::from_millis(50) {
            snap.running = true;
            snap.elapsed = elapsed;
            if capture {
                snap.spectrum = analyzer.spectrum();
                snap.reading = analyzer.reading();
                snap.waveform = analyzer.waveform();
            }
            snap.dropped = counters.overflows.load(Ordering::Relaxed) + monitor_dropped;
            snap.underruns = counters.underflows.load(Ordering::Relaxed);
            snap.message = match config.mode {
                Mode::Quiet => "Stay quiet while the room is measured.",
                Mode::Meter => {
                    if elapsed < 2. {
                        "Speak normally, then stay quiet for the remaining time."
                    } else {
                        "Stay quiet to measure the background."
                    }
                }
                Mode::Latency => "Listening for the returning chirp…",
                Mode::Voice => "Live audition · your project effects are unchanged.",
                Mode::Monitor => "Headphone monitoring active.",
                Mode::Spectrum => "Live spectrum · raw shared-mode input.",
                _ => "Test signal playing.",
            }
            .into();
            state.store(Arc::new(snap.clone()));
            published = Instant::now();
        }
        if config.mode == Mode::Latency && pulse_at.is_some_and(|at| elapsed - at > 1.7) {
            let reference = chirp(rate);
            let down = (rate / 8000).max(1) as usize;
            let reduced: Vec<f32> = echo.iter().step_by(down).copied().collect();
            let reference: Vec<f32> = reference.iter().step_by(down).copied().collect();
            let found = super::analysis::correlate(&reduced, &reference);
            if let Some((offset, score)) = found.filter(|_| snap.dropped == 0) {
                let index = offset * down;
                let rms = (echo[index..(index + reference.len() * down).min(echo.len())]
                    .iter()
                    .map(|x| x * x)
                    .sum::<f32>()
                    / (reference.len() * down) as f32)
                    .sqrt();
                if rms > 0.0005 && super::db(rms as f64) > snap.reading.floor_db + 8. {
                    let latency = (echo_times[index] - pulse_at.unwrap()) * 1000.;
                    if (0.0..=1500.).contains(&latency) {
                        snap.latency_ms = Some(latency);
                        snap.confidence = score;
                        snap.running = false;
                        state.store(Arc::new(snap));
                        return Ok("Chirp matched. This is the separate tool loop, not the project's graph latency.".into());
                    }
                }
            }
            return Ok("No reliable echo detected. Check the selected speaker/mic, turn off monitoring and try a quiet room. No latency value was inferred.".into());
        }
        if elapsed >= config.seconds
            && !matches!(
                config.mode,
                Mode::Tone | Mode::Stereo | Mode::Spectrum | Mode::Monitor | Mode::Voice
            )
        {
            snap.spectrum = analyzer.spectrum();
            snap.reading = analyzer.reading();
            snap.waveform = analyzer.waveform();
            snap.elapsed = elapsed;
            snap.running = false;
            state.store(Arc::new(snap));
            return Ok(if analyzer.reading().frames == 0 && capture {
                "No input samples arrived; check the device."
            } else {
                "Measurement complete. Devices released."
            }
            .into());
        }
        if elapsed > 600. {
            return Ok("Tool stopped after ten minutes. Start again to continue.".into());
        }
        thread::sleep(Duration::from_millis(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generators_are_stereo_correct_bounded_finite_and_fade_to_silence() {
        for wave in Wave::ALL {
            for channel in 0..4 {
                let c = Config {
                    wave,
                    channel,
                    level_db: -6.,
                    ..Config::default()
                };
                let mut osc = Oscillator::new();
                for _ in 0..10000 {
                    let f = osc.next(&c, false);
                    assert!(f.iter().all(|v| v.is_finite() && v.abs() <= 0.502));
                    match channel {
                        1 => assert_eq!(f[1], 0.),
                        2 => assert_eq!(f[0], 0.),
                        3 => assert_eq!(f[0], -f[1]),
                        _ => assert_eq!(f[0], f[1]),
                    }
                }
                let mut end = [0.; 2];
                for _ in 0..5000 {
                    end = osc.next(&c, true);
                }
                assert!(end.iter().all(|v| v.abs() < 1e-7));
            }
        }
    }
    #[test]
    fn invalid_configuration_cannot_start_a_device() {
        let c = Config {
            level_db: 0.,
            ..Config::default()
        };
        assert!(Session::start(c).is_err());
        let c = Config {
            frequency: f32::NAN,
            ..Config::default()
        };
        assert!(Session::start(c).is_err());
        let pulse = chirp(48000);
        assert_eq!(pulse.len(), 1200);
        assert!(pulse.iter().all(|x| x.abs() <= 0.125));
    }
}
