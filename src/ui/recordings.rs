use super::*;
use crate::recording_io::{self, Audio, Event, Job, Service};
use piratewave_core::recording::{RecordSlot, SlotAction};
#[derive(Clone)]
struct Wave {
    asset: String,
    duration: f32,
    bins: Vec<[f32; 2]>,
}
#[derive(Default)]
pub(super) struct RecordingUi {
    pub service: Service,
    cache: HashMap<(u64, u64), Audio>,
    waves: HashMap<(u64, u64), Wave>,
    versions: HashMap<(u64, u64), u64>,
    loading: HashSet<(u64, u64)>,
    failed: HashSet<String>,
    pending_play: HashMap<(u64, u64), SlotAction>,
    drag: Option<((u64, u64), bool)>,
}
impl PirateWaveApp {
    fn recording_job(&mut self, job: Job) -> bool {
        if self.recording_ui.service.tx.try_send(job).is_err() {
            self.notify_error("Recording worker is busy; try again shortly.");
            false
        } else {
            true
        }
    }
    pub(super) fn cleanup_recording_asset(&mut self, asset: String) {
        // A previous Save As or autosave can still refer to this immutable take.
        // Detach from this workspace; do not break another saved project's media.
        crate::logging::info(format_args!(
            "Recording asset detached; retained for saved project versions: {asset}"
        ));
    }
    pub(super) fn poll_recordings(&mut self) {
        while let Ok(event) = self.recording_ui.service.rx.try_recv() {
            match event {
                Event::Audio {
                    node,
                    slot,
                    token,
                    audio,
                    captured,
                } => {
                    let key = (node, slot);
                    let valid = self.recording_ui.versions.get(&key).copied().unwrap_or(0) == token;
                    if valid {
                        self.recording_ui.loading.remove(&key);
                    }
                    let target = self
                        .project
                        .nodes
                        .iter_mut()
                        .find(|n| n.id == node && matches!(n.kind, NodeKind::Recording))
                        .and_then(|n| n.recording.slots.iter_mut().find(|s| s.id == slot));
                    let mut old = None;
                    if let Some(target) = target
                        .filter(|s| valid && (captured || s.asset.as_deref() == Some(&audio.asset)))
                    {
                        if captured {
                            self.controller.recordings.insert(key, "Captured".into());
                            old = target.asset.replace(audio.asset.clone());
                            target.crop_start = 0.;
                            target.crop_end = audio.clip.duration();
                        }
                        self.recording_ui.waves.insert(
                            key,
                            Wave {
                                asset: audio.asset.clone(),
                                duration: audio.clip.duration(),
                                bins: audio.waveform.clone(),
                            },
                        );
                        self.recording_ui.cache.insert(key, audio);
                        if captured {
                            self.changed(false);
                            self.notify("Recording captured into slot");
                        }
                        if let Some(action) = self.recording_ui.pending_play.remove(&key) {
                            self.recording_slot_action(node, slot, action);
                        }
                    } else if captured {
                        // This result was never attached to a project and is safe to discard.
                        self.recording_job(Job::Delete(audio.asset));
                    }
                    if let Some(old) = old {
                        self.cleanup_recording_asset(old);
                    }
                }
                Event::Failed {
                    node,
                    slot,
                    token,
                    error,
                } => {
                    let key = (node, slot);
                    if self.recording_ui.versions.get(&key).copied().unwrap_or(0) == token {
                        self.controller
                            .recordings
                            .insert(key, format!("Error: {error}"));
                        self.recording_ui.loading.remove(&key);
                        self.recording_ui.pending_play.remove(&key);
                        if let Some(asset) = self
                            .project
                            .nodes
                            .iter()
                            .find(|n| n.id == node)
                            .and_then(|n| n.recording.slots.iter().find(|s| s.id == slot))
                            .and_then(|s| s.asset.clone())
                        {
                            self.recording_ui.failed.insert(asset);
                        }
                        self.notify_error(format!("Recording: {error}"));
                    }
                }
                Event::Saved(path) => self.notify(format!("Recording saved to {}", path.display())),
                Event::ExportFailed(error) => self.notify_error(format!("Export: {error}")),
            }
        }
        self.recording_ui.cache.retain(|&(node, slot), audio| {
            self.project.nodes.iter().any(|n| {
                n.id == node
                    && n.recording
                        .slots
                        .iter()
                        .any(|s| s.id == slot && s.asset.as_deref() == Some(&audio.asset))
            })
        });
        self.recording_ui.waves.retain(|&(node, slot), audio| {
            self.project.nodes.iter().any(|n| {
                n.id == node
                    && n.recording
                        .slots
                        .iter()
                        .any(|s| s.id == slot && s.asset.as_deref() == Some(&audio.asset))
            })
        });
        // Waveform/clip caches are disposable: disk assets remain when evicted.
        let mut bytes: usize = self
            .recording_ui
            .cache
            .values()
            .map(|a| a.clip.frames.len() * 8)
            .sum();
        if bytes > 256 * 1024 * 1024 {
            let keys: Vec<_> = self.recording_ui.cache.keys().copied().collect();
            for key in keys {
                if bytes <= 256 * 1024 * 1024 {
                    break;
                }
                if let Some(audio) = self.recording_ui.cache.remove(&key) {
                    bytes -= audio.clip.frames.len() * 8;
                }
            }
        }
    }
    pub(super) fn stop_recording_playback(&mut self, node: u64, slot: Option<u64>) {
        // Cancel deferred plays too: a late asset load must not restart stopped audio.
        self.recording_ui
            .pending_play
            .retain(|&(id, slot_id), _| id != node || slot.is_some_and(|slot| slot != slot_id));
        self.command(Command::StopRecordingPlayback { node, slot });
    }
    pub(super) fn set_recording(&mut self, node: u64, enabled: bool) {
        let Some(n) = self
            .project
            .nodes
            .iter_mut()
            .find(|n| n.id == node && matches!(n.kind, NodeKind::Recording))
        else {
            return;
        };
        let before = n.recording.enabled;
        n.recording.enabled = enabled;
        if let Err(error) = piratewave_core::graph::validate(&self.project) {
            if let Some(n) = self.project.nodes.iter_mut().find(|n| n.id == node) {
                n.recording.enabled = before;
            }
            self.notify_error(error);
            return;
        }
        self.changed(false);
        self.command(if self.status.running {
            Command::Update(self.project.clone())
        } else if enabled {
            Command::StartIfStopped(self.project.clone())
        } else {
            Command::Update(self.project.clone())
        });
        crate::logging::info(format_args!("Recording node {node}: enabled={enabled}"));
    }
    fn request_recording_load(&mut self, node: u64, slot: &RecordSlot) -> bool {
        let key = (node, slot.id);
        if self.recording_ui.loading.contains(&key) {
            return true;
        }
        if self.recording_ui.loading.len() >= 2 {
            return false;
        }
        let Some(asset) = &slot.asset else {
            return false;
        };
        let token = self.recording_ui.versions.get(&key).copied().unwrap_or(0);
        if self
            .recording_ui
            .service
            .tx
            .try_send(Job::Load {
                node,
                slot: slot.id,
                token,
                asset: asset.clone(),
            })
            .is_ok()
        {
            self.recording_ui.loading.insert(key);
            true
        } else {
            false
        }
    }
    pub(super) fn recording_slot_action(&mut self, node: u64, slot_id: u64, action: SlotAction) {
        let Some(slot) = self
            .project
            .nodes
            .iter()
            .find(|n| n.id == node && matches!(n.kind, NodeKind::Recording))
            .and_then(|n| n.recording.slots.iter().find(|s| s.id == slot_id))
            .cloned()
        else {
            return;
        };
        let key = (node, slot_id);
        match action {
            SlotAction::Capture => {
                self.controller
                    .recordings
                    .insert(key, "Capture queued".into());
                if !self.status.running {
                    self.notify("Start recording before capturing recent audio.");
                    return;
                }
                let token = RecordSlot::new("").id;
                self.recording_ui.versions.insert(key, token);
                self.recording_ui.pending_play.remove(&key);
                // Order a live configuration barrier before the capture request.
                self.command(Command::Update(self.project.clone()));
                self.notify("Capturing recent audio...");
                self.command(Command::CaptureRecording {
                    node,
                    slot: slot_id,
                    token,
                    jobs: self.recording_ui.service.tx.clone(),
                    events: self.recording_ui.service.events.clone(),
                });
            }
            SlotAction::Delete => {
                self.controller.recordings.remove(&key);
                self.recording_ui
                    .versions
                    .insert(key, RecordSlot::new("").id);
                self.recording_ui.cache.remove(&key);
                self.recording_ui.waves.remove(&key);
                self.recording_ui.loading.remove(&key);
                self.recording_ui.pending_play.remove(&key);
                if let Some(s) = self
                    .project
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == node)
                    .and_then(|n| n.recording.slots.iter_mut().find(|s| s.id == slot_id))
                {
                    s.asset = None;
                    s.crop_start = 0.;
                    s.crop_end = 0.;
                }
                self.command(Command::StopRecordingPlayback {
                    node,
                    slot: Some(slot_id),
                });
                self.changed(false);
                if let Some(asset) = slot.asset {
                    self.cleanup_recording_asset(asset);
                }
            }
            SlotAction::Preview | SlotAction::Send | SlotAction::PreviewAndSend => {
                if !self.status.running {
                    self.notify("Start audio before playing a recording.");
                    return;
                }
                if let Some(audio) = self.recording_ui.cache.get(&key).cloned() {
                    if let Err(error) =
                        recording_io::cropped(&audio.clip.frames, slot.crop_start, slot.crop_end)
                    {
                        self.notify_error(error.to_string());
                        return;
                    }
                    let preview = action != SlotAction::Send;
                    let send = action != SlotAction::Preview;
                    self.command(Command::Update(self.project.clone()));
                    self.command(Command::PlayRecording {
                        node,
                        slot: slot_id,
                        clip: audio.clip.clone(),
                        start: slot.crop_start,
                        end: slot.crop_end,
                        preview,
                        send,
                    });
                } else if slot.asset.is_some() {
                    if let Some(asset) = &slot.asset {
                        self.recording_ui.failed.remove(asset);
                    }
                    if self.request_recording_load(node, &slot) {
                        self.recording_ui.pending_play.insert(key, action);
                    } else {
                        self.notify_error("Recording worker is busy; try playback again shortly.");
                    }
                } else {
                    self.notify("Capture audio into this slot first.");
                }
            }
        }
    }
    fn remove_recording_slot(&mut self, node: u64, slot: u64) {
        self.recording_slot_action(node, slot, SlotAction::Delete);
        if let Some(n) = self.project.nodes.iter_mut().find(|n| n.id == node) {
            n.recording.slots.retain(|s| s.id != slot);
        }
        self.refresh_shortcuts();
        self.changed(false);
        self.command(Command::Update(self.project.clone()));
    }
    pub(super) fn open_recording_slot_keybinds(&mut self, node: u64, slot: u64) {
        self.open_node_keybinds(node);
        self.view = View::RecordingSlotKeybinds(node, slot);
    }
    fn export_recording_slot(&mut self, node: u64, slot: u64) {
        let Some(config) = self
            .project
            .nodes
            .iter()
            .find(|n| n.id == node)
            .and_then(|n| n.recording.slots.iter().find(|s| s.id == slot))
            .cloned()
        else {
            return;
        };
        let Some(audio) = self.recording_ui.cache.get(&(node, slot)).cloned() else {
            self.request_recording_load(node, &config);
            self.notify("Loading recording; click Save again when its waveform appears.");
            return;
        };
        if let Some(mut path) = rfd::FileDialog::new()
            .add_filter("WAV audio", &["wav"])
            .add_filter("MP3 audio", &["mp3"])
            .set_file_name(format!("{}.wav", config.name))
            .save_file()
        {
            if path.extension().is_none() {
                path.set_extension("wav");
            }
            let node_config = self.project.nodes.iter().find(|n| n.id == node).unwrap();
            self.recording_job(Job::Export {
                effects: node_config.effects.clone(),
                bypass: node_config.effects_disabled,
                audio: audio.clip,
                start: config.crop_start,
                end: config.crop_end,
                path,
            });
        }
    }
    pub(super) fn recording_page(&mut self, ui: &mut egui::Ui, node: u64) {
        if self.node_header(ui, node) {
            return;
        }
        let Some(n) = self
            .project
            .nodes
            .iter()
            .find(|n| n.id == node && matches!(n.kind, NodeKind::Recording))
        else {
            ui.label("Recording node removed.");
            return;
        };
        let mut bypass = n.effects_disabled;
        let effect_count = n.effects.len();
        let mut config = n.recording.clone();
        let mut changed = false;
        let mut set_enabled = None;
        let mut node_keys = false;
        let mut stop_playback = false;
        let mut open_effects = false;
        ui.small("Live passthrough, slot preview and send use separate output ports.");
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !config.enabled || !self.status.running,
                    egui::Button::new("Start recording"),
                )
                .clicked()
            {
                set_enabled = Some(true);
            }
            if ui
                .add_enabled(config.enabled, egui::Button::new("Stop recording"))
                .clicked()
            {
                set_enabled = Some(false);
            }
            node_keys = ui.button("Recording keybinds").clicked();
            stop_playback = ui.button("Stop playing recording").clicked();
        });
        ui.horizontal(|ui| {
            open_effects = ui
                .button(format!("Slot effects ({effect_count})"))
                .clicked();
            if ui.checkbox(&mut bypass, "Bypass slot effects").changed() {
                self.project
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == node)
                    .unwrap()
                    .effects_disabled = bypass;
                self.changed(false);
            }
        });
        ui.small("Slot effects apply to playback and saved files. Capture and live passthrough remain unchanged.");
        let available = self
            .status
            .recording_seconds
            .get(&node)
            .copied()
            .unwrap_or(0.);
        ui.colored_label(
            if config.enabled && self.status.running {
                theme::palette().green
            } else {
                theme::palette().muted
            },
            if config.enabled {
                format!("Recording armed - {:.1} seconds buffered", available)
            } else {
                "Recording stopped - captured slots remain available".into()
            },
        );
        ui.small(format!(
            "Shared histories across all recorders: {} buffers / {:.1} MiB",
            self.status.recording_buffers,
            self.status.recording_bytes as f32 / 1048576.
        ));
        egui::CollapsingHeader::new("Recording help").id_salt(("recording-help",node)).show(ui,|ui|{ui.label("Capture stores the most recent lookback window since Start recording. Drag waveform boundaries or adjust Start / End to crop. Playback and exports use that selection.");});
        let mut selection_changes = vec![];
        let mut actions = vec![];
        let mut remove = None;
        let mut keys = None;
        let mut save = None;
        for slot in &mut config.slots {
            let key = (node, slot.id);
            if slot.asset.is_some()
                && !self.recording_ui.waves.contains_key(&key)
                && !self.recording_ui.loading.contains(&key)
                && !self
                    .recording_ui
                    .failed
                    .contains(slot.asset.as_ref().unwrap())
            {
                self.request_recording_load(node, slot);
            }
            let audio = self.recording_ui.waves.get(&key).cloned();
            let mut selection_changed = false;
            ui.push_id(slot.id, |ui| {
                show_card(ui, |ui| {
                    ui.horizontal(|ui| {
                        changed |= ui.text_edit_singleline(&mut slot.name).changed();
                        ui.label("Look back");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut slot.seconds)
                                    .range(0.1..=piratewave_core::recording::MAX_SECONDS)
                                    .speed(0.1)
                                    .suffix(" s"),
                            )
                            .changed();
                        if ui.button("Keybinds").clicked() {
                            keys = Some(slot.id);
                        }
                        if ui.button("Remove slot").clicked() {
                            remove = Some(slot.id);
                        }
                    });
                    if let Some(audio) = &audio {
                        let duration = audio.duration;
                        selection_changed |=
                            draw_waveform(ui, key, slot, audio, &mut self.recording_ui.drag);
                        ui.horizontal(|ui| {
                            ui.label("Start");
                            selection_changed |= ui
                                .add(
                                    egui::DragValue::new(&mut slot.crop_start)
                                        .range(0. ..=(slot.crop_end - 1. / 48000.).max(0.))
                                        .speed(0.01)
                                        .suffix(" s"),
                                )
                                .changed();
                            ui.label("End");
                            selection_changed |= ui
                                .add(
                                    egui::DragValue::new(&mut slot.crop_end)
                                        .range(
                                            (slot.crop_start + 1. / 48000.).min(duration)
                                                ..=duration,
                                        )
                                        .speed(0.01)
                                        .suffix(" s"),
                                )
                                .changed();
                            if ui.button("Reset selection").clicked() {
                                slot.crop_start = 0.;
                                slot.crop_end = duration;
                                selection_changed = true;
                            }
                            ui.label(format!("Full recording: {duration:.2} s"));
                        });
                    } else {
                        ui.label(if self.recording_ui.loading.contains(&key) {
                            "Loading waveform..."
                        } else if slot.asset.is_some() {
                            "Recording file is unavailable. Capture again to replace it."
                        } else {
                            "Empty recording slot"
                        });
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(
                                config.enabled && self.status.running && available > 0.,
                                egui::Button::new("Capture recording"),
                            )
                            .clicked()
                        {
                            actions.push((slot.id, SlotAction::Capture));
                        }
                        if ui
                            .add_enabled(slot.asset.is_some(), egui::Button::new("Play preview"))
                            .clicked()
                        {
                            actions.push((slot.id, SlotAction::Preview));
                        }
                        if ui
                            .add_enabled(slot.asset.is_some(), egui::Button::new("Send"))
                            .clicked()
                        {
                            actions.push((slot.id, SlotAction::Send));
                        }
                        if ui
                            .add_enabled(slot.asset.is_some(), egui::Button::new("Preview & send"))
                            .clicked()
                        {
                            actions.push((slot.id, SlotAction::PreviewAndSend));
                        }
                        if ui
                            .add_enabled(slot.asset.is_some(), egui::Button::new("Save WAV / MP3"))
                            .clicked()
                        {
                            save = Some(slot.id);
                        }
                        if ui
                            .add_enabled(
                                slot.asset.is_some(),
                                egui::Button::new("Delete recording"),
                            )
                            .clicked()
                        {
                            actions.push((slot.id, SlotAction::Delete));
                        }
                    });
                });
            });
            if selection_changed {
                changed = true;
                selection_changes.push(slot.id);
            }
            ui.add_space(10.);
        }
        if ui
            .add_enabled(
                config.slots.len() < 64,
                egui::Button::new("+ Add recording slot"),
            )
            .clicked()
        {
            config
                .slots
                .push(RecordSlot::new(&format!("Slot {}", config.slots.len() + 1)));
            changed = true;
        }
        if changed {
            let old = self
                .project
                .nodes
                .iter()
                .find(|n| n.id == node)
                .unwrap()
                .recording
                .clone();
            self.project
                .nodes
                .iter_mut()
                .find(|n| n.id == node)
                .unwrap()
                .recording = config;
            if let Err(error) = piratewave_core::graph::validate(&self.project) {
                self.project
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == node)
                    .unwrap()
                    .recording = old;
                self.notify_error(error);
            } else {
                for slot in selection_changes {
                    self.stop_recording_playback(node, Some(slot));
                }
                self.changed(false);
            }
        }
        if let Some(enabled) = set_enabled {
            self.set_recording(node, enabled);
        }
        if node_keys {
            self.open_node_keybinds(node);
        }
        if open_effects {
            self.selected = node;
            self.view = View::Effects;
        }
        if stop_playback {
            self.stop_recording_playback(node, None);
        }
        for (slot, action) in actions {
            self.recording_slot_action(node, slot, action);
        }
        if let Some(slot) = remove {
            self.remove_recording_slot(node, slot);
        }
        if let Some(slot) = keys {
            self.open_recording_slot_keybinds(node, slot);
        }
        if let Some(slot) = save {
            self.export_recording_slot(node, slot);
        }
    }
}
fn draw_waveform(
    ui: &mut egui::Ui,
    key: (u64, u64),
    slot: &mut RecordSlot,
    audio: &Wave,
    drag: &mut Option<((u64, u64), bool)>,
) -> bool {
    let duration = audio.duration;
    if duration <= 0. {
        return false;
    }
    if slot.crop_end <= 0. {
        slot.crop_end = duration;
    }
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 96.),
        egui::Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4., theme::surface(Color32::from_rgb(12, 14, 19)));
    let start = rect.left() + slot.crop_start / duration * rect.width();
    let end = rect.left() + slot.crop_end / duration * rect.width();
    painter.rect_filled(
        Rect::from_min_max(Pos2::new(start, rect.top()), Pos2::new(end, rect.bottom())),
        0.,
        theme::surface(Color32::from_rgb(37, 51, 51)),
    );
    for (i, range) in audio.bins.iter().enumerate() {
        let x = rect.left() + i as f32 / audio.bins.len() as f32 * rect.width();
        painter.line_segment(
            [
                Pos2::new(x, rect.center().y - range[1].clamp(-1., 1.) * 40.),
                Pos2::new(x, rect.center().y - range[0].clamp(-1., 1.) * 40.),
            ],
            Stroke::new(1_f32, theme::palette().accent),
        );
    }
    for x in [start, end] {
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(2_f32, theme::palette().green),
        );
    }
    if response.drag_started() || response.clicked() {
        if let Some(p) = response.interact_pointer_pos() {
            *drag = Some((key, (p.x - start).abs() <= (p.x - end).abs()));
        }
    }
    let mut changed = false;
    if response.dragged() || response.clicked() {
        if let (Some((owner, is_start)), Some(p)) = (*drag, response.interact_pointer_pos()) {
            if owner == key {
                let value = ((p.x - rect.left()) / rect.width() * duration).clamp(0., duration);
                if is_start {
                    slot.crop_start = value.min(slot.crop_end - 1. / 48000.);
                } else {
                    slot.crop_end = value.max(slot.crop_start + 1. / 48000.);
                }
                changed = true;
            }
        }
    }
    if response.drag_stopped() {
        *drag = None;
    }
    changed
}

