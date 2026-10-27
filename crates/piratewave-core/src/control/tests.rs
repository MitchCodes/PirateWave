use super::*;
use crate::{effects::EffectSpec, plugins::*, recording::RecordSlot};
fn fixture() -> (Project, EffectRegistry) {
    let registry = EffectRegistry::default();
    let mut p = Project::default();
    for (id, kind) in [
        (6, NodeKind::Processor),
        (7, NodeKind::Recording),
        (8, NodeKind::Switch),
        (9, NodeKind::VstInstrument),
        (10, NodeKind::VstEffect),
        (11, NodeKind::VoiceChanger),
        (12, NodeKind::ApplicationCapture { target: None }),
        (13, NodeKind::AutoDucking),
    ] {
        let mut n = Node::new(id, &format!("Node {id}"), kind, [0.; 2]);
        if n.kind.is_processor() && !matches!(n.kind, NodeKind::VoiceChanger) {
            n.effects
                .push(EffectSpec::new(registry.get("vocoder").unwrap()));
        }
        if matches!(n.kind, NodeKind::Recording) {
            n.recording.slots.push(RecordSlot::new("Last / phrase 🎤"));
        }
        if n.kind.is_plugin() {
            n.plugin = Some(NodePlugin::new(PluginDescriptor {
                path: "test.dll".into(),
                class_id: "abc".into(),
                format: PluginFormat::Vst2,
                role: if matches!(n.kind, NodeKind::VstInstrument) {
                    PluginRole::Instrument
                } else {
                    PluginRole::Effect
                },
                name: "Test".into(),
                vendor: "Test".into(),
                parameters: vec![PluginParameter {
                    id: 42,
                    name: "Gain".into(),
                    unit: String::new(),
                    default: 0.5,
                    steps: 4,
                }],
            }));
        }
        p.nodes.push(n);
    }
    p.sounds.push(crate::project::Sound {
        id: 44,
        name: "Applause".into(),
        path: "applause.wav".into(),
        shortcut: String::new(),
        volume: 1.,
    });
    p.connect(8, 6).unwrap();
    (p, registry)
}
#[test]
fn catalog_resolves_every_button_without_keyboard_bindings() {
    let (p, r) = fixture();
    let entries = catalog(&p, &r, false);
    for e in entries.iter().filter(|e| !e.adjustment) {
        let a = resolve(&p, &e.selector).unwrap();
        assert!(a.exists(&p));
    }
    for category in [
        "application",
        "node",
        "effect",
        "recording_slot",
        "switch",
        "soundboard",
        "mixer",
        "effect_parameter",
        "vst_parameter",
        "ducking_parameter",
    ] {
        assert!(entries.iter().any(|e| e.selector.category == category));
    }
    assert!(entries
        .iter()
        .filter(|e| e.selector.category == "recording_slot")
        .all(|e| e.state == "Empty"));
}
#[test]
fn boolean_feedback_tracks_the_control_not_unrelated_node_flags() {
    let (mut p, r) = fixture();
    for enabled in [false, true] {
        for node in &mut p.nodes {
            node.mute = enabled;
            node.solo = !enabled;
            node.listen_enabled = !enabled;
            node.effects_disabled = enabled;
            node.recording.enabled = enabled;
            node.switch_output = None;
            for effect in &mut node.effects {
                effect.enabled = !enabled;
            }
        }
        let entries = catalog(&p, &r, enabled);
        for entry in &entries {
            let expected = match (
                entry.selector.category.as_str(),
                entry.selector.action.as_str(),
            ) {
                ("application", "start_audio" | "stop_audio" | "toggle_audio") => Some(enabled),
                ("node", "mute" | "unmute" | "toggle_mute") => Some(enabled),
                (
                    "node",
                    "solo_on" | "solo_off" | "toggle_solo" | "listen_on" | "listen_off"
                    | "toggle_listening" | "enable" | "disable" | "toggle_disabled"
                    | "enable_ducking" | "disable_ducking" | "toggle_ducking",
                ) => Some(!enabled),
                ("node", "start_recording" | "stop_recording") => Some(enabled),
                ("mixer", "volume") => Some(enabled),
                ("effect", _) => Some(!enabled),
                ("switch", _) => Some(false),
                _ => None,
            };
            assert_eq!(entry.active, expected, "{:?}", entry.selector);
            if entry.selector.action == "toggle_mute" {
                assert_eq!(entry.state, if enabled { "Muted" } else { "Unmuted" });
            }
        }
    }
    p.nodes
        .iter_mut()
        .find(|n| n.id == 8)
        .unwrap()
        .switch_output = Some(6);
    assert_eq!(
        catalog(&p, &r, false)
            .iter()
            .find(|e| e.selector.category == "switch")
            .unwrap()
            .active,
        Some(true)
    );
}

