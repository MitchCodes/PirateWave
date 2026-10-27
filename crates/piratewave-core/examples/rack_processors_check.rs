//! Allocation/performance check for all newly added native rack processors.
use piratewave_core::effects::{EffectRegistry, EffectSpec};
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
    let registry = EffectRegistry::default();
    for id in [
        "automatic_gain_control",
        "expander",
        "de_esser",
        "limiter",
        "noise_suppression",
        "invert_polarity",
        "gain",
        "three_band_equalizer",
        "upward_compressor",
        "reverb",
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
    ] {
        let spec = EffectSpec::new(registry.get(id).unwrap());
        let mut effect = registry.create(&spec, 48000.).unwrap();
        let mut audio: Vec<_> = (0..48000)
            .map(|i| {
                let x = 0.1 * (std::f32::consts::TAU * 215. * i as f32 / 48000.).sin();
                [x; 2]
            })
            .collect();
        ALLOCATIONS.store(0, Ordering::Relaxed);
        MEASURE.store(true, Ordering::Relaxed);
        let started = std::time::Instant::now();
        for block in audio.chunks_mut(128) {
            effect.process(block);
        }
        let elapsed = started.elapsed();
        MEASURE.store(false, Ordering::Relaxed);
        assert_eq!(
            ALLOCATIONS.load(Ordering::Relaxed),
            0,
            "{id} allocated during process"
        );
        assert!(audio.iter().flatten().all(|v| v.is_finite()));
        println!(
            "{id}: zero processing allocations, {:.2}% real-time budget",
            elapsed.as_secs_f64() * 100.
        );
    }
}
