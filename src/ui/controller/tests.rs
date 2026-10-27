use super::*;
fn request(selector: control::Selector, op: &str, generation: u64) -> Request {
    Request {
        id: 1,
        op: op.into(),
        version: 1,
        client: String::new(),
        generation,
        selector: Some(selector),
        ticks: 1,
        sensitivity: 1.,
    }
}
#[test]
fn controller_invokes_shared_dispatcher_and_rejects_stale_projects() {
    let mut app = crate::ui::tests::app();
    app.recording_smoke_fixture(true);
    app.controller.generation = 77;
    app.status.running = true;
    app.status.recording_seconds.insert(91, 2.);
    app.project
        .nodes
        .iter_mut()
        .find(|n| n.id == 91)
        .unwrap()
        .recording
        .enabled = true;
    let (engine, rx) = Engine::test_channel();
    app.engine = engine;
    let entries = control::catalog(&app.project, &app.registry, true);
    for entry in entries
        .iter()
        .filter(|e| e.selector.category == "recording_slot")
    {
        let mut req = request(entry.selector.clone(), "execute", 77);
        req.generation = 76;
        assert!(app
            .controller_request(&req)
            .unwrap_err()
            .contains("Project changed"));
        assert!(rx.try_recv().is_err());
        req.generation = 77;
        app.controller_request(&req).unwrap();
        let commands: Vec<_> = rx.try_iter().collect();
        match entry.selector.action.as_str() {
            "capture" => assert!(commands.iter().any(|c| matches!(
                c,
                Command::CaptureRecording {
                    node: 91,
                    slot: 9101,
                    ..
                }
            ))),
            "delete" => assert!(commands.iter().any(|c| matches!(
                c,
                Command::StopRecordingPlayback {
                    node: 91,
                    slot: Some(9101)
                }
            ))),
            _ => assert!(commands.iter().any(|c| matches!(
                c,
                Command::PlayRecording {
                    node: 91,
                    slot: 9101,
                    ..
                }
            ))),
        }
    }
    let selector = entries
        .iter()
        .find(|e| e.selector.node == "Microphone" && e.selector.action == "volume")
        .unwrap()
        .selector
        .clone();
    app.controller_request(&request(selector.clone(), "adjust", 77))
        .unwrap();
    assert_eq!(app.project.nodes[0].gain_db, 1.);
    assert!(app.dirty);
    app.controller_request(&request(selector, "reset", 77))
        .unwrap();
    assert!(app.project.nodes[0].mute);
    let selector = entries
        .iter()
        .find(|e| e.selector.action == "stop_playing_recording")
        .unwrap()
        .selector
        .clone();
    app.controller_request(&request(selector, "execute", 77))
        .unwrap();
    assert!(matches!(
        rx.try_recv().unwrap(),
        Command::StopRecordingPlayback {
            node: 91,
            slot: None
        }
    ));
}
#[test]
fn controller_reports_invalid_and_empty_recording_operations() {
    let mut app = crate::ui::tests::app();
    app.recording_smoke_fixture(false);
    app.controller.generation = 5;
    let entries = control::catalog(&app.project, &app.registry, false);
    for e in entries
        .iter()
        .filter(|e| e.selector.category == "recording_slot" && e.selector.action != "delete")
    {
        assert!(app
            .controller_request(&request(e.selector.clone(), "execute", 5))
            .is_err());
    }
    let e = entries
        .iter()
        .find(|e| e.selector.category == "mixer")
        .unwrap();
    let mut req = request(e.selector.clone(), "adjust", 5);
    req.sensitivity = f64::NAN;
    assert!(app.controller_request(&req).is_err());
}

#[test]
fn v2_parameter_reset_restores_volume_without_changing_mute() {
    let mut app = crate::ui::tests::app();
    app.recording_smoke_fixture(true);
    app.controller.generation = 77;
    app.project.nodes[0].gain_db = -12.;
    let muted = app.project.nodes[0].mute;
    let selector = control::catalog(&app.project, &app.registry, false)
        .into_iter()
        .find(|e| e.selector.node == "Microphone" && e.selector.action == "volume")
        .unwrap()
        .selector;
    let mut req = request(selector, "reset", 77);
    req.version = 2;
    app.controller_request(&req).unwrap();
    assert_eq!(app.project.nodes[0].gain_db, 0.);
    assert_eq!(app.project.nodes[0].mute, muted);
}

#[test]
fn controller_ducking_changes_use_live_dispatch_and_reject_stale_generations() {
    let mut app = crate::ui::tests::app();
    app.project
        .nodes
        .push(Node::new(95, "Duck", NodeKind::AutoDucking, [0.; 2]));
    app.controller.generation = 9;
    let entries = control::catalog(&app.project, &app.registry, false);
    let toggle = entries
        .iter()
        .find(|e| e.selector.node_kind == "auto_ducking" && e.selector.action == "toggle_ducking")
        .unwrap();
    app.controller_request(&request(toggle.selector.clone(), "execute", 9))
        .unwrap();
    assert!(app.project.nodes.last().unwrap().effects_disabled);
    let amount = entries
        .iter()
        .find(|e| e.selector.category == "ducking_parameter" && e.selector.action == "amount")
        .unwrap();
    app.controller_request(&request(amount.selector.clone(), "adjust", 9))
        .unwrap();
    assert_eq!(app.project.nodes.last().unwrap().ducking.amount, -17.);
    assert!(app
        .controller_request(&request(amount.selector.clone(), "adjust", 8))
        .is_err());
    app.controller_request(&request(amount.selector.clone(), "reset", 9))
        .unwrap();
    assert_eq!(app.project.nodes.last().unwrap().ducking.amount, -18.);
}
