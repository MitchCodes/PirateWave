//! A complete third-party-style processor: `cargo run -p piratewave-core --example custom_effect`.
use piratewave_core::{
    effects::{AudioEffect, EffectFactory, EffectRegistry, EffectSpec, ParameterDescriptor},
    Frame,
};

struct StereoWidth {
    width: f32,
}
impl AudioEffect for StereoWidth {
    fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), String> {
        match id {
            "width" => {
                self.width = value;
                Ok(())
            }
            _ => Err(format!("Unknown parameter {id}")),
        }
    }
    fn process(&mut self, frames: &mut [Frame]) {
        for frame in frames {
            let mid = (frame[0] + frame[1]) * 0.5;
            let side = (frame[0] - frame[1]) * 0.5 * self.width;
            *frame = [mid + side, mid - side];
        }
    }
    fn reset(&mut self) {}
}
fn main() {
    let mut registry = EffectRegistry::default();
    registry
        .register(EffectFactory {
            id: "stereo_width",
            name: "Stereo width",
            description: "Blend from mono to exaggerated stereo",
            parameters: &[ParameterDescriptor {
                id: "width",
                label: "Width",
                min: 0.0,
                max: 2.0,
                default: 1.0,
                unit: "",
            }],
            create: |_| Box::new(StereoWidth { width: 1.0 }),
        })
        .unwrap();
    let mut spec = EffectSpec::new(registry.get("stereo_width").unwrap());
    spec.parameters.insert("width".into(), 0.0);
    let mut effect = registry.create(&spec, 48000.0).unwrap();
    let mut frames = [[0.75, 0.25]];
    effect.process(&mut frames);
    assert_eq!(frames, [[0.5, 0.5]]);
    println!(
        "Custom effect registered and processed stereo to mono: {:?}",
        frames
    );
}
