use super::*;
use std::time::SystemTime;

pub(super) fn app() -> PirateWaveApp {
    let mut project = Project::default();
    project.nodes.push(Node::new(
        6,
        "Voice processor",
        NodeKind::Processor,
        [300.0, 100.0],
    ));
    let (load_tx, _) = mpsc::channel();
    let (_, load_rx) = mpsc::channel();
    let settings = AppSettings::default();
    PirateWaveApp {
        effect_tuning: Default::default(),
        tools_ui: Default::default(),
        layout: Default::default(),
        extension_ui: Default::default(),
        controller: Default::default(),
        voice_ui: voices::VoiceUi::default(),
        recording_ui: Default::default(),
        project,
        path: PathBuf::new(),
        workspaces: None,
        workspace_rename: None,
        workspace_delete: None,
        engine: Engine::new(),
        status: Status::default(),
        devices: Devices::default(),
        applications: application_capture::smoke_applications(),
        applications_error: String::new(),
        registry: EffectRegistry::default(),
        plugin_ui: plugins::PluginUi::new(true),
        view: View::Mixer,
        selected: 1,
        dirty: false,
        changed_at: Instant::now(),
        last_apply: Instant::now(),
        notice: String::new(),
        notice_at: Instant::now(),
        app_settings: settings.clone(),
        settings_draft: settings,
        settings_section: keybinds::SettingsSection::Audio,
        settings_dirty: false,
        node_binding_draft: BTreeMap::new(),
        node_extra_binding_draft: BTreeMap::new(),
        add_node: false,
        choose_effect: false,
        effect_help: None,
        recorder: None,
        endpoint_states: vec![],
        new_name: String::new(),
        new_kind: 0,
        pending_port: None,
        pending_output: Default::default(),
        selected_edge: None,
        hotkeys: None,
        shortcut_errors: vec![],
        clips: HashMap::new(),
        loading: HashSet::new(),
        load_tx,
        load_rx,
        needs_apply: false,
        needs_restart: false,
        smoke: true,
        smoke_stage: 0,
        smoke_at: Instant::now(),
        smoke_waiting: false,
        smoke_error: None,
    }
}
#[test]
fn voice_pages_shortcuts_and_second_activation_are_safe() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.project.nodes.push(Node::new(
        40,
        "First voice",
        NodeKind::VoiceChanger,
        [0.0; 2],
    ));
    let mut second = Node::new(41, "Second voice", NodeKind::VoiceChanger, [0.0; 2]);
    second.effects_disabled = true;
    app.project.nodes.push(second);
    app.handle_shortcut(ShortcutAction::Node {
        id: 41,
        action: NodeAction::Enable,
    });
    assert!(
        app.project
            .nodes
            .iter()
            .find(|n| n.id == 41)
            .unwrap()
            .effects_disabled
    );
    app.handle_shortcut(ShortcutAction::Node {
        id: 40,
        action: NodeAction::Disable,
    });
    app.handle_shortcut(ShortcutAction::Node {
        id: 41,
        action: NodeAction::Enable,
    });
    assert!(
        !app.project
            .nodes
            .iter()
            .find(|n| n.id == 41)
            .unwrap()
            .effects_disabled
    );
    app.view = View::VoiceNode(41);
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output).iter().any(|(s, _)| s == "Restart voice"));
    assert!(labels(&output)
        .iter()
        .any(|(s, _)| s == "LLVC voice — Beta"));
    if crate::voice::pack::manifest().directml_qualified {
        click(
            &ctx,
            &mut app,
            at(&output, "Auto"),
            egui::PointerButton::Primary,
        );
        let output = render(&ctx, &mut app, vec![]);
        click(
            &ctx,
            &mut app,
            at(&output, "GPU (DirectML + CPU)"),
            egui::PointerButton::Primary,
        );
        let voice = app
            .project
            .nodes
            .iter()
            .find(|n| n.id == 41)
            .unwrap()
            .voice
            .as_ref()
            .unwrap();
        assert_eq!(
            voice.backend,
            piratewave_core::voice::VoiceBackend::DirectMl
        );
        let saved = serde_json::to_string(&app.project).unwrap();
        let restored: Project = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            restored
                .nodes
                .iter()
                .find(|n| n.id == 41)
                .unwrap()
                .voice
                .as_ref()
                .unwrap()
                .backend,
            piratewave_core::voice::VoiceBackend::DirectMl
        );
    }
    app.open_node_keybinds(41);
    let output = render(&ctx, &mut app, vec![]);
    for action in [
        NodeAction::Disable,
        NodeAction::Enable,
        NodeAction::ToggleDisabled,
    ] {
        assert!(labels(&output).iter().any(|(s, _)| s == action.label()));
    }
    let before = serde_json::to_string(&app.project).unwrap();
    app.voice_ui.help = true;
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output)
        .iter()
        .any(|(s, _)| s == "Turn your microphone into another voice"));
    assert_eq!(before, serde_json::to_string(&app.project).unwrap());
    app.voice_ui.help = false;
    app.view = View::Voices;
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output).iter().any(|(s, _)| s == "Play preview"));
    app.remove_audio_node(41);
    assert!(!app.project.nodes.iter().any(|n| n.id == 41));
}
#[test]
fn voice_creation_preparation_opens_help_without_changing_the_mixer() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.view = View::Voices;
    let before = serde_json::to_string(&app.project).unwrap();
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Create voice…"),
        egui::PointerButton::Primary,
    );
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output)
        .iter()
        .any(|(s, _)| s.starts_with("Voice Creation — Beta.")));
    click(
        &ctx,
        &mut app,
        at(&output, "Voice Creation Guide"),
        egui::PointerButton::Primary,
    );
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output)
        .iter()
        .any(|(s, _)| s == "1. Choose one voice"));
    assert_eq!(before, serde_json::to_string(&app.project).unwrap());
    click(
        &ctx,
        &mut app,
        at(&output, "< Back to Voices"),
        egui::PointerButton::Primary,
    );
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output).iter().any(|(s, _)| s == "Play preview"));
}

