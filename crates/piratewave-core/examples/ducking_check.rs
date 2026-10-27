//! Allocation/performance check for both ducking detectors and delay modes.
use piratewave_core::effects::EffectRegistry;
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
        ducking::{Detection, DuckingConfig},
        graph::Graph,
        project::{InputPort, Node, NodeKind, OutputPort, Project},
        Frame, BLOCK_SIZE,
    };
    for mode in [Detection::Level, Detection::Speech] {
        for lookahead in [false, true] {
            for (name, connected, audible) in [
                ("both active", [true, true], [true, true]),
                ("priority only disconnected", [true, false], [true, false]),
                ("ducked only disconnected", [false, true], [false, true]),
                ("priority only silent", [true, true], [true, false]),
                ("ducked only silent", [true, true], [false, true]),
                ("neither connected", [false, false], [false, false]),
                ("both silent", [true, true], [false, false]),
                ("transitions", [true, true], [true, true]),
            ] {
                let mut p = Project {
                    nodes: vec![
                        Node::new(1, "Priority", NodeKind::Soundboard, [0.; 2]),
                        Node::new(2, "Ducked", NodeKind::Soundboard, [0.; 2]),
                        Node::new(3, "Duck", NodeKind::AutoDucking, [0.; 2]),
                    ],
                    connections: vec![],
                    ..Project::default()
                };
                p.nodes[2].ducking = DuckingConfig {
                    detection: mode,
                    lookahead_enabled: lookahead,
                    lookahead: 50.,
                    ..Default::default()
                };
                if connected[0] {
                    p.connect_ports(1, 3, OutputPort::Passthrough, InputPort::Priority)
                        .unwrap();
                }
                if connected[1] {
                    p.connect_ports(2, 3, OutputPort::Passthrough, InputPort::Ducked)
                        .unwrap();
                }
                let mut g = Graph::new(&p, &EffectRegistry::default()).unwrap();
                let signal: [Frame; BLOCK_SIZE] = std::array::from_fn(|i| {
                    let x = (i as f32 * 0.1).sin() * 0.2;
                    [x, -x]
                });
                let inputs = [
                    (
                        1,
                        if audible[0] {
                            signal
                        } else {
                            [[0.; 2]; BLOCK_SIZE]
                        },
                    ),
                    (
                        2,
                        if audible[1] {
                            signal
                        } else {
                            [[0.; 2]; BLOCK_SIZE]
                        },
                    ),
                ];
                ALLOCATIONS.store(0, Ordering::Relaxed);
                MEASURE.store(true, Ordering::Relaxed);
                let start = std::time::Instant::now();
                for i in 0..375 {
                    if name == "transitions" {
                        let phase = i % 150;
                        g.process(&[
                            (
                                1,
                                if phase < 100 {
                                    signal
                                } else {
                                    [[0.; 2]; BLOCK_SIZE]
                                },
                            ),
                            (
                                2,
                                if !(50..100).contains(&phase) {
                                    signal
                                } else {
                                    [[0.; 2]; BLOCK_SIZE]
                                },
                            ),
                        ]);
                    } else {
                        g.process(&inputs);
                    }
                }
                let elapsed = start.elapsed();
                MEASURE.store(false, Ordering::Relaxed);
                assert_eq!(
                    ALLOCATIONS.load(Ordering::Relaxed),
                    0,
                    "Ducking render allocated"
                );
                assert!(g
                    .nodes
                    .iter()
                    .flat_map(|n| n.frames)
                    .flatten()
                    .all(f32::is_finite));
                if name != "both active" && name != "transitions" {
                    assert!(g.nodes[2].ducking.as_ref().unwrap().status().fast_path);
                }
                println!("PASS: {name}, {mode:?}, lookahead {lookahead}: zero graph rendering allocations; {:.3}% real-time CPU budget",elapsed.as_secs_f64()*100.);
            }
        }
    }
}