#[test]
fn legacy_controller_entries_default_to_unknown_boolean_state() {
    let (p, r) = fixture();
    let entry = catalog(&p, &r, false).remove(0);
    let mut json = serde_json::to_value(entry).unwrap();
    json.as_object_mut().unwrap().remove("active");
    assert_eq!(serde_json::from_value::<Entry>(json).unwrap().active, None);
}
#[test]
fn portable_selectors_follow_names_not_ids_and_reject_renames_duplicates_and_missing_routes() {
    let (mut p, r) = fixture();
    let entries = catalog(&p, &r, false);
    let selector = entries
        .iter()
        .find(|e| e.selector.category == "recording_slot")
        .unwrap()
        .selector
        .clone();
    let serialized = serde_json::to_string(&selector).unwrap();
    let selector: Selector = serde_json::from_str(&serialized).unwrap();
    p.nodes.iter_mut().find(|n| n.id == 7).unwrap().id = 700;
    assert!(matches!(
        resolve(&p, &selector).unwrap(),
        ShortcutAction::RecordingSlot { id: 700, .. }
    ));
    p.nodes
        .iter_mut()
        .find(|n| n.id == 700)
        .unwrap()
        .recording
        .slots
        .push(RecordSlot::new("Last / phrase 🎤"));
    assert_eq!(resolve(&p, &selector), Err("Ambiguous target".into()));
    p.nodes
        .iter_mut()
        .find(|n| n.id == 700)
        .unwrap()
        .recording
        .slots
        .pop();
    p.nodes.iter_mut().find(|n| n.id == 700).unwrap().name = "Renamed".into();
    assert_eq!(resolve(&p, &selector), Err("Missing target".into()));
    let selector = &entries
        .iter()
        .find(|e| e.selector.category == "switch")
        .unwrap()
        .selector;
    p.connections.clear();
    assert_eq!(resolve(&p, selector), Err("Missing target".into()));
}
#[test]
fn adjustments_clamp_reset_quantize_and_validate_plugin_identity() {
    let (mut p, r) = fixture();
    let entries = catalog(&p, &r, false);
    for e in entries.iter().filter(|e| e.adjustment) {
        adjust(&mut p, &r, &e.selector, 10000, 100., false).unwrap();
        let c = catalog(&p, &r, false);
        let current = entry(&c, &e.selector).unwrap();
        assert!((current.value - current.max).abs() < 0.00001);
        adjust(&mut p, &r, &e.selector, -10000, 100., false).unwrap();
        let c = catalog(&p, &r, false);
        let current = entry(&c, &e.selector).unwrap();
        assert!((current.value - current.min).abs() < 0.00001);
        adjust(&mut p, &r, &e.selector, 0, 1., true).unwrap();
        if e.selector.action != "volume" {
            let c = catalog(&p, &r, false);
            assert!((entry(&c, &e.selector).unwrap().value - e.default).abs() < 0.00001);
        }
        assert!(adjust(&mut p, &r, &e.selector, 1, f64::NAN, false).is_err());
    }
    let e = entries
        .iter()
        .find(|e| e.selector.category == "vst_parameter")
        .unwrap();
    adjust(&mut p, &r, &e.selector, 1, 0.5, false).unwrap();
    let c = catalog(&p, &r, false);
    assert_eq!(entry(&c, &e.selector).unwrap().value, 0.75);
    p.nodes
        .iter_mut()
        .find(|n| n.name == e.selector.node)
        .unwrap()
        .plugin
        .as_mut()
        .unwrap()
        .descriptor
        .class_id = "replacement".into();
    assert!(adjust(&mut p, &r, &e.selector, 1, 1., false).is_err());
}
#[test]
fn duplicate_nodes_effects_and_sounds_never_pick_an_occurrence() {
    let (mut p, r) = fixture();
    let entries = catalog(&p, &r, false);
    let e = entries
        .iter()
        .find(|e| e.selector.category == "effect_parameter")
        .unwrap();
    let n = p
        .nodes
        .iter_mut()
        .find(|n| n.name == e.selector.node)
        .unwrap();
    n.effects.push(n.effects[0].clone());
    assert_eq!(
        adjust(&mut p, &r, &e.selector, 1, 1., false),
        Err("Ambiguous target".into())
    );
    let e = entries
        .iter()
        .find(|e| e.selector.category == "soundboard")
        .unwrap();
    p.sounds.push(p.sounds[0].clone());
    assert_eq!(resolve(&p, &e.selector), Err("Ambiguous target".into()));
    let e = entries
        .iter()
        .find(|e| e.selector.category == "mixer")
        .unwrap();
    let n = node(&p, &e.selector).unwrap().clone();
    p.nodes.push(n);
    assert_eq!(
        adjust(&mut p, &r, &e.selector, 1, 1., false),
        Err("Ambiguous target".into())
    );
}