#[test]
fn training_storage_shows_catalog_sizes_without_starting_installation() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.view = View::Settings;
    app.settings_section = keybinds::SettingsSection::Training;
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output).iter().any(|(s, _)| s.starts_with("cpu: ")));
    assert!(labels(&output).iter().any(|(s, _)| s == "Measure storage"));
    assert!(!labels(&output)
        .iter()
        .any(|(s, _)| s == "Install training tools"));
}

#[test]
fn voice_training_wizard_renders_every_stage_without_changing_saved_job() {
    use crate::training::release::{Job, Recording};
    let root = std::env::temp_dir().join(format!(
        "piratewave-wizard-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let folder = root.join("jobs/fixture");
    std::fs::create_dir_all(&folder).unwrap();
    let job = Job {
        schema: 1,
        id: "fixture".into(),
        name: "Wizard fixture".into(),
        backend: "cpu".into(),
        runtime: root.join("tools"),
        shared: root.join("shared"),
        native_worker: root.join("worker.exe"),
        status: "draft".into(),
        message: "Ready to prepare".into(),
        recipe: "test".into(),
        recordings: vec![Recording {
            id: "take".into(),
            path: "recordings/take.wav".into(),
            name: "Clean take".into(),
            role: "train".into(),
            excluded: false,
            seconds: 20.,
            clipping: 0.,
            silence: 0.1,
        }],
    };
    job.save(&folder).unwrap();
    let original = std::fs::read(folder.join("job.json")).unwrap();
    let mut app = app();
    app.view = View::Voices;
    for (phase, expected) in [
        (0, "Create training project"),
        (1, "Record raw microphone"),
        (2, "Exclude this clip"),
        (3, "Download and install cpu"),
        (4, "Start / resume local training"),
        (5, "Native streaming preview"),
        (6, "Install this CPU voice revision"),
    ] {
        let ctx = egui::Context::default();
        app.voice_ui.training.smoke_job(folder.clone(), phase);
        let _ = render(&ctx, &mut app, vec![]);
        let output = render(&ctx, &mut app, vec![]);
        assert!(
            labels(&output).iter().any(|(s, _)| s == expected),
            "Missing {expected} at phase {phase}"
        );
        assert!(labels(&output)
            .iter()
            .any(|(s, _)| s == "Help for this stage"));
        assert_eq!(original, std::fs::read(folder.join("job.json")).unwrap());
    }
    std::fs::remove_file(folder.join("job.json")).unwrap();
    std::fs::remove_dir(folder).unwrap();
    std::fs::remove_dir(root.join("jobs")).unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn voice_creation_guide_can_scroll_to_the_final_routing_instructions() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.voice_ui.training.smoke_page(true);
    let draw = |app: &mut PirateWaveApp, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| app.voice_creation_page(ui));
                });
            },
        )
    };
    let _ = draw(&mut app, vec![]);
    let output = draw(&mut app, vec![]);
    let _ = draw(
        &mut app,
        vec![egui::Event::PointerMoved(at(
            &output,
            "What creating a voice means",
        ))],
    );
    let _ = draw(
        &mut app,
        vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, -10000.0),
            modifiers: Default::default(),
        }],
    );
    for _ in 0..30 {
        let _ = draw(&mut app, vec![]);
    }
    let output = draw(&mut app, vec![]);
    assert!(labels(&output)
        .iter()
        .any(|(s, _)| s.starts_with("Once qualified, route microphone")));
}

pub(super) fn render(
    ctx: &egui::Context,
    app: &mut PirateWaveApp,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1700.0, 1100.0))),
        events,
        ..Default::default()
    };
    ctx.run(input, |ctx| {
        app.record_shortcut_input(ctx);
        egui::CentralPanel::default().show(ctx, |ui| match app.view {
            View::Cookbook => app.cookbook_page(ui),
            View::Tools => app.tools_page(ui),
            View::Tool(tool) => app.tool_page(ui, tool),
            View::Plugins => app.plugin_library_page(ui),
            View::PluginNode(id) => app.plugin_node_page(ui, id),
            View::Voices => app.voices_page(ui),
            View::VoiceNode(id) => app.voice_node_page(ui, id),
            View::Mixer => app.mixer(ui),
            View::NodeKeybinds(id) => app.node_keybinds_page(ui, id),
            View::Ducking(id) => app.ducking_page(ui, id),
            View::Routing => app.routing(ui),
            View::Recording(id) => app.recording_page(ui, id),
            View::RecordingSlotKeybinds(id, slot) => app.recording_slot_keybinds_page(ui, id, slot),
            View::Settings => app.settings_page(ui),
            View::Effects => app.effects(ui),
            View::Soundboard => app.soundboard(ui),
        });
        app.dialogs(ctx);
        app.voice_help(ctx);
    })
}
pub(super) fn labels(output: &egui::FullOutput) -> Vec<(String, Pos2)> {
    fn collect(shape: &egui::Shape, result: &mut Vec<(String, Pos2)>) {
        match shape {
            egui::Shape::Text(text) => result.push((
                text.galley.job.text.clone(),
                text.pos + text.galley.rect.center().to_vec2(),
            )),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, result)
                }
            }
            _ => {}
        }
    }
    let mut result = vec![];
    for shape in &output.shapes {
        collect(&shape.shape, &mut result);
    }
    result
}
pub(super) fn at(output: &egui::FullOutput, label: &str) -> Pos2 {
    labels(output)
        .into_iter()
        .find(|(text, _)| text == label)
        .unwrap_or_else(|| {
            panic!(
                "Missing label {label}; present labels: {:?}",
                labels(output).iter().map(|(s, _)| s).collect::<Vec<_>>()
            )
        })
        .1
}
pub(super) fn click(
    ctx: &egui::Context,
    app: &mut PirateWaveApp,
    pos: Pos2,
    button: egui::PointerButton,
) {
    let _ = render(ctx, app, vec![egui::Event::PointerMoved(pos)]);
    let _ = render(
        ctx,
        app,
        vec![egui::Event::PointerButton {
            pos,
            button,
            pressed: true,
            modifiers: Default::default(),
        }],
    );
    let _ = render(
        ctx,
        app,
        vec![egui::Event::PointerButton {
            pos,
            button,
            pressed: false,
            modifiers: Default::default(),
        }],
    );
}
#[test]
fn mixer_keybind_button_opens_source_page() {
    let ctx = egui::Context::default();
    let mut app = app();
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    assert_eq!(
        labels(&output)
            .iter()
            .filter(|(s, _)| s == "Keybinds")
            .count(),
        5,
        "Three sources and two output buses; utility pane starts collapsed"
    );
    click(
        &ctx,
        &mut app,
        at(&output, "Keybinds"),
        egui::PointerButton::Primary,
    );
    assert!(app.view == View::NodeKeybinds(1));
    let output = render(&ctx, &mut app, vec![]);
    for action in NodeAction::SOURCE {
        assert!(
            labels(&output).iter().any(|(s, _)| s == action.label()),
            "{} missing",
            action.label()
        );
    }
}

