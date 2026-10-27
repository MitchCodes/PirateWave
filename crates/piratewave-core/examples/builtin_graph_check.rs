//! Longer built-in-only benchmark for host regression comparisons.
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

use piratewave_core::{
    ducking::Detection,
    effects::{EffectRegistry, EffectSpec},
    graph::Graph,
    project::{Node, NodeKind, Project},
    BLOCK_SIZE,
};
fn main() {
    let r = EffectRegistry::default();
    for mode in 0..6 {
        let mut p = Project {
            nodes: vec![
                Node::new(1, "A", NodeKind::Soundboard, [0.; 2]),
                Node::new(2, "B", NodeKind::Soundboard, [0.; 2]),
                Node::new(
                    3,
                    "DSP",
                    if matches!(mode, 2 | 3) {
                        NodeKind::AutoDucking
                    } else {
                        NodeKind::Processor
                    },
                    [0.; 2],
                ),
                Node::new(4, "Out", NodeKind::Output { device: None }, [0.; 2]),
            ],
            connections: vec![],
            ..Project::default()
        };
        if matches!(mode, 1 | 4) {
            for id in ["gain", "delay", "gate"] {
                p.nodes[2].effects.push(EffectSpec::new(r.get(id).unwrap()));
            }
        }
        if matches!(mode, 2 | 3) {
            p.nodes[2].ducking.detection = if mode == 3 {
                Detection::Speech
            } else {
                Detection::Level
            };
            p.connect_ports(
                1,
                3,
                Default::default(),
                piratewave_core::project::InputPort::Priority,
            )
            .unwrap();
            p.connect_ports(
                2,
                3,
                Default::default(),
                piratewave_core::project::InputPort::Ducked,
            )
            .unwrap();
        } else {
            p.connect(1, 3).unwrap();
            p.connect(2, 3).unwrap();
        }
        p.connect(3, 4).unwrap();
        let mut recorder = piratewave_core::recording::Recorder::default();
        if mode == 5 {
            p.nodes[2].kind = NodeKind::Recording;
            p.nodes[2].recording.enabled = true;
            p.nodes[2]
                .recording
                .slots
                .push(piratewave_core::recording::RecordSlot::new("Take"));
            recorder.sync(&p);
        }
        let mut g = Graph::new(&p, &r).unwrap();
        if mode == 4 {
            g.monitor = Some(Box::new(piratewave_core::graph::EffectMonitor::new(
                3,
                p.nodes[2].effects[1].id,
                1,
            )));
        }
        let inputs = [
            (1, [[0.1, 0.2]; BLOCK_SIZE]),
            (2, [[0.2, -0.1]; BLOCK_SIZE]),
        ];
        for _ in 0..100 {
            g.process(&inputs);
        }
        ALLOCATIONS.store(0, Ordering::Relaxed);
        MEASURE.store(true, Ordering::Relaxed);
        let start = std::time::Instant::now();
        for _ in 0..50000 {
            if mode == 5 {
                g.process_recording(
                    std::hint::black_box(&inputs),
                    |_, _| true,
                    |_, _, _, _| {},
                    |id, port, selected, frames| recorder.tap(id, port, selected, frames),
                );
                recorder.finish_block();
            } else {
                g.process(std::hint::black_box(&inputs));
            }
            std::hint::black_box(g.nodes.last().unwrap().frames[0]);
        }
        let elapsed = start.elapsed();
        MEASURE.store(false, Ordering::Relaxed);
        assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), 0);
        println!(
            "{}: {:.6}% realtime CPU budget; zero render allocations",
            [
                "routing",
                "rack",
                "ducking_level",
                "ducking_speech",
                "rack_live_tap",
                "recording_histories"
            ][mode],
            elapsed.as_secs_f64() / (50000. * 128. / 48000.) * 100.
        );
    }
}
