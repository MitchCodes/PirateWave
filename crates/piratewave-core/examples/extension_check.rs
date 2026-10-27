//! Host-side extension graph allocation and CPU check.
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
    extensions::{self, Instance, Provider, State},
    graph::Graph,
    project::{InputPort, Node, NodeKind, OutputPort, Project},
    Frame, BLOCK_SIZE,
};
use std::sync::Arc;
struct Factory;
struct Dsp;
impl Provider for Factory {
    fn create(&self, _: f32, _: &State) -> Result<Box<dyn Instance>, String> {
        Ok(Box::new(Dsp))
    }
}
impl Instance for Dsp {
    fn set(&mut self, _: u32, _: f32) -> bool {
        true
    }
    fn reset(&mut self) -> bool {
        true
    }
    fn process(&mut self, ins: &[[Frame; BLOCK_SIZE]], outs: &mut [[Frame; BLOCK_SIZE]]) -> bool {
        for i in 0..BLOCK_SIZE {
            outs[0][i] = [ins[0][i][0] + ins[1][i][0], ins[0][i][1] + ins[1][i][1]];
            outs[1][i] = ins[0][i];
            outs[2][i] = ins[1][i];
        }
        true
    }
}
fn main() {
    let mut m = piratewave_extension_sdk::parse_manifest(include_bytes!(
        "../../piratewave-example-extension/manifest.json"
    ))
    .unwrap();
    let d = m.types.remove(1);
    extensions::install(vec![(d.clone(), Arc::new(Factory))]).unwrap();
    let mut p = Project {
        nodes: vec![
            Node::new(1, "A", NodeKind::Soundboard, [0.; 2]),
            Node::new(2, "B", NodeKind::Soundboard, [0.; 2]),
            Node::new(
                3,
                "Mix",
                NodeKind::Extension {
                    definition: Box::new(d),
                },
                [0.; 2],
            ),
        ],
        connections: vec![],
        ..Project::default()
    };
    p.connect_ports(1, 3, OutputPort::Passthrough, InputPort::Extension(101))
        .unwrap();
    p.connect_ports(2, 3, OutputPort::Passthrough, InputPort::Extension(202))
        .unwrap();
    for (i, port) in [303, 404, 505].into_iter().enumerate() {
        let id = 4 + i as u64;
        p.nodes.push(Node::new(
            id,
            "Output",
            NodeKind::Output { device: None },
            [0.; 2],
        ));
        p.connect_ports(3, id, OutputPort::Extension(port), InputPort::Main)
            .unwrap();
    }
    let mut graph = Graph::new(&p, &piratewave_core::effects::EffectRegistry::default()).unwrap();
    let input = [
        (1, [[0.1, 0.2]; BLOCK_SIZE]),
        (2, [[0.2, -0.1]; BLOCK_SIZE]),
    ];
    for _ in 0..100 {
        graph.process(&input);
    }
    ALLOCATIONS.store(0, Ordering::Relaxed);
    MEASURE.store(true, Ordering::Relaxed);
    let start = std::time::Instant::now();
    for _ in 0..20000 {
        graph.process(&input);
    }
    let elapsed = start.elapsed();
    MEASURE.store(false, Ordering::Relaxed);
    assert_eq!(
        ALLOCATIONS.load(Ordering::Relaxed),
        0,
        "Extension graph allocated during rendering"
    );
    println!("PASS: zero render allocations with two-input/three-output extension routing; {:.3}% realtime CPU budget",elapsed.as_secs_f64()/(20000.*128./48000.)*100.);
}
