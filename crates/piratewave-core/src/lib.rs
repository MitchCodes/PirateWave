//! Device-independent DSP and routing. No GUI, audio driver, or filesystem dependencies.
pub mod control;
pub mod effects;
pub mod graph;
pub mod project;
pub mod shortcuts;

pub type Frame = [f32; 2];
pub const SAMPLE_RATE: u32 = 48_000;
pub const BLOCK_SIZE: usize = 128;

pub mod plugins;

pub mod voice_correction;

pub mod rack_processors;
pub mod reverb;
pub mod voice;

pub mod voice_effects;

pub mod automatic_gain;

pub mod recording;

pub mod recording_effects;

pub mod ducking;

pub mod extensions;
