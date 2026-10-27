//! Allocation/performance check for shared recording histories in the live graph.
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
        project::{Node, NodeKind, Project},
        recording::{RecordSlot, Recorder},
        Frame, BLOCK_SIZE,
    };
    let mut project = Project {
        nodes: vec![Node::new(1, "Mic", NodeKind::Soundboard, [0.; 2])],
        connections: vec![],
        ..Project::default()
    };
    for id in 2..6 {
        let mut node = Node::new(id, "Recording", NodeKind::Recording, [0.; 2]);
        node.recording.enabled = true;
        for slot in 0..5 {
            let mut spec = RecordSlot::new(&format!("Slot {slot}"));
            spec.seconds = 60.;
            node.recording.slots.push(spec);
        }
        project.nodes.push(node);
        project.connect(1, id).unwrap();
    }
    let registry = EffectRegistry::default();
    for node in project.nodes.iter_mut().skip(1) {
        node.effects.push(piratewave_core::effects::EffectSpec::new(
            registry.get("gain").unwrap(),
        ));
        node.effects.push(piratewave_core::effects::EffectSpec::new(
            registry.get("delay").unwrap(),
        ));
    }
    let mut slot_effects: Vec<_> = project
        .nodes
        .iter()
        .skip(1)
        .map(|n| {
            piratewave_core::recording_effects::RecordingEffects::new(&n.effects, false, &registry)
                .unwrap()
        })
        .collect();
    let mut graph = piratewave_core::graph::Graph::new(&project, &registry).unwrap();
    let mut recorder = Recorder::default();
    recorder.sync(&project);
    assert_eq!(recorder.histories.len(), 1);
    let block: [Frame; BLOCK_SIZE] = [[0.2, -0.1]; BLOCK_SIZE];
    let inputs = [(1, block)];
    ALLOCATIONS.store(0, Ordering::Relaxed);
    MEASURE.store(true, Ordering::Relaxed);
    let start = std::time::Instant::now();
    for _ in 0..375 {
        graph.process_recording(
            &inputs,
            |_, _| true,
            |node, _, preview, send| {
                preview.copy_from_slice(&block);
                slot_effects[node.id as usize - 2].process(preview);
                send.copy_from_slice(preview);
            },
            |id, port, selected, frames| recorder.tap(id, port, selected, frames),
        );
        recorder.finish_block();
    }
    let elapsed = start.elapsed();
    MEASURE.store(false, Ordering::Relaxed);
    assert_eq!(
        ALLOCATIONS.load(Ordering::Relaxed),
        0,
        "Recording render allocated"
    );
    let audio = recorder.capture(2, 0.1).unwrap().materialize().unwrap();
    assert_eq!(audio.len(), 4800);
    assert!(audio.iter().all(|f| *f == [0.2, -0.1]));
    println!("PASS: four recording nodes / twenty 60-second slots share one buffer; zero render allocations with slot gain/delay processing; {:.3}% real-time CPU budget; {} bytes shared history",elapsed.as_secs_f64()*100.,recorder.histories.values().map(|h|piratewave_core::recording::history_bytes(h.capacity())).sum::<usize>());
}
