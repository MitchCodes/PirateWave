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
    #[cfg(all(windows, debug_assertions))]
    fn test_editor_key(&mut self, key: u32) -> Result<()> {
        self.0.test_editor_key(key)
    }
    #[cfg(all(windows, debug_assertions))]
    fn test_editor_timeout(&mut self) -> Result<()> {
        self.0.test_editor_timeout()
    }
    fn descriptor(&self) -> &PluginDescriptor {
        self.0.descriptor()
    }
    #[cfg(windows)]
    fn open_editor(&mut self) -> Result<()> {
        self.0.open_editor()
    }
    #[cfg(windows)]
    fn close_editor(&mut self) {
        self.0.close_editor();
    }
    #[cfg(windows)]
    fn editor_idle(&mut self) {
        self.0.editor_idle();
    }
    #[cfg(windows)]
    fn take_editor_error(&mut self) -> Option<String> {
        self.0.take_editor_error()
    }
    #[cfg(windows)]
    fn invalidate_capture(&mut self) {
        self.0.invalidate_capture();
    }
    #[cfg(windows)]
    fn editor_open(&self) -> bool {
        self.0.editor_open()
    }
    #[cfg(windows)]
    fn editor_size(&self) -> (i32, i32) {
        self.0.editor_size()
    }
    #[cfg(windows)]
    fn take_editor_edits(&mut self) -> Vec<(u32, f64)> {
        self.0.take_editor_edits()
    }
    fn capture_state(&mut self) -> Result<Option<piratewave_core::plugins::PluginState>> {
        Ok(self
            .0
            .capture_state()?
            .map(
                |(component, vst2_bank, vst2_program)| piratewave_core::plugins::PluginState {
                    component: component.into(),
                    controller: None,
                    vst2_bank,
                    vst2_program,
                },
            ))
    }
    fn capture_state_for_shutdown(
        &mut self,
    ) -> Result<Option<piratewave_core::plugins::PluginState>> {
        Ok(self
            .0
            .capture_state_for_shutdown()?
            .map(
                |(component, vst2_bank, vst2_program)| piratewave_core::plugins::PluginState {
                    component: component.into(),
                    controller: None,
                    vst2_bank,
                    vst2_program,
                },
            ))
    }
    fn restore_state(&mut self, state: &piratewave_core::plugins::PluginState) -> Result<()> {
        self.0
            .restore_state(&state.component, state.vst2_bank, state.vst2_program)
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
