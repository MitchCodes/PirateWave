use super::*;
use piratewave_core::plugins::{NodePlugin, PluginDescriptor, PluginRole};

pub(super) type PluginCapture = (
    HashMap<u64, (PluginDescriptor, piratewave_core::plugins::PluginState)>,
    Vec<String>,
    crate::plugins::PluginEdits,
);
pub(super) enum PluginChange {
    Replace(PluginDescriptor),
    Remove,
}
struct PendingPluginChange {
    id: u64,
    descriptor: PluginDescriptor,
    path: PathBuf,
    action: PluginChange,
    started: Instant,
    reply: Option<mpsc::Receiver<PluginCapture>>,
    request_parameters: Option<std::collections::BTreeMap<u32, f64>>,
    result: Option<(
        Option<piratewave_core::plugins::PluginState>,
        Option<String>,
    )>,
}

pub(super) struct PluginUi {
    pub library: crate::plugins::Library,
    scan: Option<mpsc::Receiver<Result<Vec<PluginDescriptor>, String>>>,
    pub selection: Option<usize>,
    pub held: HashSet<(u64, u8)>,
    error: String,
    editor_reply: Option<mpsc::Receiver<Result<(), String>>>,
    pub editor_used: bool,
    pending_change: Option<PendingPluginChange>,
    pub pending_export: Option<PathBuf>,
}
impl PluginUi {
    pub(super) fn change_pending(&self) -> bool {
        self.pending_change.is_some()
    }
    pub fn new(smoke: bool) -> Self {
        Self {
            library: if smoke {
                Default::default()
            } else {
                crate::plugins::Library::load()
            },
            scan: None,
            selection: None,
            held: HashSet::new(),
            error: String::new(),
            editor_reply: None,
            editor_used: false,
            pending_change: None,
            pending_export: None,
        }
    }
    pub fn choice(&self, role: PluginRole) -> Option<PluginDescriptor> {
        self.selection
            .and_then(|i| self.library.plugins.get(i))
            .filter(|p| p.role == role)
            .cloned()
    }
}
impl PirateWaveApp {
    pub(super) fn defer_plugin_change(&mut self, id: u64, action: PluginChange) -> bool {
        if !self.status.running
            || !(self.app_settings.experimental_vst_editors || self.plugin_ui.editor_used)
        {
            return false;
        }
        let Some(descriptor) = self
            .project
            .nodes
            .iter()
            .find(|node| node.id == id)
            .and_then(|node| node.plugin.as_ref())
            .map(|plugin| plugin.descriptor.clone())
        else {
            return false;
        };
        if self.plugin_ui.pending_change.is_some() || self.editing.history_pending() {
            self.notify_error("A plugin change is already waiting for state capture");
            return true;
        }
        if let Err(error) = self.engine.send(Command::ClosePluginEditor(id)) {
            self.notify_error(format!(
                "Could not capture plugin state before the change: {error}"
            ));
            return true;
        }
        self.plugin_ui.pending_change = Some(PendingPluginChange {
            id,
            descriptor,
            path: self.path.clone(),
            action,
            started: Instant::now(),
            reply: None,
            request_parameters: None,
            result: None,
        });
        self.notify("Saving plugin state before the change…");
        true
    }
    fn poll_plugin_change(&mut self, ctx: &egui::Context) {
        let Some(mut pending) = self.plugin_ui.pending_change.take() else {
            return;
        };
        let current = self
            .project
            .nodes
            .iter()
            .find(|node| node.id == pending.id)
            .and_then(|node| node.plugin.as_ref());
        if self.path != pending.path
            || !current.is_some_and(|plugin| plugin.descriptor.same_plugin(&pending.descriptor))
        {
            return;
        }
        if pending.result.is_none() && pending.started.elapsed() >= Duration::from_secs(6) {
            pending.result = Some((
                None,
                Some(
                    "Plugin state capture timed out; the previous saved state was retained.".into(),
                ),
            ));
        }
        if pending.result.is_none() {
            if let Some(reply) = &pending.reply {
                match reply.try_recv() {
                    Ok((states, errors, edits)) => {
                        pending.reply = None;
                        self.apply_plugin_editor_edits(&edits);
                        let changed = self
                            .project
                            .nodes
                            .iter()
                            .find(|node| node.id == pending.id)
                            .and_then(|node| node.plugin.as_ref())
                            .is_some_and(|plugin| {
                                Some(&plugin.parameters) != pending.request_parameters.as_ref()
                            });
                        let error = errors
                            .into_iter()
                            .find(|error| error.starts_with(&format!("Plugin {}:", pending.id)));
                        if !changed
                            && !error
                                .as_ref()
                                .is_some_and(|error| error.contains("state capture is pending"))
                        {
                            let state = if error.is_none() {
                                states
                                    .get(&pending.id)
                                    .filter(|(descriptor, _)| {
                                        descriptor.same_plugin(&pending.descriptor)
                                    })
                                    .map(|(_, state)| state.clone())
                            } else {
                                None
                            };
                            pending.result = Some((state, error));
                        }
                    }
                    Err(mpsc::TryRecvError::Disconnected) => pending.result = Some((
                        None,
                        Some(
                            "Plugin state capture stopped; the previous saved state was retained."
                                .into(),
                        ),
                    )),
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            if pending.result.is_none() && pending.reply.is_none() {
                let (tx, rx) = mpsc::sync_channel(1);
                pending.request_parameters = self
                    .project
                    .nodes
                    .iter()
                    .find(|node| node.id == pending.id)
                    .and_then(|node| node.plugin.as_ref())
                    .map(|plugin| plugin.parameters.clone());
                match self.engine.send(Command::CapturePluginStates(false, tx)) {
                    Ok(()) => pending.reply = Some(rx),
                    Err(error) => pending.result = Some((None, Some(format!("Plugin state capture failed; the previous saved state was retained: {error}")))),
                }
            }
        }
        if pending
            .result
            .as_ref()
            .is_some_and(|(state, _)| state.is_some())
            && self
                .project
                .nodes
                .iter()
                .find(|node| node.id == pending.id)
                .and_then(|node| node.plugin.as_ref())
                .is_some_and(|plugin| {
                    Some(&plugin.parameters) != pending.request_parameters.as_ref()
                })
        {
            pending.result = None;
        }
        let active_input = ctx.input(|i| i.pointer.any_down() || !i.events.is_empty());
        if pending.result.is_some() && !active_input {
            let (state, warning) = pending.result.take().unwrap();
            if let Some(state) = state {
                if let Some(plugin) = self
                    .project
                    .nodes
                    .iter_mut()
                    .find(|node| node.id == pending.id)
                    .and_then(|node| node.plugin.as_mut())
                {
                    plugin.state = Some(state);
                }
            }
            self.remember_plugin_states();
            let before = self.edit_snapshot();
            match pending.action {
                PluginChange::Remove => self.remove_audio_node_now(pending.id),
                PluginChange::Replace(descriptor) => {
                    if let Some(node) = self
                        .project
                        .nodes
                        .iter_mut()
                        .find(|node| node.id == pending.id)
                    {
                        node.plugin = Some(NodePlugin::new(descriptor));
                        self.changed(false);
                    }
                }
            }
            self.record_deferred_plugin_edit(before);
            if let Some(warning) = warning {
                self.notify_error(warning);
            }
        } else {
            self.plugin_ui.pending_change = Some(pending);
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
    pub(super) fn poll_plugins(&mut self, ctx: &egui::Context) {
        self.poll_history_capture(ctx);
        self.poll_plugin_change(ctx);
        if let Some(path) = &self.plugin_ui.pending_export {
            if path != &self.path {
                self.plugin_ui.pending_export = None;
            } else if !self.dirty {
                self.plugin_ui.pending_export = None;
                self.export_workspace_now();
            } else {
                ctx.request_repaint_after(Duration::from_millis(30));
            }
        }
        if let Some(result) = self
            .plugin_ui
            .editor_reply
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        {
            self.plugin_ui.editor_reply = None;
            match result {
                Err(error) => self.notify_error(format!("Cannot open plugin editor: {error}")),
                Ok(()) => {
                    self.plugin_ui.editor_used = true;
                    self.dirty = true;
                    self.changed_at = Instant::now();
                }
            }
        }
        if let Some(result) = self
            .plugin_ui
            .scan
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        {
            self.plugin_ui.scan = None;
            match result {
                Ok(plugins) => {
                    let count = plugins.len();
                    match self
                        .plugin_ui
                        .library
                        .import(plugins)
                        .and_then(|()| self.plugin_ui.library.save())
                    {
                        Ok(()) => {
                            self.plugin_ui.error.clear();
                            self.notify(format!(
                                "Added {count} plugin(s). Create nodes here or in Mixer / Routing."
                            ));
                        }
                        Err(error) => {
                            self.plugin_ui.error =
                                format!("Could not save plugin library: {error:#}")
                        }
                    }
                }
                Err(error) => self.plugin_ui.error = error,
            }
        }
        if !ctx.input(|i| i.pointer.any_down()) && !self.plugin_ui.held.is_empty() {
            for (id, note) in std::mem::take(&mut self.plugin_ui.held) {
                self.command(Command::Note {
                    id,
                    note,
                    velocity: 0.0,
                    on: false,
                });
            }
        }
    }
    fn scan_plugin(&mut self, path: PathBuf) {
        if self.plugin_ui.scan.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.plugin_ui.scan = Some(rx);
        self.plugin_ui.error.clear();
        thread::spawn(move || {
            let _ = tx.send(crate::plugins::scan_isolated(path).map_err(|e| format!("{e:#}")));
        });
    }
    pub(super) fn plugin_library_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let ready = self.plugin_ui.scan.is_none();
            if ui
                .add_enabled(ready, egui::Button::new("+ Add plugin file"))
                .clicked()
            {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("VST plugin (.dll / .vst3)", &["vst3", "dll"])
                    .pick_file()
                {
                    self.scan_plugin(path);
                }
            }
            if ui
                .add_enabled(ready, egui::Button::new("+ Add VST3 bundle"))
                .clicked()
            {
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory("C:/Program Files/Common Files/VST3")
                    .pick_folder()
                {
                    self.scan_plugin(path);
                }
            }
            if !ready {
                ui.spinner();
                ui.label("Inspecting plugin in a separate process…");
            }
        });
        ui.add_space(12.0);
        ui.label(RichText::new("64-bit VST3 and 32-/64-bit VST2 (.dll). 32-bit plugins run in a helper process. Mono or stereo audio; generic parameter controls and optional experimental vendor editors.").color(theme::palette().muted));
        ui.label(RichText::new("Add a node, then connect it in Routing. Removing a library entry keeps existing project nodes.").small().color(theme::palette().muted));
        if !self.plugin_ui.error.is_empty() {
            ui.colored_label(theme::palette().peach, &self.plugin_ui.error);
        }
        ui.add_space(18.0);
        if self.plugin_ui.library.plugins.is_empty() {
            show_card(ui, |ui| {
                ui.heading("Bring your sound into PirateWave.");
                ui.label("Add an installed plugin file or select its .vst3 bundle folder to get started.");
            });
        }
        let mut add = None;
        let mut remove = None;
        let mut scan = None;
        for (index, plugin) in self.plugin_ui.library.plugins.iter().enumerate() {
            show_card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(&plugin.name);
                    ui.label(
                        RichText::new(format!("{:?} · {:?}", plugin.format, plugin.role))
                            .color(theme::palette().accent),
                    );
                });
                ui.label(format!(
                    "{} · {} parameters",
                    plugin.vendor,
                    plugin.parameters.len()
                ));
                ui.label(
                    RichText::new(&plugin.path)
                        .small()
                        .color(theme::palette().muted),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            self.project.nodes.len() < 64,
                            egui::Button::new(if plugin.role == PluginRole::Instrument {
                                "Add instrument node"
                            } else {
                                "Add effect node"
                            }),
                        )
                        .clicked()
                    {
                        add = Some(plugin.clone());
                    }
                    if ui
                        .add_enabled(self.plugin_ui.scan.is_none(), egui::Button::new("Rescan"))
                        .clicked()
                    {
                        scan = Some(PathBuf::from(&plugin.path));
                    }
                    if ui.button("Remove from library").clicked() {
                        remove = Some(index);
                    }
                });
            });
            ui.add_space(10.0);
        }
        if let Some(plugin) = add {
            self.add_plugin_node(plugin);
        }
        if let Some(index) = remove {
            self.plugin_ui.library.plugins.remove(index);
            self.plugin_ui.selection = None;
            if let Err(error) = self.plugin_ui.library.save() {
                self.plugin_ui.error = format!("Could not save library: {error:#}");
            }
        }
        if let Some(path) = scan {
            self.scan_plugin(path);
        }
    }
    fn add_plugin_node(&mut self, plugin: PluginDescriptor) {
        let kind = if plugin.role == PluginRole::Instrument {
            NodeKind::VstInstrument
        } else {
            NodeKind::VstEffect
        };
        let mut node = Node::new(self.project.next_id(), &plugin.name, kind, [300.0, 200.0]);
        node.plugin = Some(NodePlugin::new(plugin));
        self.selected = node.id;
        self.project.nodes.push(node);
        self.reveal_utility(self.selected);
        self.changed(true);
        self.view = View::Routing;
    }
    pub(super) fn plugin_node_page(&mut self, ui: &mut egui::Ui, id: u64) {
        let Some(index) = self.project.nodes.iter().position(|n| n.id == id) else {
            return;
        };
        let mut changed = false;
        if self.view != View::Effects && self.node_header(ui, id) {
            return;
        }
        let role = if matches!(self.project.nodes[index].kind, NodeKind::VstInstrument) {
            PluginRole::Instrument
        } else {
            PluginRole::Effect
        };
        let editor_available = self.status.running
            && self.plugin_ui.pending_change.is_none()
            && !self.editing.history_pending();
        let node = &mut self.project.nodes[index];
        let mut open_editor = false;
        let mut replacement = None;
        let mut parameter_edits = Vec::new();
        show_card(ui, |ui| {
            ui.heading("Plugin & status");
            let selected = node
                .plugin
                .as_ref()
                .map_or("Choose a plugin", |p| p.descriptor.name.as_str());
            egui::ComboBox::from_id_salt(("node-plugin", id))
                .selected_text(selected)
                .width(320.0)
                .truncate()
                .show_ui(ui, |ui| {
                    for plugin in self
                        .plugin_ui
                        .library
                        .plugins
                        .iter()
                        .filter(|p| p.role == role)
                    {
                        if ui
                            .selectable_label(
                                node.plugin
                                    .as_ref()
                                    .is_some_and(|p| p.descriptor.same_plugin(plugin)),
                                &plugin.name,
                            )
                            .clicked()
                            && !node
                                .plugin
                                .as_ref()
                                .is_some_and(|current| current.descriptor.same_plugin(plugin))
                        {
                            replacement = Some(plugin.clone());
                        }
                    }
                });
            if ui.button("Manage VST plugins").clicked() {
                self.view = View::Plugins;
            }
            ui.add_space(10.0);
            if let Some(plugin) = &node.plugin {
                ui.small(format!(
                    "{} · {:?}",
                    plugin.descriptor.vendor, plugin.descriptor.format
                ));
                if self.app_settings.experimental_vst_editors {
                    if ui
                        .add_enabled(editor_available, egui::Button::new("Open plugin editor"))
                        .clicked()
                    {
                        open_editor = true;
                    }
                    if !self.status.running {
                        ui.small("Start audio to open the plugin editor.");
                    }
                }
            }
            ui.label(self.status.plugins.get(&id).map_or(
                if self.status.running {
                    "Not loaded"
                } else {
                    "Audio stopped"
                },
                String::as_str,
            ));
            if role == PluginRole::Effect {
                changed |= ui
                    .checkbox(&mut node.effects_disabled, "Disable (passthrough)")
                    .changed();
            }
        });
        ui.add_space(12.);
        if let Some(plugin) = &mut node.plugin {
            if role == PluginRole::Instrument {
                show_card(ui, |ui| {
                    ui.heading("Audition controls");
                    ui.small("Play notes (MIDI channel 1)");
                    ui.horizontal(|ui| {
                        ui.label("Audition note");
                        changed |= ui
                            .add(egui::DragValue::new(&mut plugin.note).range(0..=127))
                            .changed();
                        ui.label("Velocity");
                        changed |= ui
                            .add(egui::Slider::new(&mut plugin.velocity, 0.01..=1.0))
                            .changed();
                    });
                    ui.horizontal_wrapped(|ui| {
                        for (offset, name) in [
                            "C4", "C#4", "D4", "D#4", "E4", "F4", "F#4", "G4", "G#4", "A4", "A#4",
                            "B4",
                        ]
                        .iter()
                        .enumerate()
                        {
                            let note = 60 + offset as u8;
                            let response = ui.add_enabled(
                                self.status.running,
                                egui::Button::new(*name).min_size(Vec2::new(38.0, 60.0)),
                            );
                            if response.is_pointer_button_down_on()
                                && self.plugin_ui.held.insert((id, note))
                            {
                                let _ = self.engine.send(Command::Note {
                                    id,
                                    note,
                                    velocity: plugin.velocity,
                                    on: true,
                                });
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                self.status.running,
                                egui::Button::new("Play audition note"),
                            )
                            .clicked()
                        {
                            let _ = self.engine.send(Command::Note {
                                id,
                                note: plugin.note,
                                velocity: plugin.velocity,
                                on: true,
                            });
                        }
                        if ui.button("Stop all notes").clicked() {
                            let _ = self.engine.send(Command::AllNotesOff(Some(id)));
                            self.plugin_ui.held.retain(|(n, _)| *n != id);
                        }
                    });
                    egui::CollapsingHeader::new("Audition help").id_salt(("audition-help",id)).show(ui,|ui|{ui.label("Hold a key with the mouse to play. Audition-note shortcuts stay on until released or stopped. Start audio and connect an output to hear this instrument.");});
                });
            }
            ui.add_space(15.0);
            show_card(ui, |ui| {
                ui.heading("Plugin parameters");
                egui::CollapsingHeader::new("Parameter help").id_salt(("plugin-help",id)).show(ui,|ui|{ui.small("Values use the plugin's normalized 0–1 scale and are saved with this node.");});
                if ui.button("Reset plugin parameters").clicked() {
                    plugin.parameters.clear();
                    parameter_edits.extend(
                        plugin
                            .descriptor
                            .parameters
                            .iter()
                            .map(|p| (p.id, p.default)),
                    );
                    changed = true;
                }
                if plugin.descriptor.parameters.is_empty() {
                    ui.label("This plugin exposes no editable parameters.");
                }
                egui::ScrollArea::vertical()
                    .id_salt(("plugin-params", id))
                    .max_height(420.0)
                    .show_rows(ui, 28.0, plugin.descriptor.parameters.len(), |ui, rows| {
                        for i in rows {
                            let p = &plugin.descriptor.parameters[i];
                            ui.push_id(p.id, |ui| {
                                ui.horizontal(|ui| {
                                    ui.add_sized(
                                        [220.0, 24.0],
                                        egui::Label::new(&p.name).truncate(),
                                    );
                                    let mut edited =
                                        plugin.parameters.get(&p.id).copied().unwrap_or(p.default);
                                    let value = &mut edited;
                                    let mut slider =
                                        egui::Slider::new(value, 0.0..=1.0).fixed_decimals(3);
                                    if p.steps > 0 {
                                        slider = slider.step_by(1.0 / p.steps as f64);
                                    }
                                    if ui.add(slider).changed() {
                                        plugin.parameters.insert(p.id, edited);
                                        parameter_edits.push((p.id, edited));
                                        changed = true;
                                    }
                                    ui.label(
                                        RichText::new(&p.unit)
                                            .small()
                                            .color(theme::palette().muted),
                                    );
                                });
                            });
                        }
                    });
            });
        } else {
            ui.colored_label(
                theme::palette().peach,
                "Assign an imported plugin to this node.",
            );
        }
        if let Some(descriptor) = replacement {
            if !self.defer_plugin_change(id, PluginChange::Replace(descriptor.clone())) {
                if let Some(node) = self.project.nodes.get_mut(index) {
                    node.plugin = Some(NodePlugin::new(descriptor));
                    changed = true;
                }
            }
        }
        if !parameter_edits.is_empty() && self.app_settings.experimental_vst_editors {
            self.command(Command::SetPluginParameters {
                id,
                values: parameter_edits,
            });
        }
        if open_editor {
            let (tx, rx) = mpsc::sync_channel(1);
            match self
                .engine
                .send(Command::OpenPluginEditor { id, reply: tx })
            {
                Ok(()) => self.plugin_ui.editor_reply = Some(rx),
                Err(error) => self.notify_error(error),
            }
        }
        if changed {
            self.changed(false);
        }
        ui.add_space(12.);
        ui.heading("Additional PirateWave effects");
        self.rack_controls(ui, index);
    }
}

