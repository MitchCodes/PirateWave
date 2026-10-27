use super::tests::{app, at, click, labels, render};
use super::*;
use piratewave_core::shortcuts::{UiPreferences, UtilityFilter};

fn window(
    ctx: &egui::Context,
    app: &mut PirateWaveApp,
    size: Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ctx| {
            app.sidebar(ctx);
            egui::TopBottomPanel::bottom("layout-test-status")
                .exact_height(55.)
                .show(ctx, |ui| {
                    ui.small("Audio stopped");
                });
            egui::CentralPanel::default()
                .frame(egui::Frame::new().inner_margin(28.))
                .show(ctx, |ui| {
                    app.header(ui);
                    match app.view {
                        View::Mixer => app.mixer(ui),
                        View::Routing => app.routing(ui),
                        _ => {}
                    }
                });
        },
    )
}
#[test]
fn presentation_settings_legacy_defaults_roundtrip_and_validation() {
    let old: AppSettings = serde_json::from_str(r#"{"version":1}"#).unwrap();
    assert_eq!(old.ui, UiPreferences::default());
    let mut settings = old;
    settings.ui.utilities_expanded = true;
    settings.ui.utilities_filter = UtilityFilter::Recorders;
    settings.ui.utilities_height = 310.;
    let folder =
        std::env::temp_dir().join(format!("piratewave-layout-settings-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("settings.json");
    storage::save_settings(&path, &settings).unwrap();
    assert_eq!(settings, storage::load_settings_inner(&path).unwrap());
    settings.ui.utilities_height = -1.;
    storage::save_settings(&path, &settings).unwrap();
    assert!(storage::load_settings_inner(&path).is_err());
    std::fs::remove_dir_all(folder).unwrap();
}
#[test]
fn mixer_panes_remain_bounded_at_supported_sizes_and_scaling() {
    let mut app = app();
    for id in 20..35 {
        app.project.nodes.push(Node::new(
            id,
            &format!("Long input name {id} with extra descriptive text"),
            NodeKind::Soundboard,
            [0.; 2],
        ));
    }
    for id in 40..48 {
        app.project.nodes.push(Node::new(
            id,
            "Another output bus",
            NodeKind::Output { device: None },
            [0.; 2],
        ));
    }
    let before = serde_json::to_string(&app.project).unwrap();
    let (engine, rx) = Engine::test_channel();
    app.engine = engine;
    for scale in [1., 1.5, 2.] {
        for size in [[1000., 700.], [1380., 960.], [1600., 960.], [1920., 1080.]] {
            for expanded in [false, true] {
                let ctx = egui::Context::default();
                ctx.set_pixels_per_point(scale);
                app.app_settings.ui.utilities_expanded = expanded;
                for _ in 0..3 {
                    window(&ctx, &mut app, Vec2::from(size), vec![]);
                }
                let sources = app.layout.source_viewport;
                let buses = app.layout.bus_viewport;
                assert!(sources.is_positive() && buses.is_positive());
                assert!(sources.right() < buses.left());
                assert!(buses.right() <= size[0] - 27.);
                assert!(sources.bottom() < size[1] - 50.);
                if expanded {
                    assert!(app.layout.utility_viewport.top() > sources.bottom());
                    assert!(app.layout.utility_viewport.bottom() < size[1] - 50.);
                } else {
                    assert!(!app.layout.utility_viewport.is_positive());
                }
            }
        }
    }
    assert_eq!(before, serde_json::to_string(&app.project).unwrap());
    assert!(!app.dirty && !app.needs_apply && !app.needs_restart);
    assert!(rx.try_recv().is_err());
}
#[test]
fn utility_disclosure_filters_and_reveal_do_not_touch_audio() {
    let mut app = app();
    app.project
        .nodes
        .push(Node::new(90, "Route switch", NodeKind::Switch, [0.; 2]));
    app.project
        .nodes
        .push(Node::new(91, "Recorder", NodeKind::Recording, [0.; 2]));
    app.project
        .nodes
        .push(Node::new(92, "Ducker", NodeKind::AutoDucking, [0.; 2]));
    let before = serde_json::to_string(&app.project).unwrap();
    let (engine, rx) = Engine::test_channel();
    app.engine = engine;
    let ctx = egui::Context::default();
    ctx.style_mut(|s| s.animation_time = 0.);
    let _ = render(&ctx, &mut app, vec![]);
    let output = render(&ctx, &mut app, vec![]);
    assert!(!labels(&output)
        .iter()
        .any(|(s, _)| s == "Disable (passthrough)"));
    click(
        &ctx,
        &mut app,
        at(&output, "> Processors & utility nodes (4)"),
        egui::PointerButton::Primary,
    );
    assert!(app.app_settings.ui.utilities_expanded);
    let output = render(&ctx, &mut app, vec![]);
    click(
        &ctx,
        &mut app,
        at(&output, "Switches"),
        egui::PointerButton::Primary,
    );
    let output = render(&ctx, &mut app, vec![]);
    assert!(labels(&output).iter().any(|(s, _)| s == "Active output"));
    assert!(!labels(&output).iter().any(|(s, _)| s == "Bypass ducking"));
    app.reveal_utility(92);
    assert_eq!(app.app_settings.ui.utilities_filter, UtilityFilter::All);
    assert_eq!(app.layout.mixer_focus, Some(92));
    assert!(app.settings_dirty);
    assert_eq!(before, serde_json::to_string(&app.project).unwrap());
    assert!(!app.dirty && !app.needs_apply);
    assert!(rx.try_recv().is_err());
    assert!(UtilityFilter::All.includes(&NodeKind::Recording));
    assert!(UtilityFilter::Processors.includes(&NodeKind::VoiceChanger));
    assert!(!UtilityFilter::All.includes(&NodeKind::VstInstrument));
}
#[test]
fn routing_bounds_find_and_resize_reach_far_and_negative_nodes_without_editing() {
    let mut app = app();
    app.view = View::Routing;
    app.project.nodes[0].position = [8000., 6000.];
    app.project.nodes[1].position = [-900., -700.];
    let before = serde_json::to_string(&app.project).unwrap();
    let (engine, rx) = Engine::test_channel();
    app.engine = engine;
    let ctx = egui::Context::default();
    ctx.style_mut(|s| s.animation_time = 0.);
    for target in [1, 2] {
        app.layout.routing_focus = Some(target);
        for _ in 0..5 {
            window(&ctx, &mut app, Vec2::new(1600., 960.), vec![]);
        }
        assert!(
            app.layout.routing_focus.is_none(),
            "Failed to reveal {target}"
        );
        let origin = app.layout.routing_origin;
        let extent = app.layout.routing_extent;
        assert!(origin.x <= -932. && origin.y <= -732.);
        assert!(extent.x > 9000. && extent.y > 6800.);
        for _ in 0..3 {
            window(&ctx, &mut app, Vec2::new(1000., 700.), vec![]);
        }
        assert!(app.layout.routing_extent.x >= extent.x && app.layout.routing_extent.y >= extent.y);
        app.layout.routing_focus = Some(target);
        for _ in 0..5 {
            window(&ctx, &mut app, Vec2::new(1000., 700.), vec![]);
        }
        assert!(app.layout.routing_focus.is_none());
    }
    assert_eq!(before, serde_json::to_string(&app.project).unwrap());
    assert!(!app.dirty && !app.needs_apply);
    assert!(rx.try_recv().is_err());
}
#[test]
fn grouped_editors_render_without_filling_missing_parameters_or_sending_commands() {
    use piratewave_core::plugins::{NodePlugin, PluginParameter, PluginRole};
    let mut app = app();
    let mut effect = EffectSpec::new(app.registry.get("gain").unwrap());
    effect.parameters.clear();
    app.project.nodes[5].effects.push(effect);
    let mut vst = Node::new(90, "VST", NodeKind::VstEffect, [0.; 2]);
    let mut descriptor = plugins::fixture(PluginRole::Effect);
    descriptor.parameters.push(PluginParameter {
        id: 1,
        name: "Very long plugin parameter name for layout".repeat(3),
        unit: "dB".into(),
        default: 0.5,
        steps: 0,
    });
    vst.plugin = Some(NodePlugin::new(descriptor));
    app.project.nodes.push(vst);
    app.project
        .nodes
        .push(Node::new(91, "Voice", NodeKind::VoiceChanger, [0.; 2]));
    app.project.nodes.last_mut().unwrap().voice = None;
    app.project
        .nodes
        .push(Node::new(92, "Duck", NodeKind::AutoDucking, [0.; 2]));
    let before = serde_json::to_string(&app.project).unwrap();
    let (engine, rx) = Engine::test_channel();
    app.engine = engine;
    let ctx = egui::Context::default();
    for (view, id) in [
        (View::Effects, 6),
        (View::PluginNode(90), 90),
        (View::VoiceNode(91), 91),
        (View::Ducking(92), 92),
    ] {
        app.view = view;
        app.selected = id;
        for _ in 0..2 {
            render(&ctx, &mut app, vec![]);
        }
    }
    assert_eq!(before, serde_json::to_string(&app.project).unwrap());
    assert!(!app.dirty && !app.needs_apply);
    assert!(rx.try_recv().is_err());
}

fn window_click(ctx: &egui::Context, app: &mut PirateWaveApp, size: Vec2, pos: Pos2) {
    window(ctx, app, size, vec![egui::Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        window(
            ctx,
            app,
            size,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }],
        );
    }
}
#[test]
fn routing_find_popup_and_reset_view_preserve_saved_positions() {
    let mut app = app();
    app.view = View::Routing;
    app.project.nodes[0].name = "Far target".into();
    app.project.nodes[0].position = [9000., 7000.];
    let before = serde_json::to_string(&app.project).unwrap();
    let ctx = egui::Context::default();
    ctx.style_mut(|s| s.animation_time = 0.);
    let size = Vec2::new(1000., 700.);
    for _ in 0..2 {
        window(&ctx, &mut app, size, vec![]);
    }
    app.layout.find_query = "Far target".into();
    let output = window(&ctx, &mut app, size, vec![]);
    window_click(&ctx, &mut app, size, at(&output, "Find node…"));
    let output = window(&ctx, &mut app, size, vec![]);
    let target = labels(&output)
        .iter()
        .find(|(s, _)| s.starts_with("Far target ·"))
        .unwrap()
        .1;
    window_click(&ctx, &mut app, size, target);
    for _ in 0..4 {
        window(&ctx, &mut app, size, vec![]);
    }
    assert_eq!(app.selected, 1);
    assert!(app.layout.routing_focus.is_none());
    assert!(app.layout.routing_offset.x > 8000. && app.layout.routing_offset.y > 6000.);
    let output = window(&ctx, &mut app, size, vec![]);
    window_click(&ctx, &mut app, size, at(&output, "Reset view"));
    window(&ctx, &mut app, size, vec![]);
    assert_eq!(app.layout.routing_offset, Vec2::ZERO);
    assert_eq!(before, serde_json::to_string(&app.project).unwrap());
    assert!(!app.dirty && !app.needs_apply);
}
#[test]
fn routing_drag_expands_both_axes_and_auto_scrolls_without_audio_updates() {
    let mut app = app();
    app.project.nodes = vec![Node::new(1, "Drag me", NodeKind::Processor, [40., 40.])];
    app.project.connections.clear();
    app.view = View::Routing;
    let (engine, rx) = Engine::test_channel();
    app.engine = engine;
    let ctx = egui::Context::default();
    let size = Vec2::new(1000., 700.);
    for _ in 0..3 {
        window(&ctx, &mut app, size, vec![]);
    }
    let extent = app.layout.routing_extent;
    let viewport = app.layout.routing_viewport;
    let start =
        viewport.min - app.layout.routing_offset - app.layout.routing_origin + Vec2::new(110., 65.);
    window(&ctx, &mut app, size, vec![egui::Event::PointerMoved(start)]);
    window(
        &ctx,
        &mut app,
        size,
        vec![egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        }],
    );
    let end = viewport.max - Vec2::splat(8.);
    for _ in 0..100 {
        window(&ctx, &mut app, size, vec![egui::Event::PointerMoved(end)]);
    }
    window(
        &ctx,
        &mut app,
        size,
        vec![egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }],
    );
    assert!(
        app.project.nodes[0].position[0] > extent.x && app.project.nodes[0].position[1] > extent.y,
        "position {:?}, extent {:?}",
        app.project.nodes[0].position,
        extent
    );
    assert!(app.layout.routing_extent.x > extent.x && app.layout.routing_extent.y > extent.y);
    assert!(app.dirty);
    assert!(!app.needs_apply && !app.needs_restart);
    assert!(rx.try_recv().is_err());
}