#[test]
fn every_effect_help_opens_and_closes_without_changing_audio_settings() {
    let mut app = app();
    let ids: Vec<_> = app.registry.factories().iter().map(|f| f.id).collect();
    for id in ids {
        let ctx = egui::Context::default();
        app.view = View::Effects;
        app.selected = 1;
        app.project.nodes[0].effects = vec![EffectSpec::new(app.registry.get(id).unwrap())];
        // Help remains usable even on a disabled effect.
        app.project.nodes[0].effects[0].enabled = false;
        let before = serde_json::to_string(&app.project).unwrap();
        let _ = render(&ctx, &mut app, vec![]);
        let output = render(&ctx, &mut app, vec![]);
        click(
            &ctx,
            &mut app,
            at(&output, "Help"),
            egui::PointerButton::Primary,
        );
        assert_eq!(app.effect_help.as_deref(), Some(id));
        let _ = render(&ctx, &mut app, vec![]);
        let output = render(&ctx, &mut app, vec![]);
        assert!(
            labels(&output)
                .iter()
                .any(|(text, _)| text == "What it does"),
            "{id}"
        );
        click(
            &ctx,
            &mut app,
            at(&output, "Close help"),
            egui::PointerButton::Primary,
        );
        assert!(app.effect_help.is_none(), "{id}");
        assert_eq!(before, serde_json::to_string(&app.project).unwrap(), "{id}");
        assert!(
            !app.dirty,
            "Opening help must not save or reconfigure audio"
        );
    }
}

#[test]
fn long_effect_tutorial_can_scroll_to_the_end() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.effect_help = Some("automatic_voice_correction".into());
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    let pointer = at(&output, "What it does");
    let _ = render(&ctx, &mut app, vec![egui::Event::PointerMoved(pointer)]);
    let _ = render(
        &ctx,
        &mut app,
        vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, -10000.0),
            modifiers: Default::default(),
        }],
    );
    for _ in 0..30 {
        let _ = render(&ctx, &mut app, vec![]);
    }
    let output = render(&ctx, &mut app, vec![]);
    assert!(
        labels(&output)
            .iter()
            .any(|(text, _)| text.starts_with("Effects run from top to bottom.")),
        "The tutorial's final paragraph must be reachable by scrolling"
    );
}

#[test]
fn general_startup_options_are_independent_and_only_apply_on_next_launch() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.view = View::Settings;
    app.settings_section = keybinds::SettingsSection::General;
    assert_eq!(
        app.app_settings.startup,
        piratewave_core::shortcuts::StartupSettings::default()
    );
    let _ = render(&ctx, &mut app, vec![]);
    for label in [
        "Start with Windows",
        "Start minimized",
        "Start audio automatically",
    ] {
        let output = render(&ctx, &mut app, vec![]);
        click(
            &ctx,
            &mut app,
            at(&output, label),
            egui::PointerButton::Primary,
        );
    }
    assert!(app.app_settings.startup.start_with_windows);
    assert!(app.app_settings.startup.start_minimized);
    assert!(app.app_settings.startup.start_audio_automatically);
    assert!(
        !app.engine.snapshot().unwrap().running,
        "Preference changes must not start audio now"
    );
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Start with Windows"),
        egui::PointerButton::Primary,
    );
    assert!(!app.app_settings.startup.start_with_windows);
    assert!(app.app_settings.startup.start_minimized);
    assert!(app.app_settings.startup.start_audio_automatically);
    app.settings_section = keybinds::SettingsSection::Keybinds;
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Restore defaults"),
        egui::PointerButton::Primary,
    );
    assert_eq!(app.settings_draft.startup, app.app_settings.startup);
}

#[test]
fn launch_minimization_is_independent_and_bad_workspace_does_not_start_audio() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.app_settings.startup.start_minimized = true;
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        app.apply_launch_preferences(ctx, true)
    });
    assert!(output.viewport_output[&egui::ViewportId::ROOT]
        .commands
        .iter()
        .any(|cmd| matches!(cmd, egui::ViewportCommand::Minimized(true))));
    assert!(!app.engine.snapshot().unwrap().running);
    app.app_settings.startup.start_audio_automatically = true;
    app.apply_launch_preferences(&ctx, false);
    assert!(!app.engine.snapshot().unwrap().running);
    assert!(app.notice.contains("Automatic audio start skipped"));
}

fn shortcut_event(key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers,
    }
}

#[test]
fn recorder_button_captures_modifiers_into_app_draft_only_after_release() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.view = View::Settings;
    app.settings_section = keybinds::SettingsSection::Keybinds;
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Record"),
        egui::PointerButton::Primary,
    );
    assert!(app.recorder.is_some());
    let mods = egui::Modifiers {
        ctrl: true,
        alt: true,
        ..Default::default()
    };
    let output = render(
        &ctx,
        &mut app,
        vec![
            shortcut_event(egui::Key::F3, true, mods),
            egui::Event::Text("ignored".into()),
        ],
    );
    assert!(labels(&output)
        .iter()
        .any(|(text, _)| text == "Ctrl+Alt+F3"));
    assert_eq!(
        app.settings_draft.keybinds[&AppAction::StopSounds],
        "Ctrl+Alt+Space"
    );
    let _ = render(
        &ctx,
        &mut app,
        vec![shortcut_event(egui::Key::F3, false, Default::default())],
    );
    assert!(app.recorder.is_none());
    assert_eq!(
        app.settings_draft.keybinds[&AppAction::StopSounds],
        "Ctrl+Alt+F3"
    );
    assert_eq!(
        app.app_settings.keybinds[&AppAction::StopSounds],
        "Ctrl+Alt+Space"
    );
    assert!(!app.dirty);
}

