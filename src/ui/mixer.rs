use super::*;
#[derive(Clone, Copy)]
struct MixerDrag {
    id: u64,
    section: u8,
}
fn ordered_nodes<'a>(
    nodes: &'a mut [Node],
    order: &[u64],
    filter: impl Fn(&Node) -> bool,
) -> Vec<&'a mut Node> {
    let mut nodes: Vec<_> = nodes.iter_mut().filter(|n| filter(n)).collect();
    nodes.sort_by_key(|n| {
        order
            .iter()
            .position(|id| *id == n.id)
            .unwrap_or(usize::MAX)
    });
    nodes
}
fn move_node(order: &mut Vec<u64>, from: u64, to: u64, after: bool) {
    if from == to || !order.contains(&from) || !order.contains(&to) {
        return;
    }
    order.retain(|id| *id != from);
    let index = order.iter().position(|id| *id == to).unwrap();
    order.insert(index + usize::from(after), from);
}

fn mixer_type_label(
    ui: &mut egui::Ui,
    id: u64,
    section: u8,
    text: &str,
    color: Color32,
) -> egui::Response {
    ui.dnd_drag_source(
        egui::Id::new(("mixer-drag", id)),
        MixerDrag { id, section },
        |ui| {
            ui.label(RichText::new(text).small().strong().color(color));
        },
    )
    .response
    .on_hover_text("Drag to reorder within this section")
}

