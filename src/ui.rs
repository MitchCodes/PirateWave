mod cookbook;
mod theme;
mod workspaces;
use crate::{
    assets::{self, Clip},
    audio_io::{self, Devices},
    engine::{Command, Engine, Status},
    hotkeys::Hotkeys,
    storage,
};
mod application_capture;
mod audio_widgets;
mod controller;
mod ducking;
mod effect_tuning;
mod extensions;
mod keybinds;
mod layout;
#[cfg(test)]
mod layout_tests;
mod mixer;
use mixer::{switch_routes, switch_selector};
mod plugins;
mod recordings;
mod shortcut_recorder;
#[cfg(test)]
mod tests;
mod tools;
mod training;
mod tutorials;
mod voices;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Stroke, Vec2};
use piratewave_core::{
    effects::{EffectRegistry, EffectSpec},
    project::{Node, NodeKind, Project, Sound},
    shortcuts::{AppAction, AppSettings, NodeAction, ShortcutAction},
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum View {
    Cookbook,
    Tools,
    Tool(crate::tools::Tool),
    Mixer,
    Effects,
    Soundboard,
    Routing,
    Settings,
    NodeKeybinds(u64),
    Plugins,
    PluginNode(u64),
    Voices,
    VoiceNode(u64),
    Recording(u64),
    Ducking(u64),
    RecordingSlotKeybinds(u64, u64),
}
impl View {
    fn name(self) -> &'static str {
        match self {
            Self::Cookbook => "Cookbook",
            Self::Tools => "Tools",
            Self::Tool(tool) => tool.name(),
            Self::Ducking(_) => "Auto Ducking",
            Self::Recording(_) => "Recording & playback",
            Self::RecordingSlotKeybinds(..) => "Recording slot keybinds",
            Self::Voices => "Voices",
            Self::VoiceNode(_) => "Voice Changer",
            Self::Plugins => "VST Plugins",
            Self::PluginNode(_) => "Plugin controls",
            Self::Mixer => "Mixer",
            Self::Effects => "Effects rack",
            Self::Soundboard => "Soundboard",
            Self::Routing => "Routing",
            Self::Settings => "Settings",
            Self::NodeKeybinds(_) => "Node keybinds",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::Cookbook => "Recipes for your sound.",
            Self::Tools => "Your audio workbench.",
            Self::Tool(_) => "Explore your signal.",
            Self::Ducking(_) => "Make room for priority.",
            Self::Recording(_) => "Keep the moment.",
            Self::RecordingSlotKeybinds(..) => "One slot. Your shortcuts.",
            Self::Voices => "Find your other voice.",
            Self::VoiceNode(_) => "Make it your voice.",
            Self::Plugins => "Expand your sound.",
            Self::PluginNode(_) => "Make it your sound.",
            Self::Mixer => "The mixing desk.",
            Self::Effects => "Shape your sound.",
            Self::Soundboard => "Right on cue.",
            Self::Routing => "Connect the dots.",
            Self::Settings => "Make it your workspace.",
            Self::NodeKeybinds(_) => "Control it from anywhere.",
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::Cookbook => "Import an example workspace and make it your own.",
            Self::Tools => "Test, measure, learn and create without changing your mix.",
            Self::Tool(tool) => tool.description(),
            Self::Ducking(_) => "Let the priority signal turn down another audio mix.",
            Self::Recording(_) => "Capture recent mixed audio, crop it, preview it or send it.",
            Self::RecordingSlotKeybinds(..) => "Global shortcuts for one recording slot.",
            Self::Voices => "Install and preview local voices.",
            Self::VoiceNode(_) => "Choose a voice and control conversion.",
            Self::Plugins => "Your instruments and effects, ready to connect.",
            Self::PluginNode(_) => "Play instruments and configure plugin parameters.",
            Self::Mixer => "Every source. Every destination. All in your control.",
            Self::Effects => "Build a processing chain that makes every signal yours.",
            Self::Soundboard => "Your favorite sounds, one global shortcut away.",
            Self::Routing => "Turn individual sources into a signal flow that makes sense.",
            Self::Settings => "Audio diagnostics and application-wide shortcuts.",
            Self::NodeKeybinds(_) => "Optional global shortcuts for one audio node.",
        }
    }
}
struct Loaded {
    id: u64,
    path: String,
    result: Result<Clip, String>,
}
pub struct PirateWaveApp {
    effect_tuning: effect_tuning::TuningUi,
    tools_ui: tools::ToolsUi,
    layout: layout::LayoutState,
    extension_ui: extensions::ExtensionUi,
    controller: controller::Controller,
    project: Project,
    path: PathBuf,
    workspaces: Option<crate::workspaces::Workspaces>,
    workspace_rename: Option<String>,
    workspace_delete: Option<(u64, String)>,
    engine: Engine,
    status: Status,
    devices: Devices,
    applications: Vec<crate::application_capture::Application>,
    applications_error: String,
    registry: EffectRegistry,
    plugin_ui: plugins::PluginUi,
    voice_ui: voices::VoiceUi,
    recording_ui: recordings::RecordingUi,
    view: View,
    selected: u64,
    dirty: bool,
    changed_at: Instant,
    last_apply: Instant,
    notice: String,
    notice_at: Instant,
    app_settings: AppSettings,
    settings_draft: AppSettings,
    settings_section: keybinds::SettingsSection,
    settings_dirty: bool,
    node_binding_draft: BTreeMap<NodeAction, String>,
    node_extra_binding_draft: BTreeMap<ShortcutAction, String>,
    add_node: bool,
    choose_effect: bool,
    effect_help: Option<String>,
    recorder: Option<shortcut_recorder::Recorder>,
    endpoint_states: Vec<crate::windows_audio::EndpointState>,
    new_name: String,
    new_kind: usize,
    pending_port: Option<u64>,
    pending_output: piratewave_core::project::OutputPort,
    selected_edge: Option<usize>,
    hotkeys: Option<Hotkeys>,
    shortcut_errors: Vec<String>,
    clips: HashMap<u64, Clip>,
    loading: HashSet<u64>,
    load_tx: mpsc::Sender<(u64, String)>,
    load_rx: mpsc::Receiver<Loaded>,
    needs_apply: bool,
    needs_restart: bool,
    smoke: bool,
    smoke_stage: usize,
    smoke_at: Instant,
    smoke_waiting: bool,
    smoke_error: Option<String>,
}
impl PirateWaveApp {
    pub fn new(cc: &eframe::CreationContext<'_>, smoke: bool) -> Self {
        theme::apply(&cc.egui_ctx, piratewave_core::shortcuts::Theme::Dark);
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = Vec2::new(10.0, 10.0);
        style.spacing.button_padding = Vec2::new(12.0, 8.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Small, FontId::proportional(11.0));
        cc.egui_ctx.set_style(style);
        let mut path = storage::default_path();
        let workspace_result = if smoke {
            None
        } else {
            Some(crate::workspaces::Workspaces::open(
                path.with_file_name("workspaces"),
                &path,
            ))
        };
        let (workspaces, workspace_error) = match workspace_result {
            Some(Ok(store)) => {
                path = store.path(store.active());
                (Some(store), None)
            }
            Some(Err(e)) => (None, Some(format!("Workspace catalog unavailable: {e:#}"))),
            None => (None, None),
        };
        let mut workspace_restored = false;
        let (mut project, mut notice) = if !smoke && path.exists() {
            match storage::load(&path) {
                Ok(p) => {
                    workspace_restored = true;
                    (p, "Workspace restored".into())
                }
                Err(e) => (
                    Project::default(),
                    format!("Workspace could not be loaded: {e:#}. Original file preserved."),
                ),
            }
        } else {
            (
                Project::default(),
                "Ready. Select devices and start audio. Microphone is routed only to Stream mix."
                    .into(),
            )
        };
        if let Some(store) = &workspaces {
            if let Some(entry) = store.entries().iter().find(|e| e.id == store.active()) {
                project.name.clone_from(&entry.name);
            }
        }
        if let Some(error) = workspace_error {
            notice = error;
        }
        let settings_path = storage::settings_path();
        let mut app_settings = if !smoke && settings_path.exists() {
            match storage::load_settings(&settings_path) {
                Ok(settings) => settings,
                Err(error) => {
                    notice=format!("{notice} Settings could not be loaded: {error:#}. Original file preserved.");
                    AppSettings::default()
                }
            }
        } else {
            AppSettings::default()
        };
        if !smoke {
            match crate::startup::registered() {
                Ok(registered) => app_settings.startup.start_with_windows = registered,
                Err(error) => {
                    notice = format!("{notice} Windows startup status unavailable: {error:#}")
                }
            }
        }
        let devices = audio_io::devices().unwrap_or_default();
        if smoke {
            let mut processor =
                Node::new(6, "Voice processor", NodeKind::Processor, [300.0, 100.0]);
            processor.effects.push(EffectSpec::new(
                EffectRegistry::default().get("gain").unwrap(),
            ));
            project.nodes.push(processor);
        }
        let mut plugin_ui = plugins::PluginUi::new(smoke);
        if smoke {
            plugin_ui.library.plugins = plugins::smoke_plugins();
            for (i, plugin) in plugin_ui.library.plugins.iter().enumerate() {
                let kind = if plugin.role == piratewave_core::plugins::PluginRole::Instrument {
                    NodeKind::VstInstrument
                } else {
                    NodeKind::VstEffect
                };
                let mut node = Node::new(
                    7 + i as u64,
                    &plugin.name,
                    kind,
                    [35.0 + i as f32 * 265.0, 480.0],
                );
                node.plugin = Some(piratewave_core::plugins::NodePlugin::new(plugin.clone()));
                project.nodes.push(node);
            }
            project.connections.extend([
                piratewave_core::project::Connection {
                    input: Default::default(),
                    port: Default::default(),
                    from: 7,
                    to: 8,
                },
                piratewave_core::project::Connection {
                    input: Default::default(),
                    port: Default::default(),
                    from: 8,
                    to: 4,
                },
            ]);
        }
        let hotkeys = if smoke {
            None
        } else {
            match Hotkeys::new() {
                Ok(h) => Some(h),
                Err(e) => {
                    eprintln!("Global shortcuts unavailable: {e}");
                    None
                }
            }
        };
        let (job_tx, job_rx) = mpsc::channel::<(u64, String)>();
        let (result_tx, result_rx) = mpsc::channel();
        thread::Builder::new()
            .name("piratewave-asset-loader".into())
            .spawn(move || {
                while let Ok((id, path)) = job_rx.recv() {
                    let result =
                        assets::load(std::path::Path::new(&path)).map_err(|e| format!("{e:#}"));
                    if result_tx.send(Loaded { id, path, result }).is_err() {
                        break;
                    }
                }
            })
            .expect("Asset loader thread");
        let mut app = Self {
            effect_tuning: Default::default(),
            tools_ui: Default::default(),
            layout: Default::default(),
            extension_ui: Default::default(),
            project,
            path,
            workspaces,
            workspace_rename: None,
            workspace_delete: None,
            engine: Engine::new(),
            status: Status::default(),
            devices,
            applications: if smoke {
                application_capture::smoke_applications()
            } else {
                crate::application_capture::applications().unwrap_or_default()
            },
            applications_error: String::new(),
            registry: EffectRegistry::default(),
            plugin_ui,
            voice_ui: voices::VoiceUi::default(),
            recording_ui: Default::default(),
            view: View::Mixer,
            selected: 1,
            dirty: false,
            changed_at: Instant::now(),
            controller: Default::default(),
            last_apply: Instant::now(),
            notice,
            notice_at: Instant::now(),
            settings_draft: app_settings.clone(),
            app_settings,
            settings_section: keybinds::SettingsSection::General,
            settings_dirty: false,
            node_binding_draft: BTreeMap::new(),
            node_extra_binding_draft: BTreeMap::new(),
            add_node: false,
            choose_effect: false,
            effect_help: None,
            recorder: None,
            endpoint_states: crate::windows_audio::cable_endpoints().unwrap_or_default(),
            new_name: String::new(),
            new_kind: 0,
            pending_port: None,
            pending_output: Default::default(),
            selected_edge: None,
            hotkeys,
            shortcut_errors: vec![],
            clips: HashMap::new(),
            loading: HashSet::new(),
            load_tx: job_tx,
            load_rx: result_rx,
            needs_apply: false,
            needs_restart: false,
            smoke,
            smoke_stage: 0,
            smoke_at: Instant::now(),
            smoke_waiting: false,
            smoke_error: None,
        };
        if !smoke {
            app.refresh_shortcuts();
            app.load_clips();
            app.apply_launch_preferences(&cc.egui_ctx, workspace_restored);
        }
        app
    }
    fn apply_launch_preferences(&mut self, ctx: &egui::Context, workspace_restored: bool) {
        if self.app_settings.startup.start_minimized {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        if self.app_settings.startup.start_audio_automatically {
            if workspace_restored {
                self.command(Command::StartIfStopped(self.project.clone()));
            } else {
                self.notify("Automatic audio start skipped: no valid saved workspace could be restored. Review your devices and start audio manually.");
            }
        }
    }
    fn notify(&mut self, text: impl Into<String>) {
        self.notice = text.into();
        crate::logging::info(format_args!("UI: {}", self.notice));
        self.notice_at = Instant::now();
    }
    fn notify_error(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.controller.last_error = Some(text.clone());
        crate::logging::error(format_args!("UI error: {text}"));
        self.notify(text);
    }
    fn changed(&mut self, restart: bool) {
        self.dirty = true;
        self.changed_at = Instant::now();
        self.needs_apply = true;
        self.needs_restart |= restart;
    }
    fn command(&mut self, command: Command) {
        if let Err(error) = self.engine.send(command) {
            crate::logging::error(format_args!("Engine command failed: {error}"));
            self.notify_error(error);
        }
    }
    fn load_clips(&mut self) {
        for s in &self.project.sounds {
            if !self.clips.contains_key(&s.id) && self.loading.insert(s.id) {
                let _ = self.load_tx.send((s.id, s.path.clone()));
            }
        }
    }
    fn refresh_shortcuts(&mut self) {
        if self.recorder.is_some() {
            return;
        }
        self.shortcut_errors = if let Some(h) = &mut self.hotkeys {
            h.update(&self.project, &self.app_settings)
        } else {
            vec!["Global shortcuts could not initialize".into()]
        };
        for error in &self.shortcut_errors {
            crate::logging::error(format_args!("Shortcut registration: {error}"));
        }
    }
    fn play(&mut self, id: u64) {
        if !self.status.running {
            self.notify("Start audio before playing a sound.");
            return;
        }
        if let Some(clip) = self.clips.get(&id).cloned() {
            let volume = self
                .project
                .sounds
                .iter()
                .find(|s| s.id == id)
                .map_or(1.0, |s| s.volume);
            self.command(Command::Play { clip, volume });
        } else {
            self.notify("This sound has not loaded. Relink its file or wait for loading.");
        }
    }
    fn save(&mut self) {
        match storage::save(&self.path, &self.project) {
            Ok(()) => {
                self.dirty = false;
                self.notify("Workspace saved");
            }
            Err(error) => self.notify_error(format!("Save failed: {error:#}")),
        }
    }
    fn poll(&mut self) {
        if !self.smoke
            && self
                .layout
                .preferences_changed
                .is_some_and(|at| at.elapsed() > Duration::from_millis(700))
        {
            self.layout.preferences_changed = None;
            match storage::save_settings(&storage::settings_path(), &self.app_settings) {
                Ok(()) => self.settings_dirty = false,
                Err(error) => self.notify_error(format!("Cannot save layout: {error:#}")),
            }
        }

        self.poll_recordings();
        self.poll_controller();
        while let Some(status) = self.engine.statuses.pop() {
            if let Some(devices) = &status.devices {
                if !self
                    .status
                    .devices
                    .as_ref()
                    .is_some_and(|previous| std::sync::Arc::ptr_eq(previous, devices))
                {
                    self.devices = devices.as_ref().clone();
                }
            }
            self.status = status;
        }
        let mut messages = vec![];
        while let Ok(loaded) = self.load_rx.try_recv() {
            if !self
                .project
                .sounds
                .iter()
                .any(|s| s.id == loaded.id && s.path == loaded.path)
            {
                continue;
            }
            self.loading.remove(&loaded.id);
            match loaded.result {
                Ok(clip) => {
                    let size: usize = self.clips.values().map(|c| c.frames.len() * 8).sum();
                    if size + clip.frames.len() * 8 > 256 * 1024 * 1024 {
                        messages.push("Sound cache is full (256 MiB). Remove some pads.".into());
                    } else {
                        self.clips.insert(loaded.id, clip);
                    }
                }
                Err(error) => messages.push(format!("Could not load {}: {error}", loaded.path)),
            }
        }
        if !messages.is_empty() {
            self.notify(messages.join(" | "));
        }
        let events = self.hotkeys.as_ref().map(|h| h.poll()).unwrap_or_default();
        for event in events {
            self.handle_shortcut(event);
        }
        if self.needs_apply && self.last_apply.elapsed() > Duration::from_millis(33) {
            if self.needs_restart {
                self.command(Command::Reconfigure(self.project.clone()));
            } else {
                self.command(Command::Update(self.project.clone()));
            }
            self.needs_apply = false;
            self.needs_restart = false;
            self.last_apply = Instant::now();
        }
        if !self.smoke && self.dirty && self.changed_at.elapsed() > Duration::from_secs(1) {
            self.save();
        }
    }
    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("sidebar")
            .exact_width(210.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::surface(Color32::from_rgb(21, 22, 25)))
                    .inner_margin(20.0)
                    .stroke(Stroke::new(1.0_f32, theme::palette().line)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("sidebar_navigation")
                    .max_height((ui.available_height() - 180.0).max(80.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add_space(13.0);
                        ui.horizontal(|ui| {
                            let icon_id = egui::Id::new("piratewave_brand_icon");
                            let texture = ctx
                                .data(|data| data.get_temp::<egui::TextureHandle>(icon_id))
                                .unwrap_or_else(|| {
                                    let image = image::load_from_memory(include_bytes!(
                                        "../assets/branding/piratewave-64.png"
                                    ))
                                    .expect("Bundled icon")
                                    .to_rgba8();
                                    let texture = ctx.load_texture(
                                        "PirateWave",
                                        egui::ColorImage::from_rgba_unmultiplied(
                                            [64, 64],
                                            image.as_raw(),
                                        ),
                                        egui::TextureOptions::LINEAR,
                                    );
                                    ctx.data_mut(|data| data.insert_temp(icon_id, texture.clone()));
                                    texture
                                });
                            ui.image((texture.id(), Vec2::splat(30.0)));
                            ui.label(RichText::new("PirateWave").size(22.0).strong());
                        });
                        ui.add_space(30.0);
                        self.workspace_menu(ui);
                        ui.add_space(28.0);
                        caption(ui, "WORKSPACE");
                        for (view, icon) in [
                            (View::Mixer, "M"),
                            (View::Effects, "FX"),
                            (View::Soundboard, "S"),
                            (View::Routing, "R"),
                            (View::Tools, "T"),
                            (View::Cookbook, "CB"),
                            (View::Plugins, "VST"),
                            (View::Voices, "VC"),
                        ] {
                            let selected = self.view == view
                                || (view == View::Tools && matches!(self.view, View::Tool(_)));
                            let text = RichText::new(format!("{icon}   {}", view.name())).color(
                                if selected {
                                    theme::palette().accent
                                } else {
                                    theme::palette().muted
                                },
                            );
                            if ui
                                .add_sized(
                                    [170.0, 40.0],
                                    egui::Button::new(text)
                                        .fill(if selected {
                                            theme::surface(Color32::from_rgb(43, 35, 57))
                                        } else {
                                            Color32::TRANSPARENT
                                        })
                                        .stroke(Stroke::NONE),
                                )
                                .clicked()
                            {
                                self.view = view;
                            }
                        }
                        ui.add_space(25.0);
                        caption(ui, "PROJECT");
                        if ui.button("Export workspace...").clicked() {
                            self.export_workspace();
                        }
                        if ui.button("Import workspace...").clicked() {
                            self.import_workspace();
                        }
                    });
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.add_space(10.0);
                    ui.small(format!("PIRATEWAVE {}", env!("CARGO_PKG_VERSION")));
                    if ui.button("⚙  Settings").clicked() {
                        self.settings_draft = self.app_settings.clone();
                        self.view = View::Settings;
                    }
                    ui.add_space(10.0);
                    ui.colored_label(
                        if self.status.running {
                            theme::palette().green
                        } else {
                            theme::palette().muted
                        },
                        if self.status.running {
                            "* Audio running"
                        } else {
                            "○ Audio stopped"
                        },
                    );
                    ui.small("48 kHz engine · stereo");
                    let stop_key = self
                        .app_settings
                        .keybinds
                        .get(&AppAction::StopSounds)
                        .map(String::as_str)
                        .filter(|key| !key.trim().is_empty())
                        .unwrap_or("Unbound");
                    ui.small(format!("{stop_key}: stop sounds"));
                });
            });
    }
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Workspace")
                    .small()
                    .color(theme::palette().muted),
            );
            ui.label(RichText::new("/").color(theme::palette().line));
            ui.small(self.view.name());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let text = if self.status.running {
                    "■ Stop audio"
                } else {
                    "▶ Start audio"
                };
                if ui
                    .add(
                        egui::Button::new(RichText::new(text).color(if self.status.running {
                            theme::palette().peach
                        } else {
                            theme::palette().bg
                        }))
                        .fill(if self.status.running {
                            theme::surface(Color32::from_rgb(53, 39, 36))
                        } else {
                            theme::palette().accent
                        }),
                    )
                    .clicked()
                {
                    self.handle_shortcut(ShortcutAction::App(AppAction::ToggleAudio));
                }
                if self.dirty {
                    ui.small("Unsaved changes");
                }
            });
        });
        ui.add_space(7.0);
        ui.separator();
        if matches!(self.view, View::Cookbook | View::Tools | View::Tool(_)) {
            ui.add_space(12.);
            return;
        }
        if matches!(
            self.view,
            View::Mixer
                | View::Routing
                | View::Effects
                | View::Ducking(_)
                | View::VoiceNode(_)
                | View::PluginNode(_)
                | View::Recording(_)
        ) {
            ui.horizontal_wrapped(|ui| {
                ui.strong(&self.project.name);
                ui.small(format!(
                    "DSP {:.1}% · {} active outputs",
                    self.status.render_load, self.status.outputs
                ));
                if matches!(self.view, View::Mixer | View::Routing)
                    && ui.button("+ Add node").clicked()
                {
                    self.add_node = true;
                }
            });
            ui.add_space(12.);
            return;
        }
        ui.add_space(26.0);
        caption(ui, "YOUR SOUND, CONNECTED");
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(RichText::new(self.view.title()).size(30.0));
                ui.label(RichText::new(self.view.description()).color(theme::palette().muted));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if matches!(
                    self.view,
                    View::Settings
                        | View::NodeKeybinds(_)
                        | View::Plugins
                        | View::PluginNode(_)
                        | View::Voices
                        | View::VoiceNode(_)
                ) {
                    return;
                }
                if ui
                    .add_enabled(
                        self.view != View::Effects
                            || !self.project.nodes.iter().any(|n| {
                                n.id == self.selected && matches!(n.kind, NodeKind::Output { .. })
                            }),
                        egui::Button::new(match self.view {
                            View::Soundboard => "+ Import sounds",
                            View::Effects => "+ Add effect",
                            _ => "+ Add node",
                        }),
                    )
                    .clicked()
                {
                    match self.view {
                        View::Soundboard => self.import_sounds(),
                        View::Effects => {
                            if self.project.nodes.iter().any(|n| {
                                n.id == self.selected
                                    && matches!(
                                        n.kind,
                                        NodeKind::Switch
                                            | NodeKind::VoiceChanger
                                            | NodeKind::AutoDucking
                                            | NodeKind::Output { .. }
                                    )
                            }) {
                                self.notify("Choose an effect processor node to add rack effects.");
                            } else {
                                self.choose_effect = true;
                            }
                        }
                        _ => {
                            self.add_node = true;
                        }
                    }
                }
            });
        });
        ui.add_space(24.0);
        show_card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    if self.status.running {
                        theme::palette().green
                    } else {
                        theme::palette().muted
                    },
                    "*",
                );
                ui.strong(&self.project.name);
                ui.label(" | ");
                let count = self
                    .project
                    .nodes
                    .iter()
                    .filter(|n| n.kind.is_source())
                    .count();
                ui.label(
                    RichText::new(format!(
                        "{count} sources   ->   {} output buses",
                        self.project
                            .nodes
                            .iter()
                            .filter(|n| matches!(n.kind, NodeKind::Output { .. }))
                            .count()
                    ))
                    .color(theme::palette().muted),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.small(format!(
                        "DSP {:.1}%  ·  {} active outputs",
                        self.status.render_load, self.status.outputs
                    ));
                });
            });
        });
        ui.add_space(23.0);
    }
    fn effects(&mut self, ui: &mut egui::Ui) {
        if self.node_header(ui, self.selected) {
            return;
        }
        let Some(index) = self
            .project
            .nodes
            .iter()
            .position(|n| n.id == self.selected)
        else {
            return;
        };
        match self.project.nodes[index].kind {
            NodeKind::Extension { .. } => {
                self.extension_node_page(ui, index);
                return;
            }
            NodeKind::AutoDucking => {
                self.ducking_page(ui, self.selected);
                return;
            }
            NodeKind::VoiceChanger => {
                self.voice_node_page(ui, self.selected);
                return;
            }
            NodeKind::VstEffect | NodeKind::VstInstrument => {
                self.plugin_node_page(ui, self.selected);
                return;
            }
            NodeKind::Switch => {
                let routes = switch_routes(&self.project);
                let mut changed = false;
                show_card(ui, |ui| {
                    ui.heading("Destination");
                    switch_selector(ui, &mut self.project.nodes[index], &routes, &mut changed);
                    ui.small("Only the selected destination receives audio.");
                });
                if changed {
                    self.changed(false);
                }
                return;
            }
            NodeKind::Recording => {
                show_card(ui, |ui| {
                    ui.heading("Recording slot effects");
                    ui.small("Capture stays dry; this rack processes playback and export.");
                    if ui.button("< Back to recording").clicked() {
                        self.view = View::Recording(self.selected);
                    }
                    ui.collapsing("Playback and capture details",|ui|{ui.label("Each playback starts with independent effect history. The live passthrough and captured take are unchanged by this rack. Saved audio uses the selected crop and current effects.");});
                });
                ui.add_space(12.);
            }
            _ => {}
        }
        self.rack_controls(ui, index);
    }
    fn rack_controls(&mut self, ui: &mut egui::Ui, index: usize) {
        let mut changed = false;
        let mut add = None;
        if matches!(self.project.nodes[index].kind, NodeKind::Recording) {
            changed |= ui
                .checkbox(
                    &mut self.project.nodes[index].effects_disabled,
                    "Bypass slot effects",
                )
                .changed();
        }
        if self.project.nodes[index].kind.is_processor()
            && self.project.nodes[index].effects_disabled
        {
            ui.colored_label(theme::palette().peach,"This processor is disabled: audio passes through. Enable it on the Mixer page to hear this chain.");
            ui.add_space(10.0);
        }
        let allow_add = !matches!(self.project.nodes[index].kind, NodeKind::Output { .. });
        if !allow_add {
            ui.small("Route audio through a processor node before this output bus to add effects.");
        }
        ui.add_enabled_ui(allow_add, |ui| {
            ui.menu_button("+ Add processor", |ui| {
                if let Some(spec) = extensions::effect_menu(ui, &self.registry) {
                    add = Some(spec);
                }
                for factory in self.registry.factories().iter().filter(|f| {
                    !f.id.contains('/') && !piratewave_core::voice_effects::is_voice_effect(f.id)
                }) {
                    if ui
                        .button(factory.name)
                        .on_hover_text(factory.description)
                        .clicked()
                    {
                        add = Some(EffectSpec::new(factory));
                        ui.close_menu();
                    }
                }
                ui.menu_button("Voice Change Effects", |ui| {
                    for factory in self
                        .registry
                        .factories()
                        .iter()
                        .filter(|f| piratewave_core::voice_effects::is_voice_effect(f.id))
                    {
                        if ui
                            .button(factory.name)
                            .on_hover_text(factory.description)
                            .clicked()
                        {
                            add = Some(EffectSpec::new(factory));
                            ui.close_menu();
                        }
                    }
                });
            });
        });
        if let Some(spec) = add.filter(|_| allow_add) {
            if self.project.nodes[index].effects.len() < 16 {
                self.project.nodes[index].effects.push(spec);
                changed = true;
            } else {
                self.notify("A node supports at most 16 effects.");
            }
        }
        ui.add_space(14.0);
        let node = &mut self.project.nodes[index];
        if node.effects.is_empty() {
            ui.add_space(30.0);
            ui.label(
                RichText::new("A clean signal. Add your first processor above.")
                    .color(theme::palette().muted),
            );
        }
        let mut remove = None;
        let mut swap = None;
        let mut extension_action = None;
        for (i, spec) in node.effects.iter_mut().enumerate() {
            if spec.kind.contains('/') {
                let (edited, deleted, up, action) = extensions::effect_card(ui, node.id, spec, i);
                changed |= edited;
                if deleted {
                    remove = Some(i);
                }
                if up {
                    swap = Some(i);
                }
                if action.is_some() {
                    extension_action = action;
                }
                self.effect_tuning.live_panel(
                    ui,
                    node.id,
                    spec.id,
                    matches!(node.kind, NodeKind::Recording),
                );
                ui.add_space(10.);
                continue;
            }
            let Some(factory) = self.registry.get(&spec.kind) else {
                continue;
            };
            ui.push_id((node.id, spec.id), |ui| {
                show_card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(theme::palette().accent, format!("{:02}  FX", i + 1));
                        ui.strong(factory.name);
                        changed |= ui.checkbox(&mut spec.enabled, "Enabled").changed();
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Remove").clicked() {
                                remove = Some(i);
                            }
                            if ui
                                .small_button("Help")
                                .on_hover_text(
                                    "Learn what this effect does and how to use its controls",
                                )
                                .clicked()
                            {
                                self.effect_help = Some(spec.kind.clone());
                            }
                            if i > 0 && ui.small_button("Move up").clicked() {
                                swap = Some(i);
                            }
                        });
                    });
                    ui.collapsing("About this effect", |ui| {
                        ui.label(factory.description);
                    });
                    changed |= self.effect_tuning.controls(ui, node.id, spec, factory);
                    ui.add_space(8.0);
                    ui.add_enabled_ui(spec.enabled, |ui| {
                        for parameter in factory.parameters {
                            let prior = spec
                                .parameters
                                .get(parameter.id)
                                .copied()
                                .unwrap_or(parameter.default);
                            let mut edited = prior;
                            let value = &mut edited;
                            ui.horizontal(|ui| {
                                ui.add_sized([140.0, 22.0], egui::Label::new(parameter.label));
                                let choices =
                                    if spec.kind == piratewave_core::voice_correction::EFFECT_ID {
                                        piratewave_core::voice_correction::choices(parameter.id)
                                    } else {
                                        piratewave_core::rack_processors::choices(
                                            &spec.kind,
                                            parameter.id,
                                        )
                                    };
                                {
                                    if let Some(choices) = choices {
                                        let selected =
                                            (*value).round().clamp(0.0, (choices.len() - 1) as f32)
                                                as usize;
                                        egui::ComboBox::from_id_salt((
                                            node.id,
                                            spec.id,
                                            parameter.id,
                                        ))
                                        .selected_text(choices[selected])
                                        .show_ui(
                                            ui,
                                            |ui| {
                                                for (index, name) in choices.iter().enumerate() {
                                                    changed |= ui
                                                        .selectable_value(
                                                            value,
                                                            index as f32,
                                                            *name,
                                                        )
                                                        .changed();
                                                }
                                            },
                                        );
                                        return;
                                    }
                                }
                                let mut slider =
                                    egui::Slider::new(value, parameter.min..=parameter.max)
                                        .suffix(format!(" {}", parameter.unit));
                                if spec.kind == piratewave_core::voice_correction::EFFECT_ID
                                    && parameter.id == "transpose"
                                {
                                    slider = slider.integer();
                                }
                                if parameter.min > 0.0 && matches!(parameter.unit, "Hz" | "ms") {
                                    slider = slider.logarithmic(true);
                                }
                                changed |= ui.add(slider).changed();
                                if ui
                                    .small_button("Reset")
                                    .on_hover_text(format!(
                                        "Default: {} {}",
                                        parameter.default, parameter.unit
                                    ))
                                    .clicked()
                                {
                                    *value = parameter.default;
                                    changed |= prior != *value;
                                }
                            });
                            if edited != prior {
                                spec.parameters.insert(parameter.id.into(), edited);
                            }
                        }
                    });
                    self.effect_tuning.live_panel(
                        ui,
                        node.id,
                        spec.id,
                        matches!(node.kind, NodeKind::Recording),
                    );
                });
            });
            ui.add_space(10.0);
        }
        if let Some(i) = remove {
            node.effects.remove(i);
            changed = true;
        }
        if let Some(i) = swap {
            node.effects.swap(i, i - 1);
            changed = true;
        }
        ui.add_space(15.0);
        egui::CollapsingHeader::new("Signal path")
            .id_salt(("rack-signal", node.id))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(format!(
                        "{}  ->  {}  ->  routed destinations",
                        node.name,
                        if node.effects.is_empty() {
                            "passthrough".into()
                        } else {
                            node.effects
                                .iter()
                                .filter(|e| e.enabled)
                                .map(|e| {
                                    self.registry
                                        .get(&e.kind)
                                        .map_or(e.kind.as_str(), |f| f.name)
                                })
                                .collect::<Vec<_>>()
                                .join("  ->  ")
                        }
                    ))
                    .color(theme::palette().muted),
                );
            });
        if remove.is_some() {
            self.refresh_shortcuts();
        }
        if changed {
            self.changed(false);
        }
        if let Some(action) = extension_action {
            self.handle_shortcut(action);
        }
    }
    fn import_sounds(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("Audio", &["wav", "mp3", "flac", "ogg"])
            .pick_files()
        {
            for path in paths {
                if self.project.sounds.len() >= 128 {
                    self.notify("Maximum of 128 sound pads reached");
                    break;
                }
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let used: HashSet<_> = self
                    .project
                    .sounds
                    .iter()
                    .map(|s| s.shortcut.as_str())
                    .collect();
                let shortcut = (1..=12)
                    .map(|i| format!("Ctrl+Alt+F{i}"))
                    .find(|key| !used.contains(key.as_str()))
                    .unwrap_or_default();
                self.project.sounds.push(Sound {
                    id: self.project.next_id(),
                    name,
                    path: path.to_string_lossy().into(),
                    shortcut,
                    volume: 1.0,
                });
            }
            self.load_clips();
            self.refresh_shortcuts();
            self.refresh_shortcuts();
            self.changed(false);
        }
    }
    fn soundboard(&mut self, ui: &mut egui::Ui) {
        let mut record = None;
        ui.horizontal(|ui| {
            if ui.button("■ Stop all sounds").clicked() {
                self.command(Command::StopSounds);
            }
            if ui.button("♪ Test chime").clicked() {
                if self.status.running {
                    self.command(Command::Play {
                        clip: assets::demo(),
                        volume: 1.0,
                    });
                } else {
                    self.notify("Start audio to play the chime.");
                }
            }
            ui.label(
                RichText::new("WAV · MP3 · FLAC · OGG   /   32 voices   /   2 min per clip")
                    .small()
                    .color(theme::palette().muted),
            );
        });
        ui.add_space(18.0);
        if self.project.sounds.is_empty() {
            show_card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add_space(25.0);
                ui.colored_label(
                    theme::palette().accent,
                    RichText::new("Your next great moment starts here.").size(20.0),
                );
                ui.label("Import a sound, assign a global shortcut, and route Soundboard to your output buses.");
                ui.add_space(10.0);
                if ui.button("+ Import your first sounds").clicked() {
                    self.import_sounds();
                }
                ui.add_space(25.0);
            });
        }
        let mut play = None;
        let mut remove = None;
        let mut relink = None;
        let mut changed = false;
        let mut update_keys = false;
        let columns = ((ui.available_width() / 280.0) as usize).max(1);
        egui::Grid::new("sound-pads")
            .num_columns(columns)
            .spacing([14.0, 14.0])
            .show(ui, |ui| {
                for (i, sound) in self.project.sounds.iter_mut().enumerate() {
                    show_card(ui, |ui| {
                        ui.set_width(245.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add_sized(
                                    [45.0, 40.0],
                                    egui::Button::new(
                                        RichText::new("▶").color(theme::palette().accent),
                                    ),
                                )
                                .clicked()
                            {
                                play = Some(sound.id);
                            }
                            ui.vertical(|ui| {
                                changed |= ui.text_edit_singleline(&mut sound.name).changed();
                                if let Some(clip) = self.clips.get(&sound.id) {
                                    ui.small(format!("{:.1} seconds · ready", clip.duration()));
                                } else {
                                    ui.colored_label(
                                        theme::palette().peach,
                                        if self.loading.contains(&sound.id) {
                                            "Loading…"
                                        } else {
                                            "File unavailable"
                                        },
                                    );
                                }
                            });
                        });
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("GLOBAL SHORTCUT")
                                .small()
                                .color(theme::palette().muted),
                        );
                        let response = ui.text_edit_singleline(&mut sound.shortcut);
                        changed |= response.changed();
                        update_keys |= response.lost_focus() && response.changed();
                        if response.lost_focus() {
                            update_keys = true;
                        }
                        ui.horizontal(|ui| {
                            if ui.small_button("Record").clicked() {
                                record = Some((sound.id, sound.name.clone()));
                            }
                            if ui.small_button("Clear").clicked() {
                                sound.shortcut.clear();
                                changed = true;
                                update_keys = true;
                            }
                        });
                        changed |= ui
                            .add(egui::Slider::new(&mut sound.volume, 0.0..=2.0).text("Level"))
                            .changed();
                        ui.horizontal(|ui| {
                            if ui.small_button("Relink file").clicked() {
                                relink = Some(sound.id);
                            }
                            if ui.small_button("Remove").clicked() {
                                remove = Some(sound.id);
                            }
                        });
                    });
                    if (i + 1) % columns == 0 {
                        ui.end_row();
                    }
                }
            });
        if let Some(id) = play {
            self.play(id);
        }
        if let Some(id) = remove {
            self.project.sounds.retain(|s| s.id != id);
            self.clips.remove(&id);
            changed = true;
            update_keys = true;
        }
        if let Some(id) = relink {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Audio", &["wav", "mp3", "flac", "ogg"])
                .pick_file()
            {
                if let Some(sound) = self.project.sounds.iter_mut().find(|s| s.id == id) {
                    sound.path = path.to_string_lossy().into();
                    self.clips.remove(&id);
                    self.loading.remove(&id);
                    self.load_clips();
                    changed = true;
                }
            }
        }
        if update_keys {
            self.refresh_shortcuts();
        }
        if changed {
            self.changed(false);
        }
        ui.add_space(20.0);
        if let Some((id, name)) = record {
            self.begin_shortcut_recording(
                ui.ctx(),
                ShortcutAction::Sound(id),
                format!("Soundboard: {name}"),
            );
        }
        ui.label(RichText::new("Click Record, hold any modifiers, and press a key. Soundboard shortcuts apply immediately; clear a shortcut to disable it.").small().color(theme::palette().muted));
        for error in &self.shortcut_errors {
            ui.colored_label(theme::palette().peach, error);
        }
    }
    fn routing(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("routing-find")
                .selected_text("Find node…")
                .show_ui(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.layout.find_query)
                            .hint_text("Name or type")
                            .desired_width(220.),
                    );
                    let query = self.layout.find_query.to_lowercase();
                    for node in self.project.nodes.iter().filter(|node| {
                        query.is_empty()
                            || node.name.to_lowercase().contains(&query)
                            || node.kind.label().to_lowercase().contains(&query)
                    }) {
                        if ui
                            .selectable_label(
                                self.selected == node.id,
                                format!("{} · {}", truncate(&node.name, 40), node.kind.label()),
                            )
                            .on_hover_text(&node.name)
                            .clicked()
                        {
                            self.selected = node.id;
                            self.layout.routing_focus = Some(node.id);
                            ui.close_menu();
                        }
                    }
                });
            if ui.button("Reset view").clicked() {
                self.layout.routing_offset = Vec2::ZERO;
                self.layout.routing_focus = None;
            }

            ui.label(
                RichText::new(
                    "Drag nodes to arrange. Click an output port, then an input port to connect.",
                )
                .small()
                .color(theme::palette().muted),
            );
            if self.pending_port.is_some() && ui.small_button("Cancel connection").clicked() {
                self.pending_port = None;
            }
            if let Some(i) = self.selected_edge {
                if ui.small_button("Disconnect selected").clicked() {
                    if i < self.project.connections.len() {
                        let c = self.project.connections.remove(i);
                        self.project.clean_switches();
                        self.refresh_shortcuts();
                        crate::logging::info(format_args!(
                            "Disconnected route {} -> {}",
                            c.from, c.to
                        ));
                        self.changed(false);
                    }
                    self.selected_edge = None;
                }
            }
        });
        ui.add_space(12.0);
        let mut moved = false;
        let mut connect = None;
        let mut delete = None;
        let mut selected = None;
        let pending = self.pending_port;
        let pending_output = self.pending_output;
        let view_size = Vec2::new(
            ui.available_width(),
            (ui.available_height() - 38.).max(120.),
        );
        let (origin, extent) = layout::scene_bounds(&self.project.nodes, view_size);
        let shift = self.layout.routing_origin - origin;
        self.layout.routing_offset = (self.layout.routing_offset + shift).max(Vec2::ZERO);
        self.layout.routing_extent = (self.layout.routing_extent + shift).max(extent);
        self.layout.routing_origin = origin;
        let mut pan = Vec2::ZERO;
        let scroll = egui::ScrollArea::both()
            .id_salt("routing-scroll")
            .max_height(view_size.y)
            .auto_shrink([false, false])
            .drag_to_scroll(false)
            .scroll_offset(self.layout.routing_offset)
            .show_viewport(ui, |ui, _viewport| {
                let (rect, background) =
                    ui.allocate_exact_size(self.layout.routing_extent, egui::Sense::click());
                let world_origin = rect.min - self.layout.routing_origin;
                let visible = ui.clip_rect().intersect(rect);
                let painter = ui.painter_at(visible);
                painter.rect_filled(visible, 8., theme::surface(Color32::from_rgb(19, 19, 24)));
                let start = ((visible.min - world_origin) / 22.).floor() * 22.;
                for x in 0..(visible.width() / 22.).ceil() as usize + 2 {
                    for y in 0..(visible.height() / 22.).ceil() as usize + 2 {
                        painter.circle_filled(
                            world_origin + start + Vec2::new(x as f32 * 22., y as f32 * 22.),
                            0.8,
                            theme::palette().line,
                        );
                    }
                }
                for (i, c) in self.project.connections.iter().enumerate() {
                    let Some(from) = self.project.nodes.iter().find(|n| n.id == c.from) else {
                        continue;
                    };
                    let Some(to) = self.project.nodes.iter().find(|n| n.id == c.to) else {
                        continue;
                    };
                    let start = world_origin
                        + Vec2::new(
                            from.position[0] + 180.,
                            from.position[1] + from.kind.output_y(c.port),
                        );
                    let end = world_origin
                        + Vec2::new(to.position[0], to.position[1] + c.input.y(&to.kind));
                    let bend = (end.x - start.x).abs().max(90.0) * 0.5;
                    let shape = egui::epaint::CubicBezierShape::from_points_stroke(
                        [
                            start,
                            start + Vec2::new(bend, 0.0),
                            end - Vec2::new(bend, 0.0),
                            end,
                        ],
                        false,
                        Color32::TRANSPARENT,
                        Stroke::new(
                            if self.selected_edge == Some(i) {
                                3.0_f32
                            } else {
                                2.0_f32
                            },
                            if self.selected_edge == Some(i) {
                                theme::palette().peach
                            } else {
                                theme::accent(Color32::from_rgb(128, 104, 169))
                            },
                        ),
                    );
                    if !Rect::from_points(&[
                        start,
                        start + Vec2::new(bend, 0.),
                        end - Vec2::new(bend, 0.),
                        end,
                    ])
                    .intersects(visible.expand(8.))
                    {
                        continue;
                    }
                    if background.clicked() {
                        if let Some(pointer) = background.interact_pointer_pos() {
                            let points = shape.flatten(Some(1.0));
                            if points
                                .windows(2)
                                .any(|p| segment_distance(pointer, p[0], p[1]) < 8.0)
                            {
                                self.selected_edge = Some(i);
                            }
                        }
                    }
                    painter.add(shape);
                }
                for node in &mut self.project.nodes {
                    let node_rect = Rect::from_min_size(
                        world_origin + Vec2::from(node.position),
                        layout::node_size(&node.kind),
                    );
                    if self.layout.routing_focus == Some(node.id) {
                        ui.scroll_to_rect_animation(
                            node_rect.expand(32.),
                            Some(egui::Align::Center),
                            egui::style::ScrollAnimation::none(),
                        );
                        if visible.contains(node_rect.center()) {
                            self.layout.routing_focus = None;
                        }
                    }
                    if !node_rect.intersects(visible.expand(16.))
                        && !self
                            .layout
                            .routing_drag
                            .is_some_and(|(id, _)| id == node.id)
                    {
                        continue;
                    }
                    let response = ui.interact(
                        node_rect,
                        ui.id().with(("node", node.id)),
                        egui::Sense::click_and_drag(),
                    );
                    if response.drag_started() {
                        if let Some(press) = ui.input(|i| i.pointer.press_origin()) {
                            self.layout.routing_drag = Some((node.id, press - node_rect.min));
                        }
                    }
                    if response.dragged() {
                        if let (Some((owner, grab)), Some(pointer)) =
                            (self.layout.routing_drag, response.interact_pointer_pos())
                        {
                            if owner == node.id {
                                let next = pointer - world_origin - grab;
                                if node.position != [next.x, next.y] {
                                    node.position = [next.x, next.y];
                                    moved = true;
                                }
                                let dt = ui.input(|i| i.stable_dt).min(0.05);
                                for axis in 0..2 {
                                    if pointer[axis] < visible.min[axis] + 32. {
                                        pan[axis] = -600. * dt;
                                    } else if pointer[axis] > visible.max[axis] - 32. {
                                        pan[axis] = 600. * dt;
                                    }
                                }
                            }
                        }
                    }
                    if response.clicked() {
                        selected = Some(node.id);
                    }
                    response.context_menu(|ui| {
                        if ui
                            .button(if matches!(node.kind, NodeKind::Recording) {
                                "Recording / Playback"
                            } else if matches!(node.kind, NodeKind::AutoDucking) {
                                "Ducking controls"
                            } else {
                                "Open effects"
                            })
                            .clicked()
                        {
                            selected = Some(node.id);
                            self.view = if matches!(node.kind, NodeKind::Recording) {
                                View::Recording(node.id)
                            } else if matches!(node.kind, NodeKind::AutoDucking) {
                                View::Ducking(node.id)
                            } else {
                                View::Effects
                            };
                            ui.close_menu();
                        }
                        if ui.button("Remove node").clicked() {
                            delete = Some(node.id);
                            ui.close_menu();
                        }
                    });
                    let color = if matches!(node.kind, NodeKind::Output { .. }) {
                        theme::palette().green
                    } else if node.kind.is_processor() {
                        theme::palette().peach
                    } else {
                        theme::palette().accent
                    };
                    painter.rect(
                        node_rect,
                        8.0,
                        theme::surface(Color32::from_rgb(32, 29, 40)),
                        Stroke::new(
                            1.0_f32,
                            if self.selected == node.id {
                                color
                            } else {
                                theme::surface(Color32::from_rgb(72, 59, 88))
                            },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    painter.text(
                        node_rect.min + Vec2::new(13.0, 14.0),
                        Align2::LEFT_TOP,
                        node.kind.label(),
                        FontId::proportional(9.0),
                        color,
                    );
                    painter.text(
                        node_rect.min + Vec2::new(13.0, 38.0),
                        Align2::LEFT_TOP,
                        truncate(&node.name, 22),
                        FontId::proportional(13.0),
                        theme::foreground(Color32::WHITE),
                    );
                    painter.line_segment(
                        [
                            node_rect.min
                                + Vec2::new(
                                    0.0,
                                    if matches!(
                                        node.kind,
                                        NodeKind::AutoDucking | NodeKind::Extension { .. }
                                    ) {
                                        52.
                                    } else {
                                        65.
                                    },
                                ),
                            node_rect.min
                                + Vec2::new(
                                    180.0,
                                    if matches!(
                                        node.kind,
                                        NodeKind::AutoDucking | NodeKind::Extension { .. }
                                    ) {
                                        52.
                                    } else {
                                        65.
                                    },
                                ),
                        ],
                        Stroke::new(1.0_f32, theme::palette().line),
                    );
                    if matches!(
                        node.kind,
                        NodeKind::AutoDucking | NodeKind::Extension { .. }
                    ) {
                        // Port labels carry the routing information for this node.
                    } else if !matches!(node.kind, NodeKind::Recording) {
                        painter.text(
                            node_rect.min + Vec2::new(13.0, 77.0),
                            Align2::LEFT_CENTER,
                            if node.kind.is_plugin() {
                                format!("VST + {} FX", node.effects.len())
                            } else {
                                format!("{} effects", node.effects.len())
                            },
                            FontId::proportional(10.0),
                            theme::palette().muted,
                        );
                        painter.text(
                            node_rect.min + Vec2::new(167.0, 77.0),
                            Align2::RIGHT_CENTER,
                            if node.kind.is_processor() {
                                if node.effects_disabled {
                                    "PASSTHROUGH"
                                } else {
                                    "FX ENABLED"
                                }
                            } else if node.mute {
                                "theme::palette().muted"
                            } else {
                                "STEREO"
                            },
                            FontId::proportional(9.0),
                            theme::palette().muted,
                        );
                    } else {
                        painter.text(
                            node_rect.min + Vec2::new(13., 68.),
                            Align2::LEFT_CENTER,
                            if node.recording.enabled {
                                "RECORDING"
                            } else {
                                "STOPPED"
                            },
                            FontId::proportional(9.),
                            if node.recording.enabled {
                                theme::palette().green
                            } else {
                                theme::palette().muted
                            },
                        );
                        painter.text(
                            node_rect.min + Vec2::new(13., 92.),
                            Align2::LEFT_CENTER,
                            "Mixed input",
                            FontId::proportional(9.),
                            theme::palette().muted,
                        );
                    }
                    for &input in node.kind.inputs().iter() {
                        let port = node_rect.min + Vec2::new(0., input.y(&node.kind));
                        let hit = ui.interact(
                            Rect::from_center_size(port, Vec2::splat(20.)),
                            ui.id().with(("in", node.id, input)),
                            egui::Sense::click(),
                        );
                        painter.circle(port, 5., theme::palette().panel, Stroke::new(2_f32, color));
                        if matches!(
                            node.kind,
                            NodeKind::AutoDucking | NodeKind::Extension { .. }
                        ) {
                            painter.text(
                                port + Vec2::new(10., 0.),
                                Align2::LEFT_CENTER,
                                truncate(node.kind.input_label(input), 12),
                                FontId::proportional(9.),
                                color,
                            );
                        }
                        if hit
                            .on_hover_text(format!(
                                "Connect {} input",
                                node.kind.input_label(input)
                            ))
                            .clicked()
                        {
                            if let Some(from) = pending {
                                connect = Some((from, node.id, pending_output, input));
                            }
                        }
                    }
                    if matches!(
                        node.kind,
                        NodeKind::Recording | NodeKind::AutoDucking | NodeKind::Extension { .. }
                    ) {
                        for &output in node.kind.outputs().iter() {
                            let port = node_rect.min + Vec2::new(180., node.kind.output_y(output));
                            let hit = ui.interact(
                                Rect::from_center_size(port, Vec2::splat(18.)),
                                ui.id().with(("out", node.id, output)),
                                egui::Sense::click(),
                            );
                            let tint = match output {
                                piratewave_core::project::OutputPort::Passthrough => {
                                    theme::palette().green
                                }
                                piratewave_core::project::OutputPort::Preview
                                | piratewave_core::project::OutputPort::Priority => {
                                    theme::palette().peach
                                }
                                _ => theme::palette().accent,
                            };
                            painter.circle(
                                port,
                                5.,
                                theme::palette().panel,
                                Stroke::new(2_f32, tint),
                            );
                            painter.text(
                                port - Vec2::new(10., 0.),
                                Align2::RIGHT_CENTER,
                                truncate(node.kind.output_label(output), 12),
                                FontId::proportional(9.),
                                tint,
                            );
                            if hit
                                .on_hover_text(format!(
                                    "Connect {} output",
                                    node.kind.output_label(output)
                                ))
                                .clicked()
                            {
                                self.pending_port = Some(node.id);
                                self.pending_output = output;
                            }
                        }
                    } else if !matches!(node.kind, NodeKind::Output { .. }) {
                        let port = node_rect.min + Vec2::new(180.0, 47.0);
                        let hit = ui.interact(
                            Rect::from_center_size(port, Vec2::splat(24.0)),
                            ui.id().with(("out", node.id)),
                            egui::Sense::click(),
                        );
                        painter.circle(
                            port,
                            5.0,
                            if pending == Some(node.id) {
                                color
                            } else {
                                theme::palette().panel
                            },
                            Stroke::new(2.0_f32, color),
                        );
                        if hit.on_hover_text("Start a connection").clicked() {
                            self.pending_port = Some(node.id);
                            self.pending_output = Default::default();
                        }
                    }
                }
            });
        self.layout.routing_viewport = scroll.inner_rect;
        self.layout.routing_offset = (scroll.state.offset + pan).max(Vec2::ZERO);
        if !ui.input(|i| i.pointer.any_down()) {
            self.layout.routing_drag = None;
        }
        if let Some(id) = selected {
            self.selected = id;
        }
        if let Some((from, to, port, input)) = connect {
            match self.project.connect_ports(from, to, port, input) {
                Ok(()) => {
                    self.pending_port = None;
                    self.refresh_shortcuts();
                    self.changed(false);
                }
                Err(error) => self.notify_error(error),
            }
        }
        if let Some(id) = delete {
            self.remove_audio_node(id);
        }
        if moved {
            self.dirty = true;
            self.changed_at = Instant::now();
        }
        ui.add_space(4.0);
        if let Some(node) = self.project.nodes.iter().find(|n| n.id == self.selected) {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("Selected: {}", truncate(&node.name, 40)));
                if ui.button("Edit effects").clicked() {
                    self.view = View::Effects;
                }
            });
        }
    }
    fn dialogs(&mut self, ctx: &egui::Context) {
        self.shortcut_recording_popup(ctx);
        self.effect_help_popup(ctx);
        let mut choose = self.choose_effect;
        let mut effect = None;
        egui::Window::new("Add an effect")
            .vscroll(true)
            .default_height(560.0)
            .default_width(740.0)
            .open(&mut choose)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Choose a processor for the selected node.");
                if let Some(spec) = extensions::effect_menu(ui, &self.registry) {
                    effect = Some(spec);
                }
                for voice_category in [false, true] {
                    egui::CollapsingHeader::new(if voice_category {
                        "Voice Change Effects"
                    } else {
                        "Mixing Effects"
                    })
                    .default_open(voice_category)
                    .show(ui, |ui| {
                        for factory in self.registry.factories().iter().filter(|f| {
                            !f.id.contains('/')
                                && piratewave_core::voice_effects::is_voice_effect(f.id)
                                    == voice_category
                        }) {
                            ui.horizontal(|ui| {
                                if ui
                                    .add_sized([160.0, 30.0], egui::Button::new(factory.name))
                                    .clicked()
                                {
                                    effect = Some(EffectSpec::new(factory));
                                }
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(factory.description)
                                            .small()
                                            .color(theme::palette().muted),
                                    )
                                    .wrap(),
                                );
                            });
                        }
                    });
                }
            });
        self.choose_effect = choose;
        if let Some(spec) = effect {
            if let Some(node) = self.project.nodes.iter_mut().find(|n| {
                n.id == self.selected
                    && !matches!(
                        n.kind,
                        NodeKind::Switch
                            | NodeKind::VoiceChanger
                            | NodeKind::AutoDucking
                            | NodeKind::Output { .. }
                    )
            }) {
                if node.effects.len() < 16 {
                    node.effects.push(spec);
                    self.changed(false);
                    self.choose_effect = false;
                } else {
                    self.notify("A node supports at most 16 effects.");
                }
            }
        }
        let mut open = self.add_node;
        let mut create = false;
        let mut manage_plugins = false;
        egui::Window::new("Add audio node")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut self.new_name);
                egui::ComboBox::from_label("Type")
                    .selected_text(
                        [
                            "Microphone / input",
                            "Desktop loopback",
                            "Soundboard",
                            "Effect processor",
                            "Output bus",
                            "VST Instrument",
                            "VST Effect",
                            "Application Capture",
                            "Voice Changer",
                            "Switch",
                            "Recording",
                            "Auto Ducking",
                        ]
                        .get(self.new_kind)
                        .copied()
                        .unwrap_or_else(|| {
                            piratewave_core::extensions::catalog()
                                .iter()
                                .filter(|e| {
                                    e.descriptor.kind == piratewave_core::extensions::TypeKind::Node
                                })
                                .nth(self.new_kind.saturating_sub(12))
                                .map_or("Extension", |e| e.factory.name)
                        }),
                    )
                    .show_ui(ui, |ui| {
                        for (i, label) in [
                            "Microphone / input",
                            "Desktop loopback",
                            "Soundboard",
                            "Effect processor",
                            "Output bus",
                            "VST Instrument",
                            "VST Effect",
                            "Application Capture",
                            "Voice Changer",
                            "Switch",
                            "Recording",
                            "Auto Ducking",
                        ]
                        .iter()
                        .enumerate()
                        {
                            ui.selectable_value(&mut self.new_kind, i, *label);
                        }
                        for (i, e) in piratewave_core::extensions::catalog()
                            .iter()
                            .filter(|e| {
                                e.descriptor.kind == piratewave_core::extensions::TypeKind::Node
                            })
                            .enumerate()
                        {
                            ui.selectable_value(
                                &mut self.new_kind,
                                12 + i,
                                format!("Extension: {}", e.descriptor.name),
                            );
                        }
                    });
                if matches!(self.new_kind, 5 | 6) {
                    let role = if self.new_kind == 5 {
                        piratewave_core::plugins::PluginRole::Instrument
                    } else {
                        piratewave_core::plugins::PluginRole::Effect
                    };
                    if self.plugin_ui.choice(role).is_none() {
                        self.plugin_ui.selection = self
                            .plugin_ui
                            .library
                            .plugins
                            .iter()
                            .position(|p| p.role == role);
                    }
                    egui::ComboBox::from_id_salt("new-node-plugin")
                        .selected_text(
                            self.plugin_ui
                                .choice(role)
                                .map_or("No plugins imported".into(), |p| p.name),
                        )
                        .show_ui(ui, |ui| {
                            for (index, plugin) in self
                                .plugin_ui
                                .library
                                .plugins
                                .iter()
                                .enumerate()
                                .filter(|(_, p)| p.role == role)
                            {
                                ui.selectable_value(
                                    &mut self.plugin_ui.selection,
                                    Some(index),
                                    &plugin.name,
                                );
                            }
                        });
                    if ui.button("Manage VST plugins").clicked() {
                        manage_plugins = true;
                    }
                }
                ui.label(
                    RichText::new(
                        "Choose its device or application in Mixer and connect ports in Routing.",
                    )
                    .small()
                    .color(theme::palette().muted),
                );
                if ui
                    .add_enabled(
                        !self.new_name.trim().is_empty()
                            && self.project.nodes.len() < 64
                            && (!matches!(self.new_kind, 5 | 6)
                                || self.plugin_ui.selection.is_some()),
                        egui::Button::new("Create node"),
                    )
                    .clicked()
                {
                    create = true;
                }
            });
        self.add_node = open && !manage_plugins;
        if manage_plugins {
            self.view = View::Plugins;
        }
        if create {
            let kind = match self.new_kind {
                0 => NodeKind::Input {
                    device: None,
                    loopback: false,
                },
                1 => NodeKind::Input {
                    device: None,
                    loopback: true,
                },
                2 => NodeKind::Soundboard,
                3 => NodeKind::Processor,
                5 => NodeKind::VstInstrument,
                6 => NodeKind::VstEffect,
                7 => NodeKind::ApplicationCapture { target: None },
                8 => NodeKind::VoiceChanger,
                9 => NodeKind::Switch,
                10 => NodeKind::Recording,
                11 => NodeKind::AutoDucking,
                i if i >= 12 => NodeKind::Extension {
                    definition: Box::new(
                        piratewave_core::extensions::catalog()
                            .iter()
                            .filter(|e| {
                                e.descriptor.kind == piratewave_core::extensions::TypeKind::Node
                            })
                            .nth(i - 12)
                            .expect("catalog frozen")
                            .descriptor
                            .clone(),
                    ),
                },
                _ => NodeKind::Output {
                    device: Some(String::new()),
                },
            };
            let id = self.project.next_id();
            self.project
                .nodes
                .push(Node::new(id, self.new_name.trim(), kind, [300.0, 200.0]));
            self.reveal_utility(id);
            if self.new_kind == 8
                && self.project.nodes.iter().any(|n| {
                    n.id != id && matches!(n.kind, NodeKind::VoiceChanger) && !n.effects_disabled
                })
            {
                self.project.nodes.last_mut().unwrap().effects_disabled = true;
                self.notify("Voice node added in passthrough. Disable the other voice before enabling this one.");
            }
            if matches!(self.new_kind, 5 | 6) {
                let role = if self.new_kind == 5 {
                    piratewave_core::plugins::PluginRole::Instrument
                } else {
                    piratewave_core::plugins::PluginRole::Effect
                };
                if let Some(descriptor) = self.plugin_ui.choice(role) {
                    self.project.nodes.last_mut().unwrap().plugin =
                        Some(piratewave_core::plugins::NodePlugin::new(descriptor));
                }
            }
            self.selected = id;
            self.new_name.clear();
            self.add_node = false;
            if self.new_kind == 11 {
                self.view = View::Ducking(id);
            }
            if self.new_kind == 10 {
                self.view = View::Recording(id);
            }
            self.changed(self.new_kind < 12 && !matches!(self.new_kind, 8 | 10 | 11));
        }
    }
    fn smoke_tick(&mut self, ctx: &egui::Context) {
        if !self.smoke {
            return;
        }
        if std::env::args().any(|a| a == "--effect-tuning-smoke") {
            self.effect_tuning_smoke_tick(ctx);
            return;
        }
        if std::env::args().any(|a| a == "--tools-ui-smoke") {
            self.tools_smoke_tick(ctx);
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            if let egui::Event::Screenshot { image, .. } = event {
                let names = [
                    "mixer",
                    "effects",
                    "soundboard",
                    "routing",
                    "settings-audio",
                    "settings-keybinds",
                    "node-keybinds",
                    "processor-keybinds",
                    "output-keybinds",
                    "vst-plugins",
                    "vst-instrument",
                    "vst-effect",
                    "vst-instrument-keybinds",
                    "vst-effect-keybinds",
                    "vst-mixer",
                    "voice-correction",
                    "expander",
                    "de-esser",
                    "limiter",
                    "noise-suppression",
                    "invert-polarity",
                    "three-band-equalizer",
                    "upward-compressor",
                    "gain",
                    "application-capture",
                    "application-capture-add",
                    "application-capture-keybinds",
                    "application-capture-routing",
                    "effect-help-gain",
                    "effect-help-voice-correction",
                    "settings-general",
                    "shortcut-recorder",
                    "reverb",
                    "effect-help-reverb",
                    "voices",
                    "voice-changer",
                    "voice-changer-keybinds",
                    "voice-changer-help",
                    "voice-changer-mixer",
                    "voice-changer-routing",
                    "training-storage",
                    "voice-creation",
                    "voice-creation-guide",
                    "delay",
                    "vocoder",
                    "resonator",
                    "distortion",
                    "pitch",
                    "flanger",
                    "chorus",
                    "tremolo",
                    "stereo",
                    "wahwah",
                    "voice-change-effects",
                    "switch-controls",
                    "switch-keybinds",
                    "individual-effect-keybinds",
                    "automatic-gain-control",
                    "automatic-gain-control-help",
                    "switch-add",
                    "recording-empty",
                    "recording-waveform",
                    "recording-node-keybinds",
                    "recording-slot-keybinds",
                    "recording-routing",
                    "recording-mixer",
                    "recording-add",
                    "recording-effects",
                    "recording-effects-keybinds",
                    "ducking-level",
                    "ducking-speech",
                    "ducking-routing",
                    "ducking-keybinds",
                    "ducking-add",
                    "mixer-collapsed-1000",
                    "mixer-expanded-1000",
                    "mixer-expanded-1380",
                    "mixer-collapsed-1920",
                    "routing-far-1000",
                    "routing-negative-1380",
                    "ducking-grouped",
                    "vst-grouped",
                    "voice-grouped",
                    "recording-rack-grouped",
                    "mixer-scaled",
                    "extension-settings",
                    "extension-processor",
                    "extension-node",
                    "extension-routing",
                    "extension-node-keybinds",
                    "extension-effect-keybinds",
                    "extension-mixer",
                    "extension-unavailable",
                    "extension-add",
                    "cookbook-1000",
                    "cookbook-1380",
                ];
                let Some(name) = names.get(self.smoke_stage) else {
                    continue;
                };
                let path = format!("dist/native-{name}.png");
                let pixels: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                if let Err(error) = image::save_buffer(
                    &path,
                    &pixels,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                ) {
                    self.smoke_error = Some(error.to_string());
                }
                self.smoke_waiting = false;
                self.smoke_stage += 1;
                self.smoke_at = Instant::now();
            }
        }
        if self.smoke_stage >= 96 {
            let text = if let Some(error) = &self.smoke_error {
                format!("FAIL: {error}")
            } else {
                "PASS: 96 native views rendered, including Cookbook at narrow and wide sizes, including extension install/settings, node/processor controls, keybinds, named ports, unavailable fallback and creation, including responsive mixer panes, large/negative routing scenes and grouped node editors, including Auto Ducking level/speech controls, routing and keybinds, including recording effects racks and keybinds, recording nodes, waveform/crop controls, recording and slot keybinds, three recording ports, switches, voice effects, VST, capture and settings.".into()
            };
            let _ = std::fs::write("dist/ui-smoke-result.txt", text);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !self.smoke_waiting && self.smoke_at.elapsed() > Duration::from_millis(300) {
            let view = [
                View::Mixer,
                View::Effects,
                View::Soundboard,
                View::Routing,
                View::Settings,
                View::Settings,
                View::NodeKeybinds(1),
                View::NodeKeybinds(6),
                View::NodeKeybinds(4),
                View::Plugins,
                View::PluginNode(7),
                View::PluginNode(8),
                View::NodeKeybinds(7),
                View::NodeKeybinds(8),
                View::Mixer,
                View::Effects,
                View::Effects,
                View::Effects,
                View::Effects,
                View::Effects,
                View::Effects,
                View::Effects,
                View::Effects,
                View::Effects,
                View::Mixer,
                View::Mixer,
                View::NodeKeybinds(9),
                View::Routing,
                View::Effects,
                View::Effects,
                View::Settings,
                View::Settings,
                View::Effects,
                View::Effects,
                View::Voices,
                View::VoiceNode(10),
                View::NodeKeybinds(10),
                View::VoiceNode(10),
                View::Mixer,
                View::Routing,
                View::Settings,
                View::Voices,
                View::Voices,
                View::Effects,
                View::Effects,
                View::Effects,
                View::Effects,
                View::Effects,
                View::Effects,
                View::Effects,
                View::Effects,
                View::Effects,
                View::Effects,
                View::Effects,
                View::Effects,
                View::NodeKeybinds(90),
                View::NodeKeybinds(6),
                View::Effects,
                View::Effects,
                View::Routing,
                View::Recording(91),
                View::Recording(91),
                View::NodeKeybinds(91),
                View::RecordingSlotKeybinds(91, 9101),
                View::Routing,
                View::Mixer,
                View::Routing,
                View::Effects,
                View::NodeKeybinds(91),
                View::Ducking(95),
                View::Ducking(95),
                View::Routing,
                View::NodeKeybinds(95),
                View::Routing,
                View::Mixer,
                View::Mixer,
                View::Mixer,
                View::Mixer,
                View::Routing,
                View::Routing,
                View::Ducking(95),
                View::PluginNode(7),
                View::VoiceNode(10),
                View::Effects,
                View::Mixer,
                View::Settings,
                View::Effects,
                View::Effects,
                View::Routing,
                View::NodeKeybinds(97),
                View::NodeKeybinds(96),
                View::Mixer,
                View::Effects,
                View::Routing,
                View::Cookbook,
                View::Cookbook,
            ][self.smoke_stage];
            if self.smoke_stage == 40 {
                self.settings_section = keybinds::SettingsSection::Training;
            }
            if self.smoke_stage >= 41 {
                self.voice_ui.training.smoke_page(self.smoke_stage == 42);
            }
            if self.smoke_stage >= 34 {
                if !self.project.nodes.iter().any(|n| n.id == 10) {
                    self.project.nodes.push(Node::new(
                        10,
                        "LLVC voice",
                        NodeKind::VoiceChanger,
                        [315.0, 420.0],
                    ));
                    self.project
                        .connections
                        .retain(|c| !(c.from == 1 && c.to == 5));
                    self.project.connections.extend([
                        piratewave_core::project::Connection {
                            input: Default::default(),
                            port: Default::default(),
                            from: 1,
                            to: 10,
                        },
                        piratewave_core::project::Connection {
                            input: Default::default(),
                            port: Default::default(),
                            from: 10,
                            to: 5,
                        },
                    ]);
                }
                self.voice_ui.help = self.smoke_stage == 37;
                if self.smoke_stage >= 38 {
                    self.project.nodes.sort_by_key(|n| n.id != 10);
                }
            }
            if self.smoke_stage >= 32 {
                self.recorder = None; // No real hotkeys exist in the screenshot fixture.
                self.selected = 6;
                let node = self.project.nodes.iter_mut().find(|n| n.id == 6).unwrap();
                if !node.effects.first().is_some_and(|fx| fx.kind == "reverb") {
                    node.effects = vec![EffectSpec::new(self.registry.get("reverb").unwrap())];
                }
                self.effect_help = (self.smoke_stage == 33).then(|| "reverb".into());
            }
            if (28..30).contains(&self.smoke_stage) {
                let id = if self.smoke_stage == 28 {
                    "gain"
                } else {
                    "automatic_voice_correction"
                };
                self.selected = 1;
                self.project
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == 1)
                    .unwrap()
                    .effects = vec![EffectSpec::new(self.registry.get(id).unwrap())];
                self.effect_help = Some(id.into());
            }
            if self.smoke_stage == 30 {
                self.effect_help = None;
                self.settings_section = keybinds::SettingsSection::General;
            }
            if self.smoke_stage == 31 {
                self.settings_section = keybinds::SettingsSection::Keybinds;
                self.begin_shortcut_recording(
                    ctx,
                    ShortcutAction::App(AppAction::StopSounds),
                    "Stop all soundboard sounds".into(),
                );
            }
            if self.smoke_stage == 24 && !self.project.nodes.iter().any(|n| n.id == 9) {
                let app = &self.applications[0];
                self.project.nodes.push(Node::new(
                    9,
                    "Player audio",
                    NodeKind::ApplicationCapture {
                        target: Some(piratewave_core::project::ApplicationTarget {
                            executable: app.executable.clone(),
                            window_title: app.title.clone(),
                            match_title: false,
                        }),
                    },
                    [750., 45.],
                ));
                self.project
                    .connections
                    .push(piratewave_core::project::Connection {
                        input: Default::default(),
                        port: Default::default(),
                        from: 9,
                        to: 6,
                    });
                self.project
                    .connections
                    .push(piratewave_core::project::Connection {
                        input: Default::default(),
                        port: Default::default(),
                        from: 6,
                        to: 4,
                    });
                self.project.nodes.sort_by_key(|n| n.id != 9);
            }
            if self.smoke_stage == 25 {
                self.add_node = true;
                self.new_kind = 7;
                self.new_name = "Application audio".into();
            }
            if self.smoke_stage >= 26 {
                self.add_node = false;
            }
            if let View::NodeKeybinds(id) = view {
                if self.view != view {
                    self.open_node_keybinds(id);
                }
            } else {
                self.view = view;
            }
            if (16..24).contains(&self.smoke_stage) {
                let id = [
                    "expander",
                    "de_esser",
                    "limiter",
                    "noise_suppression",
                    "invert_polarity",
                    "three_band_equalizer",
                    "upward_compressor",
                    "gain",
                ][self.smoke_stage - 16];
                self.selected = 6;
                let node = self.project.nodes.iter_mut().find(|n| n.id == 6).unwrap();
                if !node.effects.first().is_some_and(|effect| effect.kind == id) {
                    node.effects = vec![EffectSpec::new(self.registry.get(id).unwrap())];
                }
            }
            if self.smoke_stage >= 43 {
                self.selected = 6;
                self.view = View::Effects;
                self.voice_ui.help = false;
                self.effect_help = None;
                self.choose_effect = self.smoke_stage == 53;
                if self.smoke_stage < 53 {
                    let id = [
                        "delay",
                        "vocoder",
                        "resonator",
                        "distortion",
                        "pitch",
                        "flanger",
                        "chorus",
                        "tremolo",
                        "stereo",
                        "wahwah",
                    ][self.smoke_stage - 43];
                    let node = self.project.nodes.iter_mut().find(|n| n.id == 6).unwrap();
                    node.effects = vec![EffectSpec::new(self.registry.get(id).unwrap())];
                }
            }
            if self.smoke_stage >= 54 {
                if !self.project.nodes.iter().any(|n| n.id == 90) {
                    let mut node =
                        Node::new(90, "Voice effect selector", NodeKind::Switch, [300., 320.]);
                    node.switch_output = Some(6);
                    self.project.nodes.push(node);
                    self.project.connections.extend([
                        piratewave_core::project::Connection {
                            input: Default::default(),
                            port: Default::default(),
                            from: 90,
                            to: 6,
                        },
                        piratewave_core::project::Connection {
                            input: Default::default(),
                            port: Default::default(),
                            from: 90,
                            to: 8,
                        },
                    ]);
                }
                self.choose_effect = false;
                self.effect_help = None;
                self.selected = 90;
                self.view = view;
                if self.smoke_stage == 55 {
                    self.open_node_keybinds(90);
                }
                if self.smoke_stage == 56 {
                    let node = self.project.nodes.iter_mut().find(|n| n.id == 6).unwrap();
                    node.effects = vec![
                        EffectSpec::new(self.registry.get("automatic_voice_correction").unwrap()),
                        EffectSpec::new(self.registry.get("distortion").unwrap()),
                    ];
                    self.open_node_keybinds(6);
                }
                if (57..59).contains(&self.smoke_stage) {
                    self.selected = 6;
                    self.project
                        .nodes
                        .iter_mut()
                        .find(|n| n.id == 6)
                        .unwrap()
                        .effects = vec![EffectSpec::new(
                        self.registry.get("automatic_gain_control").unwrap(),
                    )];
                    self.effect_help =
                        (self.smoke_stage == 58).then(|| "automatic_gain_control".into());
                }
                if self.smoke_stage == 59 {
                    self.add_node = true;
                    self.new_kind = 9;
                    self.new_name = "Voice effect selector".into();
                }
            }
            if self.smoke_stage >= 60 {
                self.recording_smoke_fixture(self.smoke_stage >= 61);
                self.shortcut_errors.clear();
                self.selected = 91;
                self.view = view;
                self.add_node = false;
                self.choose_effect = false;
                self.effect_help = None;
                if self.smoke_stage >= 67 {
                    let recorder = self.project.nodes.iter_mut().find(|n| n.id == 91).unwrap();
                    recorder.effects = ["gain", "delay"]
                        .iter()
                        .map(|kind| EffectSpec::new(self.registry.get(kind).unwrap()))
                        .collect();
                    if self.smoke_stage == 68 {
                        self.open_node_keybinds(91);
                    }
                }
                if self.smoke_stage == 62 {
                    self.open_node_keybinds(91);
                }
                if self.smoke_stage == 63 {
                    self.open_recording_slot_keybinds(91, 9101);
                }
                if self.smoke_stage == 65 {
                    self.project.nodes.sort_by_key(|n| n.id != 91);
                }
                if self.smoke_stage == 66 {
                    self.add_node = true;
                    self.new_kind = 10;
                    self.new_name = "Instant replay".into();
                }
            }
            if self.smoke_stage >= 69 {
                if !self.project.nodes.iter().any(|n| n.id == 95) {
                    self.project.nodes.push(Node::new(
                        95,
                        "Priority mixer",
                        NodeKind::AutoDucking,
                        [300., 320.],
                    ));
                    use piratewave_core::project::{InputPort, OutputPort};
                    self.project
                        .connect_ports(1, 95, OutputPort::Passthrough, InputPort::Priority)
                        .unwrap();
                    self.project
                        .connect_ports(3, 95, OutputPort::Passthrough, InputPort::Ducked)
                        .unwrap();
                    self.project
                        .connect_port(95, 4, OutputPort::Passthrough)
                        .unwrap();
                    self.project
                        .connect_port(95, 6, OutputPort::Priority)
                        .unwrap();
                    self.project
                        .connect_port(95, 5, OutputPort::Ducked)
                        .unwrap();
                }
                self.selected = 95;
                self.view = view;
                let node = self.project.nodes.iter_mut().find(|n| n.id == 95).unwrap();
                node.ducking.detection = if self.smoke_stage == 70 {
                    piratewave_core::ducking::Detection::Speech
                } else {
                    piratewave_core::ducking::Detection::Level
                };
                if self.smoke_stage == 72 {
                    self.open_node_keybinds(95);
                }
                if self.smoke_stage == 73 {
                    self.add_node = true;
                    self.new_kind = 11;
                    self.new_name = "Priority mixer".into();
                }
            }
            if self.view == View::Mixer && self.smoke_stage >= 38 {
                self.app_settings.ui.utilities_expanded = true;
            }
            if self.smoke_stage >= 74 {
                let size = match self.smoke_stage {
                    74 | 75 | 78 => Vec2::new(1000., 700.),
                    76 | 79 | 83 => Vec2::new(1380., 960.),
                    77 => Vec2::new(1920., 1080.),
                    _ => Vec2::new(1600., 960.),
                };
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                ctx.set_zoom_factor(if self.smoke_stage == 84 { 1.25 } else { 1. });
                self.view = view;
                self.app_settings.ui.utilities_expanded = !matches!(self.smoke_stage, 74 | 77);
                self.app_settings.ui.utilities_filter = Default::default();
                if self.smoke_stage == 78 {
                    self.project
                        .nodes
                        .iter_mut()
                        .find(|n| n.id == 95)
                        .unwrap()
                        .position = [8000., 6000.];
                    self.layout.routing_focus = Some(95);
                }
                if self.smoke_stage == 79 {
                    self.project
                        .nodes
                        .iter_mut()
                        .find(|n| n.id == 95)
                        .unwrap()
                        .position = [-900., -700.];
                    self.layout.routing_focus = Some(95);
                }
                if self.smoke_stage == 83 {
                    self.selected = 91;
                }
                if self.smoke_stage == 81 {
                    if let Some(p) = self
                        .project
                        .nodes
                        .iter_mut()
                        .find(|n| n.id == 7)
                        .and_then(|n| n.plugin.as_mut())
                    {
                        p.descriptor.name =
                            "Long instrument title with many exposed parameters".into();
                        if p.descriptor.parameters.len() < 24 {
                            p.descriptor.parameters = (0..24)
                                .map(|id| piratewave_core::plugins::PluginParameter {
                                    id,
                                    name: format!("Instrument parameter {id}"),
                                    unit: "%".into(),
                                    default: 0.5,
                                    steps: 0,
                                })
                                .collect();
                        }
                    }
                }
            }
            if self.smoke_stage >= 85 {
                let manifest = piratewave_extension_sdk::parse_manifest(include_bytes!(
                    "../crates/piratewave-example-extension/manifest.json"
                ))
                .unwrap();
                if !self.project.nodes.iter().any(|n| n.id == 97) {
                    let mut rack = Node::new(
                        96,
                        "SDK Stereo Processor",
                        NodeKind::Processor,
                        [450., 850.],
                    );
                    rack.effects
                        .push(piratewave_core::extensions::effect_spec(&manifest.types[0]));
                    self.project.nodes.push(rack);
                    self.project.nodes.push(Node::new(
                        97,
                        "SDK Multi Mix",
                        NodeKind::Extension {
                            definition: Box::new(manifest.types[1].clone()),
                        },
                        [450., 1100.],
                    ));
                    self.project
                        .connect_ports(
                            1,
                            97,
                            piratewave_core::project::OutputPort::Passthrough,
                            piratewave_core::project::InputPort::Extension(101),
                        )
                        .unwrap();
                    self.project
                        .connect_ports(
                            2,
                            97,
                            piratewave_core::project::OutputPort::Passthrough,
                            piratewave_core::project::InputPort::Extension(202),
                        )
                        .unwrap();
                    self.project
                        .connect_ports(
                            97,
                            4,
                            piratewave_core::project::OutputPort::Extension(303),
                            piratewave_core::project::InputPort::Main,
                        )
                        .unwrap();
                    let mut missing = Node::new(
                        98,
                        "Unavailable extension",
                        NodeKind::Extension {
                            definition: Box::new(manifest.types[1].clone()),
                        },
                        [700., 1100.],
                    );
                    if let NodeKind::Extension { definition } = &mut missing.kind {
                        definition.id = "org.piratewave.missing/node".into();
                    }
                    self.project.nodes.push(missing);
                }
                self.settings_section = keybinds::SettingsSection::General;
                if self.smoke_stage == 86 {
                    self.selected = 96;
                }
                if self.smoke_stage == 87 {
                    self.selected = 97;
                }
                if self.smoke_stage == 88 {
                    self.layout.routing_focus = Some(97);
                }
                if self.smoke_stage == 89 {
                    self.open_node_keybinds(97);
                }
                if self.smoke_stage == 90 {
                    self.open_node_keybinds(96);
                }
                if self.smoke_stage == 91 {
                    self.layout.mixer_focus = Some(97);
                }
                if self.smoke_stage == 92 {
                    self.selected = 98;
                }
                if self.smoke_stage == 93 {
                    self.add_node = true;
                    self.new_name = "New SDK node".into();
                    self.new_kind = if piratewave_core::extensions::catalog().is_empty() {
                        3
                    } else {
                        12
                    };
                }
            }
            if self.smoke_stage >= 94 {
                self.add_node = false;
                self.settings_dirty = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
                    if self.smoke_stage == 94 {
                        Vec2::new(1000., 700.)
                    } else {
                        Vec2::new(1380., 960.)
                    },
                ));
            }
            if self.smoke_stage == 15 {
                self.selected = 6;
                let processor = self.project.nodes.iter_mut().find(|n| n.id == 6).unwrap();
                if !processor
                    .effects
                    .iter()
                    .any(|effect| effect.kind == piratewave_core::voice_correction::EFFECT_ID)
                {
                    processor.effects = vec![EffectSpec::new(
                        self.registry
                            .get(piratewave_core::voice_correction::EFFECT_ID)
                            .unwrap(),
                    )];
                }
            }
            if self.smoke_stage == 14 {
                self.project.nodes.sort_by_key(|n| {
                    if n.kind.is_plugin() {
                        n.id - 7
                    } else {
                        n.id + 10
                    }
                });
            }
            if self.smoke_stage == 0 {
                self.project.nodes[3].listen_enabled = true;
            }
            if self.smoke_stage == 4 {
                self.settings_section = keybinds::SettingsSection::Audio;
            } else if self.smoke_stage == 5 {
                self.settings_section = keybinds::SettingsSection::Keybinds;
            }
            if self.view == View::Effects {
                if let Some(node) = self.project.nodes.iter_mut().find(|n| {
                    n.id == self.selected
                        && !matches!(
                            n.kind,
                            NodeKind::Switch | NodeKind::VoiceChanger | NodeKind::AutoDucking
                        )
                        && n.effects.is_empty()
                }) {
                    for id in ["high_pass", "gate", "compressor"] {
                        node.effects
                            .push(EffectSpec::new(self.registry.get(id).unwrap()));
                    }
                }
            }
            if self.view == View::Soundboard && self.project.sounds.is_empty() {
                self.project.sounds.push(Sound {
                    id: 99,
                    name: "Welcome chime".into(),
                    path: String::new(),
                    shortcut: "Ctrl+Alt+F1".into(),
                    volume: 1.0,
                });
                self.clips.insert(99, assets::demo());
            }
            ctx.request_repaint();
            if self.smoke_at.elapsed() > Duration::from_millis(600) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                self.smoke_waiting = true;
            }
        }
    }
}
impl eframe::App for PirateWaveApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        theme::apply(ctx, self.app_settings.ui.theme);
        // Keep the background screenshot fixture open without requiring focus
        // from the user's active app. Smoke mode never polls native keys.
        if self.smoke && self.smoke_stage == 31 {
            ctx.input_mut(|i| i.focused = true);
        }
        self.effect_tuning.begin_frame(&self.project);
        self.record_shortcut_input(ctx);
        self.poll();
        self.sidebar(ctx);
        self.tools_ui.poll(match self.view {
            View::Tool(tool) => Some(tool),
            _ => None,
        });
        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(theme::palette().bg)
                    .inner_margin(12.0),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(if self.status.message.is_empty() {
                            "Audio stopped"
                        } else {
                            &self.status.message
                        })
                        .small()
                        .color(
                            if self.status.message.contains("failed")
                                || self.status.message.contains("unavailable")
                            {
                                theme::palette().peach
                            } else {
                                theme::palette().muted
                            },
                        ),
                    );
                });
                if self.notice_at.elapsed() < Duration::from_secs(15) || !self.status.running {
                    ui.label(
                        RichText::new(&self.notice)
                            .small()
                            .color(theme::palette().accent),
                    );
                }
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::palette().bg)
                    .inner_margin(28.0),
            )
            .show(ctx, |ui| {
                self.header(ui);
                if self.view == View::Mixer {
                    self.mixer(ui);
                    return;
                }
                if self.view == View::Routing {
                    self.routing(ui);
                    return;
                }
                egui::ScrollArea::vertical()
                    .id_salt(("page", self.view))
                    .show(ui, |ui| match self.view {
                        View::Cookbook => self.cookbook_page(ui),
                        View::Tools => self.tools_page(ui),
                        View::Tool(tool) => self.tool_page(ui, tool),
                        View::Plugins => self.plugin_library_page(ui),
                        View::PluginNode(id) => self.plugin_node_page(ui, id),
                        View::Voices => self.voices_page(ui),
                        View::VoiceNode(id) => self.voice_node_page(ui, id),
                        View::Mixer => self.mixer(ui),
                        View::Effects => self.effects(ui),
                        View::Soundboard => self.soundboard(ui),
                        View::Routing => self.routing(ui),
                        View::Settings => self.settings_page(ui),
                        View::NodeKeybinds(id) => self.node_keybinds_page(ui, id),
                        View::Ducking(id) => self.ducking_page(ui, id),
                        View::Recording(id) => self.recording_page(ui, id),
                        View::RecordingSlotKeybinds(id, slot) => {
                            self.recording_slot_keybinds_page(ui, id, slot)
                        }
                    });
            });
        self.effect_tuning.finish_frame(
            &self.engine,
            self.status.running && !ctx.input(|i| i.viewport().minimized.unwrap_or(false)),
        );
        self.poll_plugins(ctx);
        self.poll_voice_jobs();
        self.voice_help(ctx);
        self.dialogs(ctx);
        self.smoke_tick(ctx);
        ctx.request_repaint_after(Duration::from_millis(33));
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.tools_ui.stop();
        self.voice_ui.training.shutdown();
        if !self.smoke && self.settings_dirty {
            if let Err(error) =
                storage::save_settings(&storage::settings_path(), &self.app_settings)
            {
                eprintln!("Could not save settings: {error:#}");
            }
        }
        if !self.smoke && self.dirty {
            if let Err(error) = storage::save(&self.path, &self.project) {
                eprintln!("Could not save workspace: {error:#}");
            }
        }
    }
}
fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .small()
            .color(theme::palette().muted)
            .extra_letter_spacing(1.5),
    );
}
fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(theme::palette().panel)
        .stroke(Stroke::new(1.0_f32, theme::palette().line))
        .corner_radius(8.0)
        .inner_margin(15.0)
}
fn show_card<R>(
    ui: &mut egui::Ui,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    card().show(ui, |ui| {
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), content)
            .inner
    })
}
fn device_combo(
    ui: &mut egui::Ui,
    id: u64,
    selected: &mut Option<String>,
    devices: &[audio_io::DeviceInfo],
    unassigned: bool,
) {
    let label = match selected {
        None => "System default".into(),
        Some(id) if id.is_empty() => "Not assigned".into(),
        Some(id) => devices
            .iter()
            .find(|d| &d.id == id)
            .map(|d| truncate(&d.name, 27))
            .unwrap_or_else(|| {
                format!(
                    "{} (unavailable)",
                    truncate(
                        id.rsplit_once("::").map_or(id.as_str(), |(name, _)| name),
                        27
                    )
                )
            }),
    };
    egui::ComboBox::from_id_salt(("device", id))
        .width(ui.available_width().min(320.0))
        .selected_text(RichText::new(label).small().color(theme::palette().muted))
        .show_ui(ui, |ui| {
            if unassigned {
                ui.selectable_value(selected, Some(String::new()), "Not assigned");
            }
            ui.selectable_value(selected, None, "System default");
            for device in devices {
                ui.selectable_value(selected, Some(device.id.clone()), &device.name);
            }
        });
}
fn meter(ui: &mut egui::Ui, peak: f32, size: Vec2, vertical: bool) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, theme::surface(Color32::from_rgb(11, 12, 15)));
    let db = 20.0 * peak.max(1e-6).log10();
    let value = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
    let color = if peak >= 1.0 {
        theme::palette().peach
    } else {
        theme::palette().green
    };
    let fill = if vertical {
        Rect::from_min_max(
            Pos2::new(rect.left(), rect.bottom() - rect.height() * value),
            rect.max,
        )
    } else {
        Rect::from_min_max(
            rect.min,
            Pos2::new(rect.left() + rect.width() * value, rect.bottom()),
        )
    };
    painter.rect_filled(fill, 1.0, color);
    if vertical {
        for y in (0..rect.height() as usize).step_by(5) {
            let at = rect.top() + y as f32;
            painter.line_segment(
                [Pos2::new(rect.left(), at), Pos2::new(rect.right(), at)],
                Stroke::new(2.0_f32, theme::palette().panel),
            );
        }
    }
}
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        format!("{}…", text.chars().take(max - 1).collect::<String>())
    } else {
        text.into()
    }
}
fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let v = b - a;
    let t = ((p - a).dot(v) / v.length_sq().max(0.0001)).clamp(0.0, 1.0);
    p.distance(a + v * t)
}
