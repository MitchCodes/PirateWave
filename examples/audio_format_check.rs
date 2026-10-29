//! Offline boundary-conversion benchmark; opens no devices and writes no audio.
use crossbeam_queue::ArrayQueue;
use piratewave_core::{Frame, SAMPLE_RATE};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Instant,
};
#[path = "../src/audio_io/resample.rs"]
mod resample;
struct Allocator;
static MEASURING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if MEASURING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if MEASURING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        System.realloc(ptr, layout, size)
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
fn main() {
    for (from, to, adaptive) in [
        (48000, 48000, false),
        (48000, 48000, true),
        (44100, 48000, true),
        (96000, 48000, true),
        (192000, 48000, true),
        (48000, 44100, true),
        (48000, 192000, true),
    ] {
        let q = Arc::new(ArrayQueue::new(32768));
        let target = (2048_u64 * from as u64 / SAMPLE_RATE as u64) as usize;
        let mut reader = resample::QueueReader::new(q.clone(), from, to, target);
        reader.set_clock_correction(adaptive);
        for _ in 0..target * 2 {
            q.push([0.25, -0.125]).unwrap();
        }
        let started = Instant::now();
        let before = ALLOCATIONS.load(Ordering::Relaxed);
        MEASURING.store(true, Ordering::Relaxed);
        for _ in 0..to * 10 / 128 {
            while q.len() < target + 1024 {
                q.push([0.25, -0.125]).unwrap();
            }
            for _ in 0..128 {
                std::hint::black_box(reader.next_frame().unwrap());
            }
        }
        MEASURING.store(false, Ordering::Relaxed);
        let cpu = started.elapsed().as_secs_f64() / 10. * 100.;
        assert_eq!(before, ALLOCATIONS.load(Ordering::Relaxed));
        assert_eq!(reader.underruns(), 0);
        assert!(cpu < 10., "converter exceeded one-tenth of a core");
        println!("{from} -> {to}, adaptive={adaptive}: {cpu:.4}% realtime CPU, zero allocations/underruns");
    }
}
