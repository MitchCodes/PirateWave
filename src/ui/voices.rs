use super::*;
use piratewave_core::voice::VoiceBackend;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
pub(super) struct VoiceUi {
    pub help: bool,
    pub training: super::training::TrainingUi,
    device: Option<String>,
    sample: usize,
    job: Option<(mpsc::Receiver<Result<String, String>>, bool)>,
    notice: String,
    stop: Arc<AtomicBool>,
}
#[cfg(test)]
impl VoiceUi {
    pub(super) fn has_job(&self) -> bool {
        self.job.is_some()
    }
}
impl Default for VoiceUi {
    fn default() -> Self {
        Self {
            help: false,
            training: super::training::TrainingUi::default(),
            device: None,
            sample: 0,
            job: None,
            notice: if crate::voice::pack::locate().is_ok() {
                "Installed; integrity is verified when loading".into()
            } else {
                "Not installed".into()
            },
            stop: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl Drop for VoiceUi {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl PirateWaveApp {
    pub(super) fn poll_voice_jobs(&mut self) {
        let completed = self
            .voice_ui
            .job
            .as_ref()
            .and_then(|(rx, restart)| rx.try_recv().ok().map(|r| (r, *restart)));
        if let Some((result, restart)) = completed {
            self.voice_ui.job = None;
            self.voice_ui.notice = match result {
                Ok(text) => {
                    if restart {
                        self.command(Command::RestartVoice);
                    }
                    text
                }
                Err(error) => error,
            };
        }
    }
    fn delete_voice_button(&mut self, ui: &mut egui::Ui, id: Option<&str>) {
        if ui.add_enabled(!self.status.running && self.voice_ui.job.is_none(), egui::Button::new("Delete installed voice…"))
            .on_hover_text("Stop audio and previews before deleting. Training recordings and projects are kept.").clicked()
            && rfd::MessageDialog::new().set_title("Delete installed voice?")
                .set_description("Remove this installed voice? Workspaces using it will report a missing voice. Training projects and recordings are kept.")
                .set_buttons(rfd::MessageButtons::YesNo).show() == rfd::MessageDialogResult::Yes {
            self.voice_ui.notice = match crate::voice::delete_installed(id) {
                Ok(()) => "Installed voice deleted".into(),
                Err(e) => format!("Could not delete voice: {e:#}"),
            };
        }
    }
    pub(super) fn voices_page(&mut self, ui: &mut egui::Ui) {
        if self.voice_ui.training.open {
            self.voice_creation_page(ui);
            return;
        }
        ui.horizontal(|ui| {
            if ui.button("Create voice…").clicked() {
                self.voice_ui.training.open = true;
            }
            ui.small("Local CPU / CUDA training — Beta");
        });
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            if self.voice_ui.job.is_some() {
                ui.spinner();
            }
            ui.colored_label(theme::palette().accent, &self.voice_ui.notice);
        });
        ui.small("Library downloads include a preview. The first install also downloads shared playback libraries if needed. No training tools required.");
        let installed_voices = crate::voice::local::list();
        for entry in crate::voice::library::catalog() {
            show_card(ui, |ui| {
                ui.heading(&entry.name);
                ui.label(&entry.description);
                ui.small(format!(
                    "Download: {:.1} MB",
                    entry.bytes as f64 / 1_000_000.
                ));
                let installed = installed_voices.iter().any(|v| v.id == entry.id);
                if ui
                    .add_enabled(
                        !installed && self.voice_ui.job.is_none(),
                        egui::Button::new(if installed {
                            "Installed"
                        } else {
                            "Download and install"
                        }),
                    )
                    .clicked()
                {
                    let (tx, rx) = mpsc::channel();
                    thread::spawn(move || {
                        let _ = tx.send(
                            crate::voice::library::download(&entry.id)
                                .map_err(|e| format!("Installation failed: {e:#}")),
                        );
                    });
                    self.voice_ui.job = Some((rx, false));
                    self.voice_ui.notice = "Downloading and verifying voice…".into();
                }
            });
        }
        for voice in installed_voices {
            show_card(ui, |ui| {
                ui.heading(format!("{} — Beta", voice.name));
                ui.label(if voice.id.starts_with("local-library-") { "Library LLVC voice. CPU playback; compatibility is checked on this computer when first used." } else { "User-trained LLVC voice. CPU playback; compatibility is checked on this computer when first used." });
                ui.small(format!("Revision: {}", voice.id));
                self.delete_voice_button(ui, Some(&voice.id));
                device_combo(
                    ui,
                    egui::Id::new(("custom-preview", &voice.id)).value(),
                    &mut self.voice_ui.device,
                    &self.devices.outputs,
                    false,
                );
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            self.voice_ui.job.is_none(),
                            egui::Button::new("Preview this voice"),
                        )
                        .clicked()
                    {
                        let id = voice.id.clone();
                        let device = self.voice_ui.device.clone();
                        let stop = self.voice_ui.stop.clone();
                        stop.store(false, Ordering::Release);
                        let (tx, rx) = mpsc::channel();
                        thread::spawn(move || {
                            let result = (|| {
                                crate::voice::local::resolve(&id)?;
                                play_preview_file(
                                    device,
                                    &stop,
                                    &crate::voice::local::root().join(id).join("preview.wav"),
                                )
                            })()
                            .map(|_| "Preview finished".into())
                            .map_err(|e: anyhow::Error| format!("Preview failed: {e:#}"));
                            let _ = tx.send(result);
                        });
                        self.voice_ui.job = Some((rx, false));
                    }
                    if ui.button("Stop preview").clicked() {
                        self.voice_ui.stop.store(true, Ordering::Release);
                    }
                });
            });
            ui.add_space(8.0);
        }
        let manifest = crate::voice::pack::manifest();
        show_card(ui, |ui| {
            ui.heading("LLVC voice — Beta");
            if crate::voice::pack::locate().is_ok_and(|p| p.is_dir()) {
                self.delete_voice_button(ui, None);
            }
            ui.label(
                "A locally processed voice for live speech. No reference recording is needed.",
            );

            if !manifest.release_qualified {
                ui.colored_label(theme::palette().peach, if manifest.automated_qualified {
                    "Live CPU conversion is available. Beta: listening, hardware latency, gaming impact and clean-Windows acceptance remain pending."
                } else { "Live conversion is unavailable: automated CPU qualification has not passed." });
            }
            if ui
                .add_enabled(
                    self.voice_ui.job.is_none(),
                    egui::Button::new("Install voice pack…"),
                )
                .clicked()
            {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("PirateWave voice pack", &["zip"])
                    .pick_file()
                {
                    let (tx, rx) = mpsc::channel();
                    thread::spawn(move || {
                        let result = crate::voice::pack::install(&path)
                            .map(|_| "LLVC voice installed".into())
                            .map_err(|e| format!("Installation failed: {e:#}"));
                        let _ = tx.send(result);
                    });
                    self.voice_ui.job = Some((rx, true));
                }
            }
            ui.small("Install the optional PirateWave-LLVC-Voice pack supplied with this build. The pack includes its native runtime and notices.");
            ui.hyperlink_to("Download the matching voice pack", "https://github.com/MitchCodes/PirateWave/releases/download/support-assets-v1/PirateWave-LLVC-Voice-v1-beta1.zip");
            ui.separator();
            ui.label("Preview through");
            egui::ComboBox::from_id_salt("voice-preview-sample")
                .selected_text(
                    [
                        "Normal speech",
                        "Quiet speech",
                        "Loud speech",
                        "Second speaker",
                    ][self.voice_ui.sample],
                )
                .show_ui(ui, |ui| {
                    for (i, label) in [
                        "Normal speech",
                        "Quiet speech",
                        "Loud speech",
                        "Second speaker",
                    ]
                    .iter()
                    .enumerate()
                    {
                        ui.selectable_value(&mut self.voice_ui.sample, i, *label);
                    }
                });
            device_combo(
                ui,
                900001,
                &mut self.voice_ui.device,
                &self.devices.outputs,
                false,
            );
            ui.small("Choose headphones or speakers. Preview goes directly to this device, independently of your mixer routes.");
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.voice_ui.job.is_none(),
                        egui::Button::new("Play preview"),
                    )
                    .clicked()
                {
                    let device = self.voice_ui.device.clone();
                    let sample = self.voice_ui.sample;
                    let stop = self.voice_ui.stop.clone();
                    stop.store(false, Ordering::Release);
                    let (tx, rx) = mpsc::channel();
                    thread::spawn(move || {
                        let result = preview(device, &stop, sample)
                            .map(|_| "Preview finished".into())
                            .map_err(|e| format!("Preview failed: {e:#}"));
                        let _ = tx.send(result);
                    });
                    self.voice_ui.job = Some((rx, false));
                }
                if ui.button("Stop preview").clicked() {
                    self.voice_ui.stop.store(true, Ordering::Release);
                }
                if ui.button("Help").clicked() {
                    self.voice_ui.help = true;
                }
            });
            ui.collapsing("Pack information",|ui| {
                ui.label(format!("Voice: {}",manifest.pack));ui.label(format!("Runtime: {}",manifest.runtime));
                ui.small(format!("Model SHA-256: {}",manifest.model_sha256));
                ui.label("One active voice stream. Auto prefers CPU. GPU uses a DirectML transformer with the original CPU encoder and synthesis; the qualified GPU profile requires an RTX 4070 SUPER.");
                ui.label("Conversion adds delay. Keep music on a separate route.");
            });
        });
        ui.add_space(14.0);
        let nodes: Vec<_> = self
            .project
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::VoiceChanger))
            .map(|n| (n.id, n.name.clone()))
            .collect();
        for (id, name) in nodes {
            if ui.button(format!("Configure {name}  >")).clicked() {
                self.view = View::VoiceNode(id);
            }
        }
        if ui.button("+ Add Voice Changer node").clicked() {
            self.new_kind = 8;
            self.new_name = "Voice Changer".into();
            self.add_node = true;
        }
    }
    pub(super) fn voice_node_page(&mut self, ui: &mut egui::Ui, id: u64) {
        if self.view != View::Effects && self.node_header(ui, id) {
            return;
        }
        let already_enabled = piratewave_core::voice::route_selected(&self.project, id)
            && self.project.nodes.iter().any(|n| {
                n.id != id
                    && matches!(n.kind, NodeKind::VoiceChanger)
                    && !n.effects_disabled
                    && piratewave_core::voice::route_selected(&self.project, n.id)
            });
        let Some(node) = self
            .project
            .nodes
            .iter_mut()
            .find(|n| n.id == id && matches!(n.kind, NodeKind::VoiceChanger))
        else {
            ui.label("Voice node was removed.");
            return;
        };
        let mut changed = false;
        let mut restart = false;
        let mut open_voices = false;
        let manifest = crate::voice::pack::manifest();
        let mut voice = node.voice.clone().unwrap_or_default();
        let previous_pack = voice.pack.clone();
        let local_voices = crate::voice::local::list();
        show_card(ui, |ui| {
            ui.heading("Voice & status");
            ui.colored_label(theme::palette().accent, "LLVC voice — Beta");
            if !manifest.release_qualified {
                ui.colored_label(theme::palette().peach,if manifest.automated_qualified {"Live voice conversion enabled — Beta; manual acceptance pending"}else{"Live conversion is unavailable: automated CPU qualification has not passed."});
            }
            let status = self
                .status
                .voice_changers
                .get(&id)
                .map_or("Start audio to load this voice", String::as_str);
            let important = ["fail", "error", "missing", "unavailable"]
                .iter()
                .any(|word| status.to_lowercase().contains(word));
            ui.label(if important {
                status.to_owned()
            } else {
                truncate(status, 180)
            });
            ui.add_enabled_ui(!already_enabled || !node.effects_disabled, |ui| {
                changed |= ui
                    .checkbox(&mut node.effects_disabled, "Disable (passthrough)")
                    .changed();
            });
            if already_enabled && node.effects_disabled {
                ui.small("Disable the other Voice Changer before enabling this one.");
            }
            ui.collapsing("Runtime details",|ui|{ui.label(status);ui.label("Loading, missing packs and conversion failures silence this route. Passthrough deliberately sends your original voice.");});
            if voice.pack != piratewave_core::voice::LLVC_PACK
                && !local_voices.iter().any(|v| v.id == voice.pack)
            {
                ui.colored_label(
                    theme::palette().peach,
                    format!(
                        "Missing voice pack: {}. Saved settings are retained.",
                        voice.pack
                    ),
                );
            }
        });
        ui.add_space(12.);
        show_card(ui, |ui| {
            ui.heading("Conversion controls");
            ui.label("Voice");
            egui::ComboBox::from_id_salt(("voice-preset", id))
                .selected_text(if voice.pack == piratewave_core::voice::LLVC_PACK {
                    "LLVC voice — Beta".into()
                } else {
                    local_voices
                        .iter()
                        .find(|v| v.id == voice.pack)
                        .map(|v| format!("{} — Beta", v.name))
                        .unwrap_or_else(|| format!("Missing pack: {}", voice.pack))
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(
                            &mut voice.pack,
                            piratewave_core::voice::LLVC_PACK.into(),
                            "LLVC voice — Beta",
                        )
                        .changed();
                    for custom in &local_voices {
                        changed |= ui
                            .selectable_value(
                                &mut voice.pack,
                                custom.id.clone(),
                                format!("{} — Beta", custom.name),
                            )
                            .changed();
                    }
                });
            egui::ComboBox::from_id_salt(("voice-backend", id))
                .selected_text(match voice.backend {
                    VoiceBackend::Auto => "Auto",
                    VoiceBackend::Cpu => "CPU",
                    VoiceBackend::DirectMl => "GPU (DirectML + CPU)",
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut voice.backend, VoiceBackend::Auto, "Auto")
                        .changed();
                    changed |= ui
                        .selectable_value(&mut voice.backend, VoiceBackend::Cpu, "CPU")
                        .changed();
                    ui.add_enabled_ui(
                        manifest.directml_qualified
                            && voice.pack == piratewave_core::voice::LLVC_PACK,
                        |ui| {
                            changed |= ui
                                .selectable_value(
                                    &mut voice.backend,
                                    VoiceBackend::DirectMl,
                                    "GPU (DirectML + CPU)",
                                )
                                .changed();
                        },
                    );
                });
            if voice.pack != previous_pack
                && voice.pack != piratewave_core::voice::LLVC_PACK
                && voice.backend == VoiceBackend::DirectMl
            {
                voice.backend = VoiceBackend::Auto;
                changed = true;
            }
            if voice.pack != piratewave_core::voice::LLVC_PACK {
                ui.small("Custom voices use locally validated CPU inference in this Beta. DirectML needs separate model qualification.");
            }

            ui.add_space(8.);
            changed |= ui
                .add(
                    egui::Slider::new(&mut voice.output_db, -60.0..=12.0).text("Output level (dB)"),
                )
                .changed();
        });
        ui.add_space(12.);
        show_card(ui, |ui| {
            ui.heading("Actions");
            ui.horizontal_wrapped(|ui| {
                restart = ui.button("Restart voice").clicked();
                open_voices = ui.button("Voices / installation").clicked();
                if ui.button("Help").clicked() {
                    self.voice_ui.help = true;
                }
            });
        });
        if changed {
            node.voice = Some(voice);
            self.changed(false);
        }
        if restart {
            self.command(Command::RestartVoice);
        }
        if open_voices {
            self.view = View::Voices;
        }
    }
    pub(super) fn voice_help(&mut self, ctx: &egui::Context) {
        egui::Window::new("Voice Changer help").open(&mut self.voice_ui.help).default_width(590.0).default_height(650.0).vscroll(true).show(ctx,|ui| {
            ui.heading("Turn your microphone into another voice");
            for text in [
                "1. Install LLVC voice on the Voices page. Add a Voice Changer node from Routing.",
                "2. Connect microphone -> cleanup effects -> Voice Changer -> output bus. Start audio and wait for Ready.",
                "3. Send the output bus to your usual virtual cable for Discord or OBS. Speak normally, close to your microphone.",
                "4. To hear your original voice, connect the microphone to a separate monitoring bus routed to headphones.",
                "Avoid routing the microphone directly to the same bus as the converted voice: both voices will play together.",
                "Use light cleanup before conversion. Too much noise suppression or a tight gate may cut off syllables.",
                "Conversion needs a short piece of speech before it can produce sound, so it adds delay. Keep music and soundboard audio on their own routes.",
                "Disable (passthrough) sends your original voice deliberately. Loading, missing packs, errors and temporary recovery produce silence instead.",
                "If conversion stops, check the node's status, then press Restart voice. Switching backend restarts conversion; it never switches secretly during speech.",
                "Auto prefers the qualified CPU profile. GPU (DirectML + CPU) moves the transformer decoder to the GPU while keeping the sensitive encoder and synthesis on CPU. This Beta GPU profile requires an RTX 4070 SUPER and the updated voice pack.",
                "Enable, disable and toggle shortcuts are optional and start unbound. Use Keybinds, choose Record, then press your shortcut.",
                "Previews play only on the monitoring device selected on the Voices page. Choose headphones, not a virtual cable used by a call.",
            ] {ui.label(text);ui.add_space(9.0);}
        });
    }
}
fn preview(device: Option<String>, stop: &AtomicBool, sample: usize) -> anyhow::Result<()> {
    anyhow::ensure!(
        device.as_ref().is_some_and(|d| !d.is_empty()),
        "Choose a named monitoring output before playing a preview"
    );
    let path = crate::voice::pack::load_cpu_voice()?;
    let file = [
        "preview.wav",
        "preview-quiet.wav",
        "preview-loud.wav",
        "preview-speaker.wav",
    ]
    .get(sample)
    .ok_or_else(|| anyhow::anyhow!("Invalid preview"))?;
    play_preview_file(device, stop, &path.join(file))
}
fn play_preview_file(
    device: Option<String>,
    stop: &AtomicBool,
    path: &std::path::Path,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        device.as_ref().is_some_and(|d| !d.is_empty()),
        "Choose a named monitoring output before playing a preview"
    );
    let clip = assets::load(path)?;
    let errors = Arc::new(crossbeam_queue::ArrayQueue::new(4));
    let output = audio_io::open_output(
        0,
        &device,
        Arc::new(audio_io::Counters::default()),
        errors.clone(),
    )?;
    let mut at = 0;
    while !stop.load(Ordering::Acquire) {
        if let Some(error) = errors.pop() {
            anyhow::bail!("{error}");
        }
        while at < clip.frames.len() && output.queue.len() < audio_io::TARGET_FRAMES {
            if output.queue.push(clip.frames[at]).is_ok() {
                at += 1;
            } else {
                break;
            }
        }
        if at == clip.frames.len() && output.queue.is_empty() {
            thread::sleep(Duration::from_millis(80));
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}
