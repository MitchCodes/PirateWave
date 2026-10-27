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
    let legacy = scan_isolated(vst2.to_owned())?
        .into_iter()
        .find(|p| p.role == PluginRole::Instrument)
        .context("Missing VST2 instrument")?;
    let mut report = String::new();
    let registry = EffectRegistry::default();
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
