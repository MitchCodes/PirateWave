use super::*;
use crate::audio_io::format::{AudioFormat, Capabilities};
type Key = (Option<String>, bool);
type Reply = (Key, Result<Capabilities, String>);
#[derive(Default)]
pub(super) struct FormatPicker {
    cache: std::collections::VecDeque<Reply>,
    pending: Option<(u64, mpsc::Receiver<Reply>)>,
    generation: u64,
}

impl FormatPicker {
    pub fn invalidate(&mut self) {
        self.cache.clear();
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        id: (u64, bool),
        device: &Option<String>,
        output: bool,
        preference: &mut AudioFormat,
        active: Option<&String>,
    ) -> bool {
        let mut changed = false;
        ui.push_id(("audio-format", id), |ui| {
            egui::CollapsingHeader::new("Audio format").show(ui, |ui| {
                if let Some((generation, rx)) = &self.pending {
                    match rx.try_recv() {
                        Ok(reply) => { if *generation == self.generation { self.cache.push_back(reply); } self.pending = None; while self.cache.len() > 32 { self.cache.pop_front(); } }
                        Err(mpsc::TryRecvError::Disconnected) => self.pending = None,
                        Err(mpsc::TryRecvError::Empty) => {}
                    }
                }
                let key = (device.clone(), output);
                if let Some((_, result)) = self.cache.iter().find(|(k, _)| k == &key) {
                    match result {
                        Ok(caps) => {
                            let before = preference.clone();
                            ui.small("Sample rate");
                            egui::ComboBox::from_id_salt("rate").selected_text(preference.sample_rate.map_or("Auto — device default".into(), |r| format!("{r} Hz"))).show_ui(ui, |ui| {
                                if ui.selectable_value(&mut preference.sample_rate, None, "Auto — device default").changed() { preference.sample_format = None; }
                                let mut rates: Vec<_> = caps.choices.iter().map(|c| c.rate).collect(); rates.sort_unstable(); rates.dedup();
                                for rate in rates { if ui.selectable_value(&mut preference.sample_rate, Some(rate), format!("{rate} Hz")).changed() { preference.sample_format = None; } }
                            });
                            let rate = preference.sample_rate.unwrap_or(caps.default.rate);
                            ui.small("Sample format");
                            egui::ComboBox::from_id_salt("encoding").selected_text(preference.sample_format.map_or("Auto — best supported", |f| f.label())).show_ui(ui, |ui| {
                                ui.selectable_value(&mut preference.sample_format, None, "Auto — best supported");
                                for choice in caps.choices.iter().filter(|c| c.rate == rate) {
                                    ui.selectable_value(&mut preference.sample_format, Some(choice.encoding), choice.encoding.label());
                                }
                            });
                            changed = before != *preference;
                            ui.small(format!("Device default: {} Hz · {}", caps.default.rate, caps.default.encoding.label()));
                        }
                        Err(error) => { ui.label(error); ui.small("Saved preferences are retained while unavailable."); }
                    }
                } else {
                    ui.small("Checking supported formats…");
                    if self.pending.is_none() && !device.as_ref().is_some_and(|s| s.is_empty()) {
                        let (tx, rx) = mpsc::sync_channel(1);
                        self.pending = Some((self.generation, rx));
                        let key = key.clone();
                        std::thread::spawn(move || { let result = crate::audio_io::format::capabilities(&key.0, key.1).map_err(|e| e.to_string()); let _ = tx.send((key, result)); });
                    }
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                }
                if let Some(status) = active { ui.small(status); } else { ui.small("Start audio to see the active format."); }
                ui.small("Shared mode. Windows may perform additional conversion. Processing stays at 48 kHz stereo float.");
                if ui.small_button("Refresh formats").clicked() { self.invalidate(); }
            });
        });
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_io::format::{FormatChoice, SampleEncoding};
    #[test]
    fn visible_formats_preserve_assignments_and_reject_stale_discovery() {
        let ctx = egui::Context::default();
        ctx.memory_mut(|m| m.set_everything_is_visible(true));
        let device = Some("fixture".into());
        let choice = FormatChoice {
            rate: 48000,
            encoding: SampleEncoding::F32,
            channels: 2,
        };
        let caps = Capabilities {
            default: choice.clone(),
            choices: vec![choice],
        };
        let mut picker = FormatPicker::default();
        picker
            .cache
            .push_back(((device.clone(), false), Ok(caps.clone())));
        let mut preference = AudioFormat {
            sample_rate: Some(96000),
            sample_format: Some(SampleEncoding::I16),
        };
        let original = preference.clone();
        let render = |picker: &mut FormatPicker, preference: &mut AudioFormat| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    assert!(!picker.show(ui, (1, false), &device, false, preference, None));
                });
            });
        };
        render(&mut picker, &mut preference);
        assert_eq!(preference, original);
        assert!(picker.pending.is_none());
        let (tx, rx) = mpsc::sync_channel(1);
        picker.pending = Some((picker.generation, rx));
        picker.invalidate();
        picker
            .cache
            .push_back(((device.clone(), false), Err("Device unavailable".into())));
        tx.send(((device.clone(), false), Ok(caps))).unwrap();
        render(&mut picker, &mut preference);
        assert_eq!(picker.cache.len(), 1);
        assert!(picker.cache[0].1.is_err());
        assert_eq!(preference, original);
    }
}
