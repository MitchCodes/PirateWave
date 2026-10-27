use super::*;
use crate::training::{self as backend, Settings, StorageUsage};

pub(super) struct TrainingUi {
    pub open: bool,
    guide: bool,
    settings: Settings,
    usage: Option<StorageUsage>,
    scan: Option<mpsc::Receiver<Result<StorageUsage, String>>>,
    notice: String,
    task: Option<backend::release::Task>,
    task_started: Option<Instant>,
    job: Option<std::path::PathBuf>,
    phase: usize,
    name: String,
    backend: String,
    accepted_beta: bool,
    role: String,
    input: Option<String>,
    monitor: Option<String>,
}
impl Default for TrainingUi {
    fn default() -> Self {
        let (settings, notice) = match Settings::load() {
            Ok(settings) => (settings, String::new()),
            Err(error) => (
                Settings::default(),
                format!("Could not load training storage preferences: {error:#}"),
            ),
        };
        Self {
            open: false,
            guide: false,
            settings,
            usage: None,
            scan: None,
            notice,
            task: None,
            task_started: None,
            job: None,
            phase: 0,
            name: "My voice".into(),
            backend: "cpu".into(),
            accepted_beta: false,
            role: "train".into(),
            input: None,
            monitor: None,
        }
    }
}
impl TrainingUi {
    pub(super) fn shutdown(&mut self) {
        if let Some(task) = &mut self.task {
            task.shutdown();
        }
    }
    fn poll(&mut self) {
        if self.task.is_some() && self.task_started.is_none() {
            self.task_started = Some(Instant::now());
        }
        if let Some(result) = self
            .task
            .as_ref()
            .and_then(|task| task.result.try_recv().ok())
        {
            self.task = None;
            self.task_started = None;
            self.notice = match result {
                Ok(text) => text,
                Err(error) => error,
            };
        }
    }
    pub(super) fn smoke_page(&mut self, guide: bool) {
        self.open = true;
        self.guide = guide;
    }
    #[cfg(test)]
    pub(super) fn smoke_job(&mut self, path: PathBuf, phase: usize) {
        self.smoke_page(false);
        self.job = Some(path);
        self.phase = phase;
    }
    fn scan(&mut self) {
        if self.scan.is_some() {
            return;
        }
        let root = self.settings.workspace.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(backend::scan(&root).map_err(|error| format!("{error:#}")));
        });
        self.scan = Some(rx);
    }
    fn storage(&mut self, ui: &mut egui::Ui) {
        self.poll();
        if let Some(result) = self.scan.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.scan = None;
            match result {
                Ok(usage) => self.usage = Some(usage),
                Err(error) => {
                    self.usage = None;
                    self.notice = error;
                }
            }
        }
        ui.heading("Voice Training & Storage");
        ui.label("Choose where optional training tools and working data will live. Changing this preference does not move or delete existing files.");
        ui.label(
            RichText::new(self.settings.workspace.display().to_string())
                .color(theme::palette().accent),
        );
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.scan.is_none(), egui::Button::new("Choose workspace…"))
                .clicked()
            {
                if let Some(root) = rfd::FileDialog::new().pick_folder() {
                    let candidate = Settings {
                        schema: 1,
                        workspace: root,
                    };
                    match candidate.save() {
                        Ok(()) => {
                            self.settings = candidate;
                            self.usage = None;
                            self.notice =
                                "Workspace preference saved. No tools were installed.".into();
                            self.scan();
                        }
                        Err(error) => self.notice = format!("Could not save workspace: {error:#}"),
                    }
                }
            }
            if ui
                .add_enabled(self.scan.is_none(), egui::Button::new("Measure storage"))
                .clicked()
            {
                self.scan();
            }
            if self.scan.is_some() {
                ui.spinner();
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
        });
        if !self.notice.is_empty() {
            ui.label(&self.notice);
        }
        if let Some(usage) = &self.usage {
            egui::Grid::new("training-storage")
                .num_columns(2)
                .spacing([30.0, 9.0])
                .show(ui, |ui| {
                    for (name, value) in [
                        ("Training tools", usage.tools),
                        ("Shared models and corpus", usage.shared),
                        ("Recordings", usage.recordings),
                        ("Training jobs and checkpoints", usage.jobs),
                        ("Download cache", usage.downloads),
                        ("Other workspace files", usage.other),
                    ] {
                        ui.label(name);
                        ui.label(backend::bytes(value));
                        ui.end_row();
                    }
                    ui.label("Available on workspace drive");
                    ui.label(
                        usage
                            .free
                            .map(backend::bytes)
                            .unwrap_or_else(|| "Unavailable".into()),
                    );
                    ui.end_row();
                });
            ui.small("Logical file sizes; links and junctions are excluded. Only the selected workspace is measured. Existing live voice packs are stored separately.");
        }
        ui.add_space(12.0);
        for bundle in backend::release::catalog().bundles {
            ui.label(format!(
                "{}: {} download, {} installed",
                bundle.backend,
                backend::bytes(bundle.bytes),
                backend::bytes(bundle.installed_bytes)
            ));
        }
        ui.label(format!("Beta job growth estimate: {} for prepared speech, generated pairs, latest/previous checkpoints, atomic-save staging and export, plus imported recordings. Actual usage varies; disk space is rechecked while training.", backend::bytes(backend::release::JOB_PEAK_BYTES)));
        ui.label(format!("Space planning includes archive/extraction staging and at least {} headroom, or 10% of estimated peak additional space, whichever is larger.", backend::bytes(backend::required_space(0).unwrap())));
        ui.small("No training tools download automatically. Source speech and supporting models will be listed separately before installation. Original recordings and installed voices are never automatically removed.");
    }
    fn wizard(
        &mut self,
        ui: &mut egui::Ui,
        inputs: &[audio_io::DeviceInfo],
        outputs: &[audio_io::DeviceInfo],
    ) {
        use backend::release as service;
        self.poll();
        ui.colored_label(theme::palette().peach,"Voice Creation — Beta. Current trained voices can sound robotic or metallic. Results are not guaranteed to match the recording convincingly.");
        ui.horizontal_wrapped(|ui| {
            for (i, title) in [
                "1 Introduction",
                "2 Record/import",
                "3 Review",
                "4 Tools/storage",
                "5 Train",
                "6 Compare",
                "7 Install",
            ]
            .iter()
            .enumerate()
            {
                ui.selectable_value(&mut self.phase, i, *title);
            }
        });
        if !self.notice.is_empty() {
            ui.label(&self.notice);
        }
        ui.collapsing("Help for this stage", |ui| {
            ui.label([
                "Training learns the character of one speaker from dry recordings. Choose CPU or NVIDIA CUDA and a drive with room for tools and checkpoints. Current Beta voices can sound robotic. You can keep using the mixer while recording or reviewing; training pauses when audio starts.",
                "Choose a microphone to record directly, or import recordings you have permission to use. Assign different takes to train, validation and test. Training teaches the voice; validation and test recordings check material it has not learned. Keep the same microphone distance and avoid effects, music and clipping.",
                "Listen through headphones on an explicitly selected monitoring device. Exclude noisy, clipped or inconsistent takes. Excluding a clip preserves the original. Quiet percentages measure level, not whether someone is speaking. This Beta accepts up to eight minutes of training takes and thirty minutes overall.",
                "Install the selected CPU or CUDA tools plus the shared models and source speech. Downloads are optional and checked against this build's hashes. Offline installation uses the same part files. The displayed peak includes archives, extraction and workspace growth; leave extra room for other applications.",
                "The teacher first learns your voice, then creates paired examples for the faster LLVC model. Later stages export and check native playback. Loss values are training diagnostics, not a quality score. Pause saves progress; resume is explicit. Starting PirateWave audio requests a pause immediately.",
                "Compare the same words in Original, Teacher and LLVC samples. Listen for clear consonants, intact syllables and a consistent voice. Native preview uses the exported live model. Select a separate monitoring output; previews never enter a PirateWave bus automatically.",
                "Install only after native checks pass. Select the new preset on a Voice Changer node and route microphone → cleanup effects → Voice Changer → output. Avoid sending both dry and converted speech to the same output. Failures silence conversion; Disable deliberately restores your original voice.",
            ][self.phase.min(6)]);
        });
        if let Some(task) = &self.task {
            if let Some(started) = self.task_started {
                ui.small(format!(
                    "Operation elapsed: {}m {:02}s",
                    started.elapsed().as_secs() / 60,
                    started.elapsed().as_secs() % 60
                ));
            }
            ui.horizontal(|ui| {
                ui.spinner();
                if ui.button("Pause / stop preview or recording").clicked() {
                    task.pause();
                }
                if ui.button("Cancel operation").clicked() {
                    task.cancel();
                }
            });
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        let busy = self.task.is_some();
        if self.phase == 0 {
            ui.label("Record one consistent speaker. Training runs locally using optional CPU or NVIDIA CUDA tools, with no separate Python installation. Heavy work pauses when PirateWave audio starts. Training loss is not a voice-quality score.");
            ui.checkbox(&mut self.accepted_beta,"I understand the current Beta quality limitations and have permission to use my recordings");
            ui.horizontal(|ui| {
                ui.label("Voice name");
                ui.text_edit_singleline(&mut self.name);
            });
            egui::ComboBox::from_id_salt("training-backend")
                .selected_text(&self.backend)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.backend, "cpu".into(), "CPU");
                    ui.selectable_value(&mut self.backend, "cuda".into(), "NVIDIA CUDA");
                });
            ui.small("CUDA requires a compatible NVIDIA driver. No automatic CPU fallback. DirectML, Intel XPU and AMD training are unavailable in this release; live DirectML conversion is separate.");
            if ui
                .add_enabled(!busy, egui::Button::new("Check training hardware"))
                .clicked()
            {
                self.task = Some(backend::hardware::check());
            }
            ui.label(format!(
                "New-project workspace: {}",
                self.settings.workspace.display()
            ));
            if ui
                .add_enabled(
                    !busy,
                    egui::Button::new("Choose workspace before creating project…"),
                )
                .clicked()
            {
                if let Some(workspace) = rfd::FileDialog::new().pick_folder() {
                    let settings = Settings {
                        schema: 1,
                        workspace,
                    };
                    match settings.save() {
                        Ok(()) => self.settings = settings,
                        Err(e) => self.notice = format!("{e:#}"),
                    }
                }
            }
            if ui
                .add_enabled(
                    !busy && self.accepted_beta,
                    egui::Button::new("Create training project"),
                )
                .clicked()
            {
                match service::create(
                    &self.settings.workspace,
                    self.name.clone(),
                    self.backend.clone(),
                ) {
                    Ok(path) => {
                        self.job = Some(path);
                        self.phase = 1;
                        self.notice="Project created. Install the curated LLVC voice pack on Voices before training; it supplies the trusted live runtime.".into();
                    }
                    Err(e) => self.notice = format!("{e:#}"),
                }
            }
            ui.separator();
            ui.label("Resume a saved project");
            for (path, job) in service::jobs(&self.settings.workspace) {
                if ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(format!("{} — {}", job.name, job.status)),
                    )
                    .clicked()
                {
                    self.job = Some(path);
                    self.phase = 4;
                }
            }
            return;
        }
        let Some(folder) = self.job.clone() else {
            ui.label("Create or select a project in Introduction first.");
            return;
        };
        let mut job = match service::Job::load(&folder) {
            Ok(j) => j,
            Err(e) => {
                ui.label(format!("{e:#}"));
                return;
            }
        };
        ui.label(format!("{} · {} · {}", job.name, job.backend, job.status));
        ui.small(folder.display().to_string());
        match self.phase {
            1 => {
                ui.label("Import dry recordings or record the selected microphone directly, independently of the effects graph. Make separate takes for training, validation and test. Five-minute recording limit per take; imports support up to 30 minutes per file.");
                egui::ComboBox::from_id_salt("recording-role")
                    .selected_text(&self.role)
                    .show_ui(ui, |ui| {
                        for role in ["train", "validation", "test"] {
                            ui.selectable_value(&mut self.role, role.into(), role);
                        }
                    });
                device_combo(ui, 900501, &mut self.input, inputs, true);
                ui.add_enabled_ui(!busy && job.status == "draft", |ui| {
                    if ui.button("Import WAV / FLAC / MP3 / Ogg…").clicked() {
                        if let Some(paths) = rfd::FileDialog::new()
                            .add_filter("Audio", &["wav", "flac", "mp3", "ogg"])
                            .pick_files()
                        {
                            self.task =
                                Some(service::import(folder.clone(), paths, self.role.clone()));
                        }
                    }
                    if ui.button("Record raw microphone").clicked() {
                        self.task = Some(service::record(
                            folder.clone(),
                            self.input.clone(),
                            self.role.clone(),
                        ));
                    }
                });
                ui.small("Keep a quiet room, consistent microphone distance and level, and varied speech. This Beta accepts up to eight minutes of included training takes; recording requirements for better quality remain unmeasured. Do not include music, effects or another speaker.");
            }
            2 => {
                ui.label("Quiet sections are estimated by level, not a speech detector. Noise and echo need your listening review. Exclusions preserve copied recordings and originals. Dataset edits lock once training starts.");
                ui.small("Beta limit: eight minutes included in the training role, thirty minutes across all roles. Record several shorter takes so you can exclude unsuitable clips without changing originals.");
                device_combo(ui, 900502, &mut self.monitor, outputs, false);
                let mut changed = false;
                for recording in &mut job.recordings {
                    show_card(ui, |ui| {
                        ui.label(format!(
                            "{} · {:.1}s · clipped {:.2}% · quiet {:.1}%",
                            recording.name,
                            recording.seconds,
                            recording.clipping * 100.,
                            recording.silence * 100.
                        ));
                        ui.add_enabled_ui(!busy && job.status == "draft", |ui| {
                            changed |= ui
                                .checkbox(&mut recording.excluded, "Exclude this clip")
                                .changed();
                            egui::ComboBox::from_id_salt(&recording.id)
                                .selected_text(&recording.role)
                                .show_ui(ui, |ui| {
                                    for role in ["train", "validation", "test"] {
                                        changed |= ui
                                            .selectable_value(
                                                &mut recording.role,
                                                role.into(),
                                                role,
                                            )
                                            .changed();
                                    }
                                });
                        });
                        if ui
                            .add_enabled(!busy, egui::Button::new("Audition on monitoring device"))
                            .clicked()
                        {
                            self.task = Some(service::audition(
                                folder.join(&recording.path),
                                self.monitor.clone(),
                            ));
                        }
                    });
                }
                if changed {
                    if let Err(e) = job.save(&folder) {
                        self.notice = format!("{e:#}");
                    }
                }
                for role in ["train", "validation", "test"] {
                    ui.label(format!(
                        "{role}: {:.1} seconds included",
                        job.recordings
                            .iter()
                            .filter(|r| !r.excluded && r.role == role)
                            .map(|r| r.seconds)
                            .sum::<f32>()
                    ));
                }
            }
            3 => {
                ui.label("Install the selected tools and shared licensed models/source speech explicitly. Online downloads use the matching GitHub Release; offline import selects the folder containing all matching bundle parts. Nothing installs drivers or changes system Python.");
                let workspace = folder
                    .parent()
                    .and_then(|p| p.parent())
                    .unwrap()
                    .to_path_buf();
                for backend in [job.backend.as_str(), "shared"] {
                    if let Ok(bundle) = service::bundle(backend) {
                        ui.heading(backend);
                        ui.label(if service::installed(&workspace, backend).is_ok() {
                            "Installed; integrity is checked before training"
                        } else {
                            "Not installed"
                        });
                        ui.label(format!("Download {} · Installed {} · installation peak {} including cache/staging and headroom",backend::bytes(bundle.bytes),backend::bytes(bundle.installed_bytes),backend::bytes(backend::required_space(bundle.bytes*2+bundle.installed_bytes).unwrap_or(u64::MAX))));
                        ui.add_enabled_ui(!busy, |ui| {
                            if ui
                                .button(format!("Download and install {backend}"))
                                .clicked()
                            {
                                self.task =
                                    Some(service::install(workspace.clone(), backend.into(), None));
                            }
                            if ui
                                .button(format!("Import offline {backend} bundle…"))
                                .clicked()
                            {
                                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                    self.task = Some(service::install(
                                        workspace.clone(),
                                        backend.into(),
                                        Some(path),
                                    ));
                                }
                            }
                        });
                    }
                }
                ui.small("This project keeps its workspace and tool revision. Changing Settings → Voice Training & Storage affects new projects only. Shared speech is LibriSpeech dev-clean (CC BY 4.0); its size is separate from the tools.");
                let additional = [job.backend.as_str(), "shared"]
                    .iter()
                    .filter_map(|kind| {
                        service::bundle(kind)
                            .ok()
                            .filter(|_| service::installed(&workspace, kind).is_err())
                    })
                    .fold(service::JOB_PEAK_BYTES, |total, b| {
                        total.saturating_add(b.bytes * 2 + b.installed_bytes)
                    });
                ui.label(format!("Overall estimated additional peak including staging, job growth and headroom: {}. Existing recordings already occupy space.",backend::bytes(backend::required_space(additional).unwrap_or(u64::MAX))));
            }
            4 => {
                ui.label("Recipe: no-F0 RVC teacher (250 updates), 100 paired source clips, pretrained LLVC adaptation (1,000 updates). One heavy job at a time. Start audio pauses heavy work; resume is always explicit.");
                ui.colored_label(theme::palette().peach,"Current quality limitation: the adapted voices sounded more robotic than the pretrained voice in our tests. These update counts are an experimental Beta recipe, not a guaranteed convergence target.");
                ui.label(&job.message);
                if let Ok(bytes) = std::fs::read(folder.join("live.json")) {
                    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        if let (Some(step), Some(total), Some(seconds)) = (
                            value["step"].as_u64(),
                            value["total"].as_u64(),
                            value["seconds"].as_f64(),
                        ) {
                            let estimate = (total.saturating_sub(step)) as f64 * seconds;
                            ui.small(format!("Current-stage estimate: {:.0}–{:.0} seconds remaining, based on the latest update; later stages are additional.",estimate*0.5,estimate*1.5));
                        }
                    }
                }
                for name in ["teacher.checkpoint.pt", "llvc.checkpoint.pt"] {
                    if let Ok(meta) = std::fs::metadata(folder.join(name)) {
                        ui.small(format!(
                            "Saved checkpoint: {} ({})",
                            name,
                            backend::bytes(meta.len())
                        ));
                    }
                }

                if let Ok(bytes) = std::fs::read(folder.join("live.json")) {
                    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        ui.label(format!(
                            "Stage: {} · update {} / {}",
                            value["stage"].as_str().unwrap_or("working"),
                            value["step"],
                            value["total"]
                        ));
                        if let (Some(step), Some(total)) =
                            (value["step"].as_u64(), value["total"].as_u64())
                        {
                            if total > 0 {
                                ui.add(
                                    egui::ProgressBar::new(
                                        (step as f32 / total as f32).clamp(0., 1.),
                                    )
                                    .show_percentage(),
                                );
                            }
                        }
                    }
                }
                if ui
                    .add_enabled(
                        !busy && job.status != "complete" && job.status != "cancelled",
                        egui::Button::new("Start / resume local training"),
                    )
                    .clicked()
                {
                    self.task = Some(service::start(folder.clone()));
                }
                ui.small("Reference resumed runs with short test recordings: 76 minutes for CPU preparation/training on an i9-10900K, versus 7 minutes on an RTX 4070 SUPER. Downloads, integrity checks and export are additional. Different recordings/hardware vary; current-stage estimates appear during training.");
                ui.small("Starting/resuming verifies every installed tool file. Checking the large CUDA bundle can take several minutes on a slower drive.");
                ui.small("Failed or interrupted jobs keep prior checkpoints. Diagnostics: report.json and worker.stderr.txt in this project's folder. Do not edit data or tools behind a paused job.");
                ui.add_enabled_ui(!busy, |ui| {
                    if ui
                        .button("Remove generated pairs (can be rebuilt)")
                        .clicked()
                    {
                        self.task = Some(service::cleanup(folder.clone(), false));
                    }
                    if ui
                        .button("Remove checkpoints (lose resumability)")
                        .clicked()
                    {
                        self.task = Some(service::cleanup(folder.clone(), true));
                    }
                });
            }
            5 => {
                ui.label("Compare original, teacher and LLVC on held-out source speakers. Preview goes directly to your selected monitoring output and never enters a PirateWave outgoing bus automatically. Use headphones and avoid routing monitoring loopback into your stream.");
                device_combo(ui, 900503, &mut self.monitor, outputs, false);
                for index in 3..=5 {
                    ui.heading(format!("Held-out source {}", index - 2));
                    ui.horizontal(|ui| {
                        for (label, kind) in [
                            ("Original", "original"),
                            ("Teacher", "teacher"),
                            ("LLVC", "llvc-offline"),
                        ] {
                            let path = folder.join(format!("review/{index}-{kind}.wav"));
                            if ui
                                .add_enabled(!busy && path.exists(), egui::Button::new(label))
                                .clicked()
                            {
                                self.task = Some(service::audition(path, self.monitor.clone()));
                            }
                        }
                    });
                }
                let native = folder.join("native/native-converted.wav");
                if ui
                    .add_enabled(
                        !busy && native.exists(),
                        egui::Button::new("Native streaming preview"),
                    )
                    .clicked()
                {
                    self.task = Some(service::audition(native, self.monitor.clone()));
                }
            }
            6 => {
                ui.label("A complete job has passed short native waveform/state and CPU timing checks. This Beta does not claim thirty-minute stability, gaming-impact or measured end-to-end latency qualification for each custom voice. DirectML remains unavailable for custom models until independently validated.");
                if ui
                    .add_enabled(
                        !busy && job.status == "complete",
                        egui::Button::new("Install this CPU voice revision"),
                    )
                    .clicked()
                {
                    let path = folder.clone();
                    self.task = Some(service::Task::spawn(move |_| {
                        let id = crate::voice::local::install(&path)?;
                        Ok(format!(
                            "Voice installed: {id}. Select it on a Voice Changer node."
                        ))
                    }));
                }
                ui.small("The installed revision stores model data, notices and validation results. It never installs executable code or training checkpoints. Missing voices retain project references and produce silence until reinstalled.");
            }
            _ => {}
        }
        ui.add_space(12.);
        ui.horizontal(|ui| {
            if self.phase > 0 && ui.button("< Previous").clicked() {
                self.phase -= 1;
            }
            if self.phase < 6 && ui.button("Next >").clicked() {
                self.phase += 1;
            }
        });
    }
}
impl PirateWaveApp {
    pub(super) fn training_storage_page(&mut self, ui: &mut egui::Ui) {
        self.voice_ui.training.storage(ui);
        ui.add_space(15.0);
        if ui.button("Voice Creation Guide").clicked() {
            self.voice_ui.training.open = true;
            self.voice_ui.training.guide = true;
            self.view = View::Voices;
        }
    }
    pub(super) fn voice_creation_page(&mut self, ui: &mut egui::Ui) {
        self.voice_ui.training.poll();
        if ui.button("< Back to Voices").clicked() {
            self.voice_ui.training.open = false;
        }
        ui.heading("Create a voice");
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.voice_ui.training.guide, false, "Create Voice");
            ui.selectable_value(
                &mut self.voice_ui.training.guide,
                true,
                "Voice Creation Guide",
            );
        });
        ui.add_space(12.0);
        if self.voice_ui.training.guide {
            ui.label("Recording examples (LibriSpeech, CC BY 4.0). Listen for dry speech versus distortion or room echo. Return to Create Voice to continue your saved step.");
            device_combo(
                ui,
                900504,
                &mut self.voice_ui.training.monitor,
                &self.devices.outputs,
                false,
            );
            ui.horizontal_wrapped(|ui| {
                for (title, file) in [
                    ("Clean speech", "voice-clean.wav"),
                    ("Clipped speech", "voice-clipped.wav"),
                    ("Room echo", "voice-room-echo.wav"),
                ] {
                    if ui
                        .add_enabled(
                            self.voice_ui.training.task.is_none(),
                            egui::Button::new(title),
                        )
                        .clicked()
                    {
                        if let Ok(exe) = std::env::current_exe() {
                            self.voice_ui.training.task = Some(backend::release::audition(
                                exe.parent().unwrap().join("docs/audio").join(file),
                                self.voice_ui.training.monitor.clone(),
                            ));
                        }
                    }
                }
                if ui.button("Stop example").clicked() {
                    if let Some(task) = &self.voice_ui.training.task {
                        task.pause();
                    }
                }
            });
            ui.vertical(|ui| {
                ui.set_max_width(920.0);
                for (heading, text) in backend::GUIDE {
                    show_card(ui, |ui| {
                        ui.heading(*heading);
                        ui.add(egui::Label::new(*text).wrap());
                    });
                    ui.add_space(12.0);
                }
            });
        } else {
            self.voice_ui
                .training
                .wizard(ui, &self.devices.inputs, &self.devices.outputs);
        }
    }
}
