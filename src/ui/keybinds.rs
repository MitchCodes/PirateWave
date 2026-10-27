use super::*;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum SettingsSection {
    General,
    Audio,
    Keybinds,
    Training,
}
impl PirateWaveApp {
    pub(super) fn handle_shortcut(&mut self, action: ShortcutAction) {
        if matches!(
            action,
            ShortcutAction::App(AppAction::StopSounds | AppAction::StopAudio)
        ) || (action == ShortcutAction::App(AppAction::ToggleAudio) && self.status.running)
        {
            self.tools_ui.stop();
        }
        if !action.exists(&self.project) {
            return;
        }
        match action {
            ShortcutAction::Extension { id, effect, action } => {
                match piratewave_core::extensions::apply_action(
                    &mut self.project,
                    id,
                    effect,
                    action,
                ) {
                    Ok(()) => self.changed(false),
                    Err(e) => self.notify_error(e),
                }
            }
            ShortcutAction::RecordingSlot { id, slot, action } => {
                self.recording_slot_action(id, slot, action)
            }
            ShortcutAction::SwitchOutput { id, output } => {
                if self.project.select_switch_output(id, output) {
                    self.changed(false);
                }
            }
            ShortcutAction::Effect { id, effect, action } => {
                if let Some(effect) = self
                    .project
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == id)
                    .and_then(|n| n.effects.iter_mut().find(|e| e.id == effect))
                {
                    action.apply(effect);
                    self.changed(false);
                }
            }
            ShortcutAction::Sound(id) => self.play(id),
            ShortcutAction::App(action) => {
                self.command(Command::for_app_action(action, self.project.clone()))
            }
            ShortcutAction::Node { id, action } => {
                if action == NodeAction::StopPlayingRecording {
                    self.stop_recording_playback(id, None);
                    return;
                }
                if matches!(
                    action,
                    NodeAction::StartRecording | NodeAction::StopRecording
                ) {
                    self.set_recording(id, action == NodeAction::StartRecording);
                    return;
                }
                if matches!(
                    action,
                    NodeAction::PlayNote | NodeAction::ReleaseNote | NodeAction::AllNotesOff
                ) {
                    if let Some(node) = self
                        .project
                        .nodes
                        .iter()
                        .find(|n| n.id == id && matches!(n.kind, NodeKind::VstInstrument))
                    {
                        if action == NodeAction::AllNotesOff {
                            self.command(Command::AllNotesOff(Some(id)));
                        } else if let Some(plugin) = &node.plugin {
                            self.command(Command::Note {
                                id,
                                note: plugin.note,
                                velocity: plugin.velocity,
                                on: action == NodeAction::PlayNote,
                            });
                        }
                    }
                    return;
                }
                if let Some(node) = self.project.nodes.iter_mut().find(|n| n.id == id) {
                    let before = node.clone();
                    let changed = action.apply(node);
                    if let Err(error) = piratewave_core::graph::validate(&self.project) {
                        if let Some(node) = self.project.nodes.iter_mut().find(|n| n.id == id) {
                            *node = before;
                        }
                        self.notify_error(error);
                        return;
                    }
                    match changed {
                        Ok(true) => self.changed(false),
                        Ok(false) => {}
                        Err(error) => self.notify_error(error),
                    }
                }
            }
        }
    }
    pub(super) fn remove_audio_node(&mut self, id: u64) {
        let assets: Vec<_> = self
            .project
            .nodes
            .iter()
            .filter(|n| n.id == id)
            .flat_map(|n| &n.recording.slots)
            .filter_map(|s| s.asset.clone())
            .collect();
        let restart =
            self.project.nodes.iter().any(|n| {
                n.id == id && !matches!(n.kind, NodeKind::VoiceChanger | NodeKind::Recording)
            });
        self.project.remove_node(id);
        for asset in assets {
            self.cleanup_recording_asset(asset);
        }
        self.pending_port = None;
        self.selected_edge = None;
        if self.layout.routing_focus == Some(id) {
            self.layout.routing_focus = None;
        }
        if self.layout.mixer_focus == Some(id) {
            self.layout.mixer_focus = None;
        }
        if self
            .layout
            .routing_drag
            .is_some_and(|(owner, _)| owner == id)
        {
            self.layout.routing_drag = None;
        }
        if self.selected == id {
            self.selected = self.project.nodes.first().map_or(0, |n| n.id);
        }
        if self.view == View::NodeKeybinds(id)
            || self.view == View::PluginNode(id)
            || self.view == View::VoiceNode(id)
            || self.view == View::Ducking(id)
            || self.view == View::Recording(id)
            || matches!(self.view,View::RecordingSlotKeybinds(owner,_) if owner==id)
        {
            self.view = View::Mixer;
            self.node_binding_draft.clear();
            self.node_extra_binding_draft.clear();
        }
        self.refresh_shortcuts();
        self.changed(restart);
    }
    pub(super) fn open_node_keybinds(&mut self, id: u64) {
        if let Some(node) = self.project.nodes.iter().find(|n| n.id == id) {
            if NodeAction::for_kind(&node.kind).is_empty() && !matches!(node.kind, NodeKind::Switch)
            {
                return;
            }
            self.node_binding_draft = node.keybinds.clone();
            self.node_extra_binding_draft.clear();
            for (action, _) in piratewave_core::extensions::action_rows(node) {
                if let ShortcutAction::Extension {
                    effect,
                    action: key,
                    ..
                } = action
                {
                    if let Some(text) =
                        piratewave_core::extensions::action_binding(node, effect, key)
                    {
                        self.node_extra_binding_draft.insert(action, text.into());
                    }
                }
            }
            for (output, key) in &node.switch_keybinds {
                self.node_extra_binding_draft.insert(
                    ShortcutAction::SwitchOutput {
                        id,
                        output: *output,
                    },
                    key.clone(),
                );
            }
            for slot in &node.recording.slots {
                for (action, key) in &slot.keybinds {
                    self.node_extra_binding_draft.insert(
                        ShortcutAction::RecordingSlot {
                            id,
                            slot: slot.id,
                            action: *action,
                        },
                        key.clone(),
                    );
                }
            }
            for effect in &node.effects {
                for (action, key) in &effect.keybinds {
                    self.node_extra_binding_draft.insert(
                        ShortcutAction::Effect {
                            id,
                            effect: effect.id,
                            action: *action,
                        },
                        key.clone(),
                    );
                }
            }
            self.view = View::NodeKeybinds(id);
        }
    }
    pub(super) fn settings_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.settings_section,
                SettingsSection::General,
                "General",
            );
            ui.selectable_value(
                &mut self.settings_section,
                SettingsSection::Audio,
                "Audio & diagnostics",
            );
            ui.selectable_value(
                &mut self.settings_section,
                SettingsSection::Keybinds,
                "Keybinds",
            );
            ui.selectable_value(
                &mut self.settings_section,
                SettingsSection::Training,
                "Voice Training & Storage",
            );
        });
        ui.add_space(18.0);
        match self.settings_section {
            SettingsSection::Training => self.training_storage_page(ui),
            SettingsSection::General => {
                ui.heading("Appearance");
                let previous_theme = self.app_settings.ui.theme;
                ui.horizontal(|ui| {
                    ui.label("Theme");
                    egui::ComboBox::from_id_salt("theme-setting")
                        .selected_text(self.app_settings.ui.theme.label())
                        .show_ui(ui, |ui| {
                            for theme in piratewave_core::shortcuts::Theme::ALL {
                                ui.selectable_value(
                                    &mut self.app_settings.ui.theme,
                                    theme,
                                    theme.label(),
                                );
                            }
                        });
                });
                if previous_theme != self.app_settings.ui.theme {
                    theme::apply(ui.ctx(), self.app_settings.ui.theme);
                    self.layout_preferences_changed();
                }
                ui.add_space(16.);
                self.extension_settings(ui);
                self.controller_settings(ui);
                ui.heading("Startup preferences");
                ui.label("Choose how PirateWave opens. Changes are saved immediately and take effect next time you launch it.");
                ui.add_space(16.0);
                let mut startup = self.app_settings.startup.clone();
                show_card(ui, |ui| {
                    ui.checkbox(&mut startup.start_with_windows, "Start with Windows");
                    ui.label(RichText::new("Open PirateWave when you sign in to your Windows account. No installer or administrator permission is needed. If you move PirateWave.exe, turn this off and on again.").small().color(theme::palette().muted));
                    ui.add_space(12.0);
                    ui.checkbox(&mut startup.start_minimized, "Start minimized");
                    ui.label(RichText::new("Open minimized to the taskbar, including when you launch PirateWave yourself. Click its taskbar icon to restore the window.").small().color(theme::palette().muted));
                    ui.add_space(12.0);
                    ui.checkbox(
                        &mut startup.start_audio_automatically,
                        "Start audio automatically",
                    );
                    ui.label(RichText::new("Start processing your saved workspace when PirateWave opens. This can activate microphones and playback devices. Enabling this saves the current workspace for startup.").small().color(theme::palette().muted));
                });
                if startup != self.app_settings.startup {
                    self.apply_startup_settings(startup);
                }
            }
            SettingsSection::Audio => self.audio_settings(ui),
            SettingsSection::Keybinds => {
                ui.heading("Application keybinds");
                ui.label(RichText::new("Global shortcuts work while another app is focused. Leave a field empty for Unbound.").color(theme::palette().muted));
                ui.add_space(16.0);
                let mut record = None;
                show_card(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    for action in AppAction::ALL {
                        if binding_row(
                            ui,
                            action.label(),
                            self.settings_draft.keybinds.entry(action).or_default(),
                        ) {
                            record = Some(action);
                        }
                    }
                });
                if let Some(action) = record {
                    self.begin_shortcut_recording(
                        ui.ctx(),
                        ShortcutAction::App(action),
                        action.label().into(),
                    );
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if ui.button("Apply keybinds").clicked() {
                        self.apply_app_bindings();
                    }
                    if ui.button("Revert edits").clicked() {
                        self.settings_draft.keybinds = self.app_settings.keybinds.clone();
                    }
                    if ui.button("Restore defaults").clicked() {
                        self.settings_draft.keybinds = AppSettings::default().keybinds;
                    }
                });
                ui.add_space(15.0);
                ui.label(RichText::new("Click Record and press your shortcut, or type it manually. Recorded changes take effect when you click Apply keybinds.").small().color(theme::palette().muted));
                ui.label(RichText::new("Sound-pad playback shortcuts stay in Soundboard. Node controls are configured through each mixer's Keybinds button.").small().color(theme::palette().muted));
                self.shortcut_messages(ui);
            }
        }
    }
    fn apply_app_bindings(&mut self) {
        let mut candidate = self.app_settings.clone();
        candidate.keybinds = self.settings_draft.keybinds.clone();
        let errors = if let Some(h) = &mut self.hotkeys {
            h.update(&self.project, &candidate)
        } else {
            vec!["Global shortcuts are unavailable".into()]
        };
        if !errors.is_empty() {
            self.shortcut_errors = errors;
            self.notify("Keybinds were not applied. Resolve the conflicts below; previous bindings remain active.");
            return;
        }
        self.shortcut_errors.clear();
        self.app_settings = candidate;
        self.settings_dirty = true;
        match storage::save_settings(&storage::settings_path(), &self.app_settings) {
            Ok(()) => {
                self.settings_dirty = false;
                self.notify("Application keybinds applied and saved");
            }
            Err(error) => self.notify_error(format!(
                "Keybinds applied for this session, but settings could not be saved: {error:#}"
            )),
        }
    }
    fn apply_startup_settings(&mut self, startup: piratewave_core::shortcuts::StartupSettings) {
        let mut candidate = self.app_settings.clone();
        candidate.startup = startup;
        let result = if self.smoke {
            Ok(())
        } else {
            crate::startup::apply(
                &self.app_settings.startup,
                &candidate.startup,
                crate::startup::configure,
                || {
                    if candidate.startup.start_audio_automatically {
                        storage::save(&storage::default_path(), &self.project)?;
                    }
                    storage::save_settings(&storage::settings_path(), &candidate)
                },
            )
        };
        match result {
            Ok(()) => {
                self.app_settings = candidate;
                self.settings_draft.startup = self.app_settings.startup.clone();
                self.notify(
                    "Startup preferences saved. They take effect next time PirateWave opens.",
                );
            }
            Err(error) => {
                self.notify_error(format!("Startup preferences could not be saved: {error:#}"))
            }
        }
    }
    pub(super) fn node_keybinds_page(&mut self, ui: &mut egui::Ui, id: u64) {
        self.node_keybinds_filtered(ui, id, None);
    }
    pub(super) fn recording_slot_keybinds_page(&mut self, ui: &mut egui::Ui, id: u64, slot: u64) {
        self.node_keybinds_filtered(ui, id, Some(slot));
    }
    fn node_keybinds_filtered(&mut self, ui: &mut egui::Ui, id: u64, slot_filter: Option<u64>) {
        let Some(node) = self.project.nodes.iter().find(|n| n.id == id).cloned() else {
            ui.label("This node has been removed.");
            if ui.button("Back to mixer").clicked() {
                self.view = View::Mixer;
            }
            return;
        };
        let actions = if slot_filter.is_some() {
            &[][..]
        } else {
            NodeAction::for_kind(&node.kind)
        };
        let mut extra_rows = if slot_filter.is_none() {
            piratewave_core::extensions::action_rows(&node)
        } else {
            vec![]
        };
        if matches!(node.kind, NodeKind::Switch) {
            let mut destinations = HashSet::new();
            for c in self
                .project
                .connections
                .iter()
                .filter(|c| c.from == id && destinations.insert(c.to))
            {
                if let Some(destination) = self.project.nodes.iter().find(|n| n.id == c.to) {
                    extra_rows.push((
                        ShortcutAction::SwitchOutput { id, output: c.to },
                        format!("Activate {}", destination.name),
                    ));
                }
            }
        }
        if node.kind.has_effect_controls() && slot_filter.is_none() {
            for (i, effect) in node.effects.iter().enumerate() {
                let effect_name = self
                    .registry
                    .get(&effect.kind)
                    .map_or(effect.kind.as_str(), |f| f.name);
                for action in piratewave_core::shortcuts::EffectAction::ALL {
                    extra_rows.push((
                        ShortcutAction::Effect {
                            id,
                            effect: effect.id,
                            action,
                        },
                        format!("{}: {} - {}", i + 1, effect_name, action.label()),
                    ));
                }
            }
        }
        if let Some(slot_id) = slot_filter {
            if let Some(slot) = node.recording.slots.iter().find(|s| s.id == slot_id) {
                for action in piratewave_core::recording::SlotAction::ALL {
                    extra_rows.push((
                        ShortcutAction::RecordingSlot {
                            id,
                            slot: slot.id,
                            action,
                        },
                        format!("{}: {}", slot.name, action.label()),
                    ));
                }
            }
        }
        let name = node.name.clone();
        ui.horizontal(|ui| {
            if ui
                .button(if matches!(node.kind, NodeKind::Recording) {
                    "< Back to recording"
                } else if matches!(node.kind, NodeKind::AutoDucking) {
                    "< Back to ducking"
                } else {
                    "< Back to mixer"
                })
                .clicked()
            {
                self.view = if matches!(node.kind, NodeKind::Recording) {
                    View::Recording(id)
                } else if matches!(node.kind, NodeKind::AutoDucking) {
                    View::Ducking(id)
                } else {
                    View::Mixer
                };
            }
            ui.heading(&name);
            ui.label(
                RichText::new(node.kind.label())
                    .small()
                    .color(theme::palette().muted),
            );
        });
        ui.add_space(15.0);
        ui.label(
            RichText::new("Optional global shortcuts for this node. Empty fields are unbound.")
                .color(theme::palette().muted),
        );
        ui.add_space(15.0);
        let mut record = None;
        show_card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for action in actions {
                if binding_row(
                    ui,
                    action.label(),
                    self.node_binding_draft.entry(*action).or_default(),
                ) {
                    record = Some((
                        ShortcutAction::Node {
                            id,
                            action: *action,
                        },
                        action.label().to_owned(),
                    ));
                }
            }
            if !extra_rows.is_empty() {
                ui.separator();
                ui.strong(if matches!(node.kind, NodeKind::Switch) {
                    "Switch outputs"
                } else if slot_filter.is_some() {
                    "Recording slot"
                } else {
                    "Individual effects"
                });
            }
            for (action, label) in &extra_rows {
                if binding_row(
                    ui,
                    label,
                    self.node_extra_binding_draft.entry(*action).or_default(),
                ) {
                    record = Some((*action, label.clone()));
                }
            }
        });
        if let Some((action, label)) = record {
            self.begin_shortcut_recording(ui.ctx(), action, format!("{name}: {label}"));
        }
        ui.add_space(15.0);
        ui.horizontal(|ui| {
            if ui.button("Apply keybinds").clicked() {
                self.apply_node_bindings(id);
            }
            if ui.button("Revert edits").clicked() {
                self.open_node_keybinds(id);if let Some(slot)=slot_filter{self.view=View::RecordingSlotKeybinds(id,slot);}
            }
            if ui.button("Clear all").clicked() {
                if slot_filter.is_none(){self.node_binding_draft.clear();}
                if let Some(slot)=slot_filter{self.node_extra_binding_draft.retain(|action,_|!matches!(action,ShortcutAction::RecordingSlot{slot:other,..} if *other==slot));}else if matches!(node.kind,NodeKind::Recording){self.node_extra_binding_draft.retain(|action,_|matches!(action,ShortcutAction::RecordingSlot{..}));}else{self.node_extra_binding_draft.clear();}
            }
        });
        ui.add_space(12.0);
        ui.label(RichText::new(if slot_filter.is_some(){"These shortcuts apply only to this slot. Removing the slot unregisters its keys."}else if matches!(node.kind,NodeKind::Recording){"Control rolling capture and the playback/export effects rack. Effects never change the captured take or live passthrough."}else if matches!(node.kind,NodeKind::AutoDucking){"Bypass removes attenuation while preserving lookahead timing and all routes."}else if matches!(node.kind,NodeKind::Extension {..}){"Bypass uses declared routes only; unmapped outputs are silent. Extension actions update persistent parameters."}else if actions.is_empty(){"Select one connected output with each shortcut. Removed routes unregister their shortcuts."}else if actions==NodeAction::PROCESSOR{"Changes take effect when applied. Disable passes audio through without changing the effects in your chain."}else{"Changes take effect when applied. Volume shortcuts adjust 1 dB within the mixer's -60 to +12 dB range."}).small().color(theme::palette().muted));
        self.shortcut_messages(ui);
    }
    fn apply_node_bindings(&mut self, id: u64) {
        let mut candidate = self.project.clone();
        let Some(node) = candidate.nodes.iter_mut().find(|n| n.id == id) else {
            return;
        };
        for (shortcut, _) in piratewave_core::extensions::action_rows(node) {
            if let ShortcutAction::Extension { effect, action, .. } = shortcut {
                let text = self
                    .node_extra_binding_draft
                    .get(&shortcut)
                    .cloned()
                    .unwrap_or_default();
                let _ = piratewave_core::extensions::set_binding(node, effect, action, text);
            }
        }
        node.keybinds = self
            .node_binding_draft
            .iter()
            .filter(|(_, key)| !key.trim().is_empty())
            .map(|(action, key)| (*action, key.trim().into()))
            .collect();
        if matches!(node.kind, NodeKind::Switch) {
            node.switch_keybinds = self
                .node_extra_binding_draft
                .iter()
                .filter_map(|(action, key)| match action {
                    ShortcutAction::SwitchOutput { id: owner, output }
                        if *owner == id
                            && !key.trim().is_empty()
                            && candidate
                                .connections
                                .iter()
                                .any(|c| c.from == id && c.to == *output) =>
                    {
                        Some((*output, key.trim().to_owned()))
                    }
                    _ => None,
                })
                .collect();
        }
        for effect in &mut node.effects {
            effect.keybinds = self
                .node_extra_binding_draft
                .iter()
                .filter_map(|(action, key)| match action {
                    ShortcutAction::Effect {
                        id: owner,
                        effect: effect_id,
                        action,
                    } if *owner == id && *effect_id == effect.id && !key.trim().is_empty() => {
                        Some((*action, key.trim().to_owned()))
                    }
                    _ => None,
                })
                .collect();
        }
        for slot in &mut node.recording.slots {
            slot.keybinds = self
                .node_extra_binding_draft
                .iter()
                .filter_map(|(action, key)| match action {
                    ShortcutAction::RecordingSlot {
                        id: owner,
                        slot: target,
                        action,
                    } if *owner == id && *target == slot.id && !key.trim().is_empty() => {
                        Some((*action, key.trim().into()))
                    }
                    _ => None,
                })
                .collect();
        }
        let errors = if let Some(h) = &mut self.hotkeys {
            h.update(&candidate, &self.app_settings)
        } else {
            vec!["Global shortcuts are unavailable".into()]
        };
        if !errors.is_empty() {
            self.shortcut_errors = errors;
            self.notify("Node keybinds were not applied. Previous bindings remain active.");
            return;
        }
        self.project = candidate;
        self.shortcut_errors.clear();
        self.changed(false);
        self.notify("Node keybinds applied; saving with this project");
    }
    fn shortcut_messages(&self, ui: &mut egui::Ui) {
        for error in &self.shortcut_errors {
            ui.colored_label(theme::palette().peach, error);
        }
    }
    fn audio_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Audio & diagnostics");
        ui.label(format!("Log file: {}", crate::logging::path().display()));
        ui.add_space(12.0);
        show_card(ui, |ui| {
            ui.label("Native WASAPI shared-mode audio · 48,000 Hz processing");
            ui.label("128-frame DSP blocks · 2,048-frame output queue target");
            ui.label("Devices use their native sample rates; linear conversion aligns streams.");
            ui.colored_label(
                theme::palette().muted,
                "Input and output reserves are about 43 ms each, plus device/effect latency.",
            );
            ui.separator();
            for (label, value) in [
                ("Capture frames", self.status.input_frames.to_string()),
                ("Output frames", self.status.output_frames.to_string()),
                (
                    "Output device sample rates",
                    format!("{:?}", self.status.output_rates),
                ),
                (
                    "Output underflow callbacks",
                    self.status.underflows.to_string(),
                ),
                (
                    "Input buffer underruns",
                    self.status.input_underflows.to_string(),
                ),
                ("Dropped queue frames", self.status.overflows.to_string()),
                (
                    "Render worker load",
                    format!("{:.2}%", self.status.render_load),
                ),
            ] {
                ui.horizontal(|ui| {
                    ui.add_sized([220.0, 20.0], egui::Label::new(label));
                    ui.label(value);
                });
            }
            ui.separator();
            if ui.button("Refresh device list").clicked() {
                match audio_io::devices() {
                    Ok(devices) => {
                        self.devices = devices;
                        self.endpoint_states =
                            crate::windows_audio::cable_endpoints().unwrap_or_default();
                        self.notify("Audio devices refreshed");
                    }
                    Err(error) => self.notify_error(format!("Device discovery failed: {error}")),
                }
            }
            ui.label("For Discord or OBS, select a virtual cable's playback endpoint as an output bus and its recording endpoint in the receiving app.");
            ui.colored_label(
                theme::palette().peach,
                "Microphone -> Monitor may feed back through speakers. Prefer headphones.",
            );
            for endpoint in &self.endpoint_states {
                ui.colored_label(
                    if endpoint.muted {
                        theme::palette().peach
                    } else {
                        theme::palette().muted
                    },
                    format!(
                        "{}: {} (Windows)",
                        endpoint.name,
                        if endpoint.muted {
                            "theme::palette().muted"
                        } else {
                            "unmuted"
                        }
                    ),
                );
            }
            ui.separator();
            ui.small(format!("Project: {}", self.path.display()));
            ui.small(format!(
                "Application settings: {}",
                storage::settings_path().display()
            ));
        });
    }
}
fn binding_row(ui: &mut egui::Ui, label: &str, value: &mut String) -> bool {
    let mut record = false;
    ui.horizontal(|ui| {
        ui.add_sized([220.0, 28.0], egui::Label::new(label));
        ui.add_sized(
            [240.0, 28.0],
            egui::TextEdit::singleline(value)
                .desired_width(240.0)
                .hint_text("Unbound")
                .char_limit(256),
        );
        record = ui.small_button("Record").clicked();
        if ui.small_button("Clear").clicked() {
            value.clear();
        }
    });
    record
}