fn show_mixer_card<R>(
    ui: &mut egui::Ui,
    id: u64,
    section: u8,
    reorder: &mut Option<(u64, u64, bool)>,
    delete: &mut Option<u64>,
    content: impl FnOnce(&mut egui::Ui, &mut Option<u64>) -> R,
) -> egui::InnerResponse<R> {
    let response = card().show(ui, |ui| {
        ui.scope_builder(
            egui::UiBuilder::new()
                .layout(egui::Layout::top_down(egui::Align::Min))
                .sense(egui::Sense::click()),
            |ui| {
                let result = content(ui, delete);
                // Use the scope's background response, created before its children, so
                // the context menu does not intercept sliders, toggles, or buttons.
                ui.response().context_menu(|ui| {
                    if ui.button("Remove node").clicked() {
                        *delete = Some(id);
                        ui.close_menu();
                    }
                });
                result
            },
        )
        .inner
    });
    if response
        .response
        .dnd_hover_payload::<MixerDrag>()
        .is_some_and(|p| p.section == section && p.id != id)
    {
        ui.painter().rect_stroke(
            response.response.rect,
            8.,
            Stroke::new(2_f32, theme::palette().accent),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(payload) = response.response.dnd_release_payload::<MixerDrag>() {
        if payload.section == section && payload.id != id {
            let after = ui.input(|i| i.pointer.interact_pos()).is_some_and(|pos| {
                if section == 1 {
                    pos.y > response.response.rect.center().y
                } else {
                    pos.x > response.response.rect.center().x
                }
            });
            *reorder = Some((payload.id, id, after));
        }
    }
    response
}
pub(super) fn switch_routes(project: &Project) -> HashMap<u64, Vec<(u64, String)>> {
    project
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Switch))
        .map(|n| {
            let mut seen = HashSet::new();
            let routes = project
                .connections
                .iter()
                .filter(|c| c.from == n.id && seen.insert(c.to))
                .filter_map(|c| {
                    project
                        .nodes
                        .iter()
                        .find(|n| n.id == c.to)
                        .map(|n| (n.id, n.name.clone()))
                })
                .collect();
            (n.id, routes)
        })
        .collect()
}
pub(super) fn switch_selector(
    ui: &mut egui::Ui,
    node: &mut Node,
    routes: &HashMap<u64, Vec<(u64, String)>>,
    changed: &mut bool,
) {
    let outputs = routes.get(&node.id).map_or(&[][..], Vec::as_slice);
    let selected = node
        .switch_output
        .or_else(|| outputs.first().map(|(id, _)| *id));
    ui.label("Active output");
    egui::ComboBox::from_id_salt(("switch_output", node.id))
        .selected_text(
            outputs
                .iter()
                .find(|(id, _)| Some(*id) == selected)
                .map_or("Connect an output", |(_, name)| name.as_str()),
        )
        .show_ui(ui, |ui| {
            for (id, name) in outputs {
                *changed |= ui
                    .selectable_value(&mut node.switch_output, Some(*id), name)
                    .changed();
            }
        });
    if outputs.is_empty() {
        ui.label("Connect this switch to processors or buses in Routing.");
    }
}
/// Uses the rack's EffectSpec flags and the Mixer's normal change propagation.
fn mixer_effects(ui: &mut egui::Ui, node: &mut Node, registry: &EffectRegistry) -> bool {
    if (node.kind.is_source() && node.effects.is_empty())
        || matches!(
            node.kind,
            NodeKind::Switch
                | NodeKind::AutoDucking
                | NodeKind::VoiceChanger
                | NodeKind::Output { .. }
        )
    {
        return false;
    }
    let mut changed = false;
    egui::CollapsingHeader::new(format!("Effects ({})", node.effects.len()))
        .id_salt(("mixer-effects", node.id))
        .default_open(false)
        .show(ui, |ui| {
            if node.effects.is_empty() {
                ui.small("No effects added.");
            }
            if node.effects_disabled
                && (node.kind.is_processor() || matches!(node.kind, NodeKind::Recording))
            {
                ui.colored_label(theme::palette().muted, "Rack bypassed");
            }
            for effect in &mut node.effects {
                ui.push_id(effect.id, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .small_button(if effect.enabled { "Disable" } else { "Enable" })
                            .clicked()
                        {
                            effect.enabled = !effect.enabled;
                            changed = true;
                        }
                        let name = effect
                            .extension_descriptor
                            .as_ref()
                            .map(|d| d.name.as_str())
                            .or_else(|| registry.get(&effect.kind).map(|f| f.name))
                            .unwrap_or(&effect.kind);
                        ui.add(
                            egui::Label::new(RichText::new(name).color(if effect.enabled {
                                ui.visuals().text_color()
                            } else {
                                theme::palette().muted
                            }))
                            .wrap(),
                        );
                    });
                });
            }
        });
    changed
}
impl PirateWaveApp {
    pub(super) fn mixer(&mut self, ui: &mut egui::Ui) {
        let switch_outputs = switch_routes(&self.project);
        let mut changed = false;
        let mut restart = false;
        let mut selected = None;
        let mut plugin_controls = None;
        let mut voice_controls = None;
        let mut recording_controls = None;
        let mut stop_recording = None;
        let inactive_voices: Vec<_> = self
            .project
            .nodes
            .iter()
            .filter(|node| {
                matches!(node.kind, NodeKind::VoiceChanger)
                    && !piratewave_core::voice::route_selected(&self.project, node.id)
            })
            .map(|node| node.id)
            .collect();
        let enabled_voice = self
            .project
            .nodes
            .iter()
            .find(|n| {
                matches!(n.kind, NodeKind::VoiceChanger)
                    && !n.effects_disabled
                    && !inactive_voices.contains(&n.id)
            })
            .map(|n| n.id);
        let mut keybinds = None;
        let mut delete = None;
        let mut refresh_applications = false;
        use piratewave_core::shortcuts::UtilityFilter;
        let preferences_before = self.app_settings.ui.clone();
        let workspace_key = self.path.to_string_lossy().into_owned();
        let order = self
            .app_settings
            .ui
            .mixer_orders
            .get(&workspace_key)
            .cloned()
            .unwrap_or_default();
        let mut reorder = None;
        let width = ui.available_width();
        let height = ui.available_height().max(180.);
        let output_width = 272_f32.min(width * 0.42);
        let source_width = (width - output_width - 16.).max(120.);
        let expanded = self.app_settings.ui.utilities_expanded;
        let utility_height = if expanded {
            self.app_settings
                .ui
                .utilities_height
                .min(height * 0.4)
                .max(80.)
        } else {
            0.
        };
        let upper_height = (height - utility_height - if expanded { 66. } else { 40. }).max(100.);
        let (upper, _) =
            ui.allocate_exact_size(Vec2::new(width, upper_height), egui::Sense::hover());
        let source_rect = Rect::from_min_size(upper.min, Vec2::new(source_width, upper_height));
        let bus_rect = Rect::from_min_max(
            Pos2::new(upper.right() - output_width, upper.top()),
            upper.max,
        );
        self.layout.source_viewport = source_rect;
        self.layout.bus_viewport = bus_rect;
        let mut sources_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("mixer-sources-pane")
                .max_rect(source_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        sources_ui.set_clip_rect(source_rect.intersect(ui.clip_rect()));
        caption(&mut sources_ui, "INPUTS & SOURCES");
        egui::ScrollArea::both()
            .id_salt("source-scroll")
            .max_width(source_width)
            .max_height((upper_height - 24.).max(40.))
            .auto_shrink([false, false])
            .show(&mut sources_ui, |ui| {
                if !self.project.nodes.iter().any(|n| n.kind.is_source()) {
                    ui.label("Add an input or source to begin.");
                }
                ui.horizontal_top(|ui| {
                    for node in
                        ordered_nodes(&mut self.project.nodes, &order, |n| n.kind.is_source())
                    {
                        let color = match node.kind {
                            NodeKind::Input { loopback: true, .. } => {
                                theme::accent(Color32::from_rgb(144, 188, 216))
                            }
                            NodeKind::Soundboard => theme::palette().peach,
                            _ => theme::palette().accent,
                        };
                        show_mixer_card(
                            ui,
                            node.id,
                            0,
                            &mut reorder,
                            &mut delete,
                            |ui, _delete| {
                                ui.set_width(170.);
                                ui.set_max_width(170.);
                                ui.spacing_mut().item_spacing.y = 6.;
                                mixer_type_label(ui, node.id, 0, node.kind.label(), color)
                                    .context_menu(|ui| {
                                        if ui.button("Remove node").clicked() {
                                            *_delete = Some(node.id);
                                            ui.close_menu();
                                        }
                                    });
                                changed |= ui
                                    .add(
                                        egui::TextEdit::singleline(&mut node.name)
                                            .desired_width(165.)
                                            .frame(false),
                                    )
                                    .changed();
                                ui.allocate_ui_with_layout(
                                    Vec2::new(170.0, 35.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        if let NodeKind::Input { device, loopback } = &mut node.kind
                                        {
                                            let before = device.clone();
                                            device_combo(
                                                ui,
                                                node.id,
                                                device,
                                                if *loopback {
                                                    &self.devices.outputs
                                                } else {
                                                    &self.devices.inputs
                                                },
                                                false,
                                            );
                                            restart |= *device != before;
                                        } else if let NodeKind::ApplicationCapture { target } =
                                            &mut node.kind
                                        {
                                            restart |= application_capture::picker(
                                                ui,
                                                node.id,
                                                target,
                                                &self.applications,
                                            );
                                            if ui.small_button("Refresh apps").clicked() {
                                                refresh_applications = true;
                                            }
                                            if !self.applications_error.is_empty() {
                                                ui.colored_label(
                                                    theme::palette().peach,
                                                    &self.applications_error,
                                                );
                                            }
                                        } else if matches!(node.kind, NodeKind::VstInstrument) {
                                            ui.label(
                                                RichText::new(
                                                    node.plugin
                                                        .as_ref()
                                                        .map_or("Choose a VST instrument", |p| {
                                                            p.descriptor.name.as_str()
                                                        }),
                                                )
                                                .small()
                                                .color(theme::palette().accent),
                                            );
                                        } else {
                                            ui.add_space(5.0);
                                            ui.label(
                                                RichText::new("Internal stereo playback")
                                                    .small()
                                                    .color(theme::palette().muted),
                                            );
                                        }
                                    },
                                );
                                ui.separator();
                                ui.add_space(5.0);
                                ui.horizontal(|ui| {
                                    ui.small("L");
                                    changed |= ui
                                        .add(
                                            egui::Slider::new(&mut node.pan, -1.0..=1.0)
                                                .show_value(false),
                                        )
                                        .changed();
                                    ui.small("R");
                                });
                                ui.add_space(10.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(23.0);
                                    ui.scope(|ui| {
                                        ui.spacing_mut().slider_width = 165.0;
                                        changed |= ui
                                            .add(
                                                egui::Slider::new(&mut node.gain_db, -60.0..=12.0)
                                                    .vertical()
                                                    .show_value(false),
                                            )
                                            .changed();
                                    });
                                    ui.add_space(14.0);
                                    meter(
                                        ui,
                                        self.status.peaks.get(&node.id).copied().unwrap_or(0.0),
                                        Vec2::new(16.0, 165.0),
                                        true,
                                    );
                                    ui.allocate_ui_with_layout(
                                        Vec2::new(27.0, 165.0),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            for value in ["+12", "0", "-12", "-24", "-48", "-60"] {
                                                ui.label(
                                                    RichText::new(value)
                                                        .small()
                                                        .color(theme::palette().muted),
                                                );
                                                ui.add_space(8.0);
                                            }
                                        },
                                    );
                                });
                                ui.add_space(12.0);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("{:.1}", node.gain_db))
                                            .size(17.0)
                                            .strong(),
                                    );
                                    ui.label(
                                        RichText::new("dB").small().color(theme::palette().muted),
                                    );
                                });
                                ui.add_space(5.0);
                                ui.horizontal(|ui| {
                                    changed |= ui.toggle_value(&mut node.mute, " Mute ").changed();
                                    changed |= ui.toggle_value(&mut node.solo, " Solo ").changed();
                                });
                                ui.add_space(4.0);
                                ui.separator();
                                if ui
                                    .add_sized([170.0, 28.0], egui::Button::new("Keybinds"))
                                    .clicked()
                                {
                                    keybinds = Some(node.id);
                                }
                                if matches!(node.kind, NodeKind::VstInstrument)
                                    && ui
                                        .add_sized(
                                            [170.0, 28.0],
                                            egui::Button::new("Plugin / Play notes"),
                                        )
                                        .clicked()
                                {
                                    plugin_controls = Some(node.id);
                                }
                                if ui
                                    .add_sized(
                                        [170.0, 28.0],
                                        egui::Button::new(
                                            RichText::new(format!(
                                                "FX   {} effects    >",
                                                node.effects.iter().filter(|e| e.enabled).count()
                                            ))
                                            .small()
                                            .color(color),
                                        ),
                                    )
                                    .clicked()
                                {
                                    selected = Some(node.id);
                                }
                                changed |= mixer_effects(ui, node, &self.registry);
                            },
                        );
                    }
                });
            });
        let mut buses_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("mixer-buses-pane")
                .max_rect(bus_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        buses_ui.set_clip_rect(bus_rect.intersect(ui.clip_rect()));
        caption(&mut buses_ui, "OUTPUT BUSES");
        egui::ScrollArea::vertical().id_salt("output-buses-scroll").max_height((upper_height-24.).max(40.)).auto_shrink([false,false]).show(&mut buses_ui,|ui|{
            if !self.project.nodes.iter().any(|n|matches!(n.kind,NodeKind::Output{..})){ui.label("Add an output bus in Routing.");}
            for node in ordered_nodes(&mut self.project.nodes, &order, |n|matches!(n.kind,NodeKind::Output{..})) {
                show_mixer_card(ui,node.id,1,&mut reorder,&mut delete,|ui,delete|{
                        ui.set_width(output_width-32.0);ui.spacing_mut().item_spacing.y=9.0;
                        mixer_type_label(ui,node.id,1,"OUTPUT BUS",theme::palette().green).context_menu(|ui|{if ui.button("Remove node").clicked(){*delete=Some(node.id);ui.close_menu();}});
                        changed|=ui.add(egui::TextEdit::singleline(&mut node.name).desired_width(output_width-44.).frame(false)).changed();
                        if let NodeKind::Output{device}=&mut node.kind{let before=device.clone();device_combo(ui,node.id,device,&self.devices.outputs,true);restart|=*device!=before;}
                        ui.add_space(6.0);let peak=self.status.peaks.get(&node.id).copied().unwrap_or(0.0);meter(ui,peak,Vec2::new(output_width-32.0,7.0),false);ui.horizontal(|ui|{ui.label(RichText::new("-60").small().color(theme::palette().muted));ui.add_space(65.0);ui.label(RichText::new("-24").small().color(theme::palette().muted));ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{ui.label(RichText::new("0 dB").small().color(theme::palette().muted));});});
                        changed|=ui.add(egui::Slider::new(&mut node.gain_db,-60.0..=12.0).suffix(" dB")).changed();
                        ui.horizontal(|ui|{ui.label(RichText::new("STEREO").small().color(theme::palette().muted));ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{changed|=ui.toggle_value(&mut node.mute,"Mute").changed();});});
                        changed |= ui.checkbox(&mut node.listen_enabled, "Listen").on_hover_text("Mirror this bus to another playback device. Requires an assigned, available primary output.").changed();
                        if node.listen_enabled {
                            ui.label(RichText::new("Listen on").small().color(theme::palette().muted));
                            ui.push_id(("listen", node.id), |ui| {
                                let before = node.listen_device.clone();
                                device_combo(ui, node.id, &mut node.listen_device, &self.devices.outputs, false);
                                changed |= node.listen_device != before;
                            });
                        }
                        if ui.add_sized([output_width-32.0,28.0],egui::Button::new("Keybinds")).clicked(){keybinds=Some(node.id);}
                });ui.add_space(5.);
            }
        });
        ui.add_space(6.);
        let utility_count = self
            .project
            .nodes
            .iter()
            .filter(|n| UtilityFilter::All.includes(&n.kind))
            .count();
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(format!(
                    "{} Processors & utility nodes ({utility_count})",
                    if self.app_settings.ui.utilities_expanded {
                        "v"
                    } else {
                        ">"
                    }
                ))
                .clicked()
            {
                self.app_settings.ui.utilities_expanded = !self.app_settings.ui.utilities_expanded;
            }
            if self.app_settings.ui.utilities_expanded {
                for filter in UtilityFilter::ALL {
                    ui.selectable_value(
                        &mut self.app_settings.ui.utilities_filter,
                        filter,
                        filter.label(),
                    );
                }
            }
        });
        self.layout.utility_viewport = Rect::NOTHING;
        if self.app_settings.ui.utilities_expanded {
            let (resize, _) = ui.allocate_exact_size(Vec2::new(width, 6.), egui::Sense::hover());
            let response = ui
                .interact(resize, ui.id().with("utility-resize"), egui::Sense::drag())
                .on_hover_cursor(egui::CursorIcon::ResizeVertical);
            ui.painter().line_segment(
                [resize.left_center(), resize.right_center()],
                Stroke::new(1_f32, theme::palette().line),
            );
            if response.dragged() {
                self.app_settings.ui.utilities_height = (self.app_settings.ui.utilities_height
                    - response.drag_delta().y)
                    .clamp(80., 2000.);
            }
            let (pane, _) = ui.allocate_exact_size(
                Vec2::new(width, utility_height.max(80.)),
                egui::Sense::hover(),
            );
            self.layout.utility_viewport = pane;
            let mut aux_ui = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("mixer-utilities-pane")
                    .max_rect(pane)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            aux_ui.set_clip_rect(pane.intersect(ui.clip_rect()));
            let filter = self.app_settings.ui.utilities_filter;
            let grid_width = width - 14.;
            let columns = ((grid_width + 10.) / 250.).floor().max(1.) as usize;
            let card_width = (grid_width - 10. * (columns - 1) as f32) / columns as f32 - 32.;
            egui::ScrollArea::vertical()
                .id_salt("utilities-scroll")
                .max_height(utility_height.max(80.))
                .auto_shrink([false, false])
                .show(&mut aux_ui, |ui| {
                    if !self.project.nodes.iter().any(|n| filter.includes(&n.kind)) {
                        ui.label("No nodes in this category. Add a node from Routing.");
                    }
                    egui::Grid::new("utility-grid")
                        .num_columns(columns)
                        .spacing(Vec2::new(10., 10.))
                        .show(ui, |ui| {
                            for (i, node) in ordered_nodes(&mut self.project.nodes, &order, |n| {
                                filter.includes(&n.kind)
                            })
                            .into_iter()
                            .enumerate()
                            {
                                let response = show_mixer_card(
                                    ui,
                                    node.id,
                                    2,
                                    &mut reorder,
                                    &mut delete,
                                    |ui, _delete| {
                                        ui.set_width(card_width);
                                        ui.set_max_width(card_width);
                                        mixer_type_label(
                                            ui,
                                            node.id,
                                            2,
                                            node.kind.label(),
                                            theme::palette().accent,
                                        )
                                        .context_menu(
                                            |ui| {
                                                if ui.button("Remove node").clicked() {
                                                    *_delete = Some(node.id);
                                                    ui.close_menu();
                                                }
                                            },
                                        );
                                        changed |= ui
                                            .add(
                                                egui::TextEdit::singleline(&mut node.name)
                                                    .desired_width(card_width)
                                                    .frame(false),
                                            )
                                            .changed();
                                        if matches!(node.kind, NodeKind::Switch) {
                                            switch_selector(
                                                ui,
                                                node,
                                                &switch_outputs,
                                                &mut changed,
                                            );
                                            ui.horizontal_wrapped(|ui| {
                                                if ui.button("Edit").clicked() {
                                                    selected = Some(node.id);
                                                }
                                                if ui.button("Keybinds").clicked() {
                                                    keybinds = Some(node.id);
                                                }
                                            });
                                        } else if matches!(node.kind, NodeKind::Recording) {
                                            ui.small(format!(
                                                "{} - {} slots",
                                                if node.recording.enabled {
                                                    "Recording armed"
                                                } else {
                                                    "Stopped"
                                                },
                                                node.recording.slots.len()
                                            ));
                                            changed |= ui
                                                .checkbox(
                                                    &mut node.effects_disabled,
                                                    "Bypass slot effects",
                                                )
                                                .changed();
                                            ui.horizontal_wrapped(|ui| {
                                                if ui.button("Recording / Playback").clicked() {
                                                    recording_controls = Some(node.id);
                                                }
                                                if ui.button("Stop playing recording").clicked() {
                                                    stop_recording = Some(node.id);
                                                }
                                            });
                                            ui.horizontal_wrapped(|ui| {
                                                if ui
                                                    .button(format!(
                                                        "Slot effects ({})",
                                                        node.effects.len()
                                                    ))
                                                    .clicked()
                                                {
                                                    selected = Some(node.id);
                                                }
                                                if ui.button("Keybinds").clicked() {
                                                    keybinds = Some(node.id);
                                                }
                                            });
                                        } else if matches!(node.kind, NodeKind::AutoDucking) {
                                            changed |= ui
                                                .checkbox(
                                                    &mut node.effects_disabled,
                                                    "Bypass ducking",
                                                )
                                                .changed();
                                            ui.small(format!(
                                                "{:?} detection",
                                                node.ducking.detection
                                            ));
                                            if let Some(s) = self.status.ducking.get(&node.id) {
                                                ui.small(format!(
                                                    "{:.1} dB reduction",
                                                    s.reduction_db
                                                ));
                                            }
                                            ui.horizontal_wrapped(|ui| {
                                                if ui.button("Ducking controls").clicked() {
                                                    selected = Some(node.id);
                                                }
                                                if ui.button("Keybinds").clicked() {
                                                    keybinds = Some(node.id);
                                                }
                                            });
                                        } else {
                                            ui.add_enabled_ui(
                                                !matches!(node.kind, NodeKind::VoiceChanger)
                                                    || !node.effects_disabled
                                                    || enabled_voice.is_none_or(|id| id == node.id)
                                                    || inactive_voices.contains(&node.id),
                                                |ui| {
                                                    changed |= ui
                                                        .checkbox(
                                                            &mut node.effects_disabled,
                                                            if matches!(
                                                                node.kind,
                                                                NodeKind::Extension { .. }
                                                            ) {
                                                                "Bypass declared routes"
                                                            } else {
                                                                "Disable (passthrough)"
                                                            },
                                                        )
                                                        .changed();
                                                },
                                            );
                                            if matches!(node.kind, NodeKind::VoiceChanger) {
                                                ui.small("LLVC voice — Beta");
                                                ui.horizontal_wrapped(|ui| {
                                                    if ui.button("Voice controls").clicked() {
                                                        voice_controls = Some(node.id);
                                                    }
                                                    if ui.button("Keybinds").clicked() {
                                                        keybinds = Some(node.id);
                                                    }
                                                    if ui.button("Help").clicked() {
                                                        self.voice_ui.help = true;
                                                    }
                                                });
                                            } else {
                                                if let NodeKind::Extension { definition } =
                                                    &node.kind
                                                {
                                                    ui.small(
                                                        if piratewave_core::extensions::get(
                                                            &definition.id,
                                                        )
                                                        .is_some_and(|e| {
                                                            e.descriptor == **definition
                                                        }) {
                                                            "Extension ready"
                                                        } else {
                                                            "Extension unavailable"
                                                        },
                                                    );
                                                } else {
                                                    ui.small(format!(
                                                        "{} PirateWave effects",
                                                        node.effects.len()
                                                    ));
                                                }
                                                if let Some(p) = &node.plugin {
                                                    ui.label(
                                                        egui::RichText::new(&p.descriptor.name)
                                                            .small(),
                                                    )
                                                    .on_hover_text(&p.descriptor.path);
                                                }
                                                ui.horizontal_wrapped(|ui| {
                                                    if ui
                                                        .button(
                                                            if matches!(
                                                                node.kind,
                                                                NodeKind::Extension { .. }
                                                            ) {
                                                                "Edit controls"
                                                            } else {
                                                                "Edit effects"
                                                            },
                                                        )
                                                        .clicked()
                                                    {
                                                        selected = Some(node.id);
                                                    }
                                                    if matches!(node.kind, NodeKind::VstEffect)
                                                        && ui.button("Plugin controls").clicked()
                                                    {
                                                        plugin_controls = Some(node.id);
                                                    }
                                                    if ui.button("Keybinds").clicked() {
                                                        keybinds = Some(node.id);
                                                    }
                                                });
                                            }
                                        }
                                        changed |= mixer_effects(ui, node, &self.registry);
                                    },
                                );
                                if self.layout.mixer_focus == Some(node.id) {
                                    ui.scroll_to_rect(
                                        response.response.rect,
                                        Some(egui::Align::Center),
                                    );
                                    if ui.clip_rect().contains(response.response.rect.center()) {
                                        self.layout.mixer_focus = None;
                                    }
                                }
                                if (i + 1) % columns == 0 {
                                    ui.end_row();
                                }
                            }
                        });
                });
        }
        if let Some((from, to, after)) = reorder {
            let mut full_order: Vec<_> = ordered_nodes(&mut self.project.nodes, &order, |_| true)
                .into_iter()
                .map(|n| n.id)
                .collect();
            move_node(&mut full_order, from, to, after);
            self.app_settings
                .ui
                .mixer_orders
                .insert(workspace_key, full_order);
        }
        if self.app_settings.ui != preferences_before {
            self.layout_preferences_changed();
        }
        if let Some(id) = selected {
            self.selected = id;
            self.view = if self
                .project
                .nodes
                .iter()
                .any(|n| n.id == id && matches!(n.kind, NodeKind::AutoDucking))
            {
                View::Ducking(id)
            } else {
                View::Effects
            };
        }
        if let Some(id) = plugin_controls {
            self.selected = id;
            self.view = View::PluginNode(id);
        }
        if let Some(id) = stop_recording {
            self.stop_recording_playback(id, None);
        }
        if let Some(id) = recording_controls {
            self.selected = id;
            self.view = View::Recording(id);
        }
        if let Some(id) = voice_controls {
            self.selected = id;
            self.view = View::VoiceNode(id);
        }
        if let Some(id) = keybinds {
            self.open_node_keybinds(id);
        }
        if let Some(id) = delete {
            self.remove_audio_node(id);
        }
        if refresh_applications {
            self.refresh_applications();
        }
        if changed || restart {
            self.changed(restart);
        }
    }
}

