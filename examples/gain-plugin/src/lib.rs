//! Minimal PirateWave plugin: the same DSP contributes a rack effect and a node.
use piratewave_extension_sdk::{Block, Plugin, Processor};

pub struct GainPlugin;
struct Gain {
    current: f32,
    target: f32,
    smoothing: f32,
}

impl Plugin for GainPlugin {
    // Embed the exact file distributed beside the DLL. PirateWave compares both.
    const MANIFEST: &'static str = include_str!("../manifest.json");

    fn create(
        index: u32,
        rate: f32,
        block: u32,
        schema: u32,
        state: &[u8],
    ) -> Option<Box<dyn Processor>> {
        // Manifest order selects processor (0) or node (1). Saved projects use
        // the qualified type IDs, never these temporary runtime indices.
        if index > 1 || rate != 48_000.0 || block != 128 || schema != 1 || !state.is_empty() {
            return None;
        }
        Some(Box::new(Gain {
            current: 1.0,
            target: 1.0,
            // A 5 ms one-pole transition; no audio buffer or added latency.
            smoothing: 1.0 - (-1.0 / (0.005 * rate)).exp(),
        }))
    }
}

impl Processor for Gain {
    fn set_parameter(&mut self, index: u32, value: f32) -> bool {
        if index != 0 || !value.is_finite() || !(-60.0..=12.0).contains(&value) {
            return false;
        }
        // Convert dB on the control path, rather than once per audio sample.
        self.target = 10.0_f32.powf(value / 20.0);
        true
    }

    fn process(&mut self, block: &mut Block<'_>) -> bool {
        if block.input_count() != 1 || block.output_count() != 1 {
            return false;
        }
        for index in 0..block.frames() {
            self.current += (self.target - self.current) * self.smoothing;
            let input = block.input(0)[index];
            block.output(0)[index] = [input[0] * self.current, input[1] * self.current];
        }
        true
    }

    fn reset(&mut self) {
        self.current = self.target;
    }
}

piratewave_extension_sdk::export_plugin!(GainPlugin);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_processor_and_node_apply_gain_to_both_channels() {
        piratewave_extension_sdk::parse_manifest(GainPlugin::MANIFEST.as_bytes()).unwrap();
        // Exercise the exported ABI, not just a private DSP helper.
        let api = unsafe { &*piratewave_extension_api() };
        for index in [0, 1] {
            let handle = unsafe { (api.create)(index, 48_000.0, 128, 1, std::ptr::null(), 0) };
            assert!(!handle.is_null());
            let input = [[0.8_f32, -0.4]; 128];
            let mut output = [[0.0_f32; 2]; 128];
            let inputs = [input.as_ptr().cast()];
            let outputs = [output.as_mut_ptr().cast()];
            unsafe {
                assert_eq!((api.set_parameter)(handle, 0, -6.0206), 0);
                assert_eq!((api.reset)(handle), 0);
                assert_eq!(
                    (api.process)(handle, inputs.as_ptr(), 1, outputs.as_ptr(), 1, 128),
                    0
                );
                (api.destroy)(handle);
            }
            assert!(output
                .iter()
                .all(|frame| (frame[0] - 0.4).abs() < 1e-6 && (frame[1] + 0.2).abs() < 1e-6));
        }
    }
}
