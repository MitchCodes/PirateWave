//! Adapter for the shared VST2 host. The x86 helper uses the same implementation.
use super::*;
use piratewave_vst2_host::Hosted as _;
pub struct Vst2(piratewave_vst2_host::Vst2);
impl Vst2 {
    pub fn load(path: &Path) -> Result<Self> {
        Ok(Self(piratewave_vst2_host::Vst2::load(path)?))
    }
}
impl Hosted for Vst2 {
    fn descriptor(&self) -> &PluginDescriptor {
        self.0.descriptor()
    }
    fn set_parameter(&mut self, id: u32, value: f64) {
        self.0.set_parameter(id, value);
    }
    fn note(&mut self, note: u8, velocity: f32, on: bool) {
        self.0.note(note, velocity, on);
    }
    fn process(&mut self, frames: &mut [Frame]) -> Result<()> {
        self.0.process(frames)
    }
}
