use super::*;
#[derive(Default)]
pub(super) struct ExtensionUi {
    job: Option<mpsc::Receiver<Result<(), String>>>,
    message: Option<String>,
}
impl PirateWaveApp {
    pub(super) fn extension_settings(&mut self, ui: &mut egui::Ui) {
        if let Some(result) = self
            .extension_ui
            .job
            .as_ref()
            .and_then(|r| r.try_recv().ok())
        {
            self.extension_ui.job = None;
            self.extension_ui.message = Some(result.map_or_else(
                |e| format!("Installation failed: {e}"),
                |_| "Installed. Restart PirateWave to activate the extension.".into(),
            ));
        }
        show_card(ui, |ui| {
            ui.heading("PirateWave extensions");
            ui.label("Install trusted node and processor packages. Changes take effect after restarting PirateWave.");
            ui.small("Native extensions run inside PirateWave. Only install packages from developers you trust.");
            if ui
                .add_enabled(
                    self.extension_ui.job.is_none(),
                    egui::Button::new("Install .piratewaveext…"),
                )
                .clicked()
            {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("PirateWave extension", &["piratewaveext"])
                    .pick_file()
                {
                    let (tx, rx) = mpsc::channel();
                    self.extension_ui.job = Some(rx);
                    std::thread::spawn(move || {
                        let result =
                            crate::extensions::install(&path).map_err(|e| format!("{e:#}"));
                        let _ = tx.send(result);
                    });
                }
            }
            if self.extension_ui.job.is_some() {
                ui.spinner();
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
            if let Some(m) = &self.extension_ui.message {
                ui.colored_label(theme::palette().peach, m);
            }
            let rows = crate::extensions::rows();
            if rows.is_empty() {
                ui.small("No PirateWave extensions installed. VST plugins are managed separately.");
            }
            for row in rows {
                ui.separator();
                ui.label(format!("{} · {}", row.name, row.version));
                ui.small(&row.id);
                ui.small(if row.active {
                    "Active in this session"
                } else {
                    "Not active in this session"
                });
                ui.colored_label(
                    if row.active {
                        theme::palette().green
                    } else {
                        theme::palette().peach
                    },
                    &row.status,
                );
                let mut enabled = row.enabled;
                if ui
                    .add_enabled(
                        self.extension_ui.job.is_none(),
                        egui::Checkbox::new(&mut enabled, "Enable next launch"),
                    )
                    .changed()
                {
                    if let Err(e) = crate::extensions::enable(&row.id, enabled) {
                        self.extension_ui.message = Some(e.to_string());
                    }
                }
            }
        });
        ui.add_space(18.);
    }
    pub(super) fn extension_node_page(&mut self, ui: &mut egui::Ui, index: usize) {
        let NodeKind::Extension { definition: d } = &self.project.nodes[index].kind else {
            return;
        };
        let d = d.clone();
        let available = piratewave_core::extensions::get(&d.id).is_some_and(|e| e.descriptor == *d);
        let node = &mut self.project.nodes[index];
        let mut changed = false;
        show_card(ui, |ui| {
            ui.heading(&d.name);
            ui.small(&d.description);
            if !available {
                ui.colored_label(theme::palette().peach,"Extension unavailable or its schema changed. Settings and routes are preserved; bypass can use the declared routes.");
            }
            changed |= ui
                .checkbox(
                    &mut node.effects_disabled,
                    "Bypass node (declared routes only)",
                )
                .changed();
            if d.bypass.is_empty() {
                ui.small("This node declares no bypass routes; bypass outputs silence.");
            }
        });
        ui.add_space(16.);
        show_card(ui, |ui| {
            ui.heading("Parameters");
            for p in &d.parameters {
                let mut v = *node.extension.parameters.get(&p.id).unwrap_or(&p.default);
                if parameter(ui, p, &mut v) {
                    node.extension.parameters.insert(p.id.clone(), v);
                    changed = true;
                }
            }
        });
        let id = node.id;
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            for a in &d.actions {
                if ui
                    .add_enabled(available, egui::Button::new(&a.name))
                    .clicked()
                {
                    action = Some(piratewave_core::extensions::action_key(&a.id));
                }
            }
        });
        egui::CollapsingHeader::new("Help")
            .id_salt((id, "extension-help"))
            .show(ui, |ui| {
                ui.set_max_width(680.);
                ui.label(&d.help);
                ui.small(format!("{} · state schema {}", d.id, d.state_schema));
            });
        if changed {
            self.changed(false);
        }
        if let Some(action) = action {
            self.handle_shortcut(piratewave_core::shortcuts::ShortcutAction::Extension {
                id,
                effect: None,
                action,
            });
        }
    }
}
pub(super) fn parameter(
    ui: &mut egui::Ui,
    p: &piratewave_core::extensions::Parameter,
    value: &mut f32,
) -> bool {
    let before = *value;
    ui.horizontal_wrapped(|ui| {
        ui.add_sized([180., 24.], egui::Label::new(&p.name).truncate());
        if !p.choices.is_empty() {
            egui::ComboBox::from_id_salt((&p.id, "extension-parameter"))
                .selected_text(
                    p.choices
                        .get(value.round() as usize)
                        .map_or("Unknown", String::as_str),
                )
                .show_ui(ui, |ui| {
                    for (i, c) in p.choices.iter().enumerate() {
                        ui.selectable_value(value, i as f32, c);
                    }
                });
        } else {
            let mut slider = egui::Slider::new(value, p.min..=p.max).suffix(&p.unit);
            if p.step > 0. {
                slider = slider.step_by(p.step as f64);
            }
            ui.add(slider);
        }
        if ui.small_button("Reset").clicked() {
            *value = p.default;
        }
    });
    if before != *value {
        *value = p.snap(*value);
    }
    before != *value
}

