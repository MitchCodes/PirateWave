use super::*;
use crate::{
    effects::EffectRegistry,
    graph::Graph,
    project::{InputPort, Node, NodeKind, OutputPort, Project},
    BLOCK_SIZE,
};
fn fixture() -> Project {
    let mut p = Project {
        nodes: vec![
            Node::new(1, "Priority", NodeKind::Soundboard, [0.; 2]),
            Node::new(2, "Background", NodeKind::Soundboard, [0.; 2]),
            Node::new(3, "Duck", NodeKind::AutoDucking, [0.; 2]),
        ],
        connections: vec![],
        ..Project::default()
    };
    p.nodes[2].ducking.attack = 1.;
    for id in 4..7 {
        p.nodes
            .push(Node::new(id, "Output", NodeKind::Processor, [0.; 2]));
    }
    p.connect_ports(1, 3, OutputPort::Passthrough, InputPort::Priority)
        .unwrap();
    p.connect_ports(2, 3, OutputPort::Passthrough, InputPort::Ducked)
        .unwrap();
    for (id, port) in [
        (4, OutputPort::Passthrough),
        (5, OutputPort::Priority),
        (6, OutputPort::Ducked),
    ] {
        p.connect_port(3, id, port).unwrap();
    }
    p
}
fn block(
    d: &mut AutoDucking,
    p: Frame,
    b: Frame,
    bypass: bool,
) -> (
    [Frame; BLOCK_SIZE],
    [Frame; BLOCK_SIZE],
    [Frame; BLOCK_SIZE],
) {
    let mut priority = [p; BLOCK_SIZE];
    let mut ducked = [b; BLOCK_SIZE];
    let mut mixed = [[0.; 2]; BLOCK_SIZE];
    d.process(&mut priority, &mut ducked, &mut mixed, bypass);
    (priority, ducked, mixed)
}
#[test]
fn level_envelope_stereo_linking_bypass_silence_and_live_amount() {
    let mut config = DuckingConfig {
        attack: 1.,
        hold: 50.,
        release: 10.,
        ..Default::default()
    };
    let mut d = AutoDucking::new(&config);
    let (p, b, m) = block(&mut d, [0.; 2], [0.4, -0.2], false);
    assert_eq!(p, [[0.; 2]; BLOCK_SIZE]);
    assert_eq!(b, m);
    assert_eq!(b, [[0.4, -0.2]; BLOCK_SIZE]);
    for _ in 0..100 {
        block(&mut d, [0.2, -0.2], [0.4, -0.2], false);
    }
    assert!(d.status().active);
    assert!((d.status().reduction_db + 18.).abs() < 0.001);
    let (p, b, m) = block(&mut d, [0.2, -0.2], [0.4, -0.2], false);
    for i in 0..BLOCK_SIZE {
        assert_eq!(p[i], [0.2, -0.2]);
        assert!((b[i][0] - 0.4 * db_to_gain(-18.)).abs() < 1e-5);
        assert!((b[i][0] + 2. * b[i][1]).abs() < 1e-6);
        assert_eq!(m[i], [p[i][0] + b[i][0], p[i][1] + b[i][1]]);
    }
    // Hold bridges a short gap even after the rolling RMS window clears.
    for _ in 0..8 {
        block(&mut d, [0.; 2], [0.4, -0.2], false);
    }
    assert!(d.status().reduction_db < -17.9);
    for _ in 0..100 {
        block(&mut d, [0.; 2], [0.4, -0.2], false);
    }
    assert!(d.status().reduction_db.abs() < 0.001);
    config.amount = -6.;
    d.configure(&config);
    for _ in 0..100 {
        block(&mut d, [0.2; 2], [0.4, -0.2], false);
    }
    assert!((d.status().reduction_db + 6.).abs() < 0.001);
    for _ in 0..100 {
        block(&mut d, [0.2; 2], [0.4, -0.2], true);
    }
    assert!(d.status().active);
    assert!(d.status().reduction_db.abs() < 0.001);
}
#[test]
fn deterministic_detection_hysteresis_probability_and_level_guard() {
    let mut d = AutoDucking::new(&DuckingConfig {
        threshold: -20.,
        ..Default::default()
    });
    d.detect(0.11, 0.);
    assert!(d.status.active);
    d.detect(0.06, 0.);
    assert!(d.status.active);
    d.detect(0.04, 0.);
    assert!(!d.status.active);
    let c = DuckingConfig {
        detection: Detection::Speech,
        threshold: -20.,
        ..Default::default()
    };
    d.configure(&c);
    d.detect(0.2, 0.59);
    assert!(!d.status.active);
    d.detect(0.2, 0.61);
    assert!(d.status.active);
    d.detect(0.2, 0.46);
    assert!(d.status.active);
    d.detect(0.2, 0.44);
    assert!(!d.status.active);
    d.detect(0.001, 0.99);
    assert!(!d.status.active);
    d.status.fallback = true;
    d.detect(0.2, 0.);
    assert!(d.status.active);
    d.recover();
    assert!(!d.status.fallback);
}
#[test]
fn attack_release_are_smooth_and_follow_configured_time_constants() {
    let c = DuckingConfig {
        attack: 10.,
        hold: 0.,
        release: 100.,
        threshold: -80.,
        ..Default::default()
    };
    let mut d = AutoDucking::new(&c);
    let mut p = [[0.2; 2]; 480];
    let mut b = [[1.; 2]; 480];
    let mut m = [[0.; 2]; 480];
    d.process(&mut p, &mut b, &mut m, false);
    assert!((d.gain_db - (-18. * (1. - (-1_f32).exp()))).abs() < 0.03);
    assert!(b
        .windows(2)
        .all(|w| w[1][0] <= w[0][0] && (w[0][0] - w[1][0]).abs() < 0.01));
    let before = d.gain_db;
    let mut p = vec![[0.2; 2]; 4800];
    let mut b = vec![[1.; 2]; 4800];
    let mut m = vec![[0.; 2]; 4800];
    d.process(&mut p, &mut b, &mut m, true);
    assert!((d.gain_db - before * (-1_f32).exp()).abs() < 0.03);
}
#[test]
fn lookahead_alignment_wrap_and_live_reconfiguration() {
    for delay in [0., 10., 50.] {
        let c = DuckingConfig {
            lookahead_enabled: true,
            lookahead: delay,
            ..Default::default()
        };
        let mut d = AutoDucking::new(&c);
        let mut p = vec![[0.; 2]; 7000];
        let mut b = p.clone();
        let mut m = p.clone();
        p[20] = [0.2, -0.1];
        b[20] = [0.4, -0.2];
        p[5000] = [0.1; 2];
        b[5000] = [0.2; 2];
        d.process(&mut p, &mut b, &mut m, true);
        let n = (delay * 48.) as usize;
        assert_eq!(p[20 + n], [0.2, -0.1]);
        assert_eq!(b[20 + n], [0.4, -0.2]);
        assert_eq!(m[20 + n], [0.6, -0.3]);
        if 5000 + n < 7000 {
            assert_eq!(p[5000 + n], [0.1; 2]);
        }
        for i in 0..7000 {
            assert_eq!(m[i], [p[i][0] + b[i][0], p[i][1] + b[i][1]]);
        }
        d.configure(&DuckingConfig {
            lookahead_enabled: true,
            lookahead: 5.,
            ..Default::default()
        });
        let mut p = vec![[0.2; 2]; 700];
        let mut b = vec![[0.4; 2]; 700];
        let mut m = vec![[0.; 2]; 700];
        d.process(&mut p, &mut b, &mut m, true);
        assert!(p[..240].iter().all(|f| *f == [0.; 2]));
        assert_eq!(p[240], [0.; 2]);
        assert_eq!(p[600], [0.2; 2]);
    }
}
#[test]
fn graph_ports_mix_independently_and_preserve_state_across_rebuilds() {
    let mut p = fixture();
    p.nodes[2].ducking.lookahead_enabled = true;
    p.nodes[2].ducking.lookahead = 50.;
    let registry = EffectRegistry::default();
    let mut a = Graph::new(&p, &registry).unwrap();
    let mut b = Graph::new(&p, &registry).unwrap();
    let inputs = [
        (1, [[0.2, -0.2]; BLOCK_SIZE]),
        (2, [[0.4, -0.1]; BLOCK_SIZE]),
    ];
    for _ in 0..30 {
        a.process(&inputs);
        b.process(&inputs);
    }
    p.nodes
        .push(Node::new(99, "Unrelated", NodeKind::Processor, [0.; 2]));
    a.update(&p, &registry).unwrap();
    a.process(&inputs);
    b.process(&inputs);
    for id in 3..7 {
        let af = &a.nodes.iter().find(|n| n.config.id == id).unwrap().frames;
        let bf = &b.nodes.iter().find(|n| n.config.id == id).unwrap().frames;
        assert_eq!(af, bf);
    }
    let get = |id| a.nodes.iter().find(|n| n.config.id == id).unwrap().frames;
    assert_eq!(get(5), [[0.2, -0.2]; BLOCK_SIZE]);
    assert!(get(6)[0][0] < 0.06);
    assert_eq!(get(4), get(3));
    let priority = get(5);
    let ducked = get(6);
    let mixed = get(4);
    for i in 0..BLOCK_SIZE {
        assert_eq!(
            mixed[i],
            [priority[i][0] + ducked[i][0], priority[i][1] + ducked[i][1]]
        );
    }
    p.connect_ports(1, 3, OutputPort::Passthrough, InputPort::Ducked)
        .unwrap();
    assert!(p
        .connect_ports(1, 3, OutputPort::Passthrough, InputPort::Ducked)
        .is_err());
    a.update(&p, &registry).unwrap();
    a.process(&inputs);
    // New input mixing remains behind the configured 50 ms delay.
    for _ in 0..30 {
        a.process(&inputs);
    }
    assert!(
        (a.nodes.iter().find(|n| n.config.id == 6).unwrap().frames[0][0] - 0.6 * db_to_gain(-18.))
            .abs()
            < 1e-4
    );

    assert!(p.connect(1, 3).is_err());
    assert!(p
        .connect_ports(3, 4, OutputPort::Priority, InputPort::Priority)
        .is_err());
    p.disconnect_ports(1, 3, OutputPort::Passthrough, InputPort::Ducked);
    assert!(p
        .connections
        .iter()
        .any(|c| c.from == 1 && c.input == InputPort::Priority));
    assert!(p
        .connect_ports(4, 3, OutputPort::Passthrough, InputPort::Priority)
        .is_err());
}
#[test]
fn persistence_validation_and_legacy_defaults() {
    let p = fixture();
    let text = serde_json::to_string(&p).unwrap();
    let restored: Project = serde_json::from_str(&text).unwrap();
    crate::graph::validate(&restored).unwrap();
    assert_eq!(p.connections, restored.connections);
    assert_eq!(p.nodes[2], restored.nodes[2]);
    let old = r#"{"from":1,"to":2}"#;
    let c: crate::project::Connection = serde_json::from_str(old).unwrap();
    assert_eq!(c.input, InputPort::Main);
    assert_eq!(c.port, OutputPort::Passthrough);
    for p in PARAMETERS {
        let mut config = DuckingConfig::default();
        assert!(config.set(p.id, f32::NAN).is_err());
        assert!(config.set(p.id, p.max + 1.).is_err());
        assert!(config.set(p.id, p.default).is_ok());
    }
}
#[test]
fn recording_each_ducking_output_keeps_distinct_processed_histories() {
    use crate::recording::{RecordSlot, Recorder};
    let mut p = fixture();
    for (id, port) in [
        (7, OutputPort::Passthrough),
        (8, OutputPort::Priority),
        (9, OutputPort::Ducked),
    ] {
        let mut n = Node::new(id, "Record", NodeKind::Recording, [0.; 2]);
        n.recording.enabled = true;
        n.recording.slots.push(RecordSlot::new("Take"));
        p.nodes.push(n);
        p.connect_port(3, id, port).unwrap();
    }
    let mut recorder = Recorder::default();
    recorder.sync(&p);
    assert_eq!(recorder.histories.len(), 3);
    let mut graph = Graph::new(&p, &EffectRegistry::default()).unwrap();
    for _ in 0..200 {
        graph.process_recording(
            &[(1, [[0.2; 2]; BLOCK_SIZE]), (2, [[0.4; 2]; BLOCK_SIZE])],
            |_, _| true,
            |_, _, _, _| {},
            |id, port, target, frames| recorder.tap(id, port, target, frames),
        );
        recorder.finish_block();
    }
    let mixed = recorder.capture(7, 0.1).unwrap().materialize().unwrap();
    let priority = recorder.capture(8, 0.1).unwrap().materialize().unwrap();
    let ducked = recorder.capture(9, 0.1).unwrap().materialize().unwrap();
    assert!(priority.iter().all(|f| *f == [0.2; 2]));
    assert!(ducked
        .iter()
        .all(|f| (f[0] - 0.4 * db_to_gain(-18.)).abs() < 0.0001));
    for i in 0..mixed.len() {
        assert_eq!(
            mixed[i],
            [priority[i][0] + ducked[i][0], priority[i][1] + ducked[i][1]]
        );
    }
}

