use super::*;
use crate::obs::{Action, Event, Service, State};
use piratewave_core::workflows::ObsSceneLink;
#[derive(Default)]
pub(super) struct ObsUi {
    pub service: Option<Service>,
    fixture: Option<std::sync::Arc<State>>,
    password: String,
    armed: Option<(u64, u64)>,
    barrier: Option<(u64, u64, u64)>,
    links: Vec<ObsSceneLink>,
    generation: u64,
    last_scene: String,
    selected_scene: String,
    input: String,
    volume: f64,
    volume_dragging: bool,
    selected_epoch: u64,
    message: String,
}
impl ObsUi {
    pub fn disarm(&mut self) {
        self.armed = None;
        self.barrier = None;
        self.last_scene.clear();
    }
}
impl PirateWaveApp {
    pub(super) fn connect_obs(&mut self) {
        self.obs_ui.disarm();
        self.obs_ui.service = None;
        self.obs_ui.selected_epoch = 0;
        self.obs_ui.volume_dragging = false;
        match Service::start(
            self.app_settings.obs_port,
            self.obs_ui.password.clone(),
            self.controller.generation,
        ) {
            Ok(service) => {
                self.obs_ui.service = Some(service);
                self.obs_ui.message.clear();
            }
            Err(e) => self.notify_error(format!("Cannot connect to OBS: {e}")),
        }
    }
    fn obs_command(&mut self, action: Action) -> Option<u64> {
        let result = self
            .obs_ui
            .service
            .as_ref()
            .ok_or_else(|| "Connect to OBS first".to_owned())
            .and_then(|s| s.send(action, self.controller.generation));
        match result {
            Ok(token) => Some(token),
            Err(e) => {
                self.obs_ui.message = e.clone();
                self.notify_error(e);
                None
            }
        }
    }
    pub(super) fn poll_obs(&mut self) {
        let generation = self.controller.generation;
        if self.obs_ui.generation != generation
            || self.obs_ui.links != self.project.workflows.obs_scene_links
        {
            self.obs_ui.disarm();
            self.obs_ui.generation = generation;
            self.obs_ui.links = self.project.workflows.obs_scene_links.clone();
        }
        let Some(service) = &self.obs_ui.service else {
            return;
        };
        service
            .workspace
            .store(generation, std::sync::atomic::Ordering::Release);
        let state = service.state.load_full();
        let online = service.is_connected() && state.connected;
        let events: Vec<_> = service.events.try_iter().take(64).collect();
        if !online
            || self
                .obs_ui
                .armed
                .is_some_and(|(epoch, _)| epoch != state.epoch)
            || self
                .obs_ui
                .barrier
                .is_some_and(|(_, epoch, _)| epoch != state.epoch)
        {
            self.obs_ui.disarm();
        }
        if online && self.obs_ui.selected_epoch != state.epoch {
            self.obs_ui.selected_epoch = state.epoch;
            if !self.obs_ui.input.is_empty() {
                self.obs_command(Action::SelectInput(self.obs_ui.input.clone()));
            }
        }
        for event in events {
            self.handle_obs_event(event, online, state.epoch);
        }
    }
    fn handle_obs_event(&mut self, event: Event, online: bool, current_epoch: u64) {
        match event {
            Event::Barrier {
                epoch,
                generation,
                token,
                name,
            } if online
                && epoch == current_epoch
                && generation == self.controller.generation
                && self.obs_ui.barrier == Some((token, epoch, generation)) =>
            {
                self.obs_ui.barrier = None;
                self.obs_ui.armed = Some((epoch, generation));
                self.obs_ui.last_scene = name;
                self.obs_ui.message = "Scene linking armed for future OBS scene changes".into();
            }
            Event::ProgramScene {
                epoch,
                generation,
                name,
            } if online
                && epoch == current_epoch
                && self.obs_ui.armed == Some((epoch, generation))
                && generation == self.controller.generation =>
            {
                if name != self.obs_ui.last_scene {
                    self.obs_ui.last_scene = name.clone();
                    self.apply_obs_scene(&name);
                }
            }
            Event::Completed {
                epoch,
                generation,
                result,
            } if online && epoch == current_epoch && generation == self.controller.generation => {
                match result {
                    Ok(()) => self.obs_ui.message = "OBS request completed".into(),
                    Err(e) => {
                        self.obs_ui.barrier = None;
                        self.obs_ui.message = e.clone();
                        self.notify_error(e);
                    }
                }
            }
            _ => {}
        }
    }
    fn apply_obs_scene(&mut self, name: &str) {
        let matches: Vec<_> = self
            .project
            .workflows
            .obs_scene_links
            .iter()
            .filter(|l| l.obs_scene == name)
            .map(|l| l.audio_scene)
            .collect();
        if matches.is_empty() {
            return;
        }
        if matches.len() != 1 {
            self.obs_ui.disarm();
            self.notify_error("Ambiguous OBS scene link; fix the duplicate mapping");
            return;
        }
        let scene = matches[0];
        if !self.project.workflows.scenes.iter().any(|s| s.id == scene) {
            self.obs_ui.disarm();
            self.notify_error(
                "The linked PirateWave audio scene is missing. Update the OBS scene link.",
            );
            return;
        }
        self.dispatch_action(
            ShortcutAction::Scene(scene),
            piratewave_core::automation::Origin::Obs,
        );
    }
    pub(super) fn obs_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("OBS Studio");
        ui.label("Control OBS on this PC and recall PirateWave audio scenes when OBS changes its program scene.");
        let state = if self.smoke {
            self.obs_ui.fixture.clone()
        } else {
            None
        }
        .or_else(|| self.obs_ui.service.as_ref().map(|s| s.state.load_full()))
        .unwrap_or_else(|| {
            std::sync::Arc::new(State {
                message: "Disconnected".into(),
                ..Default::default()
            })
        });
        card().show(ui,|ui|{
            ui.strong("Connection");ui.label("In OBS: Tools > WebSocket Server Settings > Enable WebSocket server. Use port 4455 unless you changed it.");
            ui.horizontal_wrapped(|ui|{
                ui.label("Port");if ui.add(egui::DragValue::new(&mut self.app_settings.obs_port).range(1..=65535)).changed(){self.settings_dirty=true;self.settings_draft.obs_port=self.app_settings.obs_port;}
                ui.label("Password");ui.add(egui::TextEdit::singleline(&mut self.obs_ui.password).password(true).desired_width(230.).hint_text("OBS WebSocket password"));
                if ui.button(if self.obs_ui.service.is_some(){"Reconnect"}else{"Connect"}).clicked(){self.connect_obs();}
                if self.obs_ui.service.is_some()&&ui.button("Disconnect").clicked(){self.obs_ui.disarm();self.obs_ui.service=None;self.obs_ui.password.clear();self.obs_ui.message.clear();}
            });
            ui.colored_label(if state.connected{theme::palette().green}else{theme::palette().peach},&state.message);
            if !state.version.is_empty(){ui.small(format!("OBS {} · WebSocket v5",state.version));}
            ui.small("Password stays in memory for this connection and its retries; it is never saved in settings or workspace exports. Connecting does not start audio or streaming.");
        });
        ui.add_space(12.);
        card().show(ui,|ui|{
            ui.strong("OBS scenes & recording");
            ui.label(format!("Program scene: {}",if state.current_scene.is_empty(){"Unknown"}else{&state.current_scene}));
            ui.horizontal_wrapped(|ui|{
                ui.label(format!("Stream: {}",status(state.streaming)));ui.label(format!("OBS recording: {}{}",status(state.recording),if state.paused{" · paused"}else{""}));
                if ui.add_enabled(state.connected,egui::Button::new("Refresh OBS lists")).clicked(){self.obs_command(Action::Refresh);}
            });
            ui.add_enabled_ui(state.connected,|ui|{
                ui.horizontal_wrapped(|ui|{
                    egui::ComboBox::from_id_salt("obs-program-scene").selected_text(if self.obs_ui.selected_scene.is_empty(){"Choose OBS scene"}else{&self.obs_ui.selected_scene}).show_ui(ui,|ui|{for name in &state.scenes{ui.selectable_value(&mut self.obs_ui.selected_scene,name.clone(),name);}});
                    if ui.add_enabled(!self.obs_ui.selected_scene.is_empty(),egui::Button::new("Switch OBS scene")).clicked(){self.obs_command(Action::ProgramScene(self.obs_ui.selected_scene.clone()));}
                });
                ui.horizontal_wrapped(|ui|{
                    if ui.add_enabled(state.recording==Some(false),egui::Button::new("Start OBS recording")).clicked(){self.obs_command(Action::StartRecord);}
                    if ui.add_enabled(state.recording==Some(true),egui::Button::new("Stop OBS recording")).clicked(){self.obs_command(Action::StopRecord);}
                    if ui.add_enabled(state.recording==Some(true),egui::Button::new(if state.paused{"Resume OBS recording"}else{"Pause OBS recording"})).clicked(){self.obs_command(Action::PauseRecord(!state.paused));}
                });
            });
            ui.small("These buttons control OBS's video recording. PirateWave's Recording page manages separate multitrack audio takes. Streaming status is read-only.");
        });
        ui.add_space(12.);
        card().show(ui,|ui|{
            ui.strong("OBS audio source");let previous=self.obs_ui.input.clone();
            egui::ComboBox::from_id_salt("obs-audio-source").selected_text(if self.obs_ui.input.is_empty(){"Choose OBS source"}else{&self.obs_ui.input}).show_ui(ui,|ui|{for input in &state.inputs{ui.selectable_value(&mut self.obs_ui.input,input.clone(),input);}});
            if previous!=self.obs_ui.input{self.obs_ui.volume_dragging=false;self.obs_command(Action::SelectInput(self.obs_ui.input.clone()));}
            if state.selected_input==self.obs_ui.input&&!self.obs_ui.input.is_empty(){
                if let Some(muted)=state.input_muted{if ui.add_enabled(state.connected,egui::Button::new(if muted{"Unmute in OBS"}else{"Mute in OBS"})).clicked(){self.obs_command(Action::InputMute(self.obs_ui.input.clone(),!muted));}}
                if let Some(db)=state.input_db{
                    if !self.obs_ui.volume_dragging && !ui.input(|i|i.pointer.any_down()){self.obs_ui.volume=db.clamp(-100.,0.);}
                    let response=ui.add_enabled(state.connected,egui::Slider::new(&mut self.obs_ui.volume,-100.0..=0.).text("OBS volume (dB)"));
                    if response.drag_stopped()||response.changed()&&!ui.input(|i|i.pointer.any_down()){self.obs_command(Action::InputVolume(self.obs_ui.input.clone(),self.obs_ui.volume));}
                    self.obs_ui.volume_dragging=response.dragged();
                    ui.small(format!("Current OBS level: {db:.1} dB"));
                }else{ui.small("Waiting for audio controls. Non-audio sources do not expose mute/volume.");}
            }
            ui.small("This changes OBS's source fader, independently of PirateWave's mixer. Levels are not polled on the audio thread.");
        });
        ui.add_space(12.);
        self.obs_links_editor(ui, &state);
        if !self.obs_ui.message.is_empty() {
            ui.colored_label(theme::palette().accent, &self.obs_ui.message);
        }
        ui.collapsing("Send PirateWave audio into OBS",|ui|{
            ui.set_max_width(740.);
            ui.label("WebSocket carries controls, not audio. Select a free VB-CABLE playback endpoint (CABLE Input) on PirateWave's Stream Mix output bus. In OBS, add Audio Input Capture and select CABLE Output. Disable duplicate Desktop Audio/Mic-Aux captures for the same mix to avoid echo.");
            ui.label("Use headphones for PirateWave's Monitor bus. Leave OBS monitoring off for the cable source unless you deliberately need another monitor path. The cable names describe its two ends; PirateWave outputs to CABLE Input and OBS receives from CABLE Output.");
            ui.hyperlink_to("Free VB-CABLE setup", "https://vb-audio.com/Cable/");
        });
        if self.smoke && self.smoke_stage == 3 {
            ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
        }
    }
    fn obs_links_editor(&mut self, ui: &mut egui::Ui, state: &State) {
        card().show(ui,|ui|{
            ui.strong("OBS scene > PirateWave audio scene");
            ui.small("Mappings save with this workspace. Linking starts disarmed and is disarmed after disconnects, workspace changes or mapping edits. Arming observes a baseline; it does not recall the current scene.");
            let mut armed=self.obs_ui.armed.is_some()||self.obs_ui.barrier.is_some();
            if ui.add_enabled(state.connected,egui::Checkbox::new(&mut armed,if self.obs_ui.barrier.is_some(){"Arming scene linking…"}else{"Arm scene linking"})).changed(){
                if armed{if let Some(token)=self.obs_command(Action::ArmBarrier){self.obs_ui.barrier=Some((token,state.epoch,self.controller.generation));}}
                else{self.obs_ui.disarm();}
            }
            let before=self.project.workflows.obs_scene_links.clone();let mut remove=None;
            for (i,link) in self.project.workflows.obs_scene_links.iter_mut().enumerate(){ui.push_id(("obs-link",i),|ui|{
                ui.horizontal_wrapped(|ui|{
                    ui.add(egui::TextEdit::singleline(&mut link.obs_scene).desired_width(190.).hint_text("Exact OBS scene name"));
                    egui::ComboBox::from_id_salt("obs-name").selected_text("Choose OBS scene").show_ui(ui,|ui|{for name in &state.scenes{ui.selectable_value(&mut link.obs_scene,name.clone(),name);}});
                    let audio=self.project.workflows.scenes.iter().find(|s|s.id==link.audio_scene);
                    egui::ComboBox::from_id_salt("audio-name").selected_text(audio.map_or("Missing audio scene",|s|s.name.as_str())).show_ui(ui,|ui|{for scene in &self.project.workflows.scenes{ui.selectable_value(&mut link.audio_scene,scene.id,&scene.name);}});
                    if ui.small_button("Remove link").clicked(){remove=Some(i);}
                });
                if !self.project.workflows.scenes.iter().any(|s|s.id==link.audio_scene){ui.colored_label(theme::palette().peach,"Missing PirateWave scene: choose a replacement");}
                if state.connected&&!state.scenes.contains(&link.obs_scene){ui.colored_label(theme::palette().peach,"OBS scene is unavailable: check its exact name or scene collection");}
            });}
            if let Some(i)=remove{self.project.workflows.obs_scene_links.remove(i);}
            if ui.add_enabled(!self.project.workflows.scenes.is_empty()&&self.project.workflows.obs_scene_links.len()<128,egui::Button::new("Add scene link")).clicked(){
                let scene=state.scenes.iter().find(|name|!self.project.workflows.obs_scene_links.iter().any(|l|&l.obs_scene==*name)).cloned().unwrap_or_else(||{
                    let mut n=1;while self.project.workflows.obs_scene_links.iter().any(|l|l.obs_scene==format!("OBS scene {n}")){n+=1;}format!("OBS scene {n}")
                });
                self.project.workflows.obs_scene_links.push(ObsSceneLink{obs_scene:scene,audio_scene:self.project.workflows.scenes[0].id});
            }
            if self.project.workflows.scenes.is_empty(){ui.small("Create an audio scene on Scenes first, then link it here.");}
            if before!=self.project.workflows.obs_scene_links{
                self.obs_ui.disarm();
                match piratewave_core::workflows::validate(&self.project){Ok(())=>{self.dirty=true;self.changed_at=Instant::now();self.workflow_ui.snapshot_dirty=true;},Err(e)=>{self.project.workflows.obs_scene_links=before;self.notify_error(e);}}
            }
            if ui.add_enabled(state.connected,egui::Button::new("Recall link for current OBS scene now")).clicked(){self.apply_obs_scene(&state.current_scene);}
        });
    }
}
fn status(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Active",
        Some(false) => "Stopped",
        None => "Unknown",
    }
}