#[test]
fn recorder_supports_node_soundboard_cancel_and_loss_of_focus() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.open_node_keybinds(1);
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Record"),
        egui::PointerButton::Primary,
    );
    let _ = render(
        &ctx,
        &mut app,
        vec![shortcut_event(
            egui::Key::Space,
            true,
            egui::Modifiers::SHIFT,
        )],
    );
    let _ = render(
        &ctx,
        &mut app,
        vec![shortcut_event(egui::Key::Space, false, Default::default())],
    );
    assert_eq!(app.node_binding_draft[&NodeAction::Mute], "Shift+Space");
    assert!(app.project.nodes[0].keybinds.is_empty());
    // Beginning another recording and pressing Escape preserves the draft.
    app.begin_shortcut_recording(
        &ctx,
        ShortcutAction::Node {
            id: 1,
            action: NodeAction::Mute,
        },
        "Mute".into(),
    );
    let _ = render(
        &ctx,
        &mut app,
        vec![shortcut_event(egui::Key::Escape, true, Default::default())],
    );
    assert!(app.recorder.is_none());
    assert_eq!(app.node_binding_draft[&NodeAction::Mute], "Shift+Space");
    let _ = render(
        &ctx,
        &mut app,
        vec![shortcut_event(egui::Key::Escape, false, Default::default())],
    );
    app.begin_shortcut_recording(
        &ctx,
        ShortcutAction::App(AppAction::StopSounds),
        "Stop".into(),
    );
    let _ = ctx.run(
        egui::RawInput {
            focused: false,
            ..Default::default()
        },
        |ctx| app.record_shortcut_input(ctx),
    );
    assert!(app.recorder.is_none());
    assert_eq!(
        app.settings_draft.keybinds[&AppAction::StopSounds],
        "Ctrl+Alt+Space"
    );

    let ctx = egui::Context::default();
    app.view = View::Soundboard;
    app.project.sounds.push(piratewave_core::project::Sound {
        id: 42,
        name: "Example".into(),
        path: "fixture.wav".into(),
        shortcut: String::new(),
        volume: 1.0,
    });
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Record"),
        egui::PointerButton::Primary,
    );
    let _ = render(
        &ctx,
        &mut app,
        vec![shortcut_event(egui::Key::F8, true, Default::default())],
    );
    let _ = render(
        &ctx,
        &mut app,
        vec![shortcut_event(egui::Key::F8, false, Default::default())],
    );
    assert_eq!(app.project.sounds[0].shortcut, "F8");
    assert!(app.dirty);
    assert!(
        !app.engine.snapshot().unwrap().running,
        "Recording a shortcut must not execute it"
    );
}
#[test]
fn processor_disable_checkbox_works_without_source_controls() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.app_settings.ui.utilities_expanded = true;
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    assert_eq!(
        labels(&output)
            .iter()
            .filter(|(s, _)| s.trim() == "Solo")
            .count(),
        3
    );
    click(
        &ctx,
        &mut app,
        at(&output, "Disable (passthrough)"),
        egui::PointerButton::Primary,
    );
    assert!(
        app.project
            .nodes
            .iter()
            .find(|n| n.id == 6)
            .unwrap()
            .effects_disabled
    );
    assert!(app.needs_apply);
}
#[test]
fn mixer_context_menu_deletes_every_node_kind() {
    for (label, id) in [
        ("AUDIO INPUT", 1),
        ("DESKTOP CAPTURE", 2),
        ("SOUNDBOARD", 3),
        ("PROCESSOR", 6),
        ("OUTPUT BUS", 4),
    ] {
        let ctx = egui::Context::default();
        let mut app = app();
        app.app_settings.ui.utilities_expanded = true;
        let _ = render(&ctx, &mut app, vec![]);
        let output = render(&ctx, &mut app, vec![]);
        click(
            &ctx,
            &mut app,
            at(&output, label),
            egui::PointerButton::Secondary,
        );
        let output = render(&ctx, &mut app, vec![]);
        click(
            &ctx,
            &mut app,
            at(&output, "Remove node"),
            egui::PointerButton::Primary,
        );
        assert!(!app.project.nodes.iter().any(|n| n.id == id));
        assert!(app
            .project
            .connections
            .iter()
            .all(|c| c.from != id && c.to != id));
        assert!(app.needs_restart);
    }
}
#[test]
fn processor_keybind_page_only_offers_processor_actions() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.open_node_keybinds(6);
    let output = render(&ctx, &mut app, vec![]);
    let texts = labels(&output);
    for action in NodeAction::PROCESSOR {
        assert!(texts.iter().any(|(s, _)| s == action.label()));
    }
    for action in NodeAction::SOURCE {
        assert!(!texts.iter().any(|(s, _)| s == action.label()));
    }
}
#[test]
fn settings_shortcut_field_displays_entire_default_key() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.view = View::Settings;
    app.settings_section = keybinds::SettingsSection::Keybinds;
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    let mut found = false;
    for shape in &output.shapes {
        if let egui::Shape::Text(text) = &shape.shape {
            if text.galley.job.text == "Ctrl+Alt+Space" {
                found = true;
                let right = text.pos.x + text.galley.rect.right();
                assert!(
                    shape.clip_rect.right() >= right,
                    "Shortcut field clips its value"
                );
            }
        }
    }
    assert!(found);
}
#[test]
fn node_dispatch_and_deletion_use_same_actions_as_global_keys() {
    let mut app = app();
    app.handle_shortcut(ShortcutAction::Node {
        id: 1,
        action: NodeAction::Mute,
    });
    assert!(app.project.nodes[0].mute);
    app.open_node_keybinds(6);
    assert!(app.node_binding_draft.is_empty());
    app.handle_shortcut(ShortcutAction::Node {
        id: 6,
        action: NodeAction::Disable,
    });
    assert!(
        app.project
            .nodes
            .iter()
            .find(|n| n.id == 6)
            .unwrap()
            .effects_disabled
    );
    app.remove_audio_node(6);
    assert!(app.view == View::Mixer);
    app.handle_shortcut(ShortcutAction::Node {
        id: 6,
        action: NodeAction::ToggleDisabled,
    });
}