#[test]
fn switches_activate_every_port_route_for_one_selected_destination() {
    let mut p = fixture();
    p.nodes
        .push(Node::new(10, "Selector", NodeKind::Switch, [0.; 2]));
    p.disconnect_ports(1, 3, OutputPort::Passthrough, InputPort::Priority);
    p.disconnect_ports(2, 3, OutputPort::Passthrough, InputPort::Ducked);
    p.connect(1, 10).unwrap();
    p.connect_ports(10, 3, OutputPort::Passthrough, InputPort::Priority)
        .unwrap();
    p.connect_ports(10, 3, OutputPort::Passthrough, InputPort::Ducked)
        .unwrap();
    p.connect(10, 6).unwrap();
    p.select_switch_output(10, 3);
    p.nodes[2].effects_disabled = true;
    let r = EffectRegistry::default();
    let mut g = Graph::new(&p, &r).unwrap();
    g.process(&[(1, [[0.2; 2]; BLOCK_SIZE])]);
    assert_eq!(
        g.nodes.iter().find(|n| n.config.id == 3).unwrap().frames,
        [[0.4; 2]; BLOCK_SIZE]
    );
    p.select_switch_output(10, 6);
    g.update(&p, &r).unwrap();
    g.process(&[(1, [[0.2; 2]; BLOCK_SIZE])]);
    assert_eq!(
        g.nodes.iter().find(|n| n.config.id == 3).unwrap().frames,
        [[0.; 2]; BLOCK_SIZE]
    );
    assert_eq!(
        g.nodes.iter().find(|n| n.config.id == 6).unwrap().frames,
        [[0.2; 2]; BLOCK_SIZE]
    );
}

