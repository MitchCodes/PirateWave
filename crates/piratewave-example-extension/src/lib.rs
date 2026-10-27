//! Two real DLL contributions used by integration and allocation tests.
use piratewave_extension_sdk::{Block, Plugin, Processor};
pub struct Example;
struct Dsp {
    kind: u32,
    current: f32,
    target: f32,
    invert: bool,
}
impl Processor for Dsp {
    fn set_parameter(&mut self, index: u32, value: f32) -> bool {
        match index {
            0 => {
                self.target = value;
                true
            }
            1 if self.kind == 0 => {
                self.invert = value >= 0.5;
                true
            }
            _ => false,
        }
    }
    fn reset(&mut self) {
        self.current = self.target;
    }
    fn process(&mut self, b: &mut Block<'_>) -> bool {
        if self.kind == 0 {
            for i in 0..b.frames() {
                self.current += (self.target - self.current) * 0.01;
                let f = b.input(0)[i];
                let mid = (f[0] + f[1]) * 0.5;
                let side = (f[0] - f[1]) * 0.5 * self.current;
                let sign = if self.invert { -1. } else { 1. };
                b.output(0)[i] = [(mid + side) * sign, (mid - side) * sign];
            }
        } else {
            for i in 0..b.frames() {
                self.current += (self.target - self.current) * 0.01;
                let a = b.input(0)[i];
                let c = b.input(1)[i];
                b.output(0)[i] = [(a[0] + c[0]) * self.current, (a[1] + c[1]) * self.current];
                b.output(1)[i] = a;
                b.output(2)[i] = c;
            }
        }
        true
    }
}
impl Plugin for Example {
    fn migrate(index: u32, old_schema: u32, state: &[u8]) -> Option<Vec<u8>> {
        (index < 2 && old_schema == 1 && state.is_empty()).then(Vec::new)
    }
    const MANIFEST: &'static str = include_str!("../manifest.json");
    fn create(
        index: u32,
        rate: f32,
        block: u32,
        schema: u32,
        state: &[u8],
    ) -> Option<Box<dyn Processor>> {
        if index > 1 || rate != 48000. || block != 128 || schema != 2 || !state.is_empty() {
            return None;
        }
        Some(Box::new(Dsp {
            kind: index,
            current: 1.,
            target: 1.,
            invert: false,
        }))
    }
}
piratewave_extension_sdk::export_plugin!(Example);
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_is_valid() {
        let m: piratewave_extension_sdk::Manifest = serde_manifest();
        m.validate().unwrap();
        assert_eq!(m.types.len(), 2);
    }
    fn serde_manifest() -> piratewave_extension_sdk::Manifest {
        piratewave_extension_sdk::parse_manifest(Example::MANIFEST.as_bytes()).unwrap()
    }
}
