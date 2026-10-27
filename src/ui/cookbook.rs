//! Built-in examples are constructed only on import; browsing creates no DSP state.
use super::*;
use piratewave_core::project::{Connection, InputPort, OutputPort};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Recipe {
    Clean,
    Music,
    Voices,
    Recording,
    Studio,
}
impl Recipe {
    const ALL: [Self; 5] = [
        Self::Clean,
        Self::Music,
        Self::Voices,
        Self::Recording,
        Self::Studio,
    ];
    fn name(self) -> &'static str {
        match self {
            Self::Clean => "Clear Waters",
            Self::Music => "Music Mate",
            Self::Voices => "Voice Party",
            Self::Recording => "Replay Deck",
            Self::Studio => "Captain’s Studio",
        }
    }
    fn description(self) -> &'static str {
        match self {
        Self::Clean => "A simple microphone cleaning chain, from rumble removal to peak control.",
        Self::Music => "Keep Spotify beneath your voice with automatic music ducking.",
        Self::Voices => "Switch between two voice changers, then add automatic tuning and reverb.",
        Self::Recording => "Capture recent application audio and preview or send it when you choose.",
        Self::Studio => "The complete setup: clean microphone, switchable voices, music ducking and replay slots.",
    }
    }
    fn route(self) -> &'static str {
        match self {
        Self::Clean => "Microphone > Clean Mic > Stream Mix",
        Self::Music => "Microphone (Priority) + Spotify (Ducked) > Auto Ducking > Stream Mix",
        Self::Voices => "Microphone > Change Voice > Voice A / Voice B > Autotune & Reverb > Stream Mix",
        Self::Recording => "Recording Input > Recorder: Preview > Monitor · Send > Stream Mix. Microphone > Stream Mix.",
        Self::Studio => "Microphone > Clean Mic > Change Voice > Voice A / Voice B > Autotune & Reverb > Auto Ducking > Stream Mix. Spotify > Ducked input. Recording Input > Recorder > Monitor / Stream Mix.",
    }
    }
    fn setup(self) -> &'static str {
        match self {
        Self::Clean => "Effects run in order: High-pass filter > Expander > De-esser > Parametric EQ > Compressor > Limiter. Start with the supplied gentle settings, then tune the thresholds to your microphone and room.",
        Self::Music => "Select Spotify in the Spotify node’s application picker. Adjust the ducking threshold until the activity indicator follows your speech. Music is reduced by 18 dB while you speak.",
        Self::Voices => "Choose and qualify voice packs in the two Voice Changer nodes. Both initially reference the default pack; choose your preferred pack for each voice. Switching voices briefly reloads conversion. Use Change Voice to select a branch or assign its keybinds. Autotune starts chromatic; choose a key and scale for singing. Unavailable voice packs need setup before conversion produces audio.",
        Self::Recording => "Choose an application in Recording Input. Open Recorder, Start recording, then capture a 5, 15 or 30 second slot. Preview goes only to Monitor; Send goes only to Stream Mix. Recording starts off and the live input is not sent through the recorder’s passthrough.",
        Self::Studio => "Configure Spotify and Recording Input in their application pickers. Choose and qualify voice packs for Voice A and Voice B (choose the desired pack in each node). Select a voice with Change Voice. Tune Clean Mic and the ducking threshold, choose a key/scale for Autotune, and start Recorder to fill the 5, 15 and 30 second histories. Preview is monitor-only; Send joins Stream Mix.",
    }
    }
    fn project(self, registry: &EffectRegistry) -> Project {
        let mut project = Project {
            name: self.name().into(),
            nodes: vec![],
            connections: vec![],
            sounds: vec![],
            version: 1,
        };
        let mut add =
            |id, name, kind, position| project.nodes.push(Node::new(id, name, kind, position));
        add(
            1,
            "Microphone",
            NodeKind::Input {
                device: None,
                loopback: false,
            },
            [30., 40.],
        );
        add(2, "Soundboard", NodeKind::Soundboard, [30., 500.]);
        add(
            3,
            "Stream Mix",
            NodeKind::Output {
                device: Some(String::new()),
            },
            [1650., 40.],
        );
        add(
            4,
            "Monitor",
            NodeKind::Output { device: None },
            [1650., 500.],
        );
        project.nodes[3].gain_db = -12.;
        route(&mut project, 2, 3, OutputPort::Passthrough, InputPort::Main);
        route(&mut project, 2, 4, OutputPort::Passthrough, InputPort::Main);
        let mut mic = 1;
        if matches!(self, Self::Clean | Self::Studio) {
            let mut node = Node::new(5, "Clean Mic", NodeKind::Processor, [300., 40.]);
            node.effects = vec![
                effect(registry, "high_pass", &[("cutoff", 80.)]),
                effect(registry, "expander", &[("threshold", -40.), ("range", 12.)]),
                effect(registry, "de_esser", &[("range", 6.)]),
                effect(
                    registry,
                    "equalizer",
                    &[("frequency", 250.), ("gain", -2.), ("q", 0.8)],
                ),
                effect(registry, "compressor", &[("ratio", 3.)]),
                effect(registry, "limiter", &[]),
            ];
            project.nodes.push(node);
            route(
                &mut project,
                mic,
                5,
                OutputPort::Passthrough,
                InputPort::Main,
            );
            mic = 5;
        }
        if matches!(self, Self::Voices | Self::Studio) {
            let mut switch = Node::new(6, "Change Voice", NodeKind::Switch, [570., 40.]);
            switch.switch_output = Some(7);
            project.nodes.push(switch);
            route(
                &mut project,
                mic,
                6,
                OutputPort::Passthrough,
                InputPort::Main,
            );
            for (id, name, y) in [(7, "Voice A", 40.), (8, "Voice B", 250.)] {
                project
                    .nodes
                    .push(Node::new(id, name, NodeKind::VoiceChanger, [840., y]));
                route(
                    &mut project,
                    6,
                    id,
                    OutputPort::Passthrough,
                    InputPort::Main,
                );
                route(
                    &mut project,
                    id,
                    9,
                    OutputPort::Passthrough,
                    InputPort::Main,
                );
            }
            let mut fx = Node::new(9, "Autotune & Reverb", NodeKind::Processor, [1110., 40.]);
            fx.effects = vec![
                effect(registry, "automatic_voice_correction", &[]),
                effect(registry, "reverb", &[("mix", 12.), ("decay", 1.2)]),
            ];
            project.nodes.push(fx);
            mic = 9;
        }
        if matches!(self, Self::Music | Self::Studio) {
            project.nodes.push(Node::new(
                10,
                "Spotify",
                NodeKind::ApplicationCapture { target: None },
                [1110., 300.],
            ));
            project.nodes.push(Node::new(
                11,
                "Auto Ducking",
                NodeKind::AutoDucking,
                [1380., 40.],
            ));
            route(
                &mut project,
                mic,
                11,
                OutputPort::Passthrough,
                InputPort::Priority,
            );
            route(
                &mut project,
                10,
                11,
                OutputPort::Passthrough,
                InputPort::Ducked,
            );
            mic = 11;
        }
        route(
            &mut project,
            mic,
            3,
            OutputPort::Passthrough,
            InputPort::Main,
        );
        if matches!(self, Self::Recording | Self::Studio) {
            project.nodes.push(Node::new(
                12,
                "Recording Input",
                NodeKind::ApplicationCapture { target: None },
                [570., 650.],
            ));
            let mut recorder = Node::new(13, "Recorder", NodeKind::Recording, [1110., 650.]);
            recorder.recording.slots = [5., 15., 30.]
                .into_iter()
                .map(|seconds| {
                    let mut slot = piratewave_core::recording::RecordSlot::new(&format!(
                        "Last {seconds} seconds"
                    ));
                    slot.seconds = seconds;
                    slot
                })
                .collect();
            project.nodes.push(recorder);
            route(
                &mut project,
                12,
                13,
                OutputPort::Passthrough,
                InputPort::Main,
            );
            route(&mut project, 13, 4, OutputPort::Preview, InputPort::Main);
            route(&mut project, 13, 3, OutputPort::Send, InputPort::Main);
        }
        // Compact columns for smaller recipes without rearranging their signal order.
        let mut columns: Vec<_> = project
            .nodes
            .iter()
            .map(|node| node.position[0] as i32)
            .collect();
        columns.sort_unstable();
        columns.dedup();
        for node in &mut project.nodes {
            node.position[0] = 30.
                + columns
                    .iter()
                    .position(|x| *x == node.position[0] as i32)
                    .unwrap() as f32
                    * 270.;
        }
        project
    }
}
fn effect(registry: &EffectRegistry, kind: &str, parameters: &[(&str, f32)]) -> EffectSpec {
    let mut effect = EffectSpec::new(registry.get(kind).expect("Built-in recipe effect"));
    for (name, value) in parameters {
        effect.parameters.insert((*name).into(), *value);
    }
    effect
}
fn route(project: &mut Project, from: u64, to: u64, port: OutputPort, input: InputPort) {
    project.connections.push(Connection {
        from,
        to,
        port,
        input,
    });
}
impl PirateWaveApp {
    pub(super) fn cookbook_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Cookbook");
        ui.label("Ready-made workspaces to make your own.");
        ui.add_space(8.);
        ui.label("Every recipe includes Microphone, Soundboard, Stream Mix and Monitor. Soundboard feeds both outputs; the microphone reaches Stream Mix through the recipe’s chain.");
        ui.label("After import: choose your microphone and Stream Mix device, check Monitor (system default at −12 dB), and add sounds on the Soundboard page. No sounds, shortcuts or voice packs are installed automatically.");
        ui.small("Import creates and opens a new workspace. Existing workspaces are kept; repeated imports receive a numbered name. If audio is running, it restarts using the recipe’s routing.");
        ui.add_space(16.);
        let mut import = None;
        for recipe in Recipe::ALL {
            ui.push_id(recipe.name(), |ui| {
                card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.heading(recipe.name());
                    ui.label(recipe.description());
                    ui.add(egui::Label::new(recipe.route()).wrap());
                    ui.collapsing("Setup & tuning", |ui| {
                        ui.add(egui::Label::new(recipe.setup()).wrap());
                    });
                    if ui
                        .add_enabled(
                            self.workspaces.is_some(),
                            egui::Button::new("Import workspace"),
                        )
                        .clicked()
                    {
                        import = Some(recipe);
                    }
                });
                ui.add_space(10.);
            });
        }
        if self.workspaces.is_none() {
            ui.colored_label(
                theme::palette().peach,
                "Workspace storage is unavailable. Check Settings/logs before importing.",
            );
        }
        if let Some(recipe) = import {
            self.add_workspace(recipe.project(&self.registry));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn connected(p: &Project, from: u64, to: u64, port: OutputPort, input: InputPort) -> bool {
        p.connections.contains(&Connection {
            from,
            to,
            port,
            input,
        })
    }
    fn reachable(p: &Project, from: u64, to: u64) -> bool {
        from == to
            || p.connections
                .iter()
                .filter(|edge| edge.from == from)
                .any(|edge| reachable(p, edge.to, to))
    }
    #[test]
    fn recipes_validate_round_trip_and_keep_required_routes() {
        let registry = EffectRegistry::default();
        for recipe in Recipe::ALL {
            let p = recipe.project(&registry);
            piratewave_core::graph::validate_with_registry(&p, &registry).unwrap();
            let restored: Project =
                serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
            assert_eq!(p.nodes, restored.nodes);
            assert_eq!(p.connections, restored.connections);
            assert_eq!(
                p.nodes
                    .iter()
                    .filter(|n| matches!(n.kind, NodeKind::Output { .. }))
                    .count(),
                2
            );
            assert!(reachable(&p, 1, 3));
            assert!(!reachable(&p, 1, 4), "No live microphone monitoring");
            assert!(connected(
                &p,
                2,
                3,
                OutputPort::Passthrough,
                InputPort::Main
            ));
            assert!(connected(
                &p,
                2,
                4,
                OutputPort::Passthrough,
                InputPort::Main
            ));
            assert!(p.sounds.is_empty());
            assert!(p
                .nodes
                .iter()
                .all(|node| node.keybinds.is_empty() && node.switch_keybinds.is_empty()));
            if matches!(recipe, Recipe::Clean | Recipe::Studio) {
                let clean = p.nodes.iter().find(|n| n.id == 5).unwrap();
                assert_eq!(
                    clean
                        .effects
                        .iter()
                        .map(|e| e.kind.as_str())
                        .collect::<Vec<_>>(),
                    [
                        "high_pass",
                        "expander",
                        "de_esser",
                        "equalizer",
                        "compressor",
                        "limiter"
                    ]
                );
                assert!(connected(
                    &p,
                    1,
                    5,
                    OutputPort::Passthrough,
                    InputPort::Main
                ));
                assert!(!connected(
                    &p,
                    1,
                    3,
                    OutputPort::Passthrough,
                    InputPort::Main
                ));
            }
            if matches!(recipe, Recipe::Voices | Recipe::Studio) {
                assert_eq!(
                    p.nodes
                        .iter()
                        .filter(|n| matches!(n.kind, NodeKind::VoiceChanger))
                        .count(),
                    2
                );
                assert_eq!(
                    p.nodes.iter().find(|n| n.id == 6).unwrap().switch_output,
                    Some(7)
                );
                for id in [7, 8] {
                    assert!(connected(
                        &p,
                        6,
                        id,
                        OutputPort::Passthrough,
                        InputPort::Main
                    ));
                    assert!(connected(
                        &p,
                        id,
                        9,
                        OutputPort::Passthrough,
                        InputPort::Main
                    ));
                }
                assert_eq!(
                    p.nodes
                        .iter()
                        .find(|n| n.id == 9)
                        .unwrap()
                        .effects
                        .iter()
                        .map(|e| e.kind.as_str())
                        .collect::<Vec<_>>(),
                    ["automatic_voice_correction", "reverb"]
                );
            }
            if matches!(recipe, Recipe::Music | Recipe::Studio) {
                assert!(connected(
                    &p,
                    if recipe == Recipe::Studio { 9 } else { 1 },
                    11,
                    OutputPort::Passthrough,
                    InputPort::Priority
                ));
                assert!(connected(
                    &p,
                    10,
                    11,
                    OutputPort::Passthrough,
                    InputPort::Ducked
                ));
                assert!(connected(
                    &p,
                    11,
                    3,
                    OutputPort::Passthrough,
                    InputPort::Main
                ));
            }
            if matches!(recipe, Recipe::Recording | Recipe::Studio) {
                assert!(connected(
                    &p,
                    12,
                    13,
                    OutputPort::Passthrough,
                    InputPort::Main
                ));
                assert!(connected(&p, 13, 4, OutputPort::Preview, InputPort::Main));
                assert!(connected(&p, 13, 3, OutputPort::Send, InputPort::Main));
                assert!(!p
                    .connections
                    .iter()
                    .any(|e| e.from == 13 && e.port == OutputPort::Passthrough));
                let recorder = &p.nodes.iter().find(|n| n.id == 13).unwrap().recording;
                assert!(!recorder.enabled);
                assert_eq!(
                    recorder.slots.iter().map(|s| s.seconds).collect::<Vec<_>>(),
                    [5., 15., 30.]
                );
                assert!(recorder
                    .slots
                    .iter()
                    .all(|s| s.asset.is_none() && s.keybinds.is_empty()));
            }
            if recipe == Recipe::Studio {
                assert!(connected(
                    &p,
                    5,
                    6,
                    OutputPort::Passthrough,
                    InputPort::Main
                ));
            }
        }
    }
    #[test]
    fn cookbook_import_button_preserves_workspace_and_suffixes_repeated_imports() {
        let root = std::env::temp_dir().join(format!(
            "piratewave-cookbook-{}",
            piratewave_core::recording::RecordSlot::new("").id
        ));
        let store =
            crate::workspaces::Workspaces::open(root.clone(), &root.join("absent.json")).unwrap();
        let original = store.active();
        let mut app = super::super::tests::app();
        app.project = store.load(original).unwrap();
        app.path = store.path(original);
        app.workspaces = Some(store);
        let ctx = egui::Context::default();
        for expected in ["Clear Waters", "Clear Waters 2"] {
            app.view = View::Cookbook;
            super::super::tests::render(&ctx, &mut app, vec![]);
            let output = super::super::tests::render(&ctx, &mut app, vec![]);
            let pos = super::super::tests::at(&output, "Import workspace");
            super::super::tests::click(&ctx, &mut app, pos, egui::PointerButton::Primary);
            assert_eq!(app.project.name, expected);
            assert_eq!(app.view, View::Mixer);
            assert!(!app.dirty);
        }
        let store = app.workspaces.as_ref().unwrap();
        assert_eq!(store.entries().len(), 3);
        assert_eq!(store.load(original).unwrap().name, "Workspace 1");
        assert_eq!(store.load(store.active()).unwrap().nodes, app.project.nodes);
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn browsing_cookbook_does_not_send_audio_commands_or_dirty_workspace() {
        let mut app = super::super::tests::app();
        let (engine, commands) = Engine::test_channel();
        app.engine = engine;
        let original = serde_json::to_string(&app.project).unwrap();
        let ctx = egui::Context::default();
        for width in [700., 1000., 1380., 1920.] {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 960.))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        app.cookbook_page(ui);
                    });
                },
            );
        }
        assert!(!app.dirty);
        assert!(commands.try_recv().is_err());
        assert_eq!(original, serde_json::to_string(&app.project).unwrap());
    }
}