impl PirateWaveApp {
    pub(super) fn obs_smoke_tick(&mut self, ctx: &egui::Context) {
        const NAMES: [&str; 4] = [
            "obs-offline-1380",
            "obs-connected-1380",
            "obs-connected-1000",
            "obs-links-1380",
        ];
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                if let Some(name) = NAMES.get(self.smoke_stage) {
                    let pixels: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    if let Err(e) = image::save_buffer(
                        format!("dist/native-{name}.png"),
                        &pixels,
                        image.width() as u32,
                        image.height() as u32,
                        image::ColorType::Rgba8,
                    ) {
                        self.smoke_error = Some(e.to_string());
                    }
                    self.smoke_stage += 1;
                    self.smoke_waiting = false;
                    self.smoke_at = Instant::now();
                }
            }
        }
        if self.smoke_stage >= NAMES.len() {
            let result=self.smoke_error.as_ref().map_or("PASS: four native OBS views rendered (offline, connected fixture, narrow fixture, scene links); no OBS connection or user settings changed".into(),|e|format!("FAIL: {e}"));
            let _ = std::fs::write("dist/obs-ui-smoke-result.txt", result);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !self.smoke_waiting && self.smoke_at.elapsed() > Duration::from_millis(300) {
            self.view = View::Obs;
            self.obs_ui.service = None;
            self.project.name = "OBS integration preview".into();
            self.project.workflows.scenes = vec![piratewave_core::workflows::Scene::capture(
                &self.project,
                &self.registry,
                "Live mix".into(),
            )];
            self.project.workflows.obs_scene_links = vec![ObsSceneLink {
                obs_scene: "Live".into(),
                audio_scene: self.project.workflows.scenes[0].id,
            }];
            self.obs_ui.fixture = if self.smoke_stage == 0 {
                None
            } else {
                Some(std::sync::Arc::new(State {
                    connected: true,
                    epoch: 1,
                    message: "Connected to OBS (screenshot fixture)".into(),
                    version: "32.fixture".into(),
                    scenes: vec!["Starting Soon".into(), "Live".into()],
                    current_scene: "Live".into(),
                    inputs: vec!["PirateWave cable".into()],
                    selected_input: "PirateWave cable".into(),
                    input_muted: Some(false),
                    input_db: Some(-6.),
                    streaming: Some(false),
                    recording: Some(true),
                    ..Default::default()
                }))
            };
            self.obs_ui.input = "PirateWave cable".into();
            self.obs_ui.selected_scene = "Live".into();
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(if self.smoke_stage == 2 {
                Vec2::new(1000., 700.)
            } else {
                Vec2::new(1380., 960.)
            }));
            if self.smoke_at.elapsed() > Duration::from_millis(800) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                self.smoke_waiting = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{app, labels, render};
    use super::*;
    #[test]
    fn obs_page_is_inert_until_explicit_connect_and_password_is_not_serialized() {
        let mut app = app();
        app.view = View::Obs;
        app.obs_ui.password = "never-save-this".into();
        let (engine, rx) = Engine::test_channel();
        app.engine = engine;
        let project = serde_json::to_string(&app.project).unwrap();
        let ctx = egui::Context::default();
        let output = render(&ctx, &mut app, vec![]);
        let texts = labels(&output);
        assert!(texts.iter().any(|(t, _)| t == "OBS Studio"));
        assert!(app.obs_ui.service.is_none());
        assert!(rx.try_recv().is_err());
        assert!(!app.dirty);
        assert_eq!(serde_json::to_string(&app.project).unwrap(), project);
        assert!(!serde_json::to_string(&app.app_settings)
            .unwrap()
            .contains("never-save-this"));
        let legacy: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.obs_port, 4455);
    }
    #[test]
    fn scene_link_requires_baseline_and_rejects_stale_duplicate_or_missing_targets() {
        let mut app = app();
        let (engine, rx) = Engine::test_channel();
        app.engine = engine;
        app.controller.generation = 77;
        let mut scene = piratewave_core::workflows::Scene::capture(
            &app.project,
            &app.registry,
            "Live mix".into(),
        );
        for s in &mut scene.settings {
            if let piratewave_core::workflows::Control::Gain { value } = &mut s.value {
                *value = -12.;
            }
        }
        let id = scene.id;
        app.project.workflows.scenes.push(scene);
        app.project.workflows.obs_scene_links.push(ObsSceneLink {
            obs_scene: "Live".into(),
            audio_scene: id,
        });
        let evt = || Event::ProgramScene {
            epoch: 2,
            generation: 77,
            name: "Live".into(),
        };
        app.obs_ui.barrier = Some((123, 2, 77));
        app.handle_obs_event(evt(), true, 2);
        assert!(rx.try_recv().is_err());
        app.handle_obs_event(
            Event::Barrier {
                epoch: 2,
                generation: 77,
                token: 123,
                name: "Starting Soon".into(),
            },
            true,
            2,
        );
        assert_eq!(app.obs_ui.armed, Some((2, 77)));
        assert!(rx.try_recv().is_err());
        app.handle_obs_event(
            Event::ProgramScene {
                epoch: 1,
                generation: 77,
                name: "Live".into(),
            },
            true,
            2,
        );
        assert!(rx.try_recv().is_err());
        app.handle_obs_event(
            Event::ProgramScene {
                epoch: 2,
                generation: 76,
                name: "Live".into(),
            },
            true,
            2,
        );
        assert!(rx.try_recv().is_err());
        app.handle_obs_event(evt(), true, 2);
        assert_eq!(app.project.nodes[0].gain_db, -12.);
        assert!(
            matches!(rx.try_recv().unwrap(),Command::Perform{context,..}if context.origin==piratewave_core::automation::Origin::Obs)
        );
        while rx.try_recv().is_ok() {}
        app.handle_obs_event(evt(), true, 2);
        assert!(rx.try_recv().is_err());
        app.project.workflows.scenes.clear();
        app.obs_ui.last_scene.clear();
        app.handle_obs_event(evt(), true, 2);
        assert!(app.obs_ui.armed.is_none());
        assert!(app.notice.contains("missing"));
    }
    #[test]
    fn links_round_trip_and_duplicate_names_validate_without_requiring_live_obs() {
        let mut app = app();
        let scene =
            piratewave_core::workflows::Scene::capture(&app.project, &app.registry, "Mix".into());
        let id = scene.id;
        app.project.workflows.scenes.push(scene);
        app.project.workflows.obs_scene_links.push(ObsSceneLink {
            obs_scene: "Live".into(),
            audio_scene: id,
        });
        let p: Project =
            serde_json::from_value(serde_json::to_value(&app.project).unwrap()).unwrap();
        assert_eq!(
            p.workflows.obs_scene_links,
            app.project.workflows.obs_scene_links
        );
        app.project
            .workflows
            .obs_scene_links
            .push(app.project.workflows.obs_scene_links[0].clone());
        assert!(piratewave_core::workflows::validate(&app.project).is_err());
        app.obs_ui.armed = Some((1, 0));
        app.reset_workflow_runtime();
        assert!(app.obs_ui.armed.is_none());
    }
}
