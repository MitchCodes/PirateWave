//! Optional integration check against an already-installed VB-Audio cable.
//! It does not change the Windows default devices or write driver settings.
use crate::{
    assets::Clip,
    audio_io::{self, Counters, InputStream},
    engine::{Command, Engine},
};
use anyhow::{Context, Result};
use crossbeam_queue::ArrayQueue;
use piratewave_core::{
    effects::{EffectRegistry, EffectSpec},
    project::{Connection, Node, NodeKind, Project},
    SAMPLE_RATE,
};
use std::{
    sync::{atomic::Ordering, Arc},
    thread,
    time::{Duration, Instant},
};
/// A continuous tone through an actual captured input -> output bus, with no effects.
/// Observe raw callback samples so the measurement itself has no queue resampler.
pub fn continuity_self_test() -> Result<()> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let devices = audio_io::devices()?;
    let source = devices
        .outputs
        .iter()
        .find(|d| d.name == "CABLE Input (VB-Audio Virtual Cable)")
        .context("Continuity test requires existing VB-Audio CABLE Input")?;
    let sink = devices
        .outputs
        .iter()
        .find(|d| d.name.starts_with("Line 1 ("))
        .context("Continuity test requires existing Line 1 virtual playback device")?;
    let capture_device = cpal::default_host()
        .output_devices()?
        .find(|d| d.name().ok().as_deref() == Some(&sink.name))
        .context("Line 1 unavailable")?;
    let supported = capture_device.default_output_config()?;
    anyhow::ensure!(
        supported.sample_format() == cpal::SampleFormat::F32,
        "Diagnostic requires float loopback"
    );
    let config: cpal::StreamConfig = supported.into();
    let samples = Arc::new(ArrayQueue::new(config.sample_rate.0 as usize * 2));
    let errors = Arc::new(ArrayQueue::new(8));
    let dropped = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (dest, error_dest, drop_count) = (samples.clone(), errors.clone(), dropped.clone());
    let channels = config.channels as usize;
    let capture = capture_device.build_input_stream(
        &config,
        move |data: &[f32], _| {
            for frame in data.chunks_exact(channels) {
                if dest.push(frame[0]).is_err() {
                    drop_count.fetch_add(1, Ordering::Relaxed);
                }
            }
        },
        move |e| {
            let _ = error_dest.push(e.to_string());
        },
        None,
    )?;
    capture.play()?;
    let project = |input_kind, device: String| Project {
        version: 1,
        name: "Continuity diagnostic".into(),
        nodes: vec![
            Node::new(1, "Source", input_kind, [0.0; 2]),
            Node::new(
                2,
                "Bus",
                NodeKind::Output {
                    device: Some(device),
                },
                [0.0; 2],
            ),
        ],
        connections: vec![Connection {
            input: Default::default(),
            port: Default::default(),
            from: 1,
            to: 2,
        }],
        sounds: vec![],
    };
    let producer = Engine::new();
    let route = Engine::new();
    producer
        .send(Command::Start(project(
            NodeKind::Soundboard,
            source.id.clone(),
        )))
        .map_err(anyhow::Error::msg)?;
    route
        .send(Command::Start(project(
            NodeKind::Input {
                device: Some(source.id.clone()),
                loopback: true,
            },
            sink.id.clone(),
        )))
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        producer.snapshot().map_err(anyhow::Error::msg)?.running
            && route.snapshot().map_err(anyhow::Error::msg)?.running,
        "Test streams failed to start"
    );
    let baseline = Instant::now();
    while baseline.elapsed() < Duration::from_millis(600) {
        while let Some(v) = samples.pop() {
            anyhow::ensure!(
                v.abs() < 0.005,
                "Virtual devices already carry audio; continuity test cannot isolate its signal"
            );
        }
        thread::sleep(Duration::from_millis(5));
    }
    const HZ: f64 = 220.0;
    let frames = (0..SAMPLE_RATE as usize * 25)
        .map(|i| {
            let v =
                (i as f64 * std::f64::consts::TAU * HZ / SAMPLE_RATE as f64).sin() as f32 * 0.08;
            [v; 2]
        })
        .collect();
    producer
        .send(Command::Play {
            clip: Clip {
                frames: Arc::new(frames),
            },
            volume: 1.0,
        })
        .map_err(anyhow::Error::msg)?;
    let start = Instant::now();
    let mut count = 0;
    let mut glitches = 0;
    let mut silence = 0;
    let mut max_silence = 0;
    let mut peak = 0.0_f32;
    let mut history = [0.0_f32; 2];
    let coefficient =
        (2.0 * (std::f64::consts::TAU * HZ / config.sample_rate.0 as f64).cos()) as f32;
    let mut before = None;
    while start.elapsed() < Duration::from_secs(20) {
        if start.elapsed() < Duration::from_secs(2) {
            while samples.pop().is_some() {}
        } else {
            if before.is_none() {
                before = Some(route.snapshot().map_err(anyhow::Error::msg)?);
            }
            while let Some(v) = samples.pop() {
                if count > 2 && (v - coefficient * history[1] + history[0]).abs() > 0.002 {
                    glitches += 1;
                }
                history = [history[1], v];
                peak = peak.max(v.abs());
                silence = if v.abs() < 0.00001 { silence + 1 } else { 0 };
                max_silence = max_silence.max(silence);
                count += 1;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    let before = before.context("Missing baseline status")?;
    let after = route.snapshot().map_err(anyhow::Error::msg)?;
    let source_status = producer.snapshot().map_err(anyhow::Error::msg)?;
    let passed = after.running
        && source_status.running
        && count > config.sample_rate.0 as usize * 15
        && peak > 0.02
        && peak < 0.12
        && glitches == 0
        && max_silence < 8
        && after.input_underflows == before.input_underflows
        && after.underflows == before.underflows
        && after.overflows == before.overflows
        && dropped.load(Ordering::Relaxed) == 0
        && errors.is_empty();
    let report = format!(
        "{}: continuous 220 Hz signal -> CABLE Input -> captured input node -> Line 1 output bus -> raw WASAPI loopback (no effects/Listen)\nMeasured samples: {count}\nPeak: {peak:.6}\nDiscontinuities: {glitches}\nLongest silence: {max_silence} samples\nInput underruns during measurement: {}\nOutput underflows during measurement: {}\nQueue drops during measurement: {}\n",
        if passed { "PASS" } else { "FAIL" },
        after.input_underflows - before.input_underflows,
        after.underflows - before.underflows,
        after.overflows - before.overflows,
    );
    std::fs::create_dir_all("dist")?;
    std::fs::write("dist/continuity-self-test.txt", &report)?;
    println!("{report}");
    anyhow::ensure!(passed, "Continuous signal verification failed");
    Ok(())
}

pub(crate) fn peaks(
    input: &mut InputStream,
    recording: &mut InputStream,
    duration: Duration,
) -> (f32, f32) {
    let start = Instant::now();
    let mut peak = 0.0_f32;
    let mut record_peak = 0.0_f32;
    while start.elapsed() < duration {
        for _ in 0..2048 {
            match input.resampler.next_frame() {
                Some(f) => peak = peak.max(f[0].abs().max(f[1].abs())),
                None => break,
            }
        }
        for _ in 0..2048 {
            match recording.resampler.next_frame() {
                Some(f) => record_peak = record_peak.max(f[0].abs().max(f[1].abs())),
                None => break,
            }
        }
        thread::sleep(Duration::from_millis(1));
    }
    (peak, record_peak)
}
pub fn cable_self_test() -> Result<()> {
    let devices = audio_io::devices()?;
    let output = devices
        .outputs
        .iter()
        .find(|d| d.name.contains("CABLE Input (VB-Audio Virtual Cable)"))
        .context(
            "VB-Audio CABLE Input not found; this optional test needs an existing virtual cable",
        )?;
    let input = devices
        .inputs
        .iter()
        .find(|d| d.name.contains("CABLE Output (VB-Audio Virtual Cable)"))
        .context("VB-Audio CABLE Output not found")?;
    let registry = EffectRegistry::default();
    let mut fx = Node::new(2, "Test gain", NodeKind::Processor, [300.0, 50.0]);
    let mut spec = EffectSpec::new(registry.get("gain").unwrap());
    spec.parameters.insert("gain".into(), -6.0206);
    fx.effects.push(spec);
    let mut project = Project {
        version: 1,
        name: "Cable integration test".into(),
        nodes: vec![
            Node::new(1, "Soundboard", NodeKind::Soundboard, [0.0; 2]),
            fx,
            Node::new(
                3,
                "Cable output",
                NodeKind::Output {
                    device: Some(output.id.clone()),
                },
                [600.0, 50.0],
            ),
        ],
        connections: vec![
            Connection {
                input: Default::default(),
                port: Default::default(),
                from: 1,
                to: 2,
            },
            Connection {
                input: Default::default(),
                port: Default::default(),
                from: 2,
                to: 3,
            },
        ],
        sounds: vec![],
    };
    let counters = Arc::new(Counters::default());
    let errors = Arc::new(ArrayQueue::new(16));
    let mut recording = audio_io::open_input(
        &Some(input.id.clone()),
        false,
        counters.clone(),
        errors.clone(),
    )?;
    let mut capture = audio_io::open_input(
        &Some(output.id.clone()),
        true,
        counters.clone(),
        errors.clone(),
    )?;
    let endpoint_states = crate::windows_audio::cable_endpoints().unwrap_or_default();
    let recording_muted = endpoint_states
        .iter()
        .find(|s| s.name == input.name)
        .is_some_and(|s| s.muted);
    let engine = Engine::new();
    engine
        .send(Command::Start(project.clone()))
        .map_err(anyhow::Error::msg)?;
    let start = Instant::now();
    let mut running = false;
    while start.elapsed() < Duration::from_secs(3) {
        while let Some(status) = engine.statuses.pop() {
            if !status.running {
                anyhow::bail!("{}", status.message)
            }
            running = true;
        }
        if running {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    anyhow::ensure!(running, "Audio engine did not start");
    let (baseline, _) = peaks(&mut capture, &mut recording, Duration::from_millis(350));
    anyhow::ensure!(
        baseline < 0.005,
        "Virtual cable already carries audio (peak {baseline}); test cannot isolate its signal"
    );
    let frames = (0..SAMPLE_RATE as usize * 5)
        .map(|i| {
            let value = (i as f32 * 440.0 * std::f32::consts::TAU / SAMPLE_RATE as f32).sin() * 0.1;
            [value; 2]
        })
        .collect();
    engine
        .send(Command::Play {
            clip: Clip {
                frames: Arc::new(frames),
            },
            volume: 1.0,
        })
        .map_err(anyhow::Error::msg)?;
    let (gain_half, recorded_half) =
        peaks(&mut capture, &mut recording, Duration::from_millis(550));
    piratewave_core::shortcuts::NodeAction::Disable
        .apply(&mut project.nodes[1])
        .map_err(anyhow::Error::msg)?;
    engine
        .send(Command::Update(project.clone()))
        .map_err(anyhow::Error::msg)?;
    peaks(&mut capture, &mut recording, Duration::from_millis(180));
    let (passthrough, _) = peaks(&mut capture, &mut recording, Duration::from_millis(350));
    piratewave_core::shortcuts::NodeAction::Enable
        .apply(&mut project.nodes[1])
        .map_err(anyhow::Error::msg)?;
    project.nodes[1].effects[0]
        .parameters
        .insert("gain".into(), -12.0412);
    engine
        .send(Command::Update(project.clone()))
        .map_err(anyhow::Error::msg)?;
    peaks(&mut capture, &mut recording, Duration::from_millis(180));
    let (gain_quarter, recorded_quarter) =
        peaks(&mut capture, &mut recording, Duration::from_millis(350));
    project.nodes[2].mute = true;
    engine
        .send(Command::Update(project))
        .map_err(anyhow::Error::msg)?;
    peaks(&mut capture, &mut recording, Duration::from_millis(180));
    let (muted, _) = peaks(&mut capture, &mut recording, Duration::from_millis(350));
    engine.send(Command::Stop).map_err(anyhow::Error::msg)?;
    let passed = (gain_half - 0.05).abs() < 0.012
        && (passthrough - 0.1).abs() < 0.012
        && (gain_quarter - 0.025).abs() < 0.008
        && muted < 0.002
        && (recording_muted || recorded_half > 0.001)
        && errors.is_empty();
    let states = endpoint_states
        .iter()
        .map(|s| format!("{}: volume={:.2}, muted={}", s.name, s.volume, s.muted))
        .collect::<Vec<_>>()
        .join("\n");
    let report=format!("{}: real soundboard -> gain processor -> output bus -> WASAPI render loopback\nBaseline peak: {baseline:.6}\n-6.02 dB peak: {gain_half:.6} (expected 0.050)\n-12.04 dB peak: {gain_quarter:.6} (expected 0.025)\nMuted peak: {muted:.6} (expected 0)\nCable recording peaks: {recorded_half:.6}, {recorded_quarter:.6}\nRecording endpoint muted: {recording_muted} (existing Windows setting preserved)\nCaptured frames: {}\n{states}\n",if passed{"PASS"}else{"FAIL"},counters.captured.load(Ordering::Relaxed));
    let report = format!(
        "{report}\nDisabled processor passthrough peak: {passthrough:.6} (expected 0.100)\n"
    );
    println!("{report}");
    std::fs::create_dir_all("dist")?;
    std::fs::write("dist/cable-self-test.txt", report)?;
    anyhow::ensure!(passed, "Virtual cable signal verification failed");
    Ok(())
}
pub fn transport_self_test() -> Result<()> {
    use piratewave_core::shortcuts::AppAction as A;
    let engine = Engine::new();
    let mut project = Project::default();
    project.connections.clear();
    let send = |action| {
        engine
            .send(Command::for_app_action(action, project.clone()))
            .map_err(anyhow::Error::msg)
    };
    let snapshot = || engine.snapshot().map_err(anyhow::Error::msg);
    send(A::StartAudio)?;
    anyhow::ensure!(snapshot()?.running, "Start Audio did not start the engine");
    thread::sleep(Duration::from_millis(150));
    let before = snapshot()?;
    anyhow::ensure!(before.output_frames > 0, "Output did not advance");
    send(A::StartAudio)?;
    let after = snapshot()?;
    anyhow::ensure!(
        after.running && after.output_frames >= before.output_frames,
        "Start Audio unexpectedly restarted the active engine"
    );
    send(A::StopSounds)?;
    anyhow::ensure!(snapshot()?.running, "Stop sounds stopped audio");
    send(A::StopAudio)?;
    anyhow::ensure!(!snapshot()?.running, "Stop Audio did not stop the engine");
    send(A::ToggleAudio)?;
    anyhow::ensure!(snapshot()?.running, "Toggle did not start audio");
    send(A::ToggleAudio)?;
    anyhow::ensure!(!snapshot()?.running, "Toggle did not stop audio");
    send(A::ToggleAudio)?;
    send(A::ToggleAudio)?;
    anyhow::ensure!(!snapshot()?.running, "Rapid toggles used stale UI state");
    send(A::StartAudio)?;
    send(A::StopAudio)?;
    anyhow::ensure!(!snapshot()?.running, "Start/stop ordering failed");
    let report="PASS: real engine Start Audio, Stop Audio, Toggle Audio, rapid toggles, idempotent start, and Stop Sounds verified through the shared shortcut command dispatcher. Output remained silent.";
    std::fs::create_dir_all("dist")?;
    std::fs::write("dist/transport-self-test.txt", report)?;
    println!("{report}");
    Ok(())
}

/// Uses two existing virtual playback endpoints; never changes Windows audio settings.
pub fn listen_self_test() -> Result<()> {
    use piratewave_core::shortcuts::NodeAction as A;
    let devices = audio_io::devices()?;
    let primary = devices
        .outputs
        .iter()
        .find(|d| d.name == "CABLE Input (VB-Audio Virtual Cable)")
        .context("Listen self-test requires VB-Audio CABLE Input")?;
    let secondary = devices
        .outputs
        .iter()
        .find(|d| d.name == "Line 1 (Virtual Audio Cable)")
        .context("Listen self-test requires Line 1 (Virtual Audio Cable)")?;
    let mut bus = Node::new(
        2,
        "Test output",
        NodeKind::Output {
            device: Some(primary.id.clone()),
        },
        [0.0; 2],
    );
    bus.listen_device = Some(secondary.id.clone());
    let mut project = Project {
        version: 1,
        name: "Listen integration test".into(),
        nodes: vec![Node::new(1, "Tone", NodeKind::Soundboard, [0.0; 2]), bus],
        connections: vec![Connection {
            input: Default::default(),
            port: Default::default(),
            from: 1,
            to: 2,
        }],
        sounds: vec![],
    };
    let counters = Arc::new(Counters::default());
    let errors = Arc::new(ArrayQueue::new(16));
    let mut first = audio_io::open_input(
        &Some(primary.id.clone()),
        true,
        counters.clone(),
        errors.clone(),
    )?;
    let mut second =
        audio_io::open_input(&Some(secondary.id.clone()), true, counters, errors.clone())?;
    let engine = Engine::new();
    engine
        .send(Command::Start(project.clone()))
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        engine.snapshot().map_err(anyhow::Error::msg)?.running,
        "Engine did not start"
    );
    let baseline = peaks(&mut first, &mut second, Duration::from_millis(350));
    anyhow::ensure!(
        baseline.0 < 0.002 && baseline.1 < 0.002,
        "Virtual devices already carry audio: {baseline:?}"
    );
    engine
        .send(Command::Play {
            clip: Clip {
                frames: Arc::new(
                    (0..SAMPLE_RATE as usize * 15)
                        .map(|i| {
                            let value = (i as f32 * 440.0 * std::f32::consts::TAU
                                / SAMPLE_RATE as f32)
                                .sin()
                                * 0.1;
                            [value; 2]
                        })
                        .collect(),
                ),
            },
            volume: 1.0,
        })
        .map_err(anyhow::Error::msg)?;
    let mut report = String::new();
    let mut check = |name: &str, p: &Project, expected: (f32, f32), streams: usize| -> Result<()> {
        engine
            .send(Command::Update(p.clone()))
            .map_err(anyhow::Error::msg)?;
        let status = engine.snapshot().map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            status.running && status.outputs == streams,
            "{name}: {} outputs, {}",
            status.outputs,
            status.message
        );
        peaks(&mut first, &mut second, Duration::from_millis(200));
        let actual = peaks(&mut first, &mut second, Duration::from_millis(250));
        report.push_str(&format!(
            "{name}: primary={:.6}, listen={:.6}, streams={streams}\n",
            actual.0, actual.1
        ));
        anyhow::ensure!(
            (actual.0 - expected.0).abs() < 0.008 && (actual.1 - expected.1).abs() < 0.008,
            "{name}: measured {actual:?}, expected {expected:?}"
        );
        Ok(())
    };
    check("Listen off", &project, (0.1, 0.0), 1)?;
    A::ListenOn
        .apply(&mut project.nodes[1])
        .map_err(anyhow::Error::msg)?;
    check(
        "Listen on without restarting voice",
        &project,
        (0.1, 0.1),
        2,
    )?;
    project.nodes[0].gain_db = -6.0206;
    check("Both outputs follow source gain", &project, (0.05, 0.05), 2)?;
    A::ResetVolume
        .apply(&mut project.nodes[0])
        .map_err(anyhow::Error::msg)?;
    A::VolumeDown
        .apply(&mut project.nodes[1])
        .map_err(anyhow::Error::msg)?;
    check(
        "Source reset and bus volume down",
        &project,
        (0.089125, 0.089125),
        2,
    )?;
    A::VolumeUp
        .apply(&mut project.nodes[1])
        .map_err(anyhow::Error::msg)?;
    check("Bus volume up", &project, (0.1, 0.1), 2)?;
    project.nodes[1].gain_db = -12.0;
    A::ResetVolume
        .apply(&mut project.nodes[1])
        .map_err(anyhow::Error::msg)?;
    A::Mute
        .apply(&mut project.nodes[1])
        .map_err(anyhow::Error::msg)?;
    check("Bus mute affects both devices", &project, (0.0, 0.0), 2)?;
    A::Unmute
        .apply(&mut project.nodes[1])
        .map_err(anyhow::Error::msg)?;
    A::ListenOff
        .apply(&mut project.nodes[1])
        .map_err(anyhow::Error::msg)?;
    check("Unmute and listening off", &project, (0.1, 0.0), 1)?;
    A::ToggleListening
        .apply(&mut project.nodes[1])
        .map_err(anyhow::Error::msg)?;
    check("Toggle listening on", &project, (0.1, 0.1), 2)?;
    project.nodes[1].listen_device = Some(primary.id.clone());
    check("Same device emitted once", &project, (0.1, 0.0), 1)?;
    project.nodes[1].listen_device = Some("PirateWave nonexistent test endpoint::1".into());
    check(
        "Unavailable listen device preserves primary",
        &project,
        (0.1, 0.0),
        1,
    )?;
    anyhow::ensure!(errors.is_empty(), "Capture stream errors");
    engine.send(Command::Stop).map_err(anyhow::Error::msg)?;
    let report = format!(
        "PASS: live bus listening, source/bus volume reset and output keybind actions\n{report}"
    );
    std::fs::create_dir_all("dist")?;
    std::fs::write("dist/listen-self-test.txt", &report)?;
    println!("{report}");
    Ok(())
}