impl PirateWaveApp {
    pub(super) fn recording_smoke_fixture(&mut self, filled: bool) {
        if !self.project.nodes.iter().any(|n| n.id == 91) {
            let mut node = Node::new(91, "Instant replay", NodeKind::Recording, [710., 380.]);
            let mut slot = RecordSlot::new("Last phrase");
            slot.id = 9101;
            node.recording.slots.push(slot);
            self.project.nodes.push(node);
            let _ = self.project.connect(1, 91);
            let _ =
                self.project
                    .connect_port(91, 6, piratewave_core::project::OutputPort::Passthrough);
            let _ = self
                .project
                .connect_port(91, 4, piratewave_core::project::OutputPort::Preview);
            let _ = self
                .project
                .connect_port(91, 5, piratewave_core::project::OutputPort::Send);
        }
        if filled && !self.recording_ui.waves.contains_key(&(91, 9101)) {
            let frames: Vec<_> = (0..96000)
                .map(|i| {
                    let t = i as f32 / 48000.;
                    let envelope = (std::f32::consts::PI * t).sin().abs();
                    let x = (std::f32::consts::TAU * 180. * t).sin() * envelope * 0.6;
                    [x; 2]
                })
                .collect();
            let audio = Audio::from_frames(frames, "smoke-recording.wav".into());
            let slot = &mut self
                .project
                .nodes
                .iter_mut()
                .find(|n| n.id == 91)
                .unwrap()
                .recording
                .slots[0];
            slot.asset = Some(audio.asset.clone());
            slot.crop_start = 0.25;
            slot.crop_end = 1.65;
            self.recording_ui.waves.insert(
                (91, 9101),
                Wave {
                    asset: audio.asset.clone(),
                    duration: audio.clip.duration(),
                    bins: audio.waveform.clone(),
                },
            );
            self.recording_ui.cache.insert((91, 9101), audio);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stopping_recording_playback_cancels_only_the_matching_pending_plays() {
        let mut app = crate::ui::tests::app();
        let (engine, rx) = Engine::test_channel();
        app.engine = engine;
        for key in [(91, 1), (91, 2), (92, 1)] {
            app.recording_ui
                .pending_play
                .insert(key, SlotAction::PreviewAndSend);
        }
        app.stop_recording_playback(91, Some(1));
        assert!(!app.recording_ui.pending_play.contains_key(&(91, 1)));
        assert!(app.recording_ui.pending_play.contains_key(&(91, 2)));
        assert!(app.recording_ui.pending_play.contains_key(&(92, 1)));
        assert!(matches!(
            rx.try_recv().unwrap(),
            Command::StopRecordingPlayback {
                node: 91,
                slot: Some(1)
            }
        ));
        app.stop_recording_playback(91, None);
        assert_eq!(app.recording_ui.pending_play.len(), 1);
        assert!(app.recording_ui.pending_play.contains_key(&(92, 1)));
        assert!(matches!(
            rx.try_recv().unwrap(),
            Command::StopRecordingPlayback {
                node: 91,
                slot: None
            }
        ));
    }
}