#[test]
fn absent_or_silent_inputs_skip_all_detector_and_neural_work() {
    for detection in [Detection::Level, Detection::Speech] {
        for connected in [[false, false], [true, false], [false, true], [true, true]] {
            for (priority, ducked) in [
                ([0.; 2], [0.; 2]),
                ([0.2, -0.1], [0.; 2]),
                ([0.; 2], [0.4, -0.2]),
            ] {
                let mut d = AutoDucking::new(&DuckingConfig {
                    detection,
                    ..Default::default()
                });
                let ep = if connected[0] { priority } else { [0.; 2] };
                let ed = if connected[1] { ducked } else { [0.; 2] };
                for _ in 0..20 {
                    let mut p = [priority; BLOCK_SIZE];
                    let mut b = [ducked; BLOCK_SIZE];
                    let mut m = [[0.; 2]; BLOCK_SIZE];
                    d.process_connected(&mut p, &mut b, &mut m, false, connected);
                    assert_eq!(p, [ep; BLOCK_SIZE]);
                    assert_eq!(b, [ed; BLOCK_SIZE]);
                    assert_eq!(m, [[ep[0] + ed[0], ep[1] + ed[1]]; BLOCK_SIZE]);
                    assert!(d.status().fast_path);
                    assert_eq!(d.status().reduction_db, 0.);
                    assert!(!d.status().active);
                }
                assert_eq!(d.detector_samples, 0);
                assert_eq!(d.inference_frames, 0);
                assert_eq!(d.delay_cursor, 0);
            }
        }
    }
}
#[test]
fn idle_transitions_preserve_release_then_resume_on_fresh_analysis_windows() {
    let config = DuckingConfig {
        detection: Detection::Speech,
        attack: 1.,
        hold: 0.,
        release: 10.,
        ..Default::default()
    };
    let mut d = AutoDucking::new(&config);
    for _ in 0..5 {
        block(&mut d, [0.2; 2], [0.4; 2], false);
    }
    assert!(d.inference_frames > 0);
    let samples = d.detector_samples;
    let inferences = d.inference_frames;
    // A silent background needs no detector, even with an active priority.
    let (p, b, m) = block(&mut d, [0.2; 2], [0.; 2], false);
    assert_eq!(p, m);
    assert_eq!(b, [[0.; 2]; BLOCK_SIZE]);
    assert_eq!(d.detector_samples, samples);
    assert_eq!(d.inference_frames, inferences);
    assert_eq!(d.speech_cursor, 0);
    assert_eq!(d.status().speech_probability, 0.);
    d.recover();
    assert!(!d.models_idle);
    block(&mut d, [0.2; 2], [0.4; 2], false);
    assert!(!d.status().fast_path);
    assert_eq!(d.speech_cursor, BLOCK_SIZE);
    assert_eq!(d.inference_frames, inferences);
    // No priority: release an existing attenuation without running detectors.
    d.gain_db = -18.;
    d.hold = 0;
    let (_, b, _) = block(&mut d, [0.; 2], [0.4; 2], false);
    assert!(b[BLOCK_SIZE - 1][0] > b[0][0]);
    assert_eq!(d.inference_frames, inferences);
    for _ in 0..100 {
        block(&mut d, [0.; 2], [0.4; 2], false);
    }
    let (_, b, m) = block(&mut d, [0.; 2], [0.4; 2], false);
    assert_eq!(b, [[0.4; 2]; BLOCK_SIZE]);
    assert_eq!(b, m);
    assert_eq!(d.gain_db, 0.);
    // Raw bypass must still advance Hold, rather than leaving stale activity
    // that could duck later when bypass/lookahead settings change.
    d.hold = 1000;
    d.gain_db = 0.;
    block(&mut d, [0.; 2], [0.4; 2], true);
    assert_eq!(d.hold, 1000 - BLOCK_SIZE);
    for _ in 0..20 {
        block(&mut d, [0.; 2], [0.4; 2], true);
    }
    assert_eq!(d.hold, 0);
    // Re-enabling inside a valid Hold resumes the envelope, not a raw copy.
    d.hold = 1000;
    let (_, b, _) = block(&mut d, [0.; 2], [0.4; 2], false);
    assert!(b[BLOCK_SIZE - 1][0] < 0.4);
    assert!(d.gain_db < 0.);
}
#[test]
fn idle_lookahead_drains_background_tail_and_keeps_priority_alignment() {
    let config = DuckingConfig {
        lookahead_enabled: true,
        lookahead: 10.,
        detection: Detection::Speech,
        ..Default::default()
    };
    let mut d = AutoDucking::new(&config);
    let mut p = vec![[0.; 2]; 640];
    let mut b = p.clone();
    let mut m = p.clone();
    p[0] = [0.2; 2];
    d.process_connected(&mut p, &mut b, &mut m, false, [true, false]);
    assert_eq!(p[480], [0.2; 2]);
    assert_eq!(p, m);
    assert_eq!(d.detector_samples, 0);
    assert_eq!(d.inference_frames, 0);
    let mut d = AutoDucking::new(&config);
    let mut p = [[0.2; 2]; BLOCK_SIZE];
    let mut b = [[0.; 2]; BLOCK_SIZE];
    b[BLOCK_SIZE - 1] = [0.4; 2];
    let mut m = [[0.; 2]; BLOCK_SIZE];
    d.process(&mut p, &mut b, &mut m, false);
    assert!(d.ducked_pending > 0);
    let mut tail = Vec::new();
    for _ in 0..5 {
        let (_, b, _) = block(&mut d, [0.2; 2], [0.; 2], false);
        tail.extend(b);
    }
    assert!(tail.iter().any(|f| f[0] != 0.));
    let count = d.inference_frames;
    for _ in 0..20 {
        block(&mut d, [0.2; 2], [0.; 2], false);
        assert!(d.status().fast_path);
    }
    assert_eq!(d.inference_frames, count);
}
#[test]
fn graph_disconnect_mute_and_reconnect_choose_the_fast_path() {
    let mut p = fixture();
    p.nodes[2].ducking.detection = Detection::Speech;
    let r = EffectRegistry::default();
    p.disconnect_ports(2, 3, OutputPort::Passthrough, InputPort::Ducked);
    let mut g = Graph::new(&p, &r).unwrap();
    let input = [(1, [[0.2; 2]; BLOCK_SIZE]), (2, [[0.4; 2]; BLOCK_SIZE])];
    g.process(&input);
    let n = g.nodes.iter().find(|n| n.config.id == 3).unwrap();
    assert!(n.ducking.as_ref().unwrap().status().fast_path);
    assert_eq!(n.frames, [[0.2; 2]; BLOCK_SIZE]);
    p.connect_ports(2, 3, OutputPort::Passthrough, InputPort::Ducked)
        .unwrap();
    g.update(&p, &r).unwrap();
    g.process(&input);
    assert!(
        !g.nodes
            .iter()
            .find(|n| n.config.id == 3)
            .unwrap()
            .ducking
            .as_ref()
            .unwrap()
            .status()
            .fast_path
    );
    p.nodes.iter_mut().find(|n| n.id == 2).unwrap().mute = true;
    g.update(&p, &r).unwrap();
    g.process(&input);
    g.process(&input);
    assert!(
        g.nodes
            .iter()
            .find(|n| n.config.id == 3)
            .unwrap()
            .ducking
            .as_ref()
            .unwrap()
            .status()
            .fast_path
    );
}
