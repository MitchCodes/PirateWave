//! Offline tone comparison and performance check. Never opens audio hardware.
//! cargo run --release -p piratewave-core --example voice_correction_check -- report.json
use piratewave_core::{effects::AudioEffect, voice_correction::VoiceCorrection};
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
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cases = vec![];
    for (name, input, scale, detune, transpose, target) in [
        ("chromatic_a", 430.0, 2.0, 0.0, 0.0, 440.0),
        ("chromatic_b", 250.0, 2.0, 0.0, 0.0, 246.94165),
        ("c_major", 307.0, 0.0, 0.0, 0.0, 293.66476),
        ("c_minor", 307.0, 1.0, 0.0, 0.0, 311.12698),
        ("transpose", 430.0, 2.0, 0.0, 7.0, 659.2551),
        ("detune", 440.0, 2.0, 25.0, 0.0, 446.39996),
    ] {
        let mut effect = VoiceCorrection::new(48000.0);
        for (id, value) in [
            ("retune", 0.0),
            ("scale", scale),
            ("detune", detune),
            ("transpose", transpose),
        ] {
            effect.set_parameter(id, value)?;
        }
        let mut audio: Vec<_> = (0..96000)
            .map(|i| {
                let phase = std::f32::consts::TAU * input * i as f32 / 48000.0;
                let s = 0.2 * phase.sin() + 0.08 * (2.0 * phase).sin() + 0.04 * (3.0 * phase).sin();
                [s; 2]
            })
            .collect();
        let start = std::time::Instant::now();
        ALLOCATIONS.store(0, Ordering::Relaxed);
        MEASURE.store(true, Ordering::Relaxed);
        for block in audio.chunks_mut(128) {
            effect.process(block);
        }
        MEASURE.store(false, Ordering::Relaxed);
        assert_eq!(
            ALLOCATIONS.load(Ordering::Relaxed),
            0,
            "DSP must not allocate"
        );
        let realtime_percent = start.elapsed().as_secs_f64() / 2.0 * 100.0;
        let steady = &audio[48000..];
        let crossings: Vec<f32> = steady
            .windows(2)
            .enumerate()
            .filter(|(_, w)| w[0][0] <= 0.0 && w[1][0] > 0.0)
            .map(|(i, w)| i as f32 - w[0][0] / (w[1][0] - w[0][0]))
            .collect();
        let hz =
            48000.0 * (crossings.len() - 1) as f32 / (crossings.last().unwrap() - crossings[0]);
        let cents = 1200.0 * (hz / target).log2();
        assert!(cents.abs() < 5.0, "{name}: {cents} cents");
        effect.reset();
        cases.push(serde_json::json!({"name":name,"input_hz":input,"target_hz":target,"output_hz":hz,"error_cents":cents,"realtime_cpu_percent":realtime_percent}));
    }
    let json = serde_json::to_string_pretty(
        &serde_json::json!({"latency_ms":piratewave_core::voice_correction::LATENCY_MS,"process_allocations":0,"cases":cases}),
    )?;
    if let Some(path) = std::env::args().nth(1) {
        std::fs::write(path, json)?;
    } else {
        println!("{json}");
    }
    Ok(())
}