#[test]
fn ducking_parameters_buttons_and_multiport_switch_targets_are_portable() {
    let (mut p, r) = fixture();
    let entries = catalog(&p, &r, false);
    for parameter in crate::ducking::PARAMETERS {
        let e = entries
            .iter()
            .find(|e| {
                e.selector.category == "ducking_parameter" && e.selector.action == parameter.id
            })
            .unwrap();
        adjust(&mut p, &r, &e.selector, 10000, 1., false).unwrap();
        assert_eq!(
            p.nodes
                .iter()
                .find(|n| n.id == 13)
                .unwrap()
                .ducking
                .value(parameter.id),
            parameter.max
        );
        adjust(&mut p, &r, &e.selector, -10000, 1., false).unwrap();
        assert_eq!(
            p.nodes
                .iter()
                .find(|n| n.id == 13)
                .unwrap()
                .ducking
                .value(parameter.id),
            parameter.min
        );
        adjust(&mut p, &r, &e.selector, 1, 1., true).unwrap();
        assert_eq!(
            p.nodes
                .iter()
                .find(|n| n.id == 13)
                .unwrap()
                .ducking
                .value(parameter.id),
            parameter.default
        );
    }
    for a in NodeAction::DUCKING {
        let e = entries
            .iter()
            .find(|e| e.selector.node_kind == "auto_ducking" && e.selector.action == code(a))
            .unwrap();
        assert_eq!(
            resolve(&p, &e.selector).unwrap(),
            ShortcutAction::Node { id: 13, action: a }
        );
        a.apply(p.nodes.iter_mut().find(|n| n.id == 13).unwrap())
            .unwrap();
    }
    let selector = entries
        .iter()
        .find(|e| e.selector.category == "ducking_parameter")
        .unwrap()
        .selector
        .clone();
    p.nodes.iter_mut().find(|n| n.id == 13).unwrap().id = 99;
    adjust(&mut p, &r, &selector, 1, 1., false).unwrap();
    p.nodes
        .push(p.nodes.iter().find(|n| n.id == 99).unwrap().clone());
    assert!(adjust(&mut p, &r, &selector, 1, 1., false)
        .unwrap_err()
        .contains("Ambiguous"));
    p.nodes.pop();
    p.nodes.iter_mut().find(|n| n.id == 99).unwrap().name = "Renamed".into();
    assert!(adjust(&mut p, &r, &selector, 1, 1., false).is_err());
    p.connect_ports(
        8,
        99,
        crate::project::OutputPort::Passthrough,
        crate::project::InputPort::Priority,
    )
    .unwrap();
    p.connect_ports(
        8,
        99,
        crate::project::OutputPort::Passthrough,
        crate::project::InputPort::Ducked,
    )
    .unwrap();
    let list = catalog(&p, &r, false);
    assert_eq!(
        list.iter()
            .filter(|e| e.selector.category == "switch" && e.selector.target == "Renamed")
            .count(),
        1
    );
}