#[test]
fn output_listen_checkbox_and_keybind_page_work() {
    let ctx = egui::Context::default();
    let mut app = app();
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    assert!(!labels(&output).iter().any(|(s, _)| s == "Listen on"));
    click(
        &ctx,
        &mut app,
        at(&output, "Listen"),
        egui::PointerButton::Primary,
    );
    assert!(app.project.nodes[3].listen_enabled);
    assert!(app.project.nodes[3].listen_device.is_none());
    assert!(!app.needs_restart);
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output).iter().any(|(s, _)| s == "Listen on"));
    let output_button = labels(&output)
        .into_iter()
        .filter(|(s, _)| s == "Keybinds")
        .nth(3)
        .unwrap()
        .1;
    click(&ctx, &mut app, output_button, egui::PointerButton::Primary);
    assert!(app.view == View::NodeKeybinds(4));
    let output = render(&ctx, &mut app, vec![]);
    for action in NodeAction::OUTPUT {
        assert!(labels(&output).iter().any(|(s, _)| s == action.label()));
    }
    app.handle_shortcut(ShortcutAction::Node {
        id: 4,
        action: NodeAction::ListenOff,
    });
    assert!(!app.project.nodes[3].listen_enabled);
    app.handle_shortcut(ShortcutAction::Node {
        id: 4,
        action: NodeAction::ToggleListening,
    });
    assert!(app.project.nodes[3].listen_enabled);
    app.project.nodes[3].gain_db = -20.0;
    app.handle_shortcut(ShortcutAction::Node {
        id: 4,
        action: NodeAction::ResetVolume,
    });
    assert_eq!(app.project.nodes[3].gain_db, 0.0);
}

#[test]
fn plugin_library_creates_nodes_and_opens_instrument_and_effect_controls() {
    use piratewave_core::plugins::PluginRole;
    let ctx = egui::Context::default();
    let mut app = app();
    app.plugin_ui.library.plugins = vec![
        plugins::fixture(PluginRole::Instrument),
        plugins::fixture(PluginRole::Effect),
    ];
    app.view = View::Plugins;
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Add instrument node"),
        egui::PointerButton::Primary,
    );
    assert!(app.view == View::Routing);
    let id = app.selected;
    assert!(app
        .project
        .nodes
        .iter()
        .any(|n| n.id == id && matches!(n.kind, NodeKind::VstInstrument) && n.plugin.is_some()));
    app.view = View::PluginNode(id);
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output)
        .iter()
        .any(|(s, _)| s == "Play notes (MIDI channel 1)"));
    app.open_node_keybinds(id);
    let output = render(&ctx, &mut app, vec![]);
    for action in NodeAction::INSTRUMENT {
        assert!(labels(&output).iter().any(|(s, _)| s == action.label()));
    }
    app.view = View::Plugins;
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Add effect node"),
        egui::PointerButton::Primary,
    );
    let fx_id = app.selected;
    assert!(app
        .project
        .nodes
        .iter()
        .any(|n| n.id == fx_id && matches!(n.kind, NodeKind::VstEffect)));
    app.view = View::PluginNode(fx_id);
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output)
        .iter()
        .any(|(s, _)| s == "Disable (passthrough)"));
    app.open_node_keybinds(fx_id);
    let output = render(&ctx, &mut app, vec![]);
    for action in NodeAction::PROCESSOR {
        assert!(labels(&output).iter().any(|(s, _)| s == action.label()));
    }
}

#[test]
fn application_node_creation_picker_and_source_controls_work() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.add_node = true;
    app.new_kind = 7;
    app.new_name = "Player audio".into();
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Create node"),
        egui::PointerButton::Primary,
    );
    let id = app.selected;
    let node = app.project.nodes.iter().find(|n| n.id == id).unwrap();
    assert!(matches!(
        node.kind,
        NodeKind::ApplicationCapture { target: None }
    ));
    assert!(node.keybinds.is_empty());
    // Let the closed modal finish fading before interacting with controls behind it.
    for _ in 0..30 {
        let _ = render(&ctx, &mut app, vec![]);
    }
    // Put the new source at the start of the horizontal mixer viewport.
    app.project.nodes.sort_by_key(|n| n.id != id);
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Choose application..."),
        egui::PointerButton::Primary,
    );
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "player.exe — Example Player - Now playing"),
        egui::PointerButton::Primary,
    );
    let node = app.project.nodes.iter().find(|n| n.id == id).unwrap();
    assert!(
        matches!(&node.kind,NodeKind::ApplicationCapture{target:Some(target)} if target.window_title=="Example Player - Now playing")
    );
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Match exact window title"),
        egui::PointerButton::Primary,
    );
    assert!(
        matches!(&app.project.nodes[0].kind,NodeKind::ApplicationCapture{target:Some(target)} if target.match_title)
    );
    app.open_node_keybinds(id);
    let output = render(&ctx, &mut app, vec![]);
    for action in NodeAction::SOURCE {
        assert!(labels(&output)
            .iter()
            .any(|(text, _)| text == action.label()));
    }
}

#[test]
fn voice_change_category_adds_all_ten_effects_from_dialog() {
    let ids = [
        "delay",
        "vocoder",
        "resonator",
        "distortion",
        "pitch",
        "flanger",
        "chorus",
        "tremolo",
        "stereo",
        "wahwah",
    ];
    let mut app = app();
    app.view = View::Effects;
    app.selected = 1;
    let ctx = egui::Context::default();
    for id in ids {
        app.choose_effect = true;
        let _ = render(&ctx, &mut app, vec![]);
        let output = render(&ctx, &mut app, vec![]);
        assert!(labels(&output)
            .iter()
            .any(|(s, _)| s == "Voice Change Effects"));
        let name = app.registry.get(id).unwrap().name;
        click(
            &ctx,
            &mut app,
            at(&output, name),
            egui::PointerButton::Primary,
        );
        assert!(!app.choose_effect, "{id} selection did not close dialog");
        assert_eq!(app.project.nodes[0].effects.last().unwrap().kind, id);
    }
}

