//! Repeatable checks against the self-authored native test plugin, without audio hardware.
use super::*;
use piratewave_core::{
    effects::EffectRegistry,
    graph::Graph,
    project::{Connection, NodeKind},
    shortcuts::NodeAction as A,
};
fn render(graph: &mut Graph, rack: &mut Rack) -> f32 {
    let mut peak = 0.0f32;
    for _ in 0..8 {
        graph.process_with_plugins(&[], |node, frames| rack.process(node, frames));
        peak = graph.nodes.iter().find(|n| n.config.id == 4).unwrap().peak;
    }
    peak
}
/// Exercise chunk persistence independently of editor availability.
pub fn no_editor_self_test(path: &Path) -> Result<()> {
    let _thread = PluginThread::default();
    let descriptor = scan_isolated(path.to_owned())?
        .into_iter()
        .next()
        .context("No fixture found")?;
    let mut plugin = super::instantiate(&descriptor)?;
    anyhow::ensure!(
        plugin.open_editor().is_err(),
        "Editor unexpectedly available"
    );
    let program_only = std::env::var_os("PIRATEWAVE_TEST_VST2_PROGRAM_ONLY").is_some();
    if program_only {
        let mut bytes = 0.5_f32.to_bits().to_le_bytes().to_vec();
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        plugin.restore_state(&piratewave_core::plugins::PluginState {
            component: bytes.into(),
            controller: None,
            vst2_bank: true,
            vst2_program: Some(2),
        })?;
    }
    plugin.set_parameter(0, 0.375);
    let state = plugin
        .capture_state_for_shutdown()?
        .context("No chunk from editor-free fixture")?;
    let mut restored = super::instantiate(&descriptor)?;
    restored.restore_state(&state)?;
    let restored_state = restored
        .capture_state_for_shutdown()?
        .context("No restored chunk")?;
    anyhow::ensure!(
        state.component == restored_state.component,
        "Editor-free plugin state did not round-trip"
    );
    if program_only {
        anyhow::ensure!(
            !state.vst2_bank
                && state.vst2_program == Some(2)
                && restored_state.vst2_program == Some(2),
            "Program-only VST2 state lost its selected program"
        );
    }
    anyhow::ensure!(
        state.component.get(..4) == Some(0.375_f32.to_bits().to_le_bytes().as_slice()),
        "Saved chunk did not contain the parameter edit"
    );
    println!("PASS: VST2 plugin without editor retains full state");
    Ok(())
}
/// Manual editor compatibility probe. Does not touch user projects or audio devices.
pub fn editor_probe(path: &Path, seconds: u64, compare: bool) -> Result<()> {
    anyhow::ensure!(
        (3..=120).contains(&seconds),
        "Probe duration must be 3–120 seconds"
    );
    let _thread = PluginThread::default();
    let descriptor = scan_isolated(path.to_owned())?
        .into_iter()
        .next()
        .context("No plugin found")?;
    let mut plugin = super::instantiate(&descriptor)?;
    plugin.open_editor()?;
    if compare {
        println!("Change a plugin-specific setting, try resizing or moving the window, then close it to reopen the saved state.");
    }
    let start = Instant::now();
    let mut blocks = 0usize;
    let mut edits = 0usize;
    let mut open_after_startup = false;
    let mut last_size = (0, 0);
    let mut resizable = false;
    while start.elapsed() < Duration::from_secs(seconds) {
        super::pump_messages();
        plugin.editor_idle();
        if let Some(error) = plugin.take_editor_error() {
            anyhow::bail!("Editor failed: {error}");
        }
        if start.elapsed() >= Duration::from_secs(2)
            && plugin.editor_open()
            && plugin.editor_size().0 > 0
        {
            open_after_startup = true;
        }
        if plugin.editor_size().0 > 0 {
            last_size = plugin.editor_size();
            resizable = plugin.editor_resizable();
        }
        if compare && open_after_startup && !plugin.editor_open() {
            break;
        }
        edits += plugin.take_editor_edits().len();
        let mut frames: [Frame; BLOCK_SIZE] = std::array::from_fn(|index| {
            let sample = if descriptor.role == PluginRole::Effect {
                let phase = (blocks * BLOCK_SIZE + index) as f32 * std::f32::consts::TAU * 440.0
                    / SAMPLE_RATE as f32;
                0.01 * phase.sin()
            } else {
                0.0
            };
            [sample, sample]
        });
        plugin.process(&mut frames)?;
        anyhow::ensure!(
            frames.iter().flatten().all(|sample| sample.is_finite()),
            "Plugin returned invalid audio while its editor was open"
        );
        blocks += 1;
        std::thread::sleep(Duration::from_millis(5));
    }
    anyhow::ensure!(open_after_startup, "Plugin editor did not remain open");
    edits += plugin.take_editor_edits().len();
    let capture_started = Instant::now();
    let state = loop {
        super::pump_messages();
        plugin.editor_idle();
        match plugin.capture_state() {
            Ok(state) => break state,
            Err(error) if format!("{error:#}").contains("state capture is pending") => {}
            Err(error) => return Err(error),
        }
        anyhow::ensure!(
            capture_started.elapsed() < Duration::from_secs(8),
            "Plugin state capture timed out"
        );
        let mut frames = [[0.0; 2]; BLOCK_SIZE];
        plugin.process(&mut frames)?;
        std::thread::sleep(Duration::from_millis(5));
    };
    if state.is_some() {
        anyhow::ensure!(
            plugin.capture_state_for_shutdown()?.is_some(),
            "Final plugin state capture failed"
        );
    }
    let editor_size = last_size;
    plugin.close_editor();
    if compare {
        let start = Instant::now();
        while plugin.editor_open() && start.elapsed() < Duration::from_secs(3) {
            super::pump_messages();
            plugin.editor_idle();
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    if let Some(state) = &state {
        let mut restored = super::instantiate(&descriptor)?;
        restored.restore_state(state)?;
        let mut frames = [[0.0; 2]; BLOCK_SIZE];
        restored.process(&mut frames)?;
        anyhow::ensure!(
            frames.iter().flatten().all(|sample| sample.is_finite()),
            "Restored plugin returned invalid audio"
        );
        if compare {
            println!("Opening a new instance with the captured state. Check the setting and close the window when finished.");
            restored.open_editor()?;
            let start = Instant::now();
            let mut shown = false;
            while start.elapsed() < Duration::from_secs(seconds) {
                super::pump_messages();
                restored.editor_idle();
                if let Some(error) = restored.take_editor_error() {
                    anyhow::bail!("Restored editor failed: {error}");
                }
                if restored.editor_open() && restored.editor_size().0 > 0 {
                    shown = true;
                }
                if shown && !restored.editor_open() {
                    break;
                }
                restored.process(&mut frames)?;
                std::thread::sleep(Duration::from_millis(5));
            }
            anyhow::ensure!(shown, "Restored editor did not open");
            restored.close_editor();
        }
    } else if compare {
        anyhow::bail!("This plugin does not expose full state for the comparison");
    }
    println!(
        "PASS: {} {:?} editor {:?} (resizable: {resizable}); {blocks} audio blocks, {edits} editor edits, state {}",
        descriptor.name,
        descriptor.format,
        editor_size,
        if state.is_some() {
            "captured and restored"
        } else {
            "unavailable"
        }
    );
    Ok(())
}
pub fn run(vst3: &Path, vst2: &Path) -> Result<()> {
    let _thread = PluginThread::default();
    let classes = scan_isolated(vst3.to_owned())?;
    let instrument = classes
        .iter()
        .find(|p| p.role == PluginRole::Instrument)
        .context("Missing test instrument")?;
    let effect = classes
        .iter()
        .find(|p| p.role == PluginRole::Effect)
        .context("Missing test gain")?;
    #[cfg(all(windows, debug_assertions))]
    {
        let mut plugin =
            super::vst3_host::Threaded::load(Path::new(&effect.path), &effect.class_id)?;
        plugin.simulate_failure_for_diagnostics()?;
        let start = Instant::now();
        loop {
            let mut frames = [[0.0; 2]; BLOCK_SIZE];
            if plugin.process(&mut frames).is_err() {
                break;
            }
            anyhow::ensure!(
                start.elapsed() < Duration::from_secs(3),
                "VST3 control-thread failure was not detected"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut plugin =
            super::vst3_host::Threaded::load(Path::new(&effect.path), &effect.class_id)?;
        plugin.open_editor()?;
        plugin.send_key_for_diagnostics(0x71)?; // Windows F2
        let start = Instant::now();
        loop {
            if plugin
                .take_editor_edits()
                .iter()
                .any(|(id, value)| *id == 0 && (*value - 0.625).abs() < 0.0001)
            {
                break;
            }
            anyhow::ensure!(
                start.elapsed() < Duration::from_secs(4),
                "VST3 editor did not receive its keyboard input"
            );
            let mut frames = [[0.4; 2]; BLOCK_SIZE];
            plugin.process(&mut frames)?;
            std::thread::sleep(Duration::from_millis(5));
        }
        plugin.set_parameter(0, 0.25);
        plugin.send_key_for_diagnostics(0x71)?;
        let start = Instant::now();
        loop {
            if plugin
                .take_editor_edits()
                .iter()
                .any(|(id, value)| *id == 0 && (*value - 0.625).abs() < 0.0001)
            {
                break;
            }
            anyhow::ensure!(
                start.elapsed() < Duration::from_secs(3),
                "VST3 editor could not return to its previous value after a host edit"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        plugin.resize_for_diagnostics(500, 300)?;
        let start = Instant::now();
        while plugin.editor_size() != (500, 300) {
            anyhow::ensure!(
                start.elapsed() < Duration::from_secs(3),
                "VST3 editor did not accept a host resize"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[cfg(windows)]
    {
        let no_editor = classes
            .iter()
            .find(|p| p.name == "PirateWave Test Without Editor")
            .context("Missing no-editor fixture")?;
        let mut plugin = super::instantiate(no_editor)?;
        plugin.open_editor()?;
        let start = Instant::now();
        let error = loop {
            if let Some(error) = plugin.take_editor_error() {
                break error;
            }
            anyhow::ensure!(
                start.elapsed() < Duration::from_secs(3),
                "Missing-editor failure was not reported"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        anyhow::ensure!(
            error.contains("no vendor editor"),
            "Unexpected missing-editor error: {error}"
        );
        let mut frames = [[0.4; 2]; BLOCK_SIZE];
        plugin.process(&mut frames)?;
        anyhow::ensure!(
            (frames[0][0] - 0.2).abs() < 0.001,
            "Missing editor interrupted plugin audio"
        );
    }
    let legacy = scan_isolated(vst2.to_owned())?
        .into_iter()
        .find(|p| p.role == PluginRole::Instrument)
        .context("Missing VST2 instrument")?;
    #[cfg(all(windows, debug_assertions))]
    for descriptor in [effect, &legacy] {
        let mut plugin = super::instantiate(descriptor)?;
        plugin.open_editor()?;
        plugin.test_editor_timeout()?;
        plugin.editor_idle();
        let error = plugin
            .take_editor_error()
            .context("Editor timeout was not reported")?;
        anyhow::ensure!(
            error.contains("within five seconds"),
            "Unexpected editor timeout: {error}"
        );
        let mut frames = [[0.1; 2]; BLOCK_SIZE];
        let started = Instant::now();
        plugin.process(&mut frames)?;
        anyhow::ensure!(
            started.elapsed() < Duration::from_millis(750),
            "Editor timeout blocked audio"
        );
    }
    #[cfg(all(windows, debug_assertions))]
    for slow_descriptor in [instrument, &legacy] {
        // A slow editor and chunk capture must not freeze the engine while a
        // stop is waiting for the plugin's final state.
        let mut project = Project::default();
        project.nodes.clear();
        project.connections.clear();
        let mut node = Node::new(71, "Slow fixture", NodeKind::VstInstrument, [0.0; 2]);
        node.plugin = Some(NodePlugin::new(slow_descriptor.clone()));
        project.nodes.push(node);
        let engine = crate::engine::Engine::new();
        engine
            .send(crate::engine::Command::Start(project.clone()))
            .map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            engine.snapshot().map_err(anyhow::Error::msg)?.running,
            "Fixture engine did not start"
        );
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        engine
            .send(crate::engine::Command::OpenPluginEditor { id: 71, reply: tx })
            .map_err(anyhow::Error::msg)?;
        rx.recv_timeout(Duration::from_secs(3))?
            .map_err(anyhow::Error::msg)?;
        let start = Instant::now();
        loop {
            if let Some(status) = engine.statuses.pop() {
                if status.plugin_editor_edits.contains_key(&71) {
                    break;
                }
            }
            anyhow::ensure!(
                start.elapsed() < Duration::from_secs(4),
                "Slow fixture edit was not observed"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        while engine.statuses.pop().is_some() {}
        engine
            .send(crate::engine::Command::Stop)
            .map_err(anyhow::Error::msg)?;
        let start = Instant::now();
        let mut running_updates = 0;
        loop {
            if let Some(status) = engine.statuses.pop() {
                if status.running {
                    running_updates += 1;
                } else {
                    break;
                }
            }
            anyhow::ensure!(
                start.elapsed() < Duration::from_secs(8),
                "Engine did not stop after state capture"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        anyhow::ensure!(
            running_updates >= 5,
            "Slow plugin state capture stopped engine progress"
        );
        engine
            .send(crate::engine::Command::Start(project))
            .map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            engine.snapshot().map_err(anyhow::Error::msg)?.running,
            "Fixture engine did not restart"
        );
        engine
            .send(crate::engine::Command::SetPluginParameters {
                id: 71,
                values: vec![(0, 0.4)],
            })
            .map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        loop {
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            engine
                .send(crate::engine::Command::CapturePluginStates(false, tx))
                .map_err(anyhow::Error::msg)?;
            let (states, errors, _) = rx.recv_timeout(Duration::from_secs(2))?;
            if errors.is_empty() {
                let state = &states
                    .get(&71)
                    .context("Restarted fixture state missing")?
                    .1;
                if slow_descriptor.format == PluginFormat::Vst2 {
                    anyhow::ensure!(
                        state.vst2_program == Some(2)
                            && state.component.ends_with(&0x12345678_u32.to_le_bytes()),
                        "Restart lost the VST2 fixture's opaque state or selected program"
                    );
                    anyhow::ensure!(
                        state.component.get(..4)
                            == Some(0.4_f32.to_bits().to_le_bytes().as_slice()),
                        "Restarted VST2 fixture did not capture its latest parameter"
                    );
                } else {
                    anyhow::ensure!(
                        state.component.get(8..) == Some(0x12345678_u64.to_le_bytes().as_slice()),
                        "Restart lost the VST3 fixture's opaque state"
                    );
                    anyhow::ensure!(
                        state.component.get(..8) == Some(0.4_f64.to_le_bytes().as_slice()),
                        "Restarted VST3 fixture did not capture its latest parameter"
                    );
                }
                break;
            }
            anyhow::ensure!(
                errors
                    .iter()
                    .all(|error| error.contains("state capture is pending"))
                    && started.elapsed() < Duration::from_secs(8),
                "Restarted fixture capture failed: {}",
                errors.join(" | ")
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    #[cfg(windows)]
    if piratewave_vst2_host::pe::architecture(vst2)? == piratewave_vst2_host::pe::Architecture::X86
    {
        let mut bridge = super::bridge::Bridge::load(vst2)?;
        bridge.open_editor()?;
        bridge.terminate_helper_for_diagnostics()?;
        let mut frames = [[0.0; 2]; BLOCK_SIZE];
        anyhow::ensure!(
            bridge.process(&mut frames).is_err(),
            "Terminated VST2 helper was not detected"
        );
        let mut bridge = super::bridge::Bridge::load(vst2)?;
        let mut bytes = 0.5_f32.to_bits().to_le_bytes().to_vec();
        bytes.extend_from_slice(&0x87654321_u32.to_le_bytes());
        bridge.restore_state(&piratewave_core::plugins::PluginState {
            component: bytes.into(),
            controller: None,
            vst2_bank: true,
            vst2_program: None,
        })?;
        let started = Instant::now();
        let mut audio_blocks = 0;
        let mut changed_during_transfer = false;
        let state = loop {
            match bridge.capture_state() {
                Ok(Some(state)) => break state,
                Err(error) if format!("{error:#}").contains("pending") => {}
                other => anyhow::bail!("Large state capture failed: {other:?}"),
            }
            if bridge.state_transfer_in_flight_for_diagnostics() && !changed_during_transfer {
                bridge.set_parameter(0, 0.125);
                changed_during_transfer = true;
            }
            let block_started = Instant::now();
            bridge.process(&mut frames)?;
            anyhow::ensure!(
                block_started.elapsed() < Duration::from_millis(250),
                "Large VST2 state transfer blocked audio"
            );
            audio_blocks += 1;
            anyhow::ensure!(
                started.elapsed() < Duration::from_secs(10),
                "Large VST2 state transfer timed out"
            );
        };
        anyhow::ensure!(
            audio_blocks > 0
                && changed_during_transfer
                && state.component.len() == 24 * 1024 * 1024
                && state.component.last() == Some(&0x5a),
            "Large VST2 state transfer was incomplete"
        );
        anyhow::ensure!(
            state.component.get(..4) == Some(0.125_f32.to_bits().to_le_bytes().as_slice()),
            "An obsolete state transfer overwrote a newer parameter edit"
        );
    }
    let mut report = String::new();
    let registry = EffectRegistry::default();
    for descriptor in [instrument, effect, &legacy] {
        let mut original = super::instantiate(descriptor)?;
        #[cfg(windows)]
        {
            original.open_editor()?;
            anyhow::ensure!(original.editor_open(), "Fixture editor did not open");
            if descriptor.format == PluginFormat::Vst3 {
                let start = Instant::now();
                let mut received = false;
                while start.elapsed() < Duration::from_secs(3) {
                    if original
                        .take_editor_edits()
                        .iter()
                        .any(|(id, value)| *id == 0 && (*value - 0.75).abs() < 0.0001)
                    {
                        received = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                anyhow::ensure!(
                    received,
                    "VST3 editor callback was not delivered during a slow UI action"
                );
                let mut frames = [[0.0; 2]; BLOCK_SIZE];
                let started = Instant::now();
                original.process(&mut frames)?;
                anyhow::ensure!(
                    started.elapsed() < Duration::from_millis(750),
                    "VST3 editor blocked audio processing"
                );
            }
            original.open_editor()?;
            original.editor_idle();
            if descriptor.format == PluginFormat::Vst2 {
                let start = Instant::now();
                let mut received = false;
                while start.elapsed() < Duration::from_secs(3) {
                    original.editor_idle();
                    if let Some(error) = original.take_editor_error() {
                        anyhow::bail!("VST2 editor: {error}");
                    }
                    if original
                        .take_editor_edits()
                        .iter()
                        .any(|(id, value)| *id == 0 && (*value - 0.75).abs() < 0.0001)
                    {
                        received = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                anyhow::ensure!(
                    received,
                    "VST2 editor idle parameter change was not delivered"
                );
                #[cfg(debug_assertions)]
                {
                    original.test_editor_key(0x71)?;
                    let started = Instant::now();
                    loop {
                        original.editor_idle();
                        if original
                            .take_editor_edits()
                            .iter()
                            .any(|(id, value)| *id == 0 && (*value - 0.625).abs() < 0.0001)
                        {
                            break;
                        }
                        anyhow::ensure!(
                            started.elapsed() < Duration::from_secs(4),
                            "VST2 editor did not receive keyboard input"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    original.set_parameter(0, 0.25);
                    original.process(&mut [[0.0; 2]; BLOCK_SIZE])?;
                    original.test_editor_key(0x71)?;
                    let started = Instant::now();
                    loop {
                        original.editor_idle();
                        if original
                            .take_editor_edits()
                            .iter()
                            .any(|(id, value)| *id == 0 && (*value - 0.625).abs() < 0.0001)
                        {
                            break;
                        }
                        anyhow::ensure!(
                            started.elapsed() < Duration::from_secs(3),
                            "VST2 editor could not return to its previous value after a host edit"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
                let mut frames = [[0.0; 2]; BLOCK_SIZE];
                let started = Instant::now();
                original.process(&mut frames)?;
                anyhow::ensure!(
                    started.elapsed() < Duration::from_millis(750),
                    "VST2 editor blocked audio processing"
                );
            }
            original.close_editor();
            let start = Instant::now();
            while original.editor_open() && start.elapsed() < Duration::from_secs(3) {
                original.editor_idle();
                std::thread::sleep(Duration::from_millis(5));
            }
            anyhow::ensure!(!original.editor_open(), "Fixture editor did not close");
            original.open_editor()?;
            anyhow::ensure!(original.editor_open(), "Fixture editor did not reopen");
            let start = Instant::now();
            while original.editor_size() != (420, 240) && start.elapsed() < Duration::from_secs(4) {
                original.editor_idle();
                if let Some(error) = original.take_editor_error() {
                    anyhow::bail!("Fixture editor: {error}");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            anyhow::ensure!(
                original.editor_size() == (420, 240),
                "Fixture editor resize request was not applied: {:?}",
                original.editor_size()
            );
        }
        original.set_parameter(0, 0.25);
        #[cfg(windows)]
        {
            let mut frames = [[0.0; 2]; BLOCK_SIZE];
            original.process(&mut frames)?;
            original.editor_idle();
            anyhow::ensure!(
                !original
                    .take_editor_edits()
                    .iter()
                    .any(|(id, value)| *id == 0 && (*value - 0.75).abs() < 0.0001),
                "An older vendor edit overwrote a newer generic parameter"
            );
        }
        #[cfg(windows)]
        if descriptor.format == PluginFormat::Vst2 {
            let first = original.capture_state();
            anyhow::ensure!(
                first
                    .as_ref()
                    .is_err_and(|e| format!("{e:#}").contains("pending")),
                "Expected asynchronous VST2 state capture"
            );
            std::thread::sleep(Duration::from_millis(50));
            let mut frames = [[0.0; 2]; BLOCK_SIZE];
            let started = Instant::now();
            original.process(&mut frames)?;
            anyhow::ensure!(
                started.elapsed() < Duration::from_millis(750),
                "VST2 state capture blocked audio processing"
            );
        }
        if descriptor.format == PluginFormat::Vst3 {
            let state = original
                .capture_state_for_shutdown()?
                .context("Fixture failed to save flushed parameters")?;
            let mut flushed = super::instantiate(descriptor)?;
            flushed.restore_state(&state)?;
            flushed.note(60, 0.8, true);
            let mut frames = [[0.4, 0.4]; BLOCK_SIZE];
            flushed.process(&mut frames)?;
            let expected = if descriptor.role == PluginRole::Effect {
                0.1
            } else {
                0.05
            };
            let peak = frames
                .iter()
                .flatten()
                .map(|v| v.abs())
                .fold(0.0_f32, f32::max);
            anyhow::ensure!(
                (peak - expected).abs() < 0.002,
                "VST3 parameter flush did not reach saved state"
            );
        }
        original.note(60, 0.8, true);
        let mut frames = [[0.4, 0.4]; BLOCK_SIZE];
        original.process(&mut frames)?;
        let state = original
            .capture_state_for_shutdown()?
            .context("Fixture failed to provide state")?;
        if descriptor.format == PluginFormat::Vst2 {
            anyhow::ensure!(state.vst2_bank, "Fixture VST2 bank state was not captured");
            anyhow::ensure!(
                state.vst2_program == Some(2),
                "Fixture VST2 selected program was not captured"
            );
        }
        anyhow::ensure!(
            state.component.ends_with(&0x12345678_u32.to_le_bytes())
                || (descriptor.format == PluginFormat::Vst3
                    && state.component.get(8..) == Some(0x12345678_u64.to_le_bytes().as_slice())),
            "Fixture opaque editor state was not captured"
        );
        let path = std::env::temp_dir().join(format!(
            "piratewave-plugin-state-{}.json",
            piratewave_core::recording::RecordSlot::new("").id
        ));
        let mut project = Project::default();
        project.nodes.clear();
        project.connections.clear();
        let kind = if descriptor.role == PluginRole::Instrument {
            NodeKind::VstInstrument
        } else {
            NodeKind::VstEffect
        };
        let mut saved_node = Node::new(101, "State fixture", kind, [0.0; 2]);
        let mut saved_plugin = NodePlugin::new(descriptor.clone());
        saved_plugin.state = Some(state.clone());
        saved_node.plugin = Some(saved_plugin);
        project.nodes.push(saved_node);
        crate::storage::save(&path, &project)?;
        let loaded = crate::storage::load(&path)?;
        std::fs::remove_file(&path)?;
        let saved_state = loaded.nodes[0]
            .plugin
            .as_ref()
            .and_then(|plugin| plugin.state.as_ref())
            .context("Project lost plugin state")?;
        anyhow::ensure!(
            *saved_state == state,
            "Project did not preserve opaque plugin state"
        );
        let mut restored = super::instantiate(descriptor)?;
        restored.restore_state(saved_state)?;
        let restored_state = restored
            .capture_state_for_shutdown()?
            .context("Restored fixture failed to provide state")?;
        anyhow::ensure!(
            restored_state.component == state.component,
            "Fixture opaque state did not survive restoration"
        );
        anyhow::ensure!(
            restored_state.vst2_program == state.vst2_program,
            "Fixture selected program did not survive restoration"
        );
        restored.note(60, 0.8, true);
        let mut restored_frames = [[0.4, 0.4]; BLOCK_SIZE];
        restored.process(&mut restored_frames)?;
        let expected = if descriptor.role == PluginRole::Effect {
            0.1
        } else {
            0.05
        };
        let peak = restored_frames
            .iter()
            .flatten()
            .map(|v| v.abs())
            .fold(0.0_f32, f32::max);
        anyhow::ensure!(
            (peak - expected).abs() < 0.002,
            "{:?} state restoration produced peak {peak}, expected {expected}",
            descriptor.format
        );
        report.push_str(&format!(
            "{:?} editor lifecycle and state restoration passed\n",
            descriptor.format
        ));
    }
    {
        let mut node = Node::new(91, "Rejected state fixture", NodeKind::VstEffect, [0.0; 2]);
        let mut config = NodePlugin::new(effect.clone());
        config.parameters.insert(0, 0.4);
        config.state = Some(piratewave_core::plugins::PluginState {
            component: vec![1].into(),
            controller: None,
            vst2_bank: false,
            vst2_program: None,
        });
        node.plugin = Some(config);
        let mut project = Project::default();
        project.nodes.clear();
        project.connections.clear();
        project.nodes.push(node.clone());
        let mut rack = Rack::default();
        rack.sync(&project);
        anyhow::ensure!(
            rack.warnings
                .iter()
                .any(|warning| warning.contains("could not be restored")),
            "Rejected VST3 state was not reported"
        );
        let mut frames = [[0.4; 2]; BLOCK_SIZE];
        rack.process(&node, &mut frames);
        anyhow::ensure!(
            (frames[0][0] - 0.16).abs() < 0.002,
            "Rejected VST3 state did not fall back to parameters"
        );
    }
    for descriptor in [instrument.clone(), legacy] {
        let mut source = Node::new(1, "Instrument", NodeKind::VstInstrument, [0.0; 2]);
        source.plugin = Some(NodePlugin::new(descriptor));
        let mut fx = Node::new(2, "First VST effect", NodeKind::VstEffect, [300.0, 0.0]);
        fx.plugin = Some(NodePlugin::new(effect.clone()));
        let mut fx2 = fx.clone();
        fx2.id = 3;
        let out = Node::new(
            4,
            "Output",
            NodeKind::Output {
                device: Some(String::new()),
            },
            [600.0, 0.0],
        );
        let mut project = Project {
            workflows: Default::default(),
            version: 1,
            name: "Plugin integration test".into(),
            nodes: vec![source, fx, fx2, out],
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
                Connection {
                    input: Default::default(),
                    port: Default::default(),
                    from: 3,
                    to: 4,
                },
            ],
            sounds: vec![],
        };
        let mut graph = Graph::new(&project, &registry).map_err(anyhow::Error::msg)?;
        let mut rack = Rack::default();
        rack.sync(&project);
        anyhow::ensure!(rack.warnings.is_empty(), "{:?}", rack.warnings);
        anyhow::ensure!(
            render(&mut graph, &mut rack) < 0.00001,
            "Instrument not silent before note-on"
        );
        rack.note(1, 60, 0.8, true);
        let peak = render(&mut graph, &mut rack);
        anyhow::ensure!(
            (peak - 0.025).abs() < 0.001,
            "Routed instrument/effect chain has wrong amplitude {peak}"
        );
        report.push_str(&format!(
            "{:?}: instrument -> VST gain -> VST gain -> output peak {peak:.6} (expected 0.025)\n",
            project.nodes[0].plugin.as_ref().unwrap().descriptor.format
        ));
        A::Disable
            .apply(&mut project.nodes[1])
            .map_err(anyhow::Error::msg)?;
        graph
            .update(&project, &registry)
            .map_err(anyhow::Error::msg)?;
        rack.sync(&project);
        anyhow::ensure!(
            (render(&mut graph, &mut rack) - 0.05).abs() < 0.001,
            "VST effect bypass failed"
        );
        A::ToggleDisabled
            .apply(&mut project.nodes[2])
            .map_err(anyhow::Error::msg)?;
        graph
            .update(&project, &registry)
            .map_err(anyhow::Error::msg)?;
        rack.sync(&project);
        anyhow::ensure!(
            (render(&mut graph, &mut rack) - 0.1).abs() < 0.001,
            "Both VST effects bypass failed"
        );
        for index in [1, 2] {
            A::Enable
                .apply(&mut project.nodes[index])
                .map_err(anyhow::Error::msg)?;
        }
        project.nodes[1]
            .plugin
            .as_mut()
            .unwrap()
            .parameters
            .insert(0, 0.25);
        graph
            .update(&project, &registry)
            .map_err(anyhow::Error::msg)?;
        rack.sync(&project);
        anyhow::ensure!(
            (render(&mut graph, &mut rack) - 0.0125).abs() < 0.001,
            "Live plugin parameter change failed or notes were interrupted"
        );
        A::Mute
            .apply(&mut project.nodes[0])
            .map_err(anyhow::Error::msg)?;
        graph
            .update(&project, &registry)
            .map_err(anyhow::Error::msg)?;
        rack.sync(&project);
        anyhow::ensure!(
            render(&mut graph, &mut rack) < 0.00001,
            "Instrument mute failed"
        );
        A::Unmute
            .apply(&mut project.nodes[0])
            .map_err(anyhow::Error::msg)?;
        graph
            .update(&project, &registry)
            .map_err(anyhow::Error::msg)?;
        rack.sync(&project);
        rack.note(1, 60, 0.0, false);
        anyhow::ensure!(render(&mut graph, &mut rack) < 0.00001, "Note-off failed");
        rack.note(1, 60, 0.8, true);
        anyhow::ensure!(render(&mut graph, &mut rack) > 0.01, "Retrigger failed");
        rack.all_notes_off(Some(1));
        anyhow::ensure!(
            render(&mut graph, &mut rack) < 0.00001,
            "All notes off failed"
        );
        let path = std::env::temp_dir().join(format!(
            "piratewave-plugin-project-{}.json",
            std::process::id()
        ));
        crate::storage::save(&path, &project)?;
        let restored = crate::storage::load(&path)?;
        std::fs::remove_file(path)?;
        let mut restored_rack = Rack::default();
        restored_rack.sync(&restored);
        restored_rack.note(1, 60, 0.8, true);
        let mut restored_graph = Graph::new(&restored, &registry).map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            (render(&mut restored_graph, &mut restored_rack) - 0.0125).abs() < 0.001,
            "Saved plugin parameters not restored"
        );
        // Drop one instance while another class from the same module is still processing.
        #[cfg(windows)]
        rack.open_editor(2)?;
        project.remove_node(2);
        project.connections = vec![
            Connection {
                input: Default::default(),
                port: Default::default(),
                from: 1,
                to: 3,
            },
            Connection {
                input: Default::default(),
                port: Default::default(),
                from: 3,
                to: 4,
            },
        ];
        graph
            .update(&project, &registry)
            .map_err(anyhow::Error::msg)?;
        rack.sync(&project);
        anyhow::ensure!(
            !rack.statuses().contains_key(&2),
            "Removed plugin instance was retained"
        );
        rack.note(1, 60, 0.8, true);
        anyhow::ensure!(
            (render(&mut graph, &mut rack) - 0.05).abs() < 0.001,
            "Removing a shared-module instance broke surviving plugins"
        );
        anyhow::ensure!(
            rack.warnings.is_empty() && restored_rack.warnings.is_empty(),
            "Processing errors"
        );
        report.push_str("PASS: bypass/enable/toggle, live parameters, mute/unmute, note on/off/retrigger, all-notes-off, project restoration and shared module lifetime\n");
    }
    let invalid =
        std::env::temp_dir().join(format!("piratewave-invalid-{}.dll", std::process::id()));
    std::fs::write(&invalid, b"not a plugin")?;
    let rejected = scan_isolated(invalid.clone()).is_err();
    std::fs::remove_file(invalid)?;
    anyhow::ensure!(rejected, "Invalid plugin accepted");
    let report=format!("PASS: VST2/VST3 native plugin scanning and real DSP routing checks\n{report}Invalid plugin scanner error handled without terminating host.\n");
    std::fs::create_dir_all("dist")?;
    std::fs::write("dist/plugin-self-test.txt", &report)?;
    println!("{report}");
    Ok(())
}

/// Optional third-party effect check. Processes a quiet sine entirely in memory.
pub fn probe(path: &Path) -> Result<()> {
    let _thread = PluginThread::default();
    let plugins = scan_isolated(path.to_owned())?;
    let descriptor = plugins
        .iter()
        .find(|p| p.role == PluginRole::Effect)
        .context("Probe requires an audio effect")?;
    let mut plugin = instantiate(descriptor)?;
    let measure = |plugin: &mut dyn Hosted| -> Result<f32> {
        let mut peak = 0.0f32;
        for block in 0..100 {
            let mut frames = [[0.0; 2]; BLOCK_SIZE];
            for (i, frame) in frames.iter_mut().enumerate() {
                let value = (((block * BLOCK_SIZE + i) as f32) * 440.0 * std::f32::consts::TAU
                    / SAMPLE_RATE as f32)
                    .sin()
                    * 0.1;
                *frame = [value; 2];
            }
            plugin.process(&mut frames)?;
            for frame in frames {
                for value in frame {
                    anyhow::ensure!(value.is_finite(), "Non-finite plugin output");
                    if block > 90 {
                        peak = peak.max(value.abs());
                    }
                }
            }
        }
        Ok(peak)
    };
    let default_peak = measure(plugin.as_mut())?;
    let pre = descriptor.parameters.iter().find(|p| p.name == "Pre Gain");
    let reduced_peak = if let Some(p) = pre {
        plugin.set_parameter(p.id, 0.0);
        Some(measure(plugin.as_mut())?)
    } else {
        None
    };
    if pre.is_some() {
        anyhow::ensure!(
            default_peak > 0.001 && reduced_peak.unwrap() < 0.001,
            "Pre Gain probe did not change signal"
        );
    }
    let mut transpose_report = String::new();
    if let Some(parameter) = descriptor.parameters.iter().find(|p| p.name == "Transpose") {
        let frequency = |plugin: &mut dyn Hosted| -> Result<f64> {
            let mut previous = 0.0f32;
            let mut first = None;
            let mut last = 0.0;
            let mut crossings = 0;
            for block in 0..400 {
                let mut frames = [[0.0; 2]; BLOCK_SIZE];
                for (i, frame) in frames.iter_mut().enumerate() {
                    *frame = [((block * BLOCK_SIZE + i) as f32 * 440.0 * std::f32::consts::TAU
                        / SAMPLE_RATE as f32)
                        .sin()
                        * 0.1; 2];
                }
                plugin.process(&mut frames)?;
                for (i, frame) in frames.iter().enumerate() {
                    if block >= 200 && previous <= 0.0 && frame[0] > 0.0 {
                        let position = (block * BLOCK_SIZE + i) as f64 - 1.0
                            + (-previous / (frame[0] - previous)) as f64;
                        first.get_or_insert(position);
                        last = position;
                        crossings += 1;
                    }
                    previous = frame[0];
                }
            }
            anyhow::ensure!(crossings > 2, "Too little audio for the transpose probe");
            Ok((crossings - 1) as f64 * SAMPLE_RATE as f64 / (last - first.unwrap()))
        };
        let before = frequency(plugin.as_mut())?;
        plugin.set_parameter(parameter.id, (parameter.default + 0.25).min(1.0));
        let after = frequency(plugin.as_mut())?;
        transpose_report=format!("Transpose parameter: measured {before:.2} Hz before, {after:.2} Hz after normalized value {:.3}.\n",(parameter.default+0.25).min(1.0));
    }
    let report=format!("PASS: {} {:?} loaded, processed 100 sine blocks, and returned finite audio. Default peak {default_peak:.6}, pre-gain-zero peak {reduced_peak:?}.\n{transpose_report}",descriptor.name,descriptor.format);
    println!("{report}");
    std::fs::create_dir_all("dist")?;
    std::fs::write(
        format!("dist/plugin-probe-{:?}.txt", descriptor.format),
        report,
    )?;
    Ok(())
}

/// Optional WASAPI integration check using existing virtual cables only.
pub fn audio(vst3: &Path) -> Result<()> {
    use crate::{
        audio_io::{self, Counters},
        engine::{Command, Engine},
    };
    use crossbeam_queue::ArrayQueue;
    use std::{sync::Arc, time::Duration};
    let classes = scan_isolated(vst3.to_owned())?;
    let instrument = classes
        .iter()
        .find(|p| p.role == PluginRole::Instrument)
        .context("Missing fixture instrument")?;
    let effect = classes
        .iter()
        .find(|p| p.role == PluginRole::Effect)
        .context("Missing fixture effect")?;
    let devices = audio_io::devices()?;
    let primary = devices
        .outputs
        .iter()
        .find(|d| d.name == "CABLE Input (VB-Audio Virtual Cable)")
        .context("Requires existing VB-Audio CABLE Input")?;
    let secondary = devices
        .outputs
        .iter()
        .find(|d| d.name == "Line 1 (Virtual Audio Cable)")
        .context("Requires existing Line 1 (Virtual Audio Cable)")?;
    let mut source = Node::new(1, "Instrument", NodeKind::VstInstrument, [0.0; 2]);
    source.plugin = Some(NodePlugin::new(instrument.clone()));
    let mut fx = Node::new(2, "Plugin FX", NodeKind::VstEffect, [0.0; 2]);
    fx.plugin = Some(NodePlugin::new(effect.clone()));
    let mut out = Node::new(
        3,
        "Bus",
        NodeKind::Output {
            device: Some(primary.id.clone()),
        },
        [0.0; 2],
    );
    out.listen_enabled = true;
    out.listen_device = Some(secondary.id.clone());
    let mut project = Project {
        workflows: Default::default(),
        version: 1,
        name: "VST audio integration".into(),
        nodes: vec![source, fx, out],
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
    let status = engine.snapshot().map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        status.running && status.outputs == 2 && status.plugins.values().all(|s| s == "Loaded"),
        "Plugin session failed: {}",
        status.message
    );
    let mut report = String::new();
    let mut check = |label: &str, expected: f32| -> Result<()> {
        crate::diagnostics::peaks(&mut first, &mut second, Duration::from_millis(180));
        let measured =
            crate::diagnostics::peaks(&mut first, &mut second, Duration::from_millis(300));
        anyhow::ensure!(
            (measured.0 - expected).abs() < 0.008 && (measured.1 - expected).abs() < 0.008,
            "{label}: {measured:?}, expected {expected}"
        );
        report.push_str(&format!(
            "{label}: primary {:.6}, listen {:.6} (expected {expected:.3})\n",
            measured.0, measured.1
        ));
        Ok(())
    };
    check("Before note", 0.0)?;
    engine
        .send(Command::Note {
            id: 1,
            note: 60,
            velocity: 0.8,
            on: true,
        })
        .map_err(anyhow::Error::msg)?;
    check("Instrument through VST effect", 0.05)?;
    A::Disable
        .apply(&mut project.nodes[1])
        .map_err(anyhow::Error::msg)?;
    engine
        .send(Command::Update(project.clone()))
        .map_err(anyhow::Error::msg)?;
    check("Effect disable keybind action", 0.1)?;
    A::Mute
        .apply(&mut project.nodes[0])
        .map_err(anyhow::Error::msg)?;
    engine
        .send(Command::Update(project.clone()))
        .map_err(anyhow::Error::msg)?;
    check("Instrument mute keybind action", 0.0)?;
    A::Unmute
        .apply(&mut project.nodes[0])
        .map_err(anyhow::Error::msg)?;
    A::Enable
        .apply(&mut project.nodes[1])
        .map_err(anyhow::Error::msg)?;
    engine
        .send(Command::Update(project.clone()))
        .map_err(anyhow::Error::msg)?;
    check("Unmute / enable preserves note", 0.05)?;
    engine
        .send(Command::AllNotesOff(Some(1)))
        .map_err(anyhow::Error::msg)?;
    check("Stop all notes command", 0.0)?;
    engine
        .send(Command::Note {
            id: 1,
            note: 60,
            velocity: 0.8,
            on: true,
        })
        .map_err(anyhow::Error::msg)?;
    engine
        .send(Command::Note {
            id: 1,
            note: 60,
            velocity: 0.0,
            on: false,
        })
        .map_err(anyhow::Error::msg)?;
    check("Release audition note command", 0.0)?;
    anyhow::ensure!(errors.is_empty(), "Capture errors");
    engine.send(Command::Stop).map_err(anyhow::Error::msg)?;
    let report = format!(
        "PASS: VST instrument -> VST effect -> bus -> primary + Listen WASAPI loopbacks\n{report}"
    );
    std::fs::create_dir_all("dist")?;
    std::fs::write("dist/plugin-audio-self-test.txt", &report)?;
    println!("{report}");
    Ok(())
}
