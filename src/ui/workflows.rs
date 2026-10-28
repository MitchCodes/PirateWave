//! Workflow pages and asynchronous control-plane acknowledgements.
use super::*;
use crate::session_io;
use piratewave_core::workflows::{
    CapturePoint, CaptureStage, Scene, SessionAction, SessionConfig, Track,
};
#[derive(Default)]
pub(super) struct WorkflowUi {
    pub snapshot_dirty: bool,
    cached_project: Option<Arc<Project>>,
    pub level_causes: HashMap<u64, (piratewave_core::automation::Context, Instant)>,
    pub context: Option<piratewave_core::automation::Context>,
    pub command_sent: bool,
    pub action_error: Option<String>,
    pub target: u64,
    pub waiting: HashSet<u64>,
    pub snapshot_context: Option<piratewave_core::automation::Context>,
    pub pending_slot_contexts: HashMap<(u64, u64), piratewave_core::automation::Context>,
    pub slot_contexts: HashMap<u64, piratewave_core::automation::Context>,
    pub automation: crate::workflow_runtime::Worker,
    pub automation_status: Option<crate::workflow_runtime::Status>,
    last_facts: Option<Instant>,
    scene: Option<u64>,
    session: Option<u64>,
    scene_reply: Option<ScenePending>,
    memory_reply: Option<HistoryReply>,
    snapshot_reply: Option<(
        u64,
        mpsc::Receiver<Result<piratewave_core::recording::SessionSnapshot, String>>,
    )>,
    attach_reply: Option<(
        piratewave_core::automation::Context,
        mpsc::Receiver<Result<(), String>>,
    )>,
    finish_context: HashMap<u64, piratewave_core::automation::Context>,
    pub media: session_io::Service,
    pub live: Option<(u64, Arc<session_io::Pool>)>,
    recording_request: Option<(u64, u64)>,
    session_outcomes: HashMap<u64, String>,
    takes: Vec<session_io::Take>,
    recovery_prompt: bool,
    recovery_offered: bool,
    recover_dest: HashMap<u64, u64>,
    loaded: bool,
    history: HashSet<u64>,
    delete_take: Option<String>,
    selected_take: Option<String>,
    selected_media: Option<session_io::Take>,
    take_page: usize,
    take_view: bool,
    loading_takes: bool,
    crop: [f64; 2],
    take_name: String,
    marker_name: String,
    rename_take: String,
    preview: Option<session_io::Preview>,
    preview_tracks: HashSet<u64>,
    preview_device: String,
    pub pending_workspace: Option<(u64, Instant)>,
    pub pending_workspace_delete: Option<u64>,
}
struct ScenePending {
    reply: mpsc::Receiver<Result<Project, String>>,
    original: Project,
    predicted: Project,
}
type HistoryReply = (u64, bool, mpsc::Receiver<Result<usize, String>>);
use std::sync::Arc;
fn id() -> u64 {
    piratewave_core::workflows::new_id()
}
fn unique(base: &str, names: impl Iterator<Item = String>) -> String {
    let names: HashSet<_> = names.map(|n| n.to_lowercase()).collect();
    if !names.contains(&base.to_lowercase()) {
        return base.into();
    }
    for n in 2.. {
        let value = format!("{base} {n}");
        if !names.contains(&value.to_lowercase()) {
            return value;
        }
    }
    unreachable!()
}
impl PirateWaveApp {
    pub(super) fn reset_workflow_runtime(&mut self) {
        self.obs_ui.disarm();
        if let Some(service) = &self.obs_ui.service {
            service.workspace.store(
                self.controller.generation,
                std::sync::atomic::Ordering::Release,
            );
        }

        self.workflow_ui.cached_project = None;
        self.workflow_ui.level_causes.clear();
        self.workflow_ui.recording_request = None;
        self.workflow_ui.session_outcomes.clear();
        let _ = self
            .workflow_ui
            .automation
            .send(crate::workflow_runtime::Command::Reset);
        self.workflow_ui.media.generation.store(
            self.controller.generation,
            std::sync::atomic::Ordering::Release,
        );
        self.workflow_ui.preview = None;
        self.workflow_ui.pending_slot_contexts.clear();
        self.workflow_ui.slot_contexts.clear();
        self.workflow_ui.waiting.clear();
        self.workflow_ui.history.clear();
        self.workflow_ui.live = None;
        self.workflow_ui.snapshot_reply = None;
        self.workflow_ui.scene_reply = None;
        self.workflow_ui.attach_reply = None;
        self.workflow_ui.scene = None;
        self.workflow_ui.selected_take = None;
        self.workflow_ui.selected_media = None;
        self.workflow_ui.take_page = 0;
        self.workflow_ui.loaded = false;
        self.workflow_ui.takes.clear();
    }
    pub(super) fn recall_scene(&mut self, id: u64) {
        let Some(scene) = self
            .project
            .workflows
            .scenes
            .iter()
            .find(|s| s.id == id)
            .cloned()
        else {
            self.notify_error("Scene is missing");
            return;
        };
        if self.status.running {
            if self.workflow_ui.scene_reply.is_some() {
                self.notify_error("A scene request is awaiting acknowledgment");
                return;
            }
            let predicted = match scene.apply(&self.project, &self.registry) {
                Ok(p) => p,
                Err(e) => {
                    self.notify_error(e);
                    return;
                }
            };
            if self.needs_apply {
                if let Err(e) = self.engine.send(Command::Update(self.project.clone())) {
                    self.notify_error(e);
                    return;
                }
                self.needs_apply = false;
            }
            let (reply, rx) = mpsc::sync_channel(1);
            let affected: Vec<_> = scene
                .settings
                .iter()
                .filter(|s| s.included)
                .map(|s| s.node)
                .collect();
            let cause_ms = scene.duration_ms as u64 + 150;
            match self.engine.send(Command::Perform {
                context: self.workflow_context(),
                target: id,
                command: Box::new(Command::RecallScene { scene, reply }),
            }) {
                Ok(()) => {
                    let original = std::mem::replace(&mut self.project, predicted.clone());
                    self.workflow_ui.scene_reply = Some(ScenePending {
                        reply: rx,
                        original,
                        predicted,
                    });
                    self.workflow_ui.snapshot_dirty = true;
                    self.dirty = true;
                    self.changed_at = Instant::now();
                    self.workflow_ui.command_sent = true;
                    for node in affected {
                        self.record_level_cause(node, cause_ms);
                    }
                }
                Err(e) => self.notify_error(e),
            }
        } else {
            match scene.apply(&self.project, &self.registry) {
                Ok(p) => {
                    self.project = p;
                    self.workflow_ui.scene = Some(id);
                    self.changed(false);
                }
                Err(e) => self.notify_error(e),
            }
        }
    }
    fn trim_take_cache(&mut self) {
        for take in &mut self.workflow_ui.takes {
            for track in &mut take.tracks {
                track.waveform.clear();
            }
            take.markers.clear();
            take.discontinuities.clear();
        }
        if self.workflow_ui.takes.len() > 128 {
            self.workflow_ui
                .takes
                .drain(..self.workflow_ui.takes.len() - 128);
        }
    }
    pub(super) fn current_control_project(&self) -> Project {
        if self.status.scene_controls.is_empty() {
            return self.project.clone();
        }
        let current = Scene {
            id: 0,
            name: String::new(),
            duration_ms: 0,
            effect_kinds: vec![],
            settings: self.status.scene_controls.clone(),
            keybind: String::new(),
        };
        current
            .apply(&self.project, &self.registry)
            .unwrap_or_else(|_| self.project.clone())
    }
    pub(super) fn take_current_adjustment(
        &mut self,
        selector: &piratewave_core::control::Selector,
    ) {
        let Ok(node) = piratewave_core::control::node(&self.project, selector) else {
            return;
        };
        let id = node.id;
        let settings: Vec<_> = self
            .status
            .scene_controls
            .iter()
            .filter(|s| {
                s.node == id
                    && match &s.value {
                        piratewave_core::workflows::Control::Gain { .. } => {
                            selector.category == "mixer" && selector.action == "volume"
                        }
                        piratewave_core::workflows::Control::Pan { .. } => {
                            selector.category == "mixer" && selector.action == "pan"
                        }
                        piratewave_core::workflows::Control::Parameter {
                            effect,
                            parameter,
                            ..
                        } => {
                            selector.category == "effect_parameter"
                                && selector.action == *parameter
                                && node
                                    .effects
                                    .iter()
                                    .any(|e| e.id == *effect && e.kind == selector.target)
                        }
                        piratewave_core::workflows::Control::DuckingParameter {
                            parameter, ..
                        } => {
                            selector.category == "ducking_parameter"
                                && selector.action == *parameter
                        }
                        piratewave_core::workflows::Control::Ducking { .. } => {
                            selector.category == "ducking_parameter"
                        }
                        _ => false,
                    }
            })
            .cloned()
            .collect();
        if !settings.is_empty() {
            let current = Scene {
                id: 0,
                name: String::new(),
                duration_ms: 0,
                effect_kinds: vec![],
                settings,
                keybind: String::new(),
            };
            if let Ok(p) = current.apply(&self.project, &self.registry) {
                self.project = p;
            }
        }
    }
    pub(super) fn record_level_cause(&mut self, origin: u64, milliseconds: u64) {
        let mut downstream = vec![origin];
        for _ in 0..self.project.nodes.len() {
            let before = downstream.len();
            for edge in &self.project.connections {
                if downstream.contains(&edge.from) && !downstream.contains(&edge.to) {
                    downstream.push(edge.to);
                }
            }
            if downstream.len() == before {
                break;
            }
        }
        for node in downstream {
            if let Some(context) = self
                .workflow_ui
                .context
                .filter(|c| c.origin == piratewave_core::automation::Origin::Automation)
            {
                self.workflow_ui.level_causes.insert(
                    node,
                    (
                        context,
                        Instant::now() + Duration::from_millis(milliseconds),
                    ),
                );
            } else {
                self.workflow_ui.level_causes.remove(&node);
            }
        }
    }
    pub(super) fn workflow_project(&mut self) -> Arc<Project> {
        if self.workflow_ui.snapshot_dirty || self.workflow_ui.cached_project.is_none() {
            self.workflow_ui.cached_project = Some(Arc::new(self.project.clone()));
            self.workflow_ui.snapshot_dirty = false;
        }
        self.workflow_ui.cached_project.as_ref().unwrap().clone()
    }
    fn workflow_facts(&self, levels: bool) -> piratewave_core::automation::Facts {
        piratewave_core::automation::Facts {
            activity: if levels {
                self.status
                    .peaks
                    .iter()
                    .map(|(id, p)| (*id, *p >= 10f32.powf(-35. / 20.)))
                    .chain(self.status.ducking.iter().map(|(id, d)| (*id, d.active)))
                    .collect()
            } else {
                Default::default()
            },
            causes: self
                .workflow_ui
                .level_causes
                .iter()
                .filter(|(_, (_, until))| *until > Instant::now())
                .map(|(id, (context, _))| (*id, *context))
                .collect(),
            running: self.status.running,
            scene: self.status.scene.map(|s| s.0),
            peaks: if levels {
                self.status.peaks.iter().map(|(id, p)| (*id, *p)).collect()
            } else {
                Default::default()
            },
            devices: self
                .status
                .available
                .iter()
                .map(|(id, v)| (*id, *v))
                .collect(),
            recording: self
                .workflow_ui
                .live
                .iter()
                .filter(|(_, p)| {
                    !p.stopped.load(std::sync::atomic::Ordering::Acquire)
                        && p.start.load(std::sync::atomic::Ordering::Acquire) != u64::MAX
                })
                .map(|(id, _)| *id)
                .collect(),
        }
    }
    pub(super) fn arm_automation(&mut self, armed: bool) {
        let command = if armed {
            crate::workflow_runtime::Command::Arm {
                project: self.workflow_project(),
                generation: self.controller.generation,
                facts: self.workflow_facts(true),
            }
        } else {
            crate::workflow_runtime::Command::Disarm
        };
        if let Err(e) = self.workflow_ui.automation.send(command) {
            self.notify_error(e);
        }
    }
    pub(super) fn run_rule(&mut self, rule: u64) {
        if !self
            .workflow_ui
            .automation
            .enabled
            .load(std::sync::atomic::Ordering::Acquire)
        {
            self.notify_error("Arm automation before running a rule");
            return;
        }
        if let Some(r) = self.project.workflows.rules.iter().find(|r| r.id == rule) {
            if !r.enabled {
                self.notify_error("Enable this rule before running it");
                return;
            }
            if let Err(e) = r.validate_targets(&self.project, &self.registry) {
                self.notify_error(e);
                return;
            }
        }
        let context = self.workflow_context();
        if let Err(e) = self
            .workflow_ui
            .automation
            .send(crate::workflow_runtime::Command::Event(
                piratewave_core::automation::Event {
                    context,
                    kind: piratewave_core::automation::EventKind::Manual(rule),
                },
            ))
        {
            self.notify_error(e);
        }
    }
    pub(super) fn workflow_context(&self) -> piratewave_core::automation::Context {
        self.workflow_ui
            .context
            .unwrap_or_else(|| piratewave_core::automation::Context {
                generation: self.controller.generation,
                operation: id(),
                ..Default::default()
            })
    }
    pub(super) fn workflow_completion(
        &mut self,
        context: piratewave_core::automation::Context,
        event: Option<piratewave_core::automation::EventKind>,
        result: Result<(), String>,
    ) {
        if context.generation != self.controller.generation {
            return;
        }
        if self.workflow_ui.waiting.remove(&context.operation) {
            let _ =
                self.workflow_ui
                    .automation
                    .send(crate::workflow_runtime::Command::Acknowledge {
                        operation: context.operation,
                        result: result.clone(),
                    });
        }
        if let Some(kind) = event {
            if let piratewave_core::automation::EventKind::Audio(running) = kind {
                self.status.running = running;
            }
            if matches!(kind, piratewave_core::automation::EventKind::Audio(false)) {
                if let Some((_, start_operation)) = self.workflow_ui.recording_request {
                    self.workflow_ui
                        .finish_context
                        .insert(start_operation, context);
                }
            }
            let _ = self
                .workflow_ui
                .automation
                .send(crate::workflow_runtime::Command::Event(
                    piratewave_core::automation::Event { context, kind },
                ));
        }
        if let Err(e) = result {
            if e.starts_with("Cancelled:") {
                crate::logging::info(format_args!("{e}"));
            } else {
                self.notify_error(e);
            }
        }
    }
    pub(super) fn poll_workflows(&mut self) {
        if self.controller.generation == 0 {
            self.controller.generation = id();
            self.command(Command::Generation(self.controller.generation));
        }
        self.workflow_ui.media.generation.store(
            self.controller.generation,
            std::sync::atomic::Ordering::Release,
        );
        if self
            .engine
            .workflow_overflow
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            self.arm_automation(false);
            self.notify_error("Workflow event queue saturated; automation disarmed");
        }
        while let Some(event) = self.engine.workflow_events.pop() {
            self.workflow_completion(event.context, event.event, event.result);
        }
        while let Ok(status) = self.workflow_ui.automation.statuses.try_recv() {
            if status.epoch
                == self
                    .workflow_ui
                    .automation
                    .epoch
                    .load(std::sync::atomic::Ordering::Acquire)
            {
                if let Some(error) = status.error.as_ref().filter(|error| {
                    self.workflow_ui
                        .automation_status
                        .as_ref()
                        .and_then(|s| s.error.as_ref())
                        != Some(*error)
                }) {
                    self.notify_error(error.clone());
                }
                self.workflow_ui.automation_status = Some(status);
            }
        }
        if self
            .workflow_ui
            .automation_status
            .as_ref()
            .is_some_and(|s| s.armed)
            && self
                .workflow_ui
                .last_facts
                .is_none_or(|at| at.elapsed() >= Duration::from_millis(50))
        {
            self.workflow_ui.last_facts = Some(Instant::now());
            let command = crate::workflow_runtime::Command::State {
                project: self.workflow_project(),
                facts: self.workflow_facts(
                    self.workflow_ui
                        .automation
                        .levels
                        .load(std::sync::atomic::Ordering::Acquire),
                ),
            };
            if let Err(e) = self.workflow_ui.automation.send(command) {
                self.notify_error(e);
            }
        }
        while let Ok(envelope) = self.workflow_ui.automation.ready.try_recv() {
            if envelope.epoch
                != self
                    .workflow_ui
                    .automation
                    .epoch
                    .load(std::sync::atomic::Ordering::Acquire)
                || !self
                    .workflow_ui
                    .automation
                    .enabled
                    .load(std::sync::atomic::Ordering::Acquire)
            {
                continue;
            }
            let ready = envelope.action;
            if ready.context.generation != self.controller.generation {
                continue;
            }
            self.workflow_ui.context = Some(ready.context);
            self.workflow_ui.command_sent = false;
            self.workflow_ui.action_error = None;
            self.workflow_ui.waiting.insert(ready.context.operation);
            let asynchronous = matches!(
                &ready.step,
                piratewave_core::workflows::Step::Scene { .. }
                    | piratewave_core::workflows::Step::Session {
                        action: SessionAction::Start | SessionAction::Capture,
                        ..
                    }
            );
            let result = match ready.step {
                piratewave_core::workflows::Step::Action { action } => {
                    if !action.exists(&self.project) {
                        Err("Automation target is missing".into())
                    } else if matches!(
                        action,
                        ShortcutAction::RecordingSlot {
                            action: piratewave_core::recording::SlotAction::Delete,
                            ..
                        }
                    ) {
                        Err("Deleting recordings is not permitted in automation".into())
                    } else {
                        self.handle_shortcut(action);
                        Ok(())
                    }
                }
                piratewave_core::workflows::Step::Set { node, control } => self.dispatch_typed(
                    piratewave_core::actions::Action::Set { node, control },
                    piratewave_core::automation::Origin::Automation,
                ),
                piratewave_core::workflows::Step::Scene { id } => {
                    self.recall_scene(id);
                    Ok(())
                }
                piratewave_core::workflows::Step::Session { id, action } => {
                    self.session_action(id, action);
                    Ok(())
                }
                piratewave_core::workflows::Step::Notify { text } => {
                    self.notify(text);
                    Ok(())
                }
                piratewave_core::workflows::Step::Wait { .. } => unreachable!(),
            };
            if self.needs_apply {
                self.command(if self.needs_restart {
                    Command::Reconfigure(self.project.clone())
                } else {
                    Command::Update(self.project.clone())
                });
                self.needs_apply = false;
                self.needs_restart = false;
            }
            let result = self.workflow_ui.action_error.take().map_or(result, Err);
            let pending = self.workflow_ui.command_sent || asynchronous;
            self.workflow_ui.context = None;
            if !pending || result.is_err() {
                self.workflow_completion(ready.context, None, result);
            }
        }
        if self
            .workflow_ui
            .preview
            .as_ref()
            .is_some_and(|p| p.done.load(std::sync::atomic::Ordering::Acquire))
        {
            if let Some(preview) = self.workflow_ui.preview.take() {
                if let Ok(Err(e)) = preview.result.try_recv() {
                    self.notify_error(format!("Preview failed: {e:#}"));
                }
            }
        }
        if !self.workflow_ui.loaded {
            let ids = self
                .project
                .workflows
                .sessions
                .iter()
                .flat_map(|s| s.take_ids.iter())
                .rev()
                .skip(self.workflow_ui.take_page * 64)
                .take(64)
                .cloned()
                .collect();
            let context = self.workflow_context();
            if self
                .workflow_ui
                .media
                .tasks
                .try_send(session_io::Task::List { context, ids })
                .is_ok()
            {
                self.workflow_ui.loaded = true;
                self.workflow_ui.loading_takes = true;
            }
        }
        if let Some(result) = self
            .workflow_ui
            .scene_reply
            .as_ref()
            .and_then(|r| r.reply.try_recv().ok())
        {
            let pending = self.workflow_ui.scene_reply.take().unwrap();
            let received = result.as_ref().unwrap_or(&pending.original);
            match Scene::merge_controls(&pending.predicted, &self.project, received, &self.registry)
            {
                Ok(project) => {
                    self.project = project;
                    self.workflow_ui.snapshot_dirty = true;
                    self.dirty = true;
                    self.changed_at = Instant::now();
                }
                Err(e) => self.notify_error(e),
            }
            if let Err(e) = result {
                self.notify_error(e);
            }
        }
        if let Some(result) = self
            .workflow_ui
            .memory_reply
            .as_ref()
            .and_then(|(id, armed, r)| r.try_recv().ok().map(|v| (*id, *armed, v)))
        {
            self.workflow_ui.memory_reply = None;
            match result.2 {
                Ok(bytes) => {
                    if result.1 {
                        self.workflow_ui.history.insert(result.0);
                    } else {
                        self.workflow_ui.history.remove(&result.0);
                    }
                    self.notify(format!(
                        "Shared history: {:.1} MiB",
                        bytes as f64 / 1048576.
                    ));
                }
                Err(e) => self.notify_error(e),
            }
        }
        if let Some((session, result)) = self
            .workflow_ui
            .snapshot_reply
            .as_ref()
            .and_then(|(s, r)| r.try_recv().ok().map(|v| (*s, v)))
        {
            self.workflow_ui.snapshot_reply = None;
            match result {
                Ok(snapshot) => {
                    let context = self
                        .workflow_ui
                        .snapshot_context
                        .take()
                        .unwrap_or_else(|| self.workflow_context());
                    let workspace = self.workspaces.as_ref().map_or(0, |s| s.active());
                    if self
                        .workflow_ui
                        .media
                        .tasks
                        .try_send(session_io::Task::Capture {
                            context,
                            workspace,
                            session,
                            name: "Retrospective capture".into(),
                            snapshot,
                        })
                        .is_err()
                    {
                        self.workflow_completion(
                            context,
                            Some(piratewave_core::automation::EventKind::Completion(
                                piratewave_core::workflows::Operation::Session,
                                session,
                                false,
                            )),
                            Err("Recording file queue is full".into()),
                        );
                    }
                }
                Err(e) => {
                    if let Some(context) = self.workflow_ui.snapshot_context.take() {
                        self.workflow_completion(
                            context,
                            Some(piratewave_core::automation::EventKind::Completion(
                                piratewave_core::workflows::Operation::Session,
                                session,
                                false,
                            )),
                            Err(e),
                        );
                    } else {
                        self.notify_error(e);
                    }
                }
            }
        }
        while let Ok(event) = self.workflow_ui.media.events.try_recv() {
            match event {
                session_io::Event::Listed { context, takes } => {
                    if context.generation == self.controller.generation {
                        self.workflow_ui.loading_takes = false;
                        let first = !self.workflow_ui.recovery_offered;
                        self.workflow_ui.recovery_offered = true;
                        self.workflow_ui.takes = takes;
                        if first && self.workflow_ui.takes.iter().any(|t| !t.complete) {
                            self.workflow_ui.recovery_prompt = true;
                        }
                    }
                }
                session_io::Event::Updated { context, take } => {
                    if context.generation == self.controller.generation {
                        if let Some(destination) =
                            self.workflow_ui.recover_dest.remove(&context.operation)
                        {
                            if let Some(session) = self
                                .project
                                .workflows
                                .sessions
                                .iter_mut()
                                .find(|s| s.id == destination)
                            {
                                if !session.take_ids.contains(&take.id) {
                                    session.take_ids.push(take.id.clone());
                                    self.changed(false);
                                }
                            }
                        }
                        self.workflow_ui.takes.retain(|t| t.id != take.id);
                        if self.workflow_ui.selected_take.as_ref() == Some(&take.id) {
                            self.workflow_ui.selected_media = Some(take.clone());
                        }
                        self.workflow_ui.takes.push(take);
                        self.trim_take_cache();
                        self.notify("Take updated");
                    }
                }
                session_io::Event::Prepared {
                    context,
                    take,
                    pool,
                } => {
                    if self.workflow_ui.pending_workspace.is_some()
                        || pool.stopped.load(std::sync::atomic::Ordering::Acquire)
                        || context.generation != self.controller.generation
                        || !self
                            .project
                            .workflows
                            .sessions
                            .iter()
                            .any(|s| s.id == take.session)
                    {
                        pool.fault.store(3, std::sync::atomic::Ordering::Release);
                        pool.stop();
                        continue;
                    }
                    if let Some(session) = self
                        .project
                        .workflows
                        .sessions
                        .iter_mut()
                        .find(|s| s.id == take.session)
                    {
                        session.take_ids.push(take.id.clone());
                        self.dirty = true;
                    }
                    let tracks = take.tracks.iter().map(|t| t.track.clone()).collect();
                    let (reply, rx) = mpsc::sync_channel(1);
                    match self.engine.send(Command::AttachTake {
                        pool: pool.clone(),
                        tracks,
                        reply,
                    }) {
                        Ok(()) => {
                            self.workflow_ui.live = Some((take.session, pool));
                            self.workflow_ui.attach_reply = Some((context, rx));
                        }
                        Err(e) => {
                            pool.fault.store(3, std::sync::atomic::Ordering::Release);
                            pool.stop();
                            self.notify_error(e);
                        }
                    }
                    if self.workflow_ui.selected_take.as_ref() == Some(&take.id) {
                        self.workflow_ui.selected_media = Some(take.clone());
                    }
                    self.workflow_ui.takes.push(take);
                    self.trim_take_cache();
                }
                session_io::Event::Finished { context, take } => {
                    if context.generation == self.controller.generation
                        && !self
                            .project
                            .workflows
                            .sessions
                            .iter()
                            .any(|s| s.id == take.session)
                    {
                        self.workflow_completion(
                            context,
                            None,
                            Err("Cancelled: recording session was removed".into()),
                        );
                        continue;
                    }
                    if self
                        .workflow_ui
                        .recording_request
                        .is_some_and(|(_, op)| op == context.operation)
                    {
                        self.workflow_ui.recording_request = None;
                    }
                    let context = self
                        .workflow_ui
                        .finish_context
                        .remove(&context.operation)
                        .unwrap_or(context);
                    if context.generation != self.controller.generation {
                        continue;
                    }
                    if let Some(session) = self
                        .project
                        .workflows
                        .sessions
                        .iter_mut()
                        .find(|s| s.id == take.session)
                    {
                        if !session.take_ids.contains(&take.id) {
                            session.take_ids.push(take.id.clone());
                            self.dirty = true;
                        }
                    }
                    self.workflow_ui.session_outcomes.insert(
                        take.session,
                        if take.complete {
                            format!(
                                "Take saved · {:.2}s",
                                take.end.saturating_sub(take.start) as f64 / 48000.
                            )
                        } else {
                            format!(
                                "Error: {}",
                                take.error.as_deref().unwrap_or("Recording interrupted")
                            )
                        },
                    );
                    self.workflow_completion(
                        context,
                        Some(piratewave_core::automation::EventKind::Completion(
                            piratewave_core::workflows::Operation::Session,
                            take.session,
                            take.complete,
                        )),
                        if take.complete {
                            Ok(())
                        } else {
                            Err(take.error.clone().unwrap_or("Take interrupted".into()))
                        },
                    );
                    self.workflow_ui.takes.retain(|t| t.id != take.id);
                    let error = take.error.clone();
                    if self.workflow_ui.selected_take.as_ref() == Some(&take.id) {
                        self.workflow_ui.selected_media = Some(take.clone());
                    }
                    self.workflow_ui.takes.push(take);
                    self.trim_take_cache();
                    if let Some(error) = error {
                        self.notify_error(error);
                    } else {
                        self.notify("Take completed");
                    }
                }
                session_io::Event::Failed {
                    context,
                    session,
                    error,
                } => {
                    self.workflow_ui.recover_dest.remove(&context.operation);
                    if let Some(session) = session {
                        self.workflow_ui
                            .session_outcomes
                            .insert(session, format!("Error: {error}"));
                    }
                    if self
                        .workflow_ui
                        .recording_request
                        .is_some_and(|(_, op)| op == context.operation)
                    {
                        self.workflow_ui.recording_request = None;
                    }
                    let context = self
                        .workflow_ui
                        .finish_context
                        .remove(&context.operation)
                        .unwrap_or(context);
                    self.workflow_completion(
                        context,
                        session.map(|id| {
                            piratewave_core::automation::EventKind::Completion(
                                piratewave_core::workflows::Operation::Session,
                                id,
                                false,
                            )
                        }),
                        Err(error),
                    );
                }
                session_io::Event::Exported { context, path } => {
                    if context.generation == self.controller.generation {
                        self.notify(format!("Completed: {}", path.display()));
                    }
                }
            }
        }
        if let Some(result) = self
            .workflow_ui
            .attach_reply
            .as_ref()
            .and_then(|(context, r)| r.try_recv().ok().map(|result| (*context, result)))
        {
            self.workflow_ui.attach_reply = None;
            self.workflow_completion(result.0, None, result.1);
        }
        if self
            .workflow_ui
            .live
            .as_ref()
            .is_some_and(|(_, p)| p.stopped.load(std::sync::atomic::Ordering::Acquire))
        {
            self.workflow_ui.live = None;
        }
        if let Some((workspace, since)) = self.workflow_ui.pending_workspace {
            if !self.workflow_ui.media.busy() || since.elapsed() > Duration::from_secs(10) {
                if self.workflow_ui.media.busy() {
                    self.notify_error("Take finalization timed out; its incomplete files remain available for recovery");
                }
                let delete = self.workflow_ui.pending_workspace_delete.take();
                self.switch_workspace(workspace);
                self.workflow_ui.pending_workspace = None;
                if let Some(id) = delete {
                    self.finish_workspace_deletion(id);
                }
            }
        }
    }
    pub(super) fn scene_selector(&mut self, ui: &mut egui::Ui) {
        let mut recall = None;
        ui.horizontal_wrapped(|ui| {
            ui.label("Scene");
            let selected = self.status.scene.map(|s| s.0).or(self.workflow_ui.scene);
            let name = selected
                .and_then(|id| self.project.workflows.scenes.iter().find(|s| s.id == id))
                .map_or("Choose scene", |s| s.name.as_str());
            egui::ComboBox::from_id_salt("mixer-scene")
                .selected_text(name)
                .show_ui(ui, |ui| {
                    for scene in &self.project.workflows.scenes {
                        if ui
                            .selectable_label(selected == Some(scene.id), &scene.name)
                            .clicked()
                        {
                            recall = Some(scene.id);
                        }
                    }
                });
            if let Some((_, progress, modified)) = self.status.scene {
                if progress < 1. {
                    ui.add(egui::ProgressBar::new(progress).desired_width(120.));
                }
                if modified {
                    ui.label("Modified");
                }
            }
            if self.view != View::Scenes && ui.small_button("Manage scenes").clicked() {
                self.view = View::Scenes;
            }
        });
        if let Some(id) = recall {
            self.recall_scene(id);
        }
    }
    pub(super) fn scenes_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Scenes");
        self.scene_selector(ui);
        if ui.button("Create scene from current mix").clicked() {
            let name = unique(
                "Scene",
                self.project.workflows.scenes.iter().map(|s| s.name.clone()),
            );
            let scene = Scene::capture(&self.current_control_project(), &self.registry, name);
            self.workflow_ui.scene = Some(scene.id);
            self.project.workflows.scenes.push(scene);
            self.changed(false);
        }
        let mut record_binding = None;
        let mut binding_cleared = false;
        let mut recall = None;
        let mut duplicate = None;
        let mut update = None;
        let mut delete = None;
        let mut changed = false;
        let names: Vec<_> = self
            .project
            .workflows
            .scenes
            .iter()
            .map(|s| (s.id, s.name.clone()))
            .collect();
        for scene in &mut self.project.workflows.scenes {
            ui.push_id(("scene-card", scene.id), |ui| {
                card().show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let before = scene.name.clone();
                        if ui.text_edit_singleline(&mut scene.name).changed() {
                            if scene.name.trim().is_empty()
                                || names.iter().any(|(id, n)| {
                                    *id != scene.id
                                        && n.trim().to_lowercase()
                                            == scene.name.trim().to_lowercase()
                                })
                            {
                                scene.name = before;
                            } else {
                                changed = true;
                            }
                        }
                        if ui.button("Recall").clicked() {
                            recall = Some(scene.id);
                        }
                        if ui.button("Update from mix").clicked() {
                            update = Some(scene.id);
                        }
                        if ui.button("Duplicate").clicked() {
                            duplicate = Some(scene.id);
                        }
                        if ui.button("Delete scene").clicked() {
                            delete = Some(scene.id);
                        }
                    });
                    match binding_control(ui, &scene.keybind) {
                        BindingChoice::Record => {
                            record_binding = Some((
                                ShortcutAction::Scene(scene.id),
                                format!("Recall {}", scene.name),
                            ))
                        }
                        BindingChoice::Clear => {
                            scene.keybind.clear();
                            changed = true;
                            binding_cleared = true;
                        }
                        BindingChoice::None => {}
                    }
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut scene.duration_ms, 0..=10000)
                                .text("Transition (ms)"),
                        )
                        .changed();
                    ui.collapsing("Included controls", |ui| {
                        let mut node_ids = Vec::new();
                        for setting in &scene.settings {
                            if !node_ids.contains(&setting.node) {
                                node_ids.push(setting.node);
                            }
                        }
                        for node in node_ids {
                            let name = self
                                .project
                                .nodes
                                .iter()
                                .find(|n| n.id == node)
                                .map_or_else(|| format!("Missing node {node}"), |n| n.name.clone());
                            ui.push_id((scene.id, node), |ui| {
                                let mut included = scene
                                    .settings
                                    .iter()
                                    .filter(|s| s.node == node)
                                    .all(|s| s.included);
                                if ui.checkbox(&mut included, &name).changed() {
                                    for setting in
                                        scene.settings.iter_mut().filter(|s| s.node == node)
                                    {
                                        setting.included = included;
                                    }
                                    changed = true;
                                }
                                ui.collapsing("Individual controls", |ui| {
                                    for setting in
                                        scene.settings.iter_mut().filter(|s| s.node == node)
                                    {
                                        changed |= ui
                                            .checkbox(&mut setting.included, setting.value.label())
                                            .changed();
                                    }
                                });
                            });
                        }
                    });
                })
            });
        }
        if let Some(id) = delete {
            self.project.workflows.scenes.retain(|s| s.id != id);
            changed = true;
        }
        if let Some(id) = duplicate {
            if let Some(mut scene) = self
                .project
                .workflows
                .scenes
                .iter()
                .find(|s| s.id == id)
                .cloned()
            {
                scene.id = crate::ui::workflows::id();
                scene.name = unique(
                    &scene.name,
                    self.project.workflows.scenes.iter().map(|s| s.name.clone()),
                );
                scene.keybind.clear();
                self.project.workflows.scenes.push(scene);
                changed = true;
            }
        }
        if let Some(id) = update {
            if let Some(index) = self
                .project
                .workflows
                .scenes
                .iter()
                .position(|s| s.id == id)
            {
                let old = &self.project.workflows.scenes[index];
                let mut next = Scene::capture(&self.project, &self.registry, old.name.clone());
                next.id = id;
                next.duration_ms = old.duration_ms;
                next.keybind = old.keybind.clone();
                for setting in &mut next.settings {
                    if let Some(old) = old
                        .settings
                        .iter()
                        .find(|s| s.node == setting.node && s.value.same_target(&setting.value))
                    {
                        setting.included = old.included;
                    }
                }
                self.project.workflows.scenes[index] = next;
                changed = true;
            }
        }
        if changed {
            self.changed(false);
        }
        if binding_cleared || delete.is_some() {
            self.refresh_shortcuts();
        }
        if let Some((target, label)) = record_binding {
            self.begin_shortcut_recording(ui.ctx(), target, label);
        }
        if let Some(id) = recall {
            self.recall_scene(id);
        }
        if self.project.workflows.scenes.is_empty() {
            ui.label("Create a scene to remember mixer and built-in effect settings. Routing and devices stay as they are.");
        }
    }
    pub(super) fn session_action(&mut self, session: u64, action: SessionAction) {
        match action {
            SessionAction::Start => {
                let Some(config) = self
                    .project
                    .workflows
                    .sessions
                    .iter()
                    .find(|s| s.id == session)
                else {
                    return;
                };
                for track in config.tracks.iter().filter(|t| t.armed) {
                    if let Err(e) = track.point.validate(&self.project) {
                        self.notify_error(e);
                        return;
                    }
                }
                let tracks = config.tracks.iter().filter(|t| t.armed).cloned().collect();
                let context = self.workflow_context();
                let workspace = self.workspaces.as_ref().map_or(0, |s| s.active());
                if !self.status.running {
                    self.notify_error("Start audio before recording");
                    return;
                }
                self.workflow_ui.command_sent = true;
                self.workflow_ui.recording_request = Some((session, context.operation));
                if let Err(e) = self.workflow_ui.media.start(
                    context,
                    workspace,
                    session,
                    if self.workflow_ui.take_name.trim().is_empty() {
                        config.name.clone()
                    } else {
                        self.workflow_ui.take_name.trim().to_owned()
                    },
                    tracks,
                ) {
                    self.workflow_ui.recording_request = None;
                    self.notify_error(format!("{e:#}"));
                }
            }
            SessionAction::Stop => {
                if self
                    .workflow_ui
                    .recording_request
                    .is_some_and(|(id, _)| id == session)
                {
                    self.workflow_ui.media.stop();
                }
                if let Some((id, pool)) = &self.workflow_ui.live {
                    if *id == session {
                        self.workflow_ui.finish_context.insert(
                            self.workflow_ui.recording_request.map_or(0, |(_, op)| op),
                            self.workflow_context(),
                        );
                        pool.stop();
                        if let Err(e) = self.engine.send(Command::StopTake) {
                            self.notify_error(e);
                        }
                        self.workflow_ui.command_sent = true;
                    }
                }
            }
            SessionAction::Capture => {
                if self.workflow_ui.snapshot_reply.is_some() {
                    self.notify_error("A capture request is pending");
                    return;
                }
                let (reply, rx) = mpsc::sync_channel(1);
                match self
                    .engine
                    .send(Command::SnapshotSession { id: session, reply })
                {
                    Ok(()) => {
                        self.workflow_ui
                            .session_outcomes
                            .insert(session, "Capture queued".into());
                        self.workflow_ui.snapshot_reply = Some((session, rx));
                        self.workflow_ui.snapshot_context = Some(self.workflow_context());
                        self.workflow_ui.command_sent = true;
                    }
                    Err(e) => self.notify_error(e),
                }
            }
            SessionAction::Marker => {
                let result = if let Some((active, pool)) = &self.workflow_ui.live {
                    if *active == session {
                        pool.add_marker(session_io::Marker {
                            frame: pool.end.load(std::sync::atomic::Ordering::Acquire),
                            name: if self.workflow_ui.marker_name.trim().is_empty() {
                                "Marker".into()
                            } else {
                                self.workflow_ui.marker_name.trim().to_owned()
                            },
                        })
                    } else {
                        Err("This session is not recording".into())
                    }
                } else {
                    Err("This session is not recording".into())
                };
                if let Err(e) = result {
                    self.notify_error(e);
                }
            }
        }
    }
    pub(super) fn sessions_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Recording");
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.workflow_ui.take_view, false, "Tracks & recording");
            ui.selectable_value(&mut self.workflow_ui.take_view, true, "Take browser");
            if self.workflow_ui.live.is_some() && ui.button("Stop current take").clicked() {
                if let Some((session, _)) = &self.workflow_ui.live {
                    self.session_action(*session, SessionAction::Stop);
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Next take name");
            ui.text_edit_singleline(&mut self.workflow_ui.take_name);
            ui.label("Next marker");
            ui.text_edit_singleline(&mut self.workflow_ui.marker_name);
        });
        ui.label("Raw tracks bypass mixer effects, volume, mute and solo. Processed tracks record the selected output as heard. All tracks share one timeline.");
        if ui.button("New recording setup").clicked() {
            let name = unique(
                "Session",
                self.project
                    .workflows
                    .sessions
                    .iter()
                    .map(|s| s.name.clone()),
            );
            let sid = id();
            self.project.workflows.sessions.push(SessionConfig {
                id: sid,
                name,
                history_seconds: 60.,
                tracks: vec![],
                take_ids: vec![],
                keybinds: Default::default(),
            });
            self.workflow_ui.session = Some(sid);
            self.workflow_ui.take_view = false;
            self.changed(false);
        }
        let mut record_binding = None;
        let mut binding_cleared = false;
        let previous_sessions = self.project.workflows.sessions.clone();
        let proposed = piratewave_core::recording::Recorder::memory_for_sessions(
            &self.project,
            &self
                .project
                .workflows
                .sessions
                .iter()
                .map(|s| s.id)
                .collect(),
        );
        ui.label(format!("History if all sessions are armed: {:.1} MiB / 512 MiB (identical capture points share memory)",proposed as f64/1048576.));
        let mut action = None;
        let mut arm = None;
        let mut changed = false;
        for session in self
            .project
            .workflows
            .sessions
            .iter_mut()
            .filter(|_| !self.workflow_ui.take_view)
        {
            let locked = self
                .workflow_ui
                .live
                .as_ref()
                .is_some_and(|(id, _)| *id == session.id)
                || self.workflow_ui.media.busy();
            ui.push_id(("session-card",session.id),|ui| card().show(ui,|ui| {
                changed|=ui.add_enabled(!locked,egui::TextEdit::singleline(&mut session.name).hint_text("Session name")).changed();
                ui.horizontal_wrapped(|ui|{for (label,a) in [("Start live",SessionAction::Start),("Stop",SessionAction::Stop),("Capture history",SessionAction::Capture),("Marker",SessionAction::Marker)]{if ui.button(label).clicked(){action=Some((session.id,a));}}});
                ui.collapsing("Recording shortcuts",|ui|{for action in [SessionAction::Start,SessionAction::Stop,SessionAction::Capture,SessionAction::Marker]{ui.push_id((session.id,action),|ui|{ui.label(format!("{action:?}"));match binding_control(ui,session.keybinds.get(&action).map_or("",String::as_str)){BindingChoice::Record=>record_binding=Some((ShortcutAction::Session{id:session.id,action},format!("{} · {action:?}",session.name))),BindingChoice::Clear=>{session.keybinds.remove(&action);changed=true;binding_cleared=true;},BindingChoice::None=>{}}});}});
                let mut armed=self.workflow_ui.history.contains(&session.id);if ui.checkbox(&mut armed,"Arm retrospective history").changed(){arm=Some((session.id,armed));}
                ui.add_enabled_ui(!locked,|ui|{
                    changed|=ui.add(egui::Slider::new(&mut session.history_seconds,0.1..=600.).text("History (seconds)")).changed();
                    let bytes=session.tracks.iter().filter(|t|t.armed).count() as f64*session.history_seconds as f64*48000.*8.;ui.label(format!("Selected history estimate: {:.1} MiB; shared limit 512 MiB",bytes/1048576.));
                    let mut remove=None;
                    for track in &mut session.tracks {ui.horizontal_wrapped(|ui|{
                        changed|=ui.checkbox(&mut track.armed,"").changed();changed|=ui.text_edit_singleline(&mut track.name).changed();
                        egui::ComboBox::from_id_salt(("track-node",track.id)).selected_text(self.project.nodes.iter().find(|n|n.id==track.point.node).map_or("Missing node",|n|&n.name)).show_ui(ui,|ui|{for n in &self.project.nodes{changed|=ui.selectable_value(&mut track.point.node,n.id,&n.name).changed();}});
                        if let Some(node)=self.project.nodes.iter().find(|n|n.id==track.point.node){
                            egui::ComboBox::from_id_salt(("track-stage",track.id)).selected_text(format!("{:?}",track.point.stage)).show_ui(ui,|ui|{changed|=ui.selectable_value(&mut track.point.stage,CaptureStage::Processed,"Processed output").changed();if node.kind.is_source(){changed|=ui.selectable_value(&mut track.point.stage,CaptureStage::Raw,"Raw source (ignores mute)").changed();}});
                            if track.point.stage==CaptureStage::Processed{egui::ComboBox::from_id_salt(("track-port",track.id)).selected_text(node.kind.output_label(track.point.port)).show_ui(ui,|ui|{let ports=node.kind.outputs();if ports.is_empty(){changed|=ui.selectable_value(&mut track.point.port,piratewave_core::project::OutputPort::Passthrough,"Output bus").changed();}for port in ports.iter(){changed|=ui.selectable_value(&mut track.point.port,*port,node.kind.output_label(*port)).changed();}});}
                        }
                        if ui.small_button("Remove").clicked(){remove=Some(track.id);}
                    });}
                    if let Some(id)=remove{session.tracks.retain(|t|t.id!=id);changed=true;}
                    if session.tracks.len()<32&&ui.button("Add track").clicked(){if let Some(n)=self.project.nodes.first(){let name=unique("Track",session.tracks.iter().map(|t|t.name.clone()));session.tracks.push(Track{id:id(),name,point:CapturePoint{node:n.id,port:piratewave_core::project::OutputPort::Passthrough,stage:CaptureStage::Processed},armed:true});changed=true;}}
                });
            }));
        }
        if let Some((id, armed)) = arm {
            if self.workflow_ui.memory_reply.is_some() {
                self.notify_error("Wait for the history configuration to apply");
                return;
            }
            let (reply, rx) = mpsc::sync_channel(1);
            if self
                .engine
                .send(Command::ArmSession { id, armed, reply })
                .is_ok()
            {
                self.workflow_ui.memory_reply = Some((id, armed, rx));
            }
        }
        if changed {
            let bytes = piratewave_core::recording::Recorder::memory_for_sessions(
                &self.project,
                &self.workflow_ui.history.iter().copied().collect(),
            );
            match piratewave_core::workflows::validate(&self.project).and_then(|()|if bytes>piratewave_core::recording::MAX_HISTORY_BYTES {Err("Shared history would exceed 512 MiB; disarm histories or reduce their duration".into())}else{Ok(())}) {
                Ok(())=>self.changed(false),Err(e)=>{self.project.workflows.sessions=previous_sessions;self.notify_error(e);}
            }
        }
        if let Some((id, a)) = action {
            self.session_action(id, a);
        }
        if binding_cleared {
            self.refresh_shortcuts();
        }
        if let Some((target, label)) = record_binding {
            self.begin_shortcut_recording(ui.ctx(), target, label);
        }
        if !self.workflow_ui.take_view {
            ui.small("Open Take browser to preview, crop, export, recover or delete recordings.");
            return;
        }
        ui.separator();
        ui.heading("Takes");
        if self.workflow_ui.takes.iter().any(|t| !t.complete) {
            let mut destination = self
                .workflow_ui
                .session
                .or_else(|| self.project.workflows.sessions.first().map(|s| s.id))
                .unwrap_or(0);
            ui.horizontal_wrapped(|ui| {
                ui.label("Recover into session");
                if pick_named(
                    ui,
                    "recovery-session",
                    &mut destination,
                    self.project
                        .workflows
                        .sessions
                        .iter()
                        .map(|s| (s.id, s.name.as_str())),
                ) {
                    self.workflow_ui.session = Some(destination);
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.workflow_ui.loading_takes && self.workflow_ui.take_page > 0,
                    egui::Button::new("Newer takes"),
                )
                .clicked()
            {
                self.workflow_ui.take_page -= 1;
                self.workflow_ui.loaded = false;
            }
            ui.label(format!("Page {}", self.workflow_ui.take_page + 1));
            let total = self
                .project
                .workflows
                .sessions
                .iter()
                .map(|s| s.take_ids.len())
                .sum::<usize>();
            if ui
                .add_enabled(
                    !self.workflow_ui.loading_takes
                        && (self.workflow_ui.take_page + 1) * 64 < total,
                    egui::Button::new("Older takes"),
                )
                .clicked()
            {
                self.workflow_ui.take_page += 1;
                self.workflow_ui.loaded = false;
            }
            if ui.button("Refresh takes").clicked() {
                self.workflow_ui.loaded = false;
            }
        });
        let refs: HashSet<_> = self
            .project
            .workflows
            .sessions
            .iter()
            .flat_map(|s| s.take_ids.iter().cloned())
            .collect();
        for take in self
            .workflow_ui
            .takes
            .clone()
            .into_iter()
            .filter(|t| refs.contains(&t.id) || !t.complete)
        {
            card().show(ui, |ui| {
                ui.label(format!(
                    "{} · {:.2}s · {} tracks{}",
                    take.name,
                    take.end.saturating_sub(take.start) as f64 / 48000.,
                    take.tracks.len(),
                    if take.complete { "" } else { " · interrupted" }
                ));
                if let Some(e) = &take.error {
                    ui.colored_label(theme::palette().peach, e);
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Open take").clicked() {
                        match self.workflow_ui.media.load(&take.id) {
                            Ok(full) => {
                                self.workflow_ui.preview_tracks = full
                                    .tracks
                                    .iter()
                                    .filter(|t| t.track.point.stage == CaptureStage::Processed)
                                    .map(|t| t.track.id)
                                    .collect();
                                self.workflow_ui.selected_media = Some(full);
                                self.workflow_ui.selected_take = Some(take.id.clone());
                            }
                            Err(e) => self.notify_error(format!("Take unavailable: {e:#}")),
                        }
                        self.workflow_ui.crop =
                            [0., take.end.saturating_sub(take.start) as f64 / 48000.];
                    }
                    if !take.complete
                        && ui
                            .add_enabled(
                                !self.workflow_ui.media.busy(),
                                egui::Button::new("Recover"),
                            )
                            .clicked()
                    {
                        let context = self.workflow_context();
                        let Some(destination) = self
                            .workflow_ui
                            .session
                            .or_else(|| self.project.workflows.sessions.first().map(|s| s.id))
                        else {
                            self.notify_error("Create a session before recovering this take");
                            return;
                        };
                        self.workflow_ui
                            .recover_dest
                            .insert(context.operation, destination);
                        let _ = self
                            .workflow_ui
                            .media
                            .tasks
                            .try_send(session_io::Task::Recover {
                                context,
                                id: take.id.clone(),
                            });
                    }
                    if ui
                        .add_enabled(!self.workflow_ui.media.busy(), egui::Button::new("Delete"))
                        .clicked()
                    {
                        self.workflow_ui.delete_take = Some(take.id.clone());
                    }
                });
            });
        }
        self.take_editor(ui);
        let page_refs: Vec<_> = self
            .project
            .workflows
            .sessions
            .iter()
            .flat_map(|s| s.take_ids.iter())
            .rev()
            .skip(self.workflow_ui.take_page * 64)
            .take(64)
            .cloned()
            .collect();
        for missing in page_refs.iter().filter(|id| {
            !self.workflow_ui.loading_takes && !self.workflow_ui.takes.iter().any(|t| &t.id == *id)
        }) {
            ui.colored_label(
                theme::palette().peach,
                format!("Unavailable media: {missing}. Workspace JSON does not embed recordings."),
            );
        }
    }
    pub(super) fn automation_page(&mut self, ui: &mut egui::Ui) {
        automation_editor(self, ui);
    }
}
fn automation_editor(app: &mut PirateWaveApp, ui: &mut egui::Ui) {
    use piratewave_core::workflows::{Condition, Operation, Step, Trigger};
    ui.heading("Automation");
    let mut armed = app
        .workflow_ui
        .automation_status
        .as_ref()
        .is_some_and(|s| s.armed);
    if ui.checkbox(&mut armed, "Arm automation").changed() {
        app.arm_automation(armed);
    }
    let mut master_binding = None;
    let mut clear_master = None;
    ui.collapsing("Master shortcuts", |ui| {
        for action in [
            piratewave_core::workflows::AutomationAction::Arm,
            piratewave_core::workflows::AutomationAction::Disarm,
        ] {
            ui.push_id(action, |ui| {
                ui.label(format!("{action:?} automation"));
                match binding_control(
                    ui,
                    app.project
                        .workflows
                        .automation_keybinds
                        .get(&action)
                        .map_or("", String::as_str),
                ) {
                    BindingChoice::Record => master_binding = Some(action),
                    BindingChoice::Clear => clear_master = Some(action),
                    BindingChoice::None => {}
                }
            });
        }
    });
    if let Some(action) = clear_master {
        app.project.workflows.automation_keybinds.remove(&action);
        app.changed(false);
        app.refresh_shortcuts();
    }
    if let Some(action) = master_binding {
        app.begin_shortcut_recording(
            ui.ctx(),
            ShortcutAction::Automation {
                armed: action == piratewave_core::workflows::AutomationAction::Arm,
            },
            format!("{action:?} automation"),
        );
    }
    ui.collapsing("Execution history", |ui| {
        if let Some(status) = &app.workflow_ui.automation_status {
            for entry in status.history.iter().rev().take(40) {
                let name = app
                    .project
                    .workflows
                    .rules
                    .iter()
                    .find(|r| r.id == entry.rule)
                    .map_or("Rule", |r| r.name.as_str());
                ui.label(format!(
                    "{:.2}s · {name} · {}",
                    entry.at_ms as f64 / 1000.,
                    entry.description
                ))
                .on_hover_text(format!(
                    "Operation {} · {:?}",
                    entry.context.operation, entry.context.origin
                ));
            }
        }
    });
    ui.small("Automation starts disarmed. Rules execute in order and stop on an error. Disarm cancels waiting steps.");
    if let Some(error) = app
        .workflow_ui
        .automation_status
        .as_ref()
        .and_then(|s| s.error.as_ref())
    {
        ui.colored_label(theme::palette().peach, error);
    }
    let mut add = None;
    ui.horizontal_wrapped(|ui| {
        for (label, trigger) in [
            ("New manual rule", Trigger::Manual),
            (
                "Speech / silence scene",
                Trigger::Level {
                    node: app.project.nodes.first().map_or(0, |n| n.id),
                    above: true,
                    threshold_db: -35.,
                    hysteresis_db: 6.,
                    hold_ms: 150,
                },
            ),
            (
                "Recording notification",
                Trigger::Completion {
                    operation: Operation::Session,
                    node: None,
                    success: true,
                },
            ),
            (
                "Device-loss scene",
                Trigger::Device {
                    node: app.project.nodes.first().map_or(0, |n| n.id),
                    available: false,
                },
            ),
        ] {
            if ui.button(label).clicked() {
                add = Some((label, trigger));
            }
        }
    });
    if let Some((template, trigger)) = add {
        match add_template(&mut app.project, &app.registry, template, trigger) {
            Ok(()) => {
                app.arm_automation(false);
                app.changed(false);
            }
            Err(e) => app.notify_error(e),
        }
    }
    let project_snapshot = app.project.clone();
    let catalog =
        piratewave_core::control::catalog(&app.project, &app.registry, app.status.running);
    let controls = Scene::capture(&app.project, &app.registry, String::new()).settings;
    let nodes = app.project.nodes.clone();
    let scenes = app.project.workflows.scenes.clone();
    let sessions = app.project.workflows.sessions.clone();
    let mut changed = false;
    let previous_rules = app.project.workflows.rules.clone();
    let mut record_binding = None;
    let mut binding_cleared = false;
    let mut remove = None;
    let mut run = None;
    for rule in &mut app.project.workflows.rules {
        ui.push_id(("rule-card",rule.id),|ui| card().show(ui,|ui|{
            ui.horizontal_wrapped(|ui|{changed|=ui.checkbox(&mut rule.enabled,"").changed();changed|=ui.text_edit_singleline(&mut rule.name).changed();if ui.button("Run").clicked(){run=Some(rule.id);}
if ui.button("Delete rule").clicked(){remove=Some(rule.id);}});
            if let Some(status)=app.workflow_ui.automation_status.as_ref().and_then(|s|s.rules.get(&rule.id)){ui.label(format!("{} · {} executions · {} skipped triggers",if status.running{"Running"}else{"Idle"},status.executions,status.skipped));if let Some(e)=&status.error{ui.colored_label(theme::palette().peach,e);}}
            match binding_control(ui,&rule.keybind){BindingChoice::Record=>record_binding=Some((ShortcutAction::Rule(rule.id),format!("Run {}",rule.name))),BindingChoice::Clear=>{rule.keybind.clear();changed=true;binding_cleared=true;},BindingChoice::None=>{}}
            ui.heading("When");
            let options=[("Manual",Trigger::Manual),("Level crossing",Trigger::Level{node:nodes.first().map_or(0,|n|n.id),above:true,threshold_db:-35.,hysteresis_db:6.,hold_ms:150}),("Silence",Trigger::Silence{node:nodes.first().map_or(0,|n|n.id),threshold_db:-35.,hold_ms:150}),("Audio starts / stops",Trigger::Audio{running:true}),("Scene completes",Trigger::Scene{id:scenes.first().map_or(0,|s|s.id)}),("Device availability",Trigger::Device{node:nodes.first().map_or(0,|n|n.id),available:false}),("Operation completes / fails",Trigger::Completion{operation:Operation::Session,node:None,success:true}),("Timer",Trigger::Timer{interval_ms:1000,repeating:false})];
            let label=options.iter().find(|(_,t)|std::mem::discriminant(t)==std::mem::discriminant(&rule.trigger)).map_or("Trigger",|(s,_)|*s);
            egui::ComboBox::from_id_salt(("trigger",rule.id)).selected_text(label).show_ui(ui,|ui|{for (label,t) in options {if ui.selectable_label(std::mem::discriminant(&t)==std::mem::discriminant(&rule.trigger),label).clicked(){rule.trigger=t;changed=true;}}});
            match &mut rule.trigger {
                Trigger::Level{node,above,threshold_db,hysteresis_db,hold_ms}=>{changed|=pick_node(ui,(rule.id,"trigger-node"),node,&nodes);changed|=ui.checkbox(above,"Cross above (uncheck for below)").changed();changed|=ui.add(egui::Slider::new(threshold_db,-100.0..=0.).text("Threshold dBFS")).changed();changed|=ui.add(egui::Slider::new(hysteresis_db,0.0..=40.).text("Hysteresis dB")).changed();changed|=ui.add(egui::Slider::new(hold_ms,0..=10000).text("Activation hold ms")).changed();}
                Trigger::Silence{node,threshold_db,hold_ms}=>{changed|=pick_node(ui,(rule.id,"trigger-node"),node,&nodes);changed|=ui.add(egui::Slider::new(threshold_db,-100.0..=0.).text("Threshold dBFS")).changed();changed|=ui.add(egui::Slider::new(hold_ms,0..=600000).text("Silence duration ms")).changed();}
                Trigger::Audio{running}=>{changed|=ui.checkbox(running,"Audio starts (uncheck for stops)").changed();}
                Trigger::Scene{id}=>{changed|=pick_named(ui,(rule.id,"scene"),id,scenes.iter().map(|s|(s.id,s.name.as_str())));}
                Trigger::Device{node,available}=>{changed|=pick_node(ui,(rule.id,"device"),node,&nodes);changed|=ui.checkbox(available,"Becomes available (uncheck for unavailable)").changed();}
                Trigger::Completion{operation,node,success}=>{
                    egui::ComboBox::from_id_salt((rule.id,"operation")).selected_text(format!("{operation:?}")).show_ui(ui,|ui|{for value in [Operation::Sound,Operation::SlotPlayback,Operation::SlotCapture,Operation::Session]{changed|=ui.selectable_value(operation,value,format!("{value:?}")).changed();}});
                    changed|=ui.checkbox(success,"Completes successfully (uncheck for failure)").changed();let mut specific=node.is_some();if ui.checkbox(&mut specific,"Specific target").changed(){*node=if specific{Some(nodes.first().map_or(0,|n|n.id))}else{None};changed=true;}
if let Some(id)=node {match operation{Operation::Session=>changed|=pick_named(ui,(rule.id,"completion-target"),id,sessions.iter().map(|s|(s.id,s.name.as_str()))),Operation::Sound=>changed|=pick_named(ui,(rule.id,"completion-target"),id,project_snapshot.sounds.iter().map(|s|(s.id,s.name.as_str()))),_=>changed|=pick_named(ui,(rule.id,"completion-target"),id,nodes.iter().filter(|n|matches!(n.kind,NodeKind::Recording)).map(|n|(n.id,n.name.as_str())))}}
                }
                Trigger::Timer{interval_ms,repeating}=>{changed|=ui.add(egui::DragValue::new(interval_ms).range(50..=86400000).prefix("Interval ms ")).changed();changed|=ui.checkbox(repeating,"Repeat").changed();}
                Trigger::Manual=>{}
            }
            changed|=ui.add(egui::Slider::new(&mut rule.cooldown_ms,0..=60000).text("Cooldown ms")).changed();
            ui.heading("If");changed|=ui.checkbox(&mut rule.match_all,"All conditions (uncheck for any)").changed();
            let mut delete_condition=None;
            for (index,condition) in rule.conditions.iter_mut().enumerate(){ui.push_id((rule.id,"condition",index),|ui|{
                let options=[("Activity",Condition::Activity{node:nodes.first().map_or(0,|n|n.id),active:true}),("Node enabled",Condition::NodeEnabled{node:nodes.first().map_or(0,|n|n.id),enabled:true}),("Recorder buffering",Condition::Recorder{node:nodes.iter().find(|n|matches!(n.kind,NodeKind::Recording)).map_or(0,|n|n.id),active:true}),("Audio state",Condition::Audio{running:true}),("Scene state",Condition::Scene{id:scenes.first().map_or(0,|s|s.id)}),("Node mute",Condition::NodeMute{node:nodes.first().map_or(0,|n|n.id),value:false}),("Effect enabled",Condition::Effect{node:nodes.first().map_or(0,|n|n.id),effect:0,enabled:true}),("Audio level",Condition::Level{node:nodes.first().map_or(0,|n|n.id),above:true,db:-35.}),("Device state",Condition::Device{node:nodes.first().map_or(0,|n|n.id),available:true}),("Recording state",Condition::Recording{session:sessions.first().map_or(0,|s|s.id),active:true})];
                let label=options.iter().find(|(_,c)|std::mem::discriminant(c)==std::mem::discriminant(condition)).map_or("Condition",|(s,_)|*s);
                egui::ComboBox::from_id_salt("condition-type").selected_text(label).show_ui(ui,|ui|{for (label,c) in options{if ui.selectable_label(false,label).clicked(){*condition=c;changed=true;}}});
                match condition {
                    Condition::Activity{node,active}=>{changed|=pick_node(ui,"node",node,&nodes);changed|=ui.checkbox(active,"Active").changed();}
                    Condition::NodeEnabled{node,enabled}=>{changed|=pick_node(ui,"node",node,&nodes);changed|=ui.checkbox(enabled,"Enabled").changed();}
                    Condition::Recorder{node,active}=>{changed|=pick_named(ui,"recorder",node,nodes.iter().filter(|n|matches!(n.kind,NodeKind::Recording)).map(|n|(n.id,n.name.as_str())));changed|=ui.checkbox(active,"Buffering").changed();}
                    Condition::Audio{running}=>changed|=ui.checkbox(running,"Audio running").changed(),
                    Condition::Scene{id}=>changed|=pick_named(ui,"scene",id,scenes.iter().map(|s|(s.id,s.name.as_str()))),
                    Condition::NodeMute{node,value}=>{changed|=pick_node(ui,"node",node,&nodes);changed|=ui.checkbox(value,"Muted").changed();}
                    Condition::Effect{node,effect,enabled}=>{changed|=pick_node(ui,"node",node,&nodes);if let Some(n)=nodes.iter().find(|n|n.id==*node){changed|=pick_named(ui,"effect",effect,n.effects.iter().map(|e|(e.id,e.kind.as_str())));}changed|=ui.checkbox(enabled,"Enabled").changed();}
                    Condition::Level{node,above,db}=>{changed|=pick_node(ui,"node",node,&nodes);changed|=ui.checkbox(above,"Above").changed();changed|=ui.add(egui::Slider::new(db,-100.0..=0.).text("dBFS")).changed();}
                    Condition::Device{node,available}=>{changed|=pick_node(ui,"node",node,&nodes);changed|=ui.checkbox(available,"Available").changed();}
                    Condition::Recording{session,active}=>{changed|=pick_named(ui,"session",session,sessions.iter().map(|s|(s.id,s.name.as_str())));changed|=ui.checkbox(active,"Recording").changed();}
                }
                if ui.small_button("Remove condition").clicked(){delete_condition=Some(index);}
            });}
            if let Some(i)=delete_condition{rule.conditions.remove(i);changed=true;}
            if rule.conditions.len()<32&&ui.button("Add condition").clicked(){rule.conditions.push(Condition::Audio{running:true});changed=true;}
            ui.heading("Then");let mut delete_step=None;let mut move_step=None;
            for (index,step) in rule.actions.iter_mut().enumerate(){ui.push_id((rule.id,"step",index),|ui|{
                ui.horizontal_wrapped(|ui|{ui.label(format!("Step {}",index+1));if index>0&&ui.small_button("Move up").clicked(){move_step=Some(index);}
if ui.small_button("Remove step").clicked(){delete_step=Some(index);}});
                let choices=[("Catalog action",Step::Action{action:ShortcutAction::App(AppAction::StopSounds)}),("Set control",Step::Set{node:nodes.first().map_or(0,|n|n.id),control:piratewave_core::workflows::Control::Gain{value:0.}}),("Recall scene",Step::Scene{id:scenes.first().map_or(0,|s|s.id)}),("Recording control",Step::Session{id:sessions.first().map_or(0,|s|s.id),action:SessionAction::Start}),("Wait",Step::Wait{milliseconds:1000}),("Notification",Step::Notify{text:"Workflow completed".into()})];
                let label=choices.iter().find(|(_,s)|std::mem::discriminant(s)==std::mem::discriminant(step)).map_or("Step",|(s,_)|*s);
                egui::ComboBox::from_id_salt("type").selected_text(label).show_ui(ui,|ui|{for (label,s) in choices {if ui.selectable_label(false,label).clicked(){*step=s;changed=true;}}});
                match step {
                    Step::Action{action}=>{let selected=catalog.iter().find(|e|piratewave_core::control::resolve(&project_snapshot,&e.selector).ok()==Some(*action)).map(|e|format!("{} {} {}",e.selector.node,e.selector.target,e.label)).unwrap_or("Missing action".into());
                        egui::ComboBox::from_id_salt("action").selected_text(selected).width(360.).show_ui(ui,|ui|{for e in &catalog {if let Ok(a)=piratewave_core::control::resolve(&project_snapshot,&e.selector){if matches!(a,ShortcutAction::RecordingSlot{action:piratewave_core::recording::SlotAction::Delete,..}){continue;}changed|=ui.selectable_value(action,a,format!("{} {} {}",e.selector.node,e.selector.target,e.label)).changed();}}});
                    }
                    Step::Set{node,control}=>{
                        egui::ComboBox::from_id_salt("control").selected_text(format!("{} · {}",nodes.iter().find(|n|n.id==*node).map_or("Missing",|n|n.name.as_str()),control.label())).width(360.).show_ui(ui,|ui|{for c in &controls {if ui.selectable_label(c.node==*node&&c.value.same_target(control),format!("{} · {}",nodes.iter().find(|n|n.id==c.node).map_or("Missing",|n|n.name.as_str()),c.value.label())).clicked(){*node=c.node;*control=c.value.clone();changed=true;}}});changed|=edit_control(ui,control,&app.registry,&nodes,*node);
                    }
                    Step::Scene{id}=>changed|=pick_named(ui,"scene",id,scenes.iter().map(|s|(s.id,s.name.as_str()))),
                    Step::Session{id,action}=>{changed|=pick_named(ui,"session",id,sessions.iter().map(|s|(s.id,s.name.as_str())));egui::ComboBox::from_id_salt("action").selected_text(format!("{action:?}")).show_ui(ui,|ui|{for a in [SessionAction::Start,SessionAction::Stop,SessionAction::Capture,SessionAction::Marker]{changed|=ui.selectable_value(action,a,format!("{a:?}")).changed();}});}
                    Step::Wait{milliseconds}=>changed|=ui.add(egui::DragValue::new(milliseconds).range(0..=86400000).suffix(" ms")).changed(),
                    Step::Notify{text}=>changed|=ui.text_edit_singleline(text).changed(),
                }
            });}
            if let Some(i)=delete_step{rule.actions.remove(i);changed=true;}else if let Some(i)=move_step{rule.actions.swap(i,i-1);changed=true;}
            if rule.actions.len()<32&&ui.button("Add step").clicked(){rule.actions.push(Step::Wait{milliseconds:1000});changed=true;}
        }));
    }
    if let Some(id) = remove {
        app.project.workflows.rules.retain(|r| r.id != id);
        changed = true;
    }
    if changed {
        app.arm_automation(false);
        match piratewave_core::workflows::validate(&app.project) {
            Ok(()) => app.changed(false),
            Err(e) => {
                app.project.workflows.rules = previous_rules;
                app.notify_error(e);
            }
        }
    }
    if binding_cleared || remove.is_some() {
        app.refresh_shortcuts();
    }
    if let Some((target, label)) = record_binding {
        app.begin_shortcut_recording(ui.ctx(), target, label);
    }
    if let Some(id) = run {
        app.run_rule(id);
    }
}
fn pick_named<'a>(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash,
    value: &mut u64,
    items: impl Iterator<Item = (u64, &'a str)>,
) -> bool {
    let items: Vec<_> = items.collect();
    let mut changed = false;
    egui::ComboBox::from_id_salt(salt)
        .selected_text(
            items
                .iter()
                .find(|(id, _)| id == value)
                .map_or("Missing target", |(_, n)| *n),
        )
        .show_ui(ui, |ui| {
            for (id, name) in items {
                changed |= ui.selectable_value(value, id, name).changed();
            }
        });
    changed
}
fn pick_node(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash,
    value: &mut u64,
    nodes: &[Node],
) -> bool {
    pick_named(
        ui,
        salt,
        value,
        nodes.iter().map(|n| (n.id, n.name.as_str())),
    )
}
fn edit_control(
    ui: &mut egui::Ui,
    control: &mut piratewave_core::workflows::Control,
    registry: &EffectRegistry,
    nodes: &[Node],
    node: u64,
) -> bool {
    use piratewave_core::workflows::Control;
    match control {
        Control::DuckingParameter { parameter, value } => {
            if let Some(p) = piratewave_core::ducking::PARAMETERS
                .iter()
                .find(|p| p.id == parameter)
            {
                super::ducking::parameter_widget(ui, value, p)
            } else {
                ui.label("Ducking parameter unavailable");
                false
            }
        }
        Control::Gain { value } => ui
            .add(egui::Slider::new(value, -60.0..=24.).text("dB"))
            .changed(),
        Control::Pan { value } => ui
            .add(egui::Slider::new(value, -1.0..=1.).text("Pan"))
            .changed(),
        Control::Mute { value }
        | Control::Solo { value }
        | Control::Listening { value }
        | Control::Bypass { value }
        | Control::EffectEnabled { value, .. } => ui.checkbox(value, "On").changed(),
        Control::Parameter {
            effect,
            parameter,
            value,
        } => {
            if let Some(p) = nodes
                .iter()
                .find(|n| n.id == node)
                .and_then(|n| n.effects.iter().find(|e| e.id == *effect))
                .and_then(|e| registry.get(&e.kind))
                .and_then(|e| e.parameters.iter().find(|p| p.id == parameter))
            {
                ui.add(egui::Slider::new(value, p.min..=p.max).text(parameter.as_str()))
                    .changed()
            } else {
                ui.colored_label(theme::palette().peach, "Parameter unavailable");
                false
            }
        }
        Control::Switch { value } => {
            let mut id = value.unwrap_or(0);
            let changed = pick_node(ui, "destination", &mut id, nodes);
            if changed {
                *value = Some(id);
            }
            changed
        }
        Control::Ducking { value } => ui
            .add(egui::Slider::new(&mut value.amount, -60.0..=0.).text("Duck amount dB"))
            .changed(),
    }
}
impl PirateWaveApp {
    fn take_editor(&mut self, ui: &mut egui::Ui) {
        if let Some(take) = self.workflow_ui.selected_media.clone() {
            ui.separator();
            ui.heading(&take.name);
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.workflow_ui.rename_take)
                        .hint_text("New take name"),
                );
                if ui.button("Rename take").clicked() {
                    let context = self.workflow_context();
                    if self
                        .workflow_ui
                        .media
                        .tasks
                        .try_send(session_io::Task::Rename {
                            context,
                            id: take.id.clone(),
                            name: self.workflow_ui.rename_take.clone(),
                        })
                        .is_err()
                    {
                        self.notify_error("Recording file queue is full");
                    }
                }
            });
            let duration = take.end.saturating_sub(take.start) as f64 / 48000.;
            let mut crop_changed = false;
            ui.horizontal_wrapped(|ui| {
                ui.label("Common crop:");
                crop_changed |= ui
                    .add(
                        egui::DragValue::new(&mut self.workflow_ui.crop[0])
                            .range(0.0..=duration)
                            .speed(0.01)
                            .prefix("Start ")
                            .suffix(" s"),
                    )
                    .changed();
                crop_changed |= ui
                    .add(
                        egui::DragValue::new(&mut self.workflow_ui.crop[1])
                            .range(0.0..=duration)
                            .speed(0.01)
                            .prefix("End ")
                            .suffix(" s"),
                    )
                    .changed();
                if ui.button("Reset selection").clicked() {
                    self.workflow_ui.crop = [0., duration];
                    crop_changed = true;
                }
            });
            let playhead = self
                .workflow_ui
                .preview
                .as_ref()
                .map(|p| p.frame.load(std::sync::atomic::Ordering::Acquire) as f64 / 48000.);
            for track in &take.tracks {
                let mut selected = self.workflow_ui.preview_tracks.contains(&track.track.id);
                if ui.checkbox(&mut selected, &track.track.name).changed() {
                    if selected {
                        self.workflow_ui.preview_tracks.insert(track.track.id);
                    } else {
                        self.workflow_ui.preview_tracks.remove(&track.track.id);
                    }
                    self.workflow_ui.preview = None;
                }
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width().max(120.), 64.),
                    egui::Sense::click_and_drag(),
                );
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 4., theme::palette().bg);
                let width = rect.width();
                let mid = rect.center().y;
                for (i, range) in track.waveform.iter().enumerate() {
                    let x = rect.left() + i as f32 / track.waveform.len().max(1) as f32 * width;
                    painter.line_segment(
                        [
                            Pos2::new(x, mid - range[0].clamp(-1., 1.) * 28.),
                            Pos2::new(x, mid - range[1].clamp(-1., 1.) * 28.),
                        ],
                        Stroke::new(1_f32, theme::palette().accent),
                    );
                }
                if duration > 0. {
                    for sec in self.workflow_ui.crop {
                        let x = rect.left() + (sec / duration) as f32 * width;
                        painter.line_segment(
                            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                            Stroke::new(2_f32, Color32::YELLOW),
                        );
                    }
                    if let Some(sec) = playhead {
                        let x = rect.left() + (sec / duration) as f32 * width;
                        painter.line_segment(
                            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                            Stroke::new(2_f32, Color32::WHITE),
                        );
                    }
                    if response.dragged() || response.clicked() {
                        if let Some(pos) = response.interact_pointer_pos() {
                            let time =
                                ((pos.x - rect.left()) / width).clamp(0., 1.) as f64 * duration;
                            let target = if (time - self.workflow_ui.crop[0]).abs()
                                < (time - self.workflow_ui.crop[1]).abs()
                            {
                                0
                            } else {
                                1
                            };
                            self.workflow_ui.crop[target] = time;
                            crop_changed = true;
                        }
                    }
                }
            }
            for marker in &take.markers {
                ui.label(format!(
                    "{:.2}s · {}",
                    marker.frame.saturating_sub(take.start) as f64 / 48000.,
                    marker.name
                ));
            }
            if crop_changed {
                self.workflow_ui.preview = None;
            }
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_id_salt("session-monitor")
                    .selected_text(
                        self.devices
                            .outputs
                            .iter()
                            .find(|d| d.id == self.workflow_ui.preview_device)
                            .map_or("Choose monitoring device", |d| d.name.as_str()),
                    )
                    .show_ui(ui, |ui| {
                        for d in &self.devices.outputs {
                            if ui
                                .selectable_value(
                                    &mut self.workflow_ui.preview_device,
                                    d.id.clone(),
                                    &d.name,
                                )
                                .changed()
                            {
                                self.workflow_ui.preview = None;
                            }
                        }
                    });
                if ui.button("Preview selected tracks").clicked() {
                    self.workflow_ui.preview = None;
                    match self.workflow_ui.media.preview(
                        &take.id,
                        &self
                            .workflow_ui
                            .preview_tracks
                            .iter()
                            .copied()
                            .collect::<Vec<_>>(),
                        (self.workflow_ui.crop[0] * 48000.).round() as u64,
                        (self.workflow_ui.crop[1] * 48000.).round() as u64,
                        self.workflow_ui.preview_device.clone(),
                    ) {
                        Ok(p) => self.workflow_ui.preview = Some(p),
                        Err(e) => self.notify_error(format!("{e:#}")),
                    }
                }
                if ui.button("Stop preview").clicked() {
                    self.workflow_ui.preview = None;
                }
                if ui.button("Export selected interval").clicked() {
                    if let Some(parent) = rfd::FileDialog::new()
                        .set_title("Choose folder to contain the exported take")
                        .pick_folder()
                    {
                        let destination = parent.join(format!("{}-export-{}", take.id, id()));
                        let context = piratewave_core::automation::Context {
                            generation: self.controller.generation,
                            ..Default::default()
                        };
                        if self
                            .workflow_ui
                            .media
                            .tasks
                            .try_send(session_io::Task::Export {
                                context,
                                id: take.id.clone(),
                                destination,
                                start: (self.workflow_ui.crop[0] * 48000.).round() as u64,
                                end: (self.workflow_ui.crop[1] * 48000.).round() as u64,
                            })
                            .is_err()
                        {
                            self.notify_error("Recording file queue is full");
                        }
                    }
                }
            });
        }
        if let Some(take) = self.workflow_ui.delete_take.clone() {
            let mut confirm = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("delete-session-take")).show(ui.ctx(),|ui|{ui.heading("Delete this take?");ui.label("This removes it from this workspace. Media shared with another workspace will be kept.");confirm=ui.button("Delete take").clicked();cancel=ui.button("Cancel").clicked();});
            if confirm {
                self.workflow_ui.preview = None;
                let shared = self.workspaces.as_ref().is_some_and(|store| {
                    store
                        .entries()
                        .iter()
                        .filter(|e| e.id != store.active())
                        .any(|e| {
                            store.load(e.id).map_or(true, |p| {
                                p.workflows
                                    .sessions
                                    .iter()
                                    .any(|s| s.take_ids.contains(&take))
                            })
                        })
                });
                for session in &mut self.project.workflows.sessions {
                    session.take_ids.retain(|id| id != &take);
                }
                self.changed(false);
                if !shared {
                    let context = piratewave_core::automation::Context {
                        generation: self.controller.generation,
                        ..Default::default()
                    };
                    if self
                        .workflow_ui
                        .media
                        .tasks
                        .try_send(session_io::Task::Delete {
                            context,
                            id: take.clone(),
                        })
                        .is_err()
                    {
                        self.notify_error("File queue is full; media was retained");
                    }
                }
                self.workflow_ui.takes.retain(|t| t.id != take);
                self.workflow_ui.selected_take = None;
                self.workflow_ui.selected_media = None;
            }
            if confirm || cancel {
                self.workflow_ui.delete_take = None;
            }
        }
    }
}
impl PirateWaveApp {
    pub(super) fn workflow_feedback(&self) -> HashMap<(String, String), (String, Option<bool>)> {
        let mut values = HashMap::new();
        for scene in &self.project.workflows.scenes {
            let selected = self.status.scene.filter(|(id, _, _)| *id == scene.id);
            let state = selected.map_or_else(
                || "Ready".into(),
                |(_, progress, modified)| {
                    format!(
                        "{}{}",
                        if progress < 1. {
                            format!("Transition {:.0}%", progress * 100.)
                        } else {
                            "Selected".into()
                        },
                        if modified { " · Modified" } else { "" }
                    )
                },
            );
            values.insert(
                ("scene".into(), scene.name.clone()),
                (state, Some(selected.is_some())),
            );
        }
        for session in &self.project.workflows.sessions {
            let live = self
                .workflow_ui
                .live
                .as_ref()
                .filter(|(id, _)| *id == session.id);
            let started = live.is_some_and(|(_, p)| {
                p.start.load(std::sync::atomic::Ordering::Acquire) != u64::MAX
                    && !p.stopped.load(std::sync::atomic::Ordering::Acquire)
            });
            let finalizing = self
                .workflow_ui
                .recording_request
                .is_some_and(|(id, _)| id == session.id)
                && self.workflow_ui.media.busy();
            let state = if started {
                "Recording".into()
            } else if live.is_some() {
                "Preparing".into()
            } else if finalizing {
                "Preparing / finalizing".into()
            } else if let Some(outcome) = self.workflow_ui.session_outcomes.get(&session.id) {
                outcome.clone()
            } else if self.workflow_ui.history.contains(&session.id) {
                "History armed".into()
            } else {
                "Ready".into()
            };
            values.insert(
                ("session".into(), session.name.clone()),
                (state, Some(started)),
            );
        }
        let armed = self
            .workflow_ui
            .automation_status
            .as_ref()
            .is_some_and(|s| s.armed);
        values.insert(
            ("automation".into(), String::new()),
            (if armed { "Armed" } else { "Disarmed" }.into(), Some(armed)),
        );
        for rule in &self.project.workflows.rules {
            let status = self
                .workflow_ui
                .automation_status
                .as_ref()
                .and_then(|s| s.rules.get(&rule.id));
            let state = if let Some(error) = status.and_then(|s| s.error.as_ref()) {
                format!("Error: {error}")
            } else if status.is_some_and(|s| s.running) {
                "Running".into()
            } else if rule.enabled {
                "Enabled".into()
            } else {
                "Disabled".into()
            };
            values.insert(
                ("automation_rule".into(), rule.name.clone()),
                (state, Some(rule.enabled)),
            );
        }
        values
    }
}