#[test]
fn switch_dropdown_and_shortcuts_select_connected_output() {
    let mut app = app();
    let mut switch = Node::new(90, "Voice selector", NodeKind::Switch, [0.; 2]);
    switch.switch_output = Some(4);
    app.project.nodes.push(switch);
    app.project.connections.extend([
        piratewave_core::project::Connection {
            input: Default::default(),
            port: Default::default(),
            from: 90,
            to: 4,
        },
        piratewave_core::project::Connection {
            input: Default::default(),
            port: Default::default(),
            from: 90,
            to: 5,
        },
    ]);
    let ctx = egui::Context::default();
    app.view = View::Effects;
    app.selected = 90;
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output).iter().any(|(s, _)| s == "Active output"));
    click(
        &ctx,
        &mut app,
        at(&output, "Monitor"),
        egui::PointerButton::Primary,
    );
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Stream mix"),
        egui::PointerButton::Primary,
    );
    assert_eq!(
        app.project
            .nodes
            .iter()
            .find(|n| n.id == 90)
            .unwrap()
            .switch_output,
        Some(5)
    );
    app.handle_shortcut(ShortcutAction::SwitchOutput { id: 90, output: 4 });
    assert_eq!(
        app.project
            .nodes
            .iter()
            .find(|n| n.id == 90)
            .unwrap()
            .switch_output,
        Some(4)
    );
    app.open_node_keybinds(90);
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output).iter().any(|(s, _)| s == "Activate Monitor"));
    assert!(labels(&output)
        .iter()
        .any(|(s, _)| s == "Activate Stream mix"));
    app.project.disconnect(90, 4);
    app.handle_shortcut(ShortcutAction::SwitchOutput { id: 90, output: 4 });
    assert_eq!(
        app.project
            .nodes
            .iter()
            .find(|n| n.id == 90)
            .unwrap()
            .switch_output,
        Some(5)
    );
}
#[test]
fn effect_shortcuts_follow_identity_after_reorder_and_ignore_deleted_targets() {
    use piratewave_core::shortcuts::EffectAction;
    let mut app = app();
    let mut node = Node::new(90, "Effects", NodeKind::Processor, [0.; 2]);
    node.effects = vec![
        EffectSpec::new(app.registry.get("gain").unwrap()),
        EffectSpec::new(app.registry.get("delay").unwrap()),
    ];
    let gain = node.effects[0].id;
    let delay = node.effects[1].id;
    app.project.nodes.push(node);
    app.handle_shortcut(ShortcutAction::Effect {
        id: 90,
        effect: gain,
        action: EffectAction::Disable,
    });
    let node = app.project.nodes.iter_mut().find(|n| n.id == 90).unwrap();
    assert!(!node.effects[0].enabled);
    assert!(node.effects[1].enabled);
    node.effects.swap(0, 1);
    app.handle_shortcut(ShortcutAction::Effect {
        id: 90,
        effect: gain,
        action: EffectAction::Enable,
    });
    let node = app.project.nodes.iter_mut().find(|n| n.id == 90).unwrap();
    assert!(node.effects[1].enabled);
    node.effects.remove(1);
    app.handle_shortcut(ShortcutAction::Effect {
        id: 90,
        effect: gain,
        action: EffectAction::Disable,
    });
    assert!(
        app.project
            .nodes
            .iter()
            .find(|n| n.id == 90)
            .unwrap()
            .effects[0]
            .enabled
    );
    app.open_node_keybinds(90);
    let action = ShortcutAction::Effect {
        id: 90,
        effect: delay,
        action: EffectAction::Toggle,
    };
    assert!(action.exists(&app.project));
    let ctx = egui::Context::default();
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output)
        .iter()
        .any(|(s, _)| s.contains("Delay - Toggle effect")));
}
#[test]
fn recording_rack_controls_and_shortcuts_are_scoped_and_removed_effects_stay_deleted() {
    use piratewave_core::shortcuts::EffectAction;
    let mut app = app();
    app.recording_smoke_fixture(true);
    app.view = View::Recording(91);
    let ctx = egui::Context::default();
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Slot effects (0)"),
        egui::PointerButton::Primary,
    );
    assert!(matches!(app.view, View::Effects));
    assert_eq!(app.selected, 91);
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "+ Add processor"),
        egui::PointerButton::Primary,
    );
    let output = render(&ctx, &mut app, vec![]);
    let label = app.registry.get("gain").unwrap().name;
    click(
        &ctx,
        &mut app,
        at(&output, label),
        egui::PointerButton::Primary,
    );
    let node = app.project.nodes.iter_mut().find(|n| n.id == 91).unwrap();
    assert_eq!(node.effects.len(), 1);
    let effect = node.effects[0].id;
    let asset = node.recording.slots[0].asset.clone();
    node.effects[0]
        .keybinds
        .insert(EffectAction::Toggle, "Ctrl+Alt+F8".into());
    app.handle_shortcut(ShortcutAction::Effect {
        id: 91,
        effect,
        action: EffectAction::Toggle,
    });
    assert!(
        !app.project
            .nodes
            .iter()
            .find(|n| n.id == 91)
            .unwrap()
            .effects[0]
            .enabled
    );
    app.handle_shortcut(ShortcutAction::Node {
        id: 91,
        action: NodeAction::Disable,
    });
    assert!(
        app.project
            .nodes
            .iter()
            .find(|n| n.id == 91)
            .unwrap()
            .effects_disabled
    );
    app.open_node_keybinds(91);
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output)
        .iter()
        .any(|(label, _)| label.contains("Gain") && label.contains("Toggle effect")));
    app.open_recording_slot_keybinds(91, 9101);
    let output = render(&ctx, &mut app, vec![]);
    assert!(!labels(&output)
        .iter()
        .any(|(label, _)| label.contains("Toggle effect")));
    app.project
        .nodes
        .iter_mut()
        .find(|n| n.id == 91)
        .unwrap()
        .effects
        .clear();
    let action = ShortcutAction::Effect {
        id: 91,
        effect,
        action: EffectAction::Enable,
    };
    assert!(!action.exists(&app.project));
    app.handle_shortcut(action);
    let node = app.project.nodes.iter().find(|n| n.id == 91).unwrap();
    assert!(node.effects.is_empty());
    assert_eq!(node.recording.slots[0].asset, asset);
}
#[test]
fn automatic_voice_correction_is_in_voice_effect_category() {
    assert!(piratewave_core::voice_effects::is_voice_effect(
        "automatic_voice_correction"
    ));
    let mut app = app();
    let ctx = egui::Context::default();
    app.view = View::Effects;
    app.choose_effect = true;
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output)
        .iter()
        .any(|(s, _)| s == "Automatic Voice Correction"));
}