#[cfg(test)]
pub(super) fn fixture(role: PluginRole) -> PluginDescriptor {
    PluginDescriptor {
        path: "test.vst3".into(),
        class_id: format!("{role:?}"),
        format: piratewave_core::plugins::PluginFormat::Vst3,
        role,
        name: format!("Test {role:?}"),
        vendor: "PirateWave".into(),
        parameters: vec![],
    }
}
/// Display-only metadata for the native screenshot diagnostic; no external code is loaded.
pub(super) fn smoke_plugins() -> Vec<PluginDescriptor> {
    [PluginRole::Instrument, PluginRole::Effect]
        .into_iter()
        .map(|role| PluginDescriptor {
            path: if role == PluginRole::Instrument {
                "C:/Audio/Plugins/Studio Synth.vst3"
            } else {
                "C:/Audio/Plugins/Studio Filter.vst3"
            }
            .into(),
            class_id: if role == PluginRole::Instrument {
                "1112131415161718191a1b1c1d1e1f20"
            } else {
                "0102030405060708090a0b0c0d0e0f10"
            }
            .into(),
            format: piratewave_core::plugins::PluginFormat::Vst3,
            role,
            name: if role == PluginRole::Instrument {
                "Studio Synth"
            } else {
                "Studio Filter"
            }
            .into(),
            vendor: "PirateWave demo".into(),
            parameters: ["Gain", "Tone", "Attack", "Release"]
                .iter()
                .enumerate()
                .map(|(i, name)| piratewave_core::plugins::PluginParameter {
                    id: i as u32,
                    name: (*name).into(),
                    unit: String::new(),
                    default: 0.5,
                    steps: 0,
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::tests::app;
    use super::*;
    #[test]
    fn library_deduplicates_and_clearing_library_preserves_node_settings() {
        let plugin = fixture(PluginRole::Instrument);
        let mut library = crate::plugins::Library::default();
        library
            .import(vec![plugin.clone(), plugin.clone()])
            .unwrap();
        assert_eq!(library.plugins.len(), 1);
        let node = NodePlugin::new(plugin.clone());
        library.plugins.clear();
        assert_eq!(node.descriptor, plugin);
        assert!(node.parameters.is_empty());
    }
    #[test]
    fn deferred_removal_keeps_the_captured_state_for_undo() {
        let mut app = app();
        let descriptor = fixture(PluginRole::Effect);
        let mut node = Node::new(77, "Plugin", NodeKind::VstEffect, [0.0; 2]);
        node.plugin = Some(NodePlugin::new(descriptor.clone()));
        app.project.nodes.push(node);
        let state = piratewave_core::plugins::PluginState {
            component: vec![4, 5, 6].into(),
            controller: None,
            vst2_bank: false,
            vst2_program: None,
        };
        let (tx, rx) = mpsc::sync_channel(1);
        tx.send((
            HashMap::from([(77, (descriptor.clone(), state.clone()))]),
            Vec::new(),
            Default::default(),
        ))
        .unwrap();
        app.plugin_ui.pending_change = Some(PendingPluginChange {
            id: 77,
            descriptor,
            path: app.path.clone(),
            action: PluginChange::Remove,
            started: Instant::now(),
            reply: Some(rx),
            request_parameters: Some(Default::default()),
            result: None,
        });
        app.poll_plugin_change(&egui::Context::default());
        assert!(!app.project.nodes.iter().any(|node| node.id == 77));
        app.undo_edit(false);
        let restored = app.project.nodes.iter().find(|node| node.id == 77).unwrap();
        assert_eq!(
            restored.plugin.as_ref().unwrap().state.as_ref(),
            Some(&state)
        );
    }
    #[test]
    fn deferred_replacement_cannot_change_another_workspace_or_plugin() {
        let mut app = app();
        let old = fixture(PluginRole::Effect);
        let mut node = Node::new(77, "Plugin", NodeKind::VstEffect, [0.0; 2]);
        node.plugin = Some(NodePlugin::new(old.clone()));
        app.project.nodes.push(node);
        let mut replacement = old.clone();
        replacement.class_id = "replacement".into();
        app.plugin_ui.pending_change = Some(PendingPluginChange {
            id: 77,
            descriptor: old.clone(),
            path: PathBuf::from("another-project.json"),
            action: PluginChange::Replace(replacement),
            started: Instant::now(),
            reply: None,
            request_parameters: None,
            result: Some((None, None)),
        });
        app.poll_plugin_change(&egui::Context::default());
        assert!(app.plugin_ui.pending_change.is_none());
        assert!(app
            .project
            .nodes
            .iter()
            .find(|node| node.id == 77)
            .unwrap()
            .plugin
            .as_ref()
            .unwrap()
            .descriptor
            .same_plugin(&old));
    }
    #[test]
    fn deferred_replacement_preserves_old_state_for_undo() {
        let mut app = app();
        let old = fixture(PluginRole::Effect);
        let mut node = Node::new(77, "Plugin", NodeKind::VstEffect, [0.0; 2]);
        node.plugin = Some(NodePlugin::new(old.clone()));
        app.project.nodes.push(node);
        let state = piratewave_core::plugins::PluginState {
            component: vec![9, 8, 7].into(),
            controller: None,
            vst2_bank: false,
            vst2_program: None,
        };
        let mut replacement = old.clone();
        replacement.class_id = "replacement".into();
        let (tx, rx) = mpsc::sync_channel(1);
        tx.send((
            HashMap::from([(77, (old.clone(), state.clone()))]),
            Vec::new(),
            Default::default(),
        ))
        .unwrap();
        app.plugin_ui.pending_change = Some(PendingPluginChange {
            id: 77,
            descriptor: old.clone(),
            path: app.path.clone(),
            action: PluginChange::Replace(replacement.clone()),
            started: Instant::now(),
            reply: Some(rx),
            request_parameters: Some(Default::default()),
            result: None,
        });
        app.poll_plugin_change(&egui::Context::default());
        assert!(app
            .project
            .nodes
            .iter()
            .find(|node| node.id == 77)
            .unwrap()
            .plugin
            .as_ref()
            .unwrap()
            .descriptor
            .same_plugin(&replacement));
        app.undo_edit(false);
        let restored = app
            .project
            .nodes
            .iter()
            .find(|node| node.id == 77)
            .unwrap()
            .plugin
            .as_ref()
            .unwrap();
        assert!(restored.descriptor.same_plugin(&old));
        assert_eq!(restored.state.as_ref(), Some(&state));
    }
    #[test]
    fn failed_capture_retains_previous_state_and_reports_failure() {
        let mut app = app();
        let descriptor = fixture(PluginRole::Effect);
        let state = piratewave_core::plugins::PluginState {
            component: vec![1, 2].into(),
            controller: None,
            vst2_bank: false,
            vst2_program: None,
        };
        let mut config = NodePlugin::new(descriptor.clone());
        config.state = Some(state.clone());
        let mut node = Node::new(77, "Plugin", NodeKind::VstEffect, [0.0; 2]);
        node.plugin = Some(config);
        app.project.nodes.push(node);
        let (tx, rx) = mpsc::sync_channel(1);
        tx.send((
            HashMap::new(),
            vec!["Plugin 77: state capture failed".into()],
            Default::default(),
        ))
        .unwrap();
        app.plugin_ui.pending_change = Some(PendingPluginChange {
            id: 77,
            descriptor,
            path: app.path.clone(),
            action: PluginChange::Remove,
            started: Instant::now(),
            reply: Some(rx),
            request_parameters: Some(Default::default()),
            result: None,
        });
        app.poll_plugin_change(&egui::Context::default());
        assert!(app.notice.contains("state capture failed"));
        app.undo_edit(false);
        assert_eq!(
            app.project
                .nodes
                .iter()
                .find(|node| node.id == 77)
                .unwrap()
                .plugin
                .as_ref()
                .unwrap()
                .state
                .as_ref(),
            Some(&state)
        );
    }
    #[test]
    fn deferred_change_discards_snapshot_when_parameters_changed_after_capture() {
        let mut app = app();
        let descriptor = fixture(PluginRole::Effect);
        let mut node = Node::new(77, "Plugin", NodeKind::VstEffect, [0.0; 2]);
        let mut config = NodePlugin::new(descriptor.clone());
        config.parameters.insert(0, 0.75);
        node.plugin = Some(config);
        app.project.nodes.push(node);
        app.plugin_ui.pending_change = Some(PendingPluginChange {
            id: 77,
            descriptor,
            path: app.path.clone(),
            action: PluginChange::Remove,
            started: Instant::now(),
            reply: None,
            request_parameters: Some(Default::default()),
            result: Some((
                Some(piratewave_core::plugins::PluginState {
                    component: vec![1].into(),
                    controller: None,
                    vst2_bank: false,
                    vst2_program: None,
                }),
                None,
            )),
        });
        app.poll_plugin_change(&egui::Context::default());
        assert!(app.project.nodes.iter().any(|node| node.id == 77));
        assert!(app
            .plugin_ui
            .pending_change
            .as_ref()
            .unwrap()
            .result
            .is_none());
    }
}