enum BindingChoice {
    None,
    Record,
    Clear,
}
fn binding_control(ui: &mut egui::Ui, value: &str) -> BindingChoice {
    let mut action = BindingChoice::None;
    ui.horizontal_wrapped(|ui| {
        ui.label(if value.is_empty() {
            "No keyboard shortcut"
        } else {
            value
        });
        if ui.small_button("Record shortcut").clicked() {
            action = BindingChoice::Record;
        }
        if !value.is_empty() && ui.small_button("Clear shortcut").clicked() {
            action = BindingChoice::Clear;
        }
    });
    action
}

impl PirateWaveApp {
    pub(super) fn workflow_recovery_dialog(&mut self, ctx: &egui::Context) {
        if !self.workflow_ui.recovery_prompt || self.smoke {
            return;
        }
        let mut open = false;
        let mut later = false;
        egui::Modal::new(egui::Id::new("interrupted-session-takes")).show(ctx,|ui|{ui.set_width(440.);ui.heading("An interrupted recording was found");ui.label("Your available audio was kept. Open Recording to recover synchronized tracks, or delete the take after reviewing it.");ui.horizontal_wrapped(|ui|{open=ui.button("Open recording recovery").clicked();later=ui.button("Later").clicked();});});
        if open {
            self.view = View::Sessions;
        }
        if open || later {
            self.workflow_ui.recovery_prompt = false;
        }
    }
}