#[cfg(test)]
mod reorder_tests {
    use super::*;
    #[test]
    fn presentation_order_handles_new_removed_and_filtered_nodes_without_project_edits() {
        let mut project = Project::default();
        let before = serde_json::to_value(&project).unwrap();
        let ids: Vec<_> = project.nodes.iter().map(|n| n.id).collect();
        let mut order = ids.clone();
        move_node(&mut order, ids[0], *ids.last().unwrap(), true);
        assert_eq!(order.last(), Some(&ids[0]));
        let actual: Vec<_> = ordered_nodes(&mut project.nodes, &order, |_| true)
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert_eq!(actual, order);
        assert_eq!(serde_json::to_value(&project).unwrap(), before);
        order.insert(0, u64::MAX);
        let source_ids: Vec<_> = ordered_nodes(&mut project.nodes, &order, |n| n.kind.is_source())
            .into_iter()
            .map(|n| n.id)
            .collect();
        let expected: Vec<_> = order
            .iter()
            .copied()
            .filter(|id| {
                project
                    .nodes
                    .iter()
                    .any(|n| n.id == *id && n.kind.is_source())
            })
            .collect();
        assert_eq!(source_ids, expected);
        let partial: Vec<_> = ordered_nodes(&mut project.nodes, &[ids[1]], |_| true)
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert_eq!(partial[0], ids[1]);
        assert_eq!(partial.len(), ids.len());
        let mut settings = piratewave_core::shortcuts::UiPreferences::default();
        settings
            .mixer_orders
            .insert("workspace-a".into(), order.clone());
        settings.mixer_orders.insert("workspace-b".into(), ids);
        let restored: piratewave_core::shortcuts::UiPreferences =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored.mixer_orders["workspace-a"], order);
        let legacy: piratewave_core::shortcuts::UiPreferences = serde_json::from_str("{}").unwrap();
        assert!(legacy.mixer_orders.is_empty());
    }
}