#[test]
fn recording_slots_have_scoped_controls_and_deleting_a_slot_removes_actions() {
    use piratewave_core::recording::SlotAction;
    let mut app = app();
    app.recording_smoke_fixture(true);
    let ctx = egui::Context::default();
    app.view = View::Recording(91);
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    for text in [
        "Start recording",
        "Stop recording",
        "Capture recording",
        "Play preview",
        "Save WAV / MP3",
        "+ Add recording slot",
    ] {
        assert!(labels(&output).iter().any(|(s, _)| s == text), "{text}");
    }
    click(
        &ctx,
        &mut app,
        at(&output, "+ Add recording slot"),
        egui::PointerButton::Primary,
    );
    assert_eq!(
        app.project
            .nodes
            .iter()
            .find(|n| n.id == 91)
            .unwrap()
            .recording
            .slots
            .len(),
        2
    );
    let action = ShortcutAction::RecordingSlot {
        id: 91,
        slot: 9101,
        action: SlotAction::Preview,
    };
    assert!(action.exists(&app.project));
    app.open_recording_slot_keybinds(91, 9101);
    let output = render(&ctx, &mut app, vec![]);
    for action in SlotAction::ALL {
        assert!(labels(&output)
            .iter()
            .any(|(s, _)| s.contains(action.label())));
    }
    app.project
        .nodes
        .iter_mut()
        .find(|n| n.id == 91)
        .unwrap()
        .recording
        .slots
        .retain(|s| s.id != 9101);
    assert!(!action.exists(&app.project));
    app.handle_shortcut(action);
}
#[test]
fn deleting_recording_content_preserves_slot_and_keybinds() {
    use piratewave_core::recording::SlotAction;
    let mut app = app();
    app.recording_smoke_fixture(true);
    app.project
        .nodes
        .iter_mut()
        .find(|n| n.id == 91)
        .unwrap()
        .recording
        .slots[0]
        .keybinds
        .insert(SlotAction::Preview, "Ctrl+Shift+F8".into());
    app.recording_slot_action(91, 9101, SlotAction::Delete);
    let audio =
        crate::recording_io::Audio::from_frames(vec![[0.2; 2]; 128], "late-capture.wav".into());
    app.recording_ui
        .service
        .events
        .try_send(crate::recording_io::Event::Audio {
            node: 91,
            slot: 9101,
            token: 0,
            audio,
            captured: true,
        })
        .unwrap_or_else(|_| panic!("Event queue full"));
    app.poll_recordings();
    let slot = &app
        .project
        .nodes
        .iter()
        .find(|n| n.id == 91)
        .unwrap()
        .recording
        .slots[0];
    assert!(slot.asset.is_none());
    assert_eq!(slot.keybinds.len(), 1);
    assert_eq!(slot.crop_start, 0.);
    assert_eq!(slot.crop_end, 0.);
}

#[test]
fn clearing_one_recording_slots_keys_preserves_node_and_other_slots() {
    use piratewave_core::recording::{RecordSlot, SlotAction};
    let mut app = app();
    app.recording_smoke_fixture(true);
    let node = app.project.nodes.iter_mut().find(|n| n.id == 91).unwrap();
    node.keybinds
        .insert(NodeAction::StartRecording, "Ctrl+Shift+F11".into());
    node.recording.slots[0]
        .keybinds
        .insert(SlotAction::Preview, "Ctrl+Shift+F9".into());
    let mut other = RecordSlot::new("Other");
    other
        .keybinds
        .insert(SlotAction::Capture, "Ctrl+Shift+F10".into());
    let other_id = other.id;
    node.recording.slots.push(other);
    app.open_recording_slot_keybinds(91, 9101);
    let ctx = egui::Context::default();
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Clear all"),
        egui::PointerButton::Primary,
    );
    assert_eq!(
        app.node_binding_draft[&NodeAction::StartRecording],
        "Ctrl+Shift+F11"
    );
    assert!(app
        .node_extra_binding_draft
        .contains_key(&ShortcutAction::RecordingSlot {
            id: 91,
            slot: other_id,
            action: SlotAction::Capture
        }));
    assert!(!app
        .node_extra_binding_draft
        .contains_key(&ShortcutAction::RecordingSlot {
            id: 91,
            slot: 9101,
            action: SlotAction::Preview
        }));
}

#[test]
fn armed_recording_node_can_restart_stopped_audio_from_its_start_button() {
    let mut app = app();
    app.recording_smoke_fixture(false);
    app.project.nodes.retain(|n| n.id == 91);
    app.project.connections.clear();
    app.project.nodes[0].recording.enabled = true;
    app.view = View::Recording(91);
    app.status.running = false;
    app.dirty = false;
    let ctx = egui::Context::default();
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Start recording"),
        egui::PointerButton::Primary,
    );
    assert!(
        app.dirty,
        "Start should remain usable when an armed workspace is opened with stopped audio"
    );
}

#[test]
fn recorder_stop_button_and_shortcut_stop_all_slots_for_only_that_node() {
    let mut app = app();
    app.app_settings.ui.utilities_expanded = true;
    app.recording_smoke_fixture(true);
    let (engine, rx) = Engine::test_channel();
    app.engine = engine;
    let ctx = egui::Context::default();
    for view in [View::Mixer, View::Recording(91)] {
        app.view = view;
        let _ = render(&ctx, &mut app, vec![]);
        let output = render(&ctx, &mut app, vec![]);
        click(
            &ctx,
            &mut app,
            at(&output, "Stop playing recording"),
            egui::PointerButton::Primary,
        );
        assert!(matches!(
            rx.try_recv().unwrap(),
            Command::StopRecordingPlayback {
                node: 91,
                slot: None
            }
        ));
        assert!(rx.try_recv().is_err());
    }
    app.handle_shortcut(ShortcutAction::Node {
        id: 91,
        action: NodeAction::StopPlayingRecording,
    });
    assert!(matches!(
        rx.try_recv().unwrap(),
        Command::StopRecordingPlayback {
            node: 91,
            slot: None
        }
    ));
    app.open_node_keybinds(91);
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output)
        .iter()
        .any(|(s, _)| s.contains("Stop playing recording")));
    app.project.nodes.retain(|n| n.id != 91);
    app.handle_shortcut(ShortcutAction::Node {
        id: 91,
        action: NodeAction::StopPlayingRecording,
    });
    assert!(rx.try_recv().is_err());
}

