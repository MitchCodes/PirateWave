use super::*;
use piratewave_core::plugins::{NodePlugin, PluginDescriptor, PluginRole};

pub(super) struct PluginUi {
    pub library: crate::plugins::Library,
    scan: Option<mpsc::Receiver<Result<Vec<PluginDescriptor>, String>>>,
    pub selection: Option<usize>,
    pub held: HashSet<(u64, u8)>,
    error: String,
}
impl PluginUi {
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
    pub(super) fn poll_plugins(&mut self, ctx: &egui::Context) {
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
        ui.label(RichText::new("64-bit VST3 and 32-/64-bit VST2 (.dll). 32-bit plugins run in a helper process. Mono or stereo audio; generic parameter controls.").color(theme::palette().muted));
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
        let node = &mut self.project.nodes[index];
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
                        {
                            node.plugin = Some(NodePlugin::new(plugin.clone()));
                            changed = true;
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
}
