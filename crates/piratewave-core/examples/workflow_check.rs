//! Allocation/performance check for shared recording histories in the live graph.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
struct MeasuredAllocator;
static MEASURE: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if MEASURE.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if MEASURE.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        System.realloc(ptr, layout, size)
    }
}
#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;
fn main() {
    use piratewave_core::{
        effects::{EffectRegistry, EffectSpec},
        graph::Graph,
        project::{Node, NodeKind, OutputPort, Project},
        recording::Recorder,
        workflows::{CapturePoint, CaptureStage, Control, Scene, SessionConfig, Track},
        BLOCK_SIZE,
    };
    let registry = EffectRegistry::default();
    for factory in registry.factories() {
        let mut p = Project::default();
        p.connections.clear();
        p.nodes = vec![Node::new(1, "Source", NodeKind::Soundboard, [0.; 2])];
        p.nodes[0].effects.push(EffectSpec::new(factory));
        let mut graph = Graph::new(&p, &registry).unwrap();
        let mut scene = Scene::capture(&p, &registry, "Sweep".into());
        scene.duration_ms = 100;
        for setting in &mut scene.settings {
            if let Control::Parameter {
                parameter, value, ..
            } = &mut setting.value
            {
                let descriptor = factory
                    .parameters
                    .iter()
                    .find(|p| p.id == parameter)
                    .unwrap();
                *value = descriptor.min + (descriptor.max - descriptor.min) * 0.6;
            }
        }
        graph.recall_scene(&scene, &p, &registry).unwrap();
        ALLOCATIONS.store(0, Ordering::Relaxed);
        MEASURE.store(true, Ordering::Relaxed);
        for _ in 0..100 {
            graph.process(&[(1, [[0.1; 2]; BLOCK_SIZE])]);
        }
        MEASURE.store(false, Ordering::Relaxed);
        assert_eq!(
            ALLOCATIONS.load(Ordering::Relaxed),
            0,
            "{} scene allocated",
            factory.id
        );
        println!("{}: allocation-free scene transition", factory.id);
    }
    {
        let p = Project {
            nodes: vec![Node::new(1, "Ducking", NodeKind::AutoDucking, [0.; 2])],
            connections: vec![],
            ..Project::default()
        };
        let mut graph = Graph::new(&p, &registry).unwrap();
        let mut scene = Scene::capture(&p, &registry, "Speech and lookahead".into());
        scene.duration_ms = 0;
        for setting in &mut scene.settings {
            if let Control::DuckingParameter { parameter, value } = &mut setting.value {
                match parameter.as_str() {
                    "detection" | "lookahead_enabled" => *value = 1.,
                    "lookahead" => *value = 50.,
                    _ => {}
                }
            }
        }
        graph.recall_scene(&scene, &p, &registry).unwrap();
        ALLOCATIONS.store(0, Ordering::Relaxed);
        MEASURE.store(true, Ordering::Relaxed);
        for _ in 0..100 {
            graph.process(&[]);
        }
        MEASURE.store(false, Ordering::Relaxed);
        assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), 0);
        println!("Auto Ducking detection/lookahead transition: zero render allocations");
    }
    let mut p = Project {
        nodes: (1..=16)
            .map(|id| Node::new(id, &format!("Source {id}"), NodeKind::Soundboard, [0.; 2]))
            .collect(),
        connections: vec![],
        ..Project::default()
    };
    p.workflows.sessions.push(SessionConfig {
        id: 100,
        name: "Stress".into(),
        history_seconds: 1.,
        tracks: (0..32)
            .map(|i| Track {
                id: 200 + i,
                name: format!("Track {i}"),
                armed: true,
                point: CapturePoint {
                    node: 1 + i / 2,
                    port: OutputPort::Passthrough,
                    stage: if i % 2 == 0 {
                        CaptureStage::Raw
                    } else {
                        CaptureStage::Processed
                    },
                },
            })
            .collect(),
        take_ids: vec![],
        keybinds: Default::default(),
    });
    let inputs: Vec<_> = p
        .nodes
        .iter()
        .map(|n| (n.id, [[0.1; 2]; BLOCK_SIZE]))
        .collect();
    let mut graph = Graph::new(&p, &registry).unwrap();
    let mut recorder = Recorder::default();
    recorder.arm_session(&p, 100, true).unwrap();
    graph.workflow_taps = true;
    for n in &mut graph.nodes {
        n.capture_raw = true;
    }
    let mut scene = Scene::capture(&p, &registry, "Long transition".into());
    scene.duration_ms = 10000;
    for s in &mut scene.settings {
        if let Control::Gain { value } = &mut s.value {
            *value = -12.;
        }
    }
    graph.recall_scene(&scene, &p, &registry).unwrap();
    ALLOCATIONS.store(0, Ordering::Relaxed);
    MEASURE.store(true, Ordering::Relaxed);
    let start = std::time::Instant::now();
    for _ in 0..20000 {
        graph.process_workflows(
            &inputs,
            |_, _| false,
            |_, _, _, _| {},
            |id, f| recorder.tap_stage(id, OutputPort::Passthrough, CaptureStage::Raw, None, f),
            |id, port, target, f| recorder.tap(id, port, target, f),
        );
        recorder.finish_block();
    }
    let elapsed = start.elapsed();
    MEASURE.store(false, Ordering::Relaxed);
    assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), 0);
    println!(
        "32 raw/processed tracks and scene: {:.4}% realtime budget; zero render allocations",
        elapsed.as_secs_f64() / (20000. * 128. / 48000.) * 100.
    );
}
