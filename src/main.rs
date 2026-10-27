#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
mod application_capture;
mod application_capture_diagnostics;
mod assets;
mod audio_io;
mod audio_scheduling;
mod control;
mod diagnostics;
mod engine;
mod extensions;
mod hotkeys;
mod logging;
mod plugins;
mod recording_io;
mod startup;
mod storage;
mod tools;
mod training;
mod ui;
mod voice;
mod windows_audio;
mod workspaces;

fn main() -> anyhow::Result<()> {
    let _logger = match logging::init() {
        Ok(guard) => Some(guard),
        Err(error) => {
            eprintln!("Cannot initialize PirateWave logging: {error}");
            None
        }
    };
    logging::info(format_args!(
        "PirateWave {} starting on {} / {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    let result = run_app();
    match &result {
        Ok(()) => logging::info(format_args!("PirateWave shutdown complete")),
        Err(error) => logging::error(format_args!("Application failed: {error:#}")),
    }
    result
}
fn run_app() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args
        .get(1)
        .is_some_and(|a| a == "--migrate-extension-project")
    {
        anyhow::ensure!(
            args.len() == 4,
            "Usage: PirateWave --migrate-extension-project <old.json> <new.json>"
        );
        return extensions::migrate_project(
            std::path::Path::new(&args[2]),
            std::path::Path::new(&args[3]),
        );
    }
    if args.get(1).is_some_and(|a| a == "--scan-extension") {
        anyhow::ensure!(
            args.len() == 4,
            "Usage: PirateWave --scan-extension <folder> <report>"
        );
        return extensions::scan_child(
            std::path::Path::new(&args[2]),
            std::path::Path::new(&args[3]),
        );
    }
    if args.get(1).is_some_and(|a| a == "--extension-self-test") {
        anyhow::ensure!(
            args.len() == 3,
            "Usage: PirateWave --extension-self-test <folder>"
        );
        return extensions::self_test(std::path::Path::new(&args[2]));
    }

    if args
        .get(1)
        .is_some_and(|a| a == "--voice-training-self-test")
    {
        anyhow::ensure!(args.len()==5,"Usage: PirateWave --voice-training-self-test <workspace> <offline-bundle-folder> <cpu|cuda>");
        return training::release::self_test(
            args[2].clone().into(),
            args[3].clone().into(),
            args[4].clone(),
        );
    }
    if args
        .get(1)
        .is_some_and(|a| a == "--application-capture-self-test")
    {
        return application_capture_diagnostics::run();
    }
    if args.get(1).is_some_and(|a| {
        a == "--application-capture-tone" || a == "--application-capture-tone-worker"
    }) {
        return application_capture_diagnostics::fixture(
            &args[2..],
            args[1] == "--application-capture-tone-worker",
        );
    }
    if let Some(index) = args.iter().position(|a| a == "--plugin-audio-self-test") {
        let path = args
            .get(index + 1)
            .ok_or_else(|| anyhow::anyhow!("Missing fixture path"))?;
        return plugins::diagnostics::audio(std::path::Path::new(path));
    }
    if let Some(index) = args.iter().position(|a| a == "--plugin-probe") {
        let path = args
            .get(index + 1)
            .ok_or_else(|| anyhow::anyhow!("Missing plugin path"))?;
        return plugins::diagnostics::probe(std::path::Path::new(path));
    }
    if let Some(index) = args.iter().position(|a| a == "--plugin-self-test") {
        let vst3 = args
            .get(index + 1)
            .ok_or_else(|| anyhow::anyhow!("Missing test .vst3 path"))?;
        let vst2 = args
            .get(index + 2)
            .ok_or_else(|| anyhow::anyhow!("Missing test VST2 .dll path"))?;
        return plugins::diagnostics::run(std::path::Path::new(vst3), std::path::Path::new(vst2));
    }
    if let Some(index) = args.iter().position(|a| a == "--scan-plugin") {
        let path = args
            .get(index + 1)
            .ok_or_else(|| anyhow::anyhow!("Missing plugin path"))?;
        let report = args
            .get(index + 2)
            .ok_or_else(|| anyhow::anyhow!("Missing scanner report path"))?;
        return plugins::scan_child(std::path::Path::new(path), std::path::Path::new(report));
    }
    if args.iter().any(|a| a == "--listen-self-test") {
        return diagnostics::listen_self_test();
    }
    if args.iter().any(|a| a == "--transport-self-test") {
        return diagnostics::transport_self_test();
    }
    if args.iter().any(|a| a == "--cable-self-test") {
        return diagnostics::cable_self_test();
    }
    if args.iter().any(|a| a == "--continuity-self-test") {
        return diagnostics::continuity_self_test();
    }
    #[cfg(windows)]
    if args.iter().any(|a| a == "--controller-test-host") {
        if let Some(folder) = std::env::var_os("PIRATEWAVE_EXTENSION_FIXTURE") {
            extensions::fixture_catalog(std::path::Path::new(&folder))?;
        }
        return control::diagnostic_host();
    }
    if args.iter().any(|a| a == "--shortcut-self-test") {
        return hotkeys::self_test().map_err(anyhow::Error::msg);
    }
    if args.iter().any(|a| a == "--devices") {
        let devices = audio_io::devices()?;
        let report = format!(
            "INPUTS\n{}\nOUTPUTS\n{}\n",
            devices
                .inputs
                .iter()
                .map(|d| format!("{} | {}", d.id, d.name))
                .collect::<Vec<_>>()
                .join("\n"),
            devices
                .outputs
                .iter()
                .map(|d| format!("{} | {}", d.id, d.name))
                .collect::<Vec<_>>()
                .join("\n")
        );
        println!("{report}");
        std::fs::create_dir_all("dist")?;
        std::fs::write("dist/devices.txt", report)?;
        return Ok(());
    }
    if args.iter().any(|a| a == "--audio-self-test") {
        return audio_self_test();
    }
    let smoke = args.iter().any(|a| {
        a == "--ui-smoke-test"
            || a == "--ui-smoke"
            || a == "--tools-ui-smoke"
            || a == "--effect-tuning-smoke"
    });
    if smoke {
        std::fs::create_dir_all("dist")?;
        if let Some(index) = args.iter().position(|a| a == "--extension-fixture") {
            extensions::fixture_catalog(std::path::Path::new(
                args.get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("Missing extension fixture folder"))?,
            ))?;
        }
    } else if let Err(error) = extensions::initialize() {
        logging::error(format_args!("Extension startup: {error:#}"));
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!(
                "../assets/branding/piratewave-256.png"
            ))?)
            .with_title("PirateWave — Audio workspace")
            .with_inner_size([if smoke { 1600.0 } else { 1380.0 }, 960.0])
            .with_min_inner_size([1000.0, 700.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "PirateWave",
        options,
        Box::new(move |cc| Ok(Box::new(ui::PirateWaveApp::new(cc, smoke)))),
    )
    .map_err(|e| anyhow::anyhow!("Native UI failed: {e}"))
}
fn audio_self_test() -> anyhow::Result<()> {
    use std::time::{Duration, Instant};
    let engine = engine::Engine::new();
    let mut project = piratewave_core::project::Project::default();
    // Capture is opened but no input is routed to the monitor. Output remains silent.
    project.connections.clear();
    engine
        .send(engine::Command::Start(project))
        .map_err(anyhow::Error::msg)?;
    let start = Instant::now();
    let mut last = None;
    while start.elapsed() < Duration::from_secs(3) {
        while let Some(status) = engine.statuses.pop() {
            last = Some(status);
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    engine
        .send(engine::Command::Stop)
        .map_err(anyhow::Error::msg)?;
    let status = last.ok_or_else(|| anyhow::anyhow!("No response from engine"))?;
    let passed = status.running && status.output_frames > 0;
    let report=format!("{}\n{}\nCaptured frames: {}\nRendered frames: {}\nUnderflow callbacks: {}\nDropped frames: {}\nOutput sample rates: {:?}\nDSP load: {:.2}%\n",if passed{"PASS"}else{"FAIL"},status.message,status.input_frames,status.output_frames,status.underflows,status.overflows,status.output_rates,status.render_load);
    println!("{report}");
    std::fs::create_dir_all("dist")?;
    std::fs::write("dist/audio-self-test.txt", report)?;
    anyhow::ensure!(passed, "Audio self-test failed");
    Ok(())
}
