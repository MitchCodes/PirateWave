use super::*;
use crate::control::{Request, Service, Source};
use piratewave_core::control;
#[derive(Default)]
pub(super) struct Controller {
    pub service: Option<Service>,
    pub error: Option<String>,
    pub last_error: Option<String>,
    pub generation: u64,
    pub recordings: HashMap<(u64, u64), String>,
    published: Option<Instant>,
    attempted: bool,
}
impl PirateWaveApp {
    pub(super) fn controller_request(&mut self, req: &Request) -> Result<String, String> {
        if req.generation != self.controller.generation {
            return Err("Project changed; refresh controller state".into());
        }
        let selector = req.selector.as_ref().ok_or("Missing selector")?;
        // Validate against the live project, never the client's cached catalog.
        let catalog = control::catalog(&self.project, &self.registry, self.status.running);
        control::entry(&catalog, selector)?;
        self.controller.last_error = None;
        if req.op == "execute" {
            let action = control::resolve(&self.project, selector)?;
            if let ShortcutAction::RecordingSlot {
                id,
                slot,
                action: slot_action,
            } = action
            {
                let n = self.project.nodes.iter().find(|n| n.id == id).unwrap();
                let s = n.recording.slots.iter().find(|s| s.id == slot).unwrap();
                if slot_action == piratewave_core::recording::SlotAction::Capture {
                    if !self.status.running
                        || !n.recording.enabled
                        || self
                            .status
                            .recording_seconds
                            .get(&id)
                            .copied()
                            .unwrap_or(0.)
                            <= 0.
                    {
                        return Err("Start recording and wait for buffered audio".into());
                    }
                } else if matches!(
                    slot_action,
                    piratewave_core::recording::SlotAction::Preview
                        | piratewave_core::recording::SlotAction::Send
                        | piratewave_core::recording::SlotAction::PreviewAndSend
                ) {
                    if !self.status.running {
                        return Err("Start audio before playback".into());
                    }
                    if s.asset.is_none() {
                        return Err("Capture audio into this slot first".into());
                    }
                }
            }
            if let ShortcutAction::Sound(id) = action {
                if !self.status.running {
                    return Err("Start audio before playing a sound".into());
                }
                if !self.clips.contains_key(&id) {
                    return Err("Sound is still loading or its file is unavailable".into());
                }
            }
            self.handle_shortcut(action);
        } else if req.op == "adjust" || req.op == "reset" {
            let before = self.project.clone();
            if req.op == "reset" && req.version >= 2 {
                control::reset_parameter(&mut self.project, &self.registry, selector)?;
            } else {
                control::adjust(
                    &mut self.project,
                    &self.registry,
                    selector,
                    req.ticks,
                    req.sensitivity,
                    req.op == "reset",
                )?;
            }
            if let Err(error) = piratewave_core::graph::validate(&self.project) {
                self.project = before;
                return Err(error);
            }
            self.changed(false);
        } else {
            return Err("Unsupported operation".into());
        }
        if let Some(error) = self.controller.last_error.take() {
            return Err(error);
        }
        Ok("Accepted".into())
    }
    pub(super) fn poll_controller(&mut self) {
        let enabled = self.app_settings.loupedeck_enabled && !self.smoke;
        if !enabled {
            self.controller.service = None;
            self.controller.attempted = false;
            return;
        }
        if !self.controller.attempted {
            self.controller.attempted = true;
            if self.controller.generation == 0 {
                self.controller.generation = piratewave_core::recording::RecordSlot::new("").id;
            }
            match Service::start() {
                Ok(service) => {
                    self.controller.service = Some(service);
                    self.controller.error = None;
                }
                Err(error) => {
                    crate::logging::error(format_args!("External controllers: {error}"));
                    self.controller.error = Some(error);
                }
            }
        }
        let requests: Vec<_> = self
            .controller
            .service
            .as_ref()
            .map(|s| s.rx.try_iter().take(16).collect())
            .unwrap_or_default();
        for pending in requests {
            if pending.live.load(std::sync::atomic::Ordering::Acquire) {
                let result = self.controller_request(&pending.request);
                let _ = pending.reply.try_send(result);
            }
        }
        if self
            .controller
            .published
            .is_none_or(|at| at.elapsed() >= Duration::from_millis(100))
        {
            if let Some(service) = &self.controller.service {
                service.source.store(Some(std::sync::Arc::new(Source {
                    project: self.project.clone(),
                    generation: self.controller.generation,
                    running: self.status.running,
                    notice: self.notice.clone(),
                    buffered: self.status.recording_seconds.clone(),
                    recordings: self.controller.recordings.clone(),
                })));
            }
            self.controller.published = Some(Instant::now());
        }
    }
    pub(super) fn controller_settings(&mut self, ui: &mut egui::Ui) {
        ui.add_space(16.);
        ui.heading("External controllers");
        let mut enabled = self.app_settings.loupedeck_enabled;
        if ui
            .checkbox(&mut enabled, "Enable external controllers")
            .changed()
        {
            self.app_settings.loupedeck_enabled = enabled;
            self.settings_draft.loupedeck_enabled = enabled;
            self.settings_dirty = true;
            self.controller.attempted = false;
            self.controller.error = None;
            if let Err(error) =
                storage::save_settings(&storage::settings_path(), &self.app_settings)
            {
                self.notify_error(format!("Cannot save controller setting: {error}"));
            } else {
                self.settings_dirty = false;
            }
        }
        ui.label("Install PirateWave.lplug4 in Loupedeck / Logi Plugin Service or PirateWave.streamDeckPlugin in Stream Deck, then assign PirateWave actions to buttons and knobs. Both controllers can connect simultaneously. Targets match exact names in your current project.");
        if let Some(error) = &self.controller.error {
            ui.colored_label(theme::palette().peach, error);
            if ui.button("Retry connection").clicked() {
                self.controller.attempted = false;
            }
        } else if let Some(service) = &self.controller.service {
            ui.label(service.message.lock().unwrap().clone());
            ui.small(format!(
                "{} available controls",
                service.state.load().entries.len()
            ));
        } else {
            ui.label("External controllers disabled");
        }
    }
}

#[cfg(test)]
mod tests;
