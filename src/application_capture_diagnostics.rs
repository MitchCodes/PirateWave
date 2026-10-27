//! Opt-in, owned-process audio isolation/reconnection check. Never changes devices.
use anyhow::{Context, Result};
#[cfg(not(windows))]
pub fn run() -> Result<()> {
    anyhow::bail!("Application Capture is Windows-only")
}
#[cfg(not(windows))]
pub fn fixture(_: &[String], _: bool) -> Result<()> {
    anyhow::bail!("Windows-only fixture")
}
#[cfg(windows)]
mod win {
    use super::*;
    use crate::{
        application_capture,
        audio_io::{self, Counters},
    };
    use crossbeam_queue::ArrayQueue;
    use piratewave_core::{
        effects::{EffectRegistry, EffectSpec},
        graph::Graph,
        project::{ApplicationTarget, Connection, Node, NodeKind, Project},
        shortcuts::NodeAction,
        BLOCK_SIZE,
    };
    use std::{
        os::windows::process::CommandExt,
        process::{Child, Command},
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    struct OwnedChild(Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn spawn(args: &[&str]) -> Result<OwnedChild> {
        Ok(OwnedChild(
            Command::new(std::env::current_exe()?)
                .args(args)
                .creation_flags(0x08000000)
                .spawn()?,
        ))
    }
    pub fn fixture(args: &[String], worker: bool) -> Result<()> {
        if worker {
            let hz: f32 = args.first().context("Missing tone frequency")?.parse()?;
            let device = args.get(1).context("Missing tone playback device")?.clone();
            let counters = Arc::new(Counters::default());
            let errors = Arc::new(ArrayQueue::new(4));
            let output = audio_io::open_output(99, &Some(device), counters, errors.clone())?;
            let started = Instant::now();
            let mut sample = 0u64;
            while started.elapsed() < Duration::from_secs(40) {
                if let Some(error) = errors.pop() {
                    anyhow::bail!("{error}");
                }
                if output.queue.len() < 2048 {
                    for _ in 0..128 {
                        let v = 0.08 * (std::f32::consts::TAU * hz * sample as f32 / 48000.).sin();
                        let _ = output.queue.push([v; 2]);
                        sample += 1;
                    }
                } else {
                    thread::sleep(Duration::from_millis(2));
                }
            }
            return Ok(());
        }
        let title = args.first().context("Missing fixture window title")?;
        let hz = args.get(1).context("Missing frequency")?;
        let device = args.get(2).context("Missing output device")?;
        let _child = spawn(&["--application-capture-tone-worker", hz, device])?;
        unsafe {
            let hwnd = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide(title).as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                240,
                100,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            anyhow::ensure!(!hwnd.is_null(), "Could not create owned fixture window");
            ShowWindow(hwnd, SW_SHOWMINNOACTIVE);
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(35) && IsWindow(hwnd) != 0 {
                let mut message = std::mem::zeroed();
                while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                thread::sleep(Duration::from_millis(5));
            }
            if IsWindow(hwnd) != 0 {
                DestroyWindow(hwnd);
            }
        }
        Ok(())
    }
    fn stop(child: &mut OwnedChild, title: &str) -> Result<()> {
        unsafe {
            let hwnd = FindWindowW(wide("STATIC").as_ptr(), wide(title).as_ptr());
            if !hwnd.is_null() {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
        }
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(2) {
            if child.0.try_wait()?.is_some() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(20));
        }
        child.0.kill()?;
        child.0.wait()?;
        Ok(())
    }
    fn collect(input: &mut application_capture::ApplicationStream, seconds: f32) -> Vec<[f32; 2]> {
        let start = Instant::now();
        let mut samples = vec![];
        while start.elapsed().as_secs_f32() < seconds {
            for _ in 0..4096 {
                if let Some(frame) = input.resampler.next_frame() {
                    samples.push(frame)
                } else {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(2));
        }
        samples
    }
    fn spectral(samples: &[[f32; 2]], hz: f64) -> f64 {
        let (mut real, mut imaginary) = (0., 0.);
        for (i, frame) in samples.iter().enumerate() {
            let phase = std::f64::consts::TAU * hz * i as f64 / 48000.;
            real += frame[0] as f64 * phase.cos();
            imaginary += frame[0] as f64 * phase.sin();
        }
        real * real + imaginary * imaginary
    }
    pub fn run() -> Result<()> {
        let devices = audio_io::devices()?;
        let output=devices.outputs.iter().find(|d|d.name.contains("CABLE Input")).context("This optional integration test needs an installed virtual cable playback device to avoid audible test tones")?;
        let title_a = format!("PirateWave capture fixture A {}", std::process::id());
        let title_b = format!("PirateWave capture fixture B {}", std::process::id());
        let mut a = spawn(&["--application-capture-tone", &title_a, "440", &output.id])?;
        let _b = spawn(&["--application-capture-tone", &title_b, "880", &output.id])?;
        let target = ApplicationTarget {
            executable: std::env::current_exe()?.to_string_lossy().into_owned(),
            window_title: title_a.clone(),
            match_title: true,
        };
        let mut found = false;
        for _ in 0..100 {
            if application_capture::applications()?
                .iter()
                .any(|app| app.title == title_a)
            {
                found = true;
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        anyhow::ensure!(found, "Owned application fixture did not appear in picker");
        let counters = Arc::new(Counters::default());
        let messages = Arc::new(ArrayQueue::new(32));
        let mut input = application_capture::open(&target, counters.clone(), messages.clone())?;
        let samples = collect(&mut input, 3.0);
        let diagnostic_messages = std::iter::from_fn(|| messages.pop()).collect::<Vec<_>>();
        anyhow::ensure!(
            samples.len() > 24000,
            "No capture data: {diagnostic_messages:?}"
        );
        let samples = &samples[samples.len() / 2..];
        let peak = samples.iter().map(|f| f[0].abs()).fold(0., f32::max);
        anyhow::ensure!(peak > 0.02, "Capture silent: {diagnostic_messages:?}");
        let wanted = spectral(samples, 440.);
        let unwanted = spectral(samples, 880.);
        let isolation_db = 10. * (wanted / unwanted.max(1e-20)).log10();
        anyhow::ensure!(
            isolation_db > 30.,
            "Other application leaked: isolation {isolation_db}dB"
        );
        let registry = EffectRegistry::default();
        let mut source = Node::new(
            1,
            "Application",
            NodeKind::ApplicationCapture {
                target: Some(target.clone()),
            },
            [0.; 2],
        );
        let mut gain = EffectSpec::new(registry.get("gain").unwrap());
        gain.parameters.insert("gain".into(), -6.0206);
        source.effects.push(gain);
        let mut project = Project {
            version: 1,
            name: "Application capture test".into(),
            nodes: vec![
                source,
                Node::new(2, "Output", NodeKind::Output { device: None }, [0.; 2]),
            ],
            connections: vec![Connection {
                input: Default::default(),
                port: Default::default(),
                from: 1,
                to: 2,
            }],
            sounds: vec![],
        };
        let mut graph = Graph::new(&project, &registry).map_err(anyhow::Error::msg)?;
        let block = std::array::from_fn::<_, BLOCK_SIZE, _>(|i| samples[i]);
        graph.process(&[(1, block)]);
        for (frame, original) in graph.nodes[1].frames.iter().zip(block) {
            anyhow::ensure!(
                (frame[0] - original[0] * 0.5).abs() < 1e-5,
                "Routed effects gain wrong"
            );
        }
        NodeAction::Mute
            .apply(&mut project.nodes[0])
            .map_err(anyhow::Error::msg)?;
        graph
            .update(&project, &registry)
            .map_err(anyhow::Error::msg)?;
        graph.process(&[(1, block)]);
        graph.process(&[(1, block)]);
        anyhow::ensure!(
            graph.nodes[1].frames.iter().all(|f| f[0].abs() < 1e-6),
            "Application mute failed"
        );
        // Exercise a second reader of the same application through the real
        // Session adapter while the direct capture remains subscribed.
        NodeAction::Unmute
            .apply(&mut project.nodes[0])
            .map_err(anyhow::Error::msg)?;
        project.nodes[1].kind = NodeKind::Output {
            device: Some(output.id.clone()),
        };
        let engine = crate::engine::Engine::new();
        engine
            .send(crate::engine::Command::Start(project.clone()))
            .map_err(anyhow::Error::msg)?;
        std::thread::sleep(Duration::from_millis(1500));
        let status = engine.snapshot().map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            status.running && status.peaks.get(&2).copied().unwrap_or(0.) > 0.02,
            "Application Session route did not render: {}",
            status.message
        );
        NodeAction::Mute
            .apply(&mut project.nodes[0])
            .map_err(anyhow::Error::msg)?;
        engine
            .send(crate::engine::Command::Update(project.clone()))
            .map_err(anyhow::Error::msg)?;
        std::thread::sleep(Duration::from_millis(250));
        let status = engine.snapshot().map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            status.running && status.peaks.get(&2).copied().unwrap_or(1.) < 1e-6,
            "Live application source mute failed"
        );
        engine
            .send(crate::engine::Command::Stop)
            .map_err(anyhow::Error::msg)?;
        drop(engine);
        stop(&mut a, &title_a)?;
        let _ = collect(&mut input, 1.5);
        let quiet = collect(&mut input, 0.5);
        anyhow::ensure!(
            quiet.iter().all(|f| f[0].abs() < 0.005),
            "Wrong app captured while target absent"
        );
        let _restarted = spawn(&["--application-capture-tone", &title_a, "440", &output.id])?;
        let reconnected = collect(&mut input, 3.0);
        anyhow::ensure!(
            reconnected.iter().any(|f| f[0].abs() > 0.02),
            "Capture did not reconnect after PID change"
        );
        drop(input);
        let report=format!("PASS: application process-tree capture, separate-app isolation {isolation_db:.1} dB, routed effects, concurrent readers, live Session input/output and source mute, absent-target silence, reconnect after new PID, and clean shutdown.\nCaptured {} frames using {}.\n",samples.len(),output.name);
        std::fs::create_dir_all("dist")?;
        std::fs::write("dist/application-capture-self-test.txt", &report)?;
        println!("{report}");
        Ok(())
    }
}
#[cfg(windows)]
pub use win::{fixture, run};
