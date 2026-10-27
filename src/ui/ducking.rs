use super::*;
impl PirateWaveApp {
    pub(super) fn ducking_page(&mut self, ui: &mut egui::Ui, id: u64) {
        if self.view != View::Effects && self.node_header(ui, id) {
            return;
        }
        let Some(node) = self
            .project
            .nodes
            .iter_mut()
            .find(|n| n.id == id && matches!(n.kind, NodeKind::AutoDucking))
        else {
            ui.label("Auto Ducking node removed.");
            return;
        };
        ui.small("Priority lowers Ducked audio; Mixed combines both outputs.");
        let mut changed = ui
            .checkbox(&mut node.effects_disabled, "Bypass ducking")
            .changed();
        show_card(ui, |ui| {
            ui.heading("Activity");
            if let Some(status) = self.status.ducking.get(&id) {
                if status.fast_path {
                    ui.small("Detection paused: one input is disconnected or silent. Remaining release and lookahead keep running.");
                }
                ui.horizontal(|ui| {
                    ui.label("Priority input");
                    meter(ui, status.priority_peak, Vec2::new(160., 8.), false);
                    ui.label("Ducked input");
                    meter(ui, status.ducked_peak, Vec2::new(160., 8.), false);
                });
                ui.label(format!(
                    "{} • {:.1} dB gain reduction",
                    if status.active {
                        "Priority active"
                    } else {
                        "Priority inactive"
                    },
                    status.reduction_db
                ));
                if node.ducking.detection == piratewave_core::ducking::Detection::Speech {
                    ui.label(format!(
                        "Speech probability: {:.0}%",
                        status.speech_probability * 100.
                    ));
                }
                if status.fallback {
                    ui.colored_label(
                    theme::palette().peach,
                    "Speech detection unavailable: using Level detection; retrying automatically.",
                );
                }
            } else {
                ui.label("Start audio to see input activity and gain reduction.");
            }
        });
        ui.add_space(10.);
        for (title, ids) in [
            ("Detection", &["detection", "threshold", "sensitivity"][..]),
            (
                "Attenuation & timing",
                &["amount", "attack", "hold", "release"][..],
            ),
            ("Lookahead", &["lookahead_enabled", "lookahead"][..]),
        ] {
            show_card(ui, |ui| {
                ui.heading(title);
                for p in piratewave_core::ducking::PARAMETERS
                    .iter()
                    .filter(|p| ids.contains(&p.id))
                {
                    if p.id == "sensitivity"
                        && node.ducking.detection != piratewave_core::ducking::Detection::Speech
                    {
                        continue;
                    }
                    let mut value = node.ducking.value(p.id);
                    ui.horizontal(|ui| {
                        ui.add_sized([150., 22.], egui::Label::new(p.label));
                        if p.choices.is_empty() {
                            changed |= ui
                                .add(
                                    egui::Slider::new(&mut value, p.min..=p.max)
                                        .suffix(format!(" {}", p.unit)),
                                )
                                .changed();
                        } else {
                            egui::ComboBox::from_id_salt(("duck", id, p.id))
                                .selected_text(p.choices[value as usize])
                                .show_ui(ui, |ui| {
                                    for (i, label) in p.choices.iter().enumerate() {
                                        changed |= ui
                                            .selectable_value(&mut value, i as f32, *label)
                                            .changed();
                                    }
                                });
                        }
                    });
                    node.ducking
                        .set(p.id, value)
                        .expect("UI constrains ducking parameters");
                }
            });
            ui.add_space(10.);
        }
        ui.add_space(10.);
        egui::CollapsingHeader::new("Auto Ducking help").id_salt(("duck-help",id)).show(ui,|ui| {
            ui.label("Level mode responds to any sufficiently loud Priority audio. Speech mode responds to likely speech above the level threshold; it does not denoise the audible signal. A higher probability threshold requires more confident speech detection.");
            ui.label("Duck amount sets how far Ducked audio is lowered. Attack fades down, Hold bridges pauses, and Release fades back up. Both stereo channels share the same attenuation.");
            ui.label("Lookahead delays both audio paths equally by the configured time. Off adds no audio delay. Speech analysis takes 10 ms frames; lookahead can help cover that detection time and the attack. Changing lookahead clears its buffer and briefly fades audio back in.");
            ui.label("Bypass removes ducking but keeps lookahead timing. Priority is never ducked. Mixed is the sum of the two separate outputs, so allow headroom on the output bus. Route through separate processor nodes for other effects.");
        });
        if changed {
            self.changed(false);
        }
    }
}