fn add_template(
    project: &mut Project,
    registry: &EffectRegistry,
    template: &str,
    trigger: piratewave_core::workflows::Trigger,
) -> Result<(), String> {
    use piratewave_core::workflows::{Condition, Control, Rule, Step, Trigger};
    fn rule(project: &Project, name: &str, trigger: Trigger, actions: Vec<Step>) -> Rule {
        Rule {
            id: id(),
            name: unique(name, project.workflows.rules.iter().map(|r| r.name.clone())),
            enabled: true,
            trigger,
            match_all: true,
            conditions: vec![],
            actions,
            cooldown_ms: 1000,
            keybind: String::new(),
        }
    }
    if template == "Speech / silence scene" {
        let voice = project
            .nodes
            .iter()
            .find(|n| {
                matches!(
                    n.kind,
                    NodeKind::Input {
                        loopback: false,
                        ..
                    }
                )
            })
            .ok_or("Add a microphone input before using this template")?
            .id;
        let music = project
            .nodes
            .iter()
            .find(|n| matches!(n.kind, NodeKind::ApplicationCapture { .. }))
            .or_else(|| {
                project
                    .nodes
                    .iter()
                    .find(|n| matches!(n.kind, NodeKind::Soundboard))
            })
            .ok_or("Add a music source before using this template")?;
        let music_id = music.id;
        let gain = music.gain_db;
        let mut normal = Scene::capture(
            project,
            registry,
            unique(
                "Fair Winds",
                project.workflows.scenes.iter().map(|s| s.name.clone()),
            ),
        );
        for setting in &mut normal.settings {
            setting.included =
                setting.node == music_id && matches!(setting.value, Control::Gain { .. });
        }
        let mut quiet = normal.clone();
        quiet.id = id();
        quiet.name = unique(
            "Voice at the Helm",
            project.workflows.scenes.iter().map(|s| s.name.clone()),
        );
        for setting in quiet.settings.iter_mut().filter(|s| s.included) {
            setting.value = Control::Gain {
                value: (gain - 18.).max(-60.),
            };
        }
        let mut speech = rule(
            project,
            "Lower music while speaking",
            Trigger::Level {
                node: voice,
                above: true,
                threshold_db: -35.,
                hysteresis_db: 6.,
                hold_ms: 150,
            },
            vec![Step::Scene { id: quiet.id }],
        );
        speech.conditions.push(Condition::Audio { running: true });
        project.workflows.rules.push(speech);
        let mut silence = rule(
            project,
            "Restore music after speech",
            Trigger::Level {
                node: voice,
                above: false,
                threshold_db: -41.,
                hysteresis_db: 6.,
                hold_ms: 1000,
            },
            vec![Step::Scene { id: normal.id }],
        );
        silence.conditions.push(Condition::Audio { running: true });
        project.workflows.rules.push(silence);
        project.workflows.scenes.extend([normal, quiet]);
    } else if template == "Device-loss scene" {
        let fallback = Scene::capture(
            project,
            registry,
            unique(
                "Safe Harbor",
                project.workflows.scenes.iter().map(|s| s.name.clone()),
            ),
        );
        let created = rule(
            project,
            "Recall fallback when a device disappears",
            trigger,
            vec![
                Step::Scene { id: fallback.id },
                Step::Notify {
                    text: "A device became unavailable. Safe Harbor was recalled.".into(),
                },
            ],
        );
        project.workflows.scenes.push(fallback);
        project.workflows.rules.push(created);
    } else {
        let created = rule(
            project,
            if template == "Recording notification" {
                "Recording finished notification"
            } else {
                "Manual rule"
            },
            trigger,
            vec![Step::Notify {
                text: "Workflow completed".into(),
            }],
        );
        project.workflows.rules.push(created);
    }
    Ok(())
}
impl PirateWaveApp {
    pub(super) fn workflow_smoke_fixture(&mut self, stage: usize) {
        self.project = super::cookbook::workflow_recipe(&self.registry, "Captain’s Log");
        self.workflow_ui.loaded = true;
        self.workflow_ui.recovery_prompt = false;
        self.workflow_ui.take_view = stage == 98;
        self.project.workflows.sessions[0].tracks.truncate(2);
        let session = self.project.workflows.sessions[0].clone();
        self.workflow_ui.session = Some(session.id);
        self.view = match stage {
            96 => View::Scenes,
            97 | 98 => View::Sessions,
            100 => View::Mixer,
            _ => View::Automation,
        };
        self.status.scene = Some((self.project.workflows.scenes[0].id, 0.43, false));
        if stage == 98 {
            let mut take = session_io::new_take(
                1,
                session.id,
                "Captain’s Log · rehearsal".into(),
                session.tracks,
            );
            take.id = "take-9000".into();
            take.complete = true;
            take.start = 48000;
            take.end = 192000;
            for (lane, track) in take.tracks.iter_mut().enumerate() {
                track.waveform = (0..256)
                    .map(|i| {
                        let peak =
                            ((i as f32 * 0.17).sin().abs() * 0.6 + 0.06) * (1. - lane as f32 * 0.2);
                        [-peak, peak]
                    })
                    .collect();
            }
            take.markers.push(session_io::Marker {
                frame: 96000,
                name: "Opening line".into(),
            });
            self.project.workflows.sessions[0].take_ids = vec![take.id.clone()];
            self.workflow_ui.selected_take = Some(take.id.clone());
            self.workflow_ui.selected_media = Some(take.clone());
            self.workflow_ui.preview_tracks = take.tracks.iter().map(|t| t.track.id).collect();
            self.workflow_ui.takes = vec![take];
            self.workflow_ui.crop = [0.25, 2.65];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{app, at, click, labels, render};
    use super::*;
    #[test]
    fn workflow_navigation_is_presentation_only_and_scene_creation_is_recallable() {
        let mut app = app();
        let ctx = egui::Context::default();
        let original = serde_json::to_string(&app.project).unwrap();
        let (engine, rx) = Engine::test_channel();
        app.engine = engine;
        for view in [View::Scenes, View::Sessions, View::Automation] {
            app.view = view;
            let _ = render(&ctx, &mut app, vec![]);
            assert_eq!(serde_json::to_string(&app.project).unwrap(), original);
            assert!(!app.dirty);
            assert!(rx.try_recv().is_err());
        }
        app.view = View::Scenes;
        let out = render(&ctx, &mut app, vec![]);
        click(
            &ctx,
            &mut app,
            at(&out, "Create scene from current mix"),
            egui::PointerButton::Primary,
        );
        assert_eq!(app.project.workflows.scenes.len(), 1);
        let scene = app.project.workflows.scenes[0].id;
        app.project.nodes[0].gain_db = -12.;
        app.recall_scene(scene);
        assert_eq!(app.project.nodes[0].gain_db, 0.);
        let out = render(&ctx, &mut app, vec![]);
        assert!(labels(&out)
            .iter()
            .any(|(text, _)| text == "Record shortcut"));
    }
    #[test]
    fn templates_resolve_with_unique_names_and_keep_master_disarmed() {
        let mut app = app();
        let mut p = app.project.clone();
        for _ in 0..2 {
            add_template(
                &mut p,
                &app.registry,
                "Speech / silence scene",
                piratewave_core::workflows::Trigger::Manual,
            )
            .unwrap();
        }
        assert_eq!(p.workflows.scenes.len(), 4);
        assert_eq!(p.workflows.rules.len(), 4);
        piratewave_core::workflows::validate(&p).unwrap();
        for rule in &p.workflows.rules {
            rule.validate_targets(&p, &app.registry).unwrap();
        }
        app.project = p;
        assert!(!app
            .workflow_ui
            .automation
            .enabled
            .load(std::sync::atomic::Ordering::Acquire));
    }
    #[test]
    fn controller_knob_takes_ownership_at_current_interpolated_gain() {
        let mut app = app();
        app.controller.generation = 77;
        app.project.nodes[0].gain_db = -20.;
        let node = app.project.nodes[0].id;
        app.status.scene_controls = vec![piratewave_core::workflows::SceneSetting {
            node,
            included: true,
            value: piratewave_core::workflows::Control::Gain { value: -5. },
        }];
        let selector = piratewave_core::control::catalog(&app.project, &app.registry, false)
            .into_iter()
            .find(|e| {
                e.selector.category == "mixer"
                    && e.selector.action == "volume"
                    && e.selector.node == app.project.nodes[0].name
            })
            .unwrap()
            .selector;
        app.controller_request(&crate::control::Request {
            id: 1,
            op: "adjust".into(),
            version: 2,
            generation: 77,
            client: String::new(),
            selector: Some(selector),
            ticks: 1,
            sensitivity: 1.,
        })
        .unwrap();
        assert_eq!(app.project.nodes[0].gain_db, -4.);
    }
    #[test]
    fn guided_start_wait_stop_finishes_a_real_aligned_take_without_keyboard_binding() {
        use piratewave_core::{
            project::OutputPort,
            workflows::{Condition, Rule, Step, Trigger},
        };
        let root = std::env::temp_dir().join(format!("piratewave-guided-take-{}", id()));
        let mut app = app();
        app.workflow_ui.media = session_io::Service::new(root.clone());
        app.controller.generation = 77;
        app.project.nodes = vec![Node::new(1, "Source", NodeKind::Soundboard, [0.; 2])];
        app.project.connections.clear();
        app.project.workflows.sessions = vec![SessionConfig {
            id: 10,
            name: "Test session".into(),
            history_seconds: 1.,
            tracks: [CaptureStage::Raw, CaptureStage::Processed]
                .into_iter()
                .enumerate()
                .map(|(i, stage)| Track {
                    id: 20 + i as u64,
                    name: format!("Track {i}"),
                    point: CapturePoint {
                        node: 1,
                        port: OutputPort::Passthrough,
                        stage,
                    },
                    armed: true,
                })
                .collect(),
            take_ids: vec![],
            keybinds: Default::default(),
        }];
        app.project.workflows.rules = vec![Rule {
            id: 30,
            name: "Record briefly".into(),
            enabled: true,
            trigger: Trigger::Manual,
            match_all: true,
            conditions: vec![Condition::Audio { running: true }],
            actions: vec![
                Step::Session {
                    id: 10,
                    action: SessionAction::Start,
                },
                Step::Wait { milliseconds: 50 },
                Step::Session {
                    id: 10,
                    action: SessionAction::Capture,
                },
                Step::Session {
                    id: 10,
                    action: SessionAction::Marker,
                },
                Step::Session {
                    id: 10,
                    action: SessionAction::Stop,
                },
                Step::Notify {
                    text: "Guided take ready".into(),
                },
            ],
            cooldown_ms: 1000,
            keybind: String::new(),
        }];
        app.workflow_ui.snapshot_dirty = true;
        app.engine.send(Command::Generation(77)).unwrap();
        app.engine
            .send(Command::Start(app.project.clone()))
            .unwrap();
        app.status = app.engine.snapshot().unwrap();
        assert!(app.status.running);
        let (reply, rx) = mpsc::sync_channel(1);
        app.engine
            .send(Command::ArmSession {
                id: 10,
                armed: true,
                reply,
            })
            .unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
        app.workflow_ui.history.insert(10);
        app.poll_workflows();
        app.arm_automation(true);
        let deadline = Instant::now() + Duration::from_secs(8);
        while !app
            .workflow_ui
            .automation
            .enabled
            .load(std::sync::atomic::Ordering::Acquire)
        {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        app.run_rule(30);
        while app.notice != "Guided take ready" {
            assert!(
                Instant::now() < deadline,
                "Guided recording stalled: {}",
                app.notice
            );
            app.poll_workflows();
            while let Some(status) = app.engine.statuses.pop() {
                app.status = status;
            }
            thread::sleep(Duration::from_millis(3));
        }
        assert_eq!(app.project.workflows.sessions[0].take_ids.len(), 2);
        let take = app.project.workflows.sessions[0].take_ids[0].clone();
        let saved = app.workflow_ui.media.load(&take).unwrap();
        assert!(saved.complete);
        assert_eq!(saved.markers.len(), 1);
        assert!(saved.end > saved.start);
        assert_eq!(saved.tracks.len(), 2);
        for track in saved.tracks {
            let wav = hound::WavReader::open(root.join(&take).join(track.file)).unwrap();
            assert_eq!(wav.duration() as u64, saved.end - saved.start);
        }
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
}