#[test]
fn recording_selection_reset_and_both_waveform_bars_stop_the_slots_outputs() {
    let mut app = app();
    app.recording_smoke_fixture(true);
    let (engine, rx) = Engine::test_channel();
    app.engine = engine;
    app.view = View::Recording(91);
    let ctx = egui::Context::default();
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Reset selection"),
        egui::PointerButton::Primary,
    );
    assert!(matches!(
        rx.try_recv().unwrap(),
        Command::StopRecordingPlayback {
            node: 91,
            slot: Some(9101)
        }
    ));
    let slot = &app
        .project
        .nodes
        .iter()
        .find(|n| n.id == 91)
        .unwrap()
        .recording
        .slots[0];
    assert_eq!((slot.crop_start, slot.crop_end), (0., 2.));
    // Locate the real waveform background in the rendered paint output.
    let output = render(&ctx, &mut app, vec![]);
    let rect = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Rect(r)
                if r.fill == Color32::from_rgb(12, 14, 19) && r.rect.height() == 96. =>
            {
                Some(r.rect)
            }
            _ => None,
        })
        .expect("waveform background");
    for (from, to) in [(0.01, 0.2), (0.99, 0.8)] {
        let start = Pos2::new(rect.left() + rect.width() * from, rect.center().y);
        let end = Pos2::new(rect.left() + rect.width() * to, rect.center().y);
        let _ = render(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        let _ = render(&ctx, &mut app, vec![egui::Event::PointerMoved(end)]);
        let _ = render(
            &ctx,
            &mut app,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        let commands: Vec<_> = rx.try_iter().collect();
        assert!(!commands.is_empty());
        assert!(commands.iter().all(|cmd| matches!(
            cmd,
            Command::StopRecordingPlayback {
                node: 91,
                slot: Some(9101)
            }
        )));
    }
    let slot = &app
        .project
        .nodes
        .iter()
        .find(|n| n.id == 91)
        .unwrap()
        .recording
        .slots[0];
    assert!((slot.crop_start - 0.4).abs() < 0.01);
    assert!((slot.crop_end - 1.6).abs() < 0.01);
}

#[test]
fn ducking_controls_keybinds_and_deleted_node_actions() {
    let mut app = app();
    app.new_kind = 11;
    app.new_name = "Priority mixer".into();
    app.add_node = true;
    let ctx = egui::Context::default();
    ctx.style_mut(|s| s.animation_time = 0.);
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Create node"),
        egui::PointerButton::Primary,
    );
    let id = app
        .project
        .nodes
        .iter()
        .find(|n| matches!(n.kind, NodeKind::AutoDucking))
        .unwrap()
        .id;
    assert!(matches!(app.view, View::Ducking(_)));
    let _ = render(&ctx, &mut app, vec![]);
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    assert!(!labels(&output)
        .iter()
        .any(|(s, _)| s == "Speech sensitivity"));
    click(
        &ctx,
        &mut app,
        at(&output, "Bypass ducking"),
        egui::PointerButton::Primary,
    );
    assert!(
        app.project
            .nodes
            .iter()
            .find(|n| n.id == id)
            .unwrap()
            .effects_disabled
    );
    app.project
        .nodes
        .iter_mut()
        .find(|n| n.id == id)
        .unwrap()
        .ducking
        .detection = piratewave_core::ducking::Detection::Speech;
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output)
        .iter()
        .any(|(s, _)| s == "Speech sensitivity"));
    click(
        &ctx,
        &mut app,
        at(&output, "Ducking keybinds"),
        egui::PointerButton::Primary,
    );
    let output = render(&ctx, &mut app, vec![]);
    for a in NodeAction::DUCKING {
        assert!(labels(&output).iter().any(|(s, _)| s == a.label()));
        app.handle_shortcut(ShortcutAction::Node { id, action: a });
    }
    app.view = View::Ducking(id);
    app.remove_audio_node(id);
    assert!(matches!(app.view, View::Mixer));
    let stale = ShortcutAction::Node {
        id,
        action: NodeAction::ToggleDucking,
    };
    assert!(!stale.exists(&app.project));
    app.handle_shortcut(stale);
}

#[test]
fn routing_connects_both_ducking_inputs_and_disconnects_one_wire() {
    use piratewave_core::project::InputPort;
    let mut app = app();
    app.project.nodes = vec![
        Node::new(1, "Trigger", NodeKind::Soundboard, [10., 10.]),
        Node::new(2, "Duck", NodeKind::AutoDucking, [300., 50.]),
        Node::new(3, "Bus", NodeKind::Processor, [600., 100.]),
    ];
    app.project.connections.clear();
    app.view = View::Routing;
    let ctx = egui::Context::default();
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    let rects: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.fill == Color32::from_rgb(32, 29, 40) => Some(r.rect),
            _ => None,
        })
        .collect();
    let source = *rects
        .iter()
        .min_by(|a, b| a.min.x.total_cmp(&b.min.x))
        .unwrap();
    let duck = *rects.iter().find(|r| r.height() == 138.).unwrap();
    for input in [InputPort::Priority, InputPort::Ducked] {
        click(
            &ctx,
            &mut app,
            source.min + Vec2::new(180., 47.),
            egui::PointerButton::Primary,
        );
        click(
            &ctx,
            &mut app,
            duck.min + Vec2::new(0., input.y(&NodeKind::AutoDucking)),
            egui::PointerButton::Primary,
        );
        assert!(app
            .project
            .connections
            .iter()
            .any(|c| c.from == 1 && c.to == 2 && c.input == input));
    }
    app.selected_edge = Some(0);
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Disconnect selected"),
        egui::PointerButton::Primary,
    );
    assert_eq!(app.project.connections.len(), 1);
    assert_eq!(app.project.connections[0].input, InputPort::Ducked);
}