pub(super) fn effect_menu(ui: &mut egui::Ui, registry: &EffectRegistry) -> Option<EffectSpec> {
    let mut add = None;
    let mut categories = std::collections::BTreeSet::new();
    for e in piratewave_core::extensions::catalog()
        .iter()
        .filter(|e| e.descriptor.kind == piratewave_core::extensions::TypeKind::Processor)
    {
        categories.insert(e.descriptor.category.as_str());
    }
    for category in categories {
        ui.menu_button(category, |ui| {
            for e in piratewave_core::extensions::catalog().iter().filter(|e| {
                e.descriptor.kind == piratewave_core::extensions::TypeKind::Processor
                    && e.descriptor.category == category
            }) {
                if ui
                    .button(&e.descriptor.name)
                    .on_hover_text(&e.descriptor.description)
                    .clicked()
                {
                    add = registry.get(&e.descriptor.id).map(EffectSpec::new);
                    ui.close_menu();
                }
            }
        });
    }
    add
}

pub(super) fn effect_card(
    ui: &mut egui::Ui,
    node: u64,
    spec: &mut EffectSpec,
    index: usize,
) -> (bool, bool, bool, Option<ShortcutAction>) {
    let (mut changed, mut remove, mut up, mut action) = (false, false, false, None);
    ui.push_id((node, spec.id), |ui| {
        show_card(ui, |ui| {
            let d = spec.extension_descriptor.as_ref();
            ui.horizontal_wrapped(|ui| {
                ui.strong(d.map_or(spec.kind.as_str(), |d| d.name.as_str()));
                changed |= ui.checkbox(&mut spec.enabled, "Enabled").changed();
                remove = ui.small_button("Remove").clicked();
                up = index > 0 && ui.small_button("Move up").clicked();
            });
            let available = piratewave_core::extensions::get(&spec.kind)
                .is_some_and(|e| d.is_some_and(|d| d == &e.descriptor));
            if !available {
                ui.colored_label(
                    theme::palette().peach,
                    "Extension unavailable or incompatible; this effect passes through.",
                );
            }
            if let Some(d) = d {
                ui.collapsing("Help", |ui| {
                    ui.set_max_width(680.);
                    ui.label(&d.description);
                    ui.label(&d.help);
                });
                for p in &d.parameters {
                    let mut value = *spec.parameters.get(&p.id).unwrap_or(&p.default);
                    if parameter(ui, p, &mut value) {
                        spec.parameters.insert(p.id.clone(), value);
                        changed = true;
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    for a in &d.actions {
                        if ui
                            .add_enabled(available, egui::Button::new(&a.name))
                            .clicked()
                        {
                            action = Some(ShortcutAction::Extension {
                                id: node,
                                effect: Some(spec.id),
                                action: piratewave_core::extensions::action_key(&a.id),
                            });
                        }
                    }
                });
            }
        });
    });
    (changed, remove, up, action)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{app, at, click, labels, render};
    use super::*;
    fn setup() -> PirateWaveApp {
        let mut app = app();
        let m = piratewave_extension_sdk::parse_manifest(include_bytes!(
            "../../crates/piratewave-example-extension/manifest.json"
        ))
        .unwrap();
        app.project.nodes.push(Node::new(
            97,
            "Extension fixture",
            NodeKind::Extension {
                definition: Box::new(m.types[1].clone()),
            },
            [300., 200.],
        ));
        app.project
            .nodes
            .iter_mut()
            .find(|n| n.id == 6)
            .unwrap()
            .effects
            .push(piratewave_core::extensions::effect_spec(&m.types[0]));
        app
    }
    #[test]
    fn unavailable_extension_controls_and_effects_remain_visible_without_audio_updates() {
        let mut app = setup();
        let before = serde_json::to_vec(&app.project).unwrap();
        let (engine, rx) = Engine::test_channel();
        app.engine = engine;
        let ctx = egui::Context::default();
        for id in [97, 6] {
            app.selected = id;
            app.view = View::Effects;
            let out = render(&ctx, &mut app, vec![]);
            let text = labels(&out);
            assert!(text
                .iter()
                .any(|(t, _)| t.to_lowercase().contains("unavailable")));
            assert!(text.iter().any(|(t, _)| t == "Reset"));
        }
        assert_eq!(serde_json::to_vec(&app.project).unwrap(), before);
        assert!(!app.needs_apply && !app.dirty);
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn extension_parameter_reset_uses_normal_update_path() {
        let mut app = setup();
        app.selected = 97;
        app.view = View::Effects;
        app.project
            .nodes
            .last_mut()
            .unwrap()
            .extension
            .parameters
            .insert("level".into(), 0.25);
        let ctx = egui::Context::default();
        let out = render(&ctx, &mut app, vec![]);
        let point = at(&out, "Reset");
        click(&ctx, &mut app, point, egui::PointerButton::Primary);
        assert_eq!(
            app.project.nodes.last().unwrap().extension.parameters["level"],
            1.
        );
        assert!(app.dirty && app.needs_apply);
    }
    #[test]
    fn extension_keybind_rows_use_descriptors_and_survive_absence() {
        let mut app = setup();
        app.open_node_keybinds(97);
        let ctx = egui::Context::default();
        let out = render(&ctx, &mut app, vec![]);
        assert!(labels(&out)
            .iter()
            .any(|(t, _)| t.contains("Toggle quiet mix")));
        app.open_node_keybinds(6);
        let out = render(&ctx, &mut app, vec![]);
        assert!(labels(&out)
            .iter()
            .any(|(t, _)| t.contains("Toggle polarity")));
    }
}
