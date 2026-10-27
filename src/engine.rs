use crate::{
    assets::Clip,
    audio_io::{self, Counters, InputStream, OutputStream, TARGET_FRAMES},
};
use crossbeam_queue::ArrayQueue;
use piratewave_core::{
    effects::EffectRegistry,
    graph::Graph,
    project::{NodeKind, Project},
    Frame, BLOCK_SIZE, SAMPLE_RATE,
};
fn validate_hardware_routes(project: &Project) -> Result<(), String> {
    let default = audio_io::default_output_identity();
    piratewave_core::graph::validate_loopback_routes(project, |id| {
        id.clone().unwrap_or_else(|| default.clone())
    })
}
use std::{
    collections::HashMap,
    sync::{atomic::Ordering, mpsc, Arc},
    thread,
    time::{Duration, Instant},
};

pub enum Command {
    /// UI-only lease: no project update or device reconfiguration.
    MonitorEffect(Option<(u64, u64, u64)>),
    Start(Project),
    StartIfStopped(Project),
    Toggle(Project),
    Reconfigure(Project),
    Stop,
    Update(Project),
    Play {
        clip: Clip,
        volume: f32,
    },
    CaptureRecording {
        node: u64,
        slot: u64,
        token: u64,
        jobs: mpsc::SyncSender<crate::recording_io::Job>,
        events: mpsc::SyncSender<crate::recording_io::Event>,
    },
    PlayRecording {
        node: u64,
        slot: u64,
        clip: Clip,
        start: f32,
        end: f32,
        preview: bool,
        send: bool,
    },
    StopRecordingPlayback {
        node: u64,
        slot: Option<u64>,
    },
    StopSounds,
    Note {
        id: u64,
        note: u8,
        velocity: f32,
        on: bool,
    },
    AllNotesOff(Option<u64>),
    RestartVoice,
    Shutdown,
    Inspect(mpsc::SyncSender<Status>),
}
impl Command {
    pub fn for_app_action(action: piratewave_core::shortcuts::AppAction, project: Project) -> Self {
        use piratewave_core::shortcuts::AppAction;
        match action {
            AppAction::StopSounds => Self::StopSounds,
            AppAction::StartAudio => Self::StartIfStopped(project),
            AppAction::StopAudio => Self::Stop,
            AppAction::ToggleAudio => Self::Toggle(project),
        }
    }
}
#[derive(Clone, Default)]
pub struct Status {
    pub devices: Option<Arc<audio_io::Devices>>,
    pub running: bool,
    pub peaks: HashMap<u64, f32>,
    pub ducking: HashMap<u64, piratewave_core::ducking::DuckingStatus>,
    pub input_frames: u64,
    pub output_frames: u64,
    pub underflows: u64,
    pub input_underflows: u64,
    pub overflows: u64,
    pub render_load: f32,
    pub message: String,
    pub outputs: usize,
    pub output_rates: Vec<u32>,
    pub plugins: HashMap<u64, String>,
    pub voice_changers: HashMap<u64, String>,
    pub recording_seconds: HashMap<u64, f32>,
    pub recording_buffers: usize,
    pub recording_bytes: usize,
}
pub struct Engine {
    tx: mpsc::SyncSender<Command>,
    pub statuses: Arc<ArrayQueue<Status>>,
    pub effect_audio: Arc<ArrayQueue<piratewave_core::graph::EffectMonitor>>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Engine {
    #[cfg(test)]
    pub(crate) fn test_channel() -> (Self, mpsc::Receiver<Command>) {
        let (tx, rx) = mpsc::sync_channel(64);
        (
            Self {
                tx,
                statuses: Arc::new(ArrayQueue::new(8)),
                effect_audio: Arc::new(ArrayQueue::new(32)),
                thread: None,
            },
            rx,
        )
    }
    pub fn new() -> Self {
        let (tx, rx) = mpsc::sync_channel(64);
        let statuses = Arc::new(ArrayQueue::new(8));
        let dest = statuses.clone();
        let effect_audio = Arc::new(ArrayQueue::new(32));
        let monitor_dest = effect_audio.clone();
        let thread = thread::Builder::new()
            .name("piratewave-audio-engine".into())
            .spawn(move || run(rx, dest, monitor_dest))
            .expect("Unable to start audio worker");
        Self {
            tx,
            statuses,
            effect_audio,
            thread: Some(thread),
        }
    }
    pub fn send(&self, command: Command) -> Result<(), String> {
        if matches!(
            &command,
            Command::Start(_) | Command::StartIfStopped(_) | Command::Toggle(_)
        ) {
            crate::training::audio_requested();
        }
        self.tx.try_send(command).map_err(|error| {
            crate::logging::error(format_args!(
                "Audio command queue rejected a command: {error}"
            ));
            "Audio engine is busy; please try again".into()
        })
    }
    /// Blocking control-plane barrier for diagnostics; never call on an audio callback.
    pub fn snapshot(&self) -> Result<Status, String> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.send(Command::Inspect(tx))?;
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
struct Voice {
    clip: Clip,
    position: usize,
    volume: f32,
}
struct RecordingVoice {
    effects: piratewave_core::recording_effects::RecordingEffects,
    node: u64,
    slot: u64,
    clip: Clip,
    position: usize,
    end: usize,
    preview: bool,
    send: bool,
}
struct Listener {
    stream: OutputStream,
}

/// Control-side route compilation; no graph searches in source sample loops.
#[derive(Default)]
struct RenderMappings {
    sources: Vec<(Option<usize>, Option<usize>)>,
    soundboards: Vec<usize>,
    outputs: HashMap<u64, usize>,
}
impl RenderMappings {
    fn new(
        graph: &Graph,
        blocks: &[(u64, [Frame; BLOCK_SIZE])],
        inputs: impl Iterator<Item = u64>,
        application_inputs: impl Iterator<Item = u64>,
    ) -> Self {
        let inputs: HashMap<_, _> = inputs.enumerate().map(|(i, id)| (id, i)).collect();
        let applications: HashMap<_, _> = application_inputs
            .enumerate()
            .map(|(i, id)| (id, i))
            .collect();
        Self {
            sources: blocks
                .iter()
                .map(|(id, _)| (inputs.get(id).copied(), applications.get(id).copied()))
                .collect(),
            soundboards: blocks
                .iter()
                .enumerate()
                .filter_map(|(i, (id, _))| {
                    graph
                        .nodes
                        .iter()
                        .any(|n| {
                            n.config.id == *id && matches!(n.config.kind, NodeKind::Soundboard)
                        })
                        .then_some(i)
                })
                .collect(),
            outputs: graph
                .nodes
                .iter()
                .enumerate()
                .map(|(i, n)| (n.config.id, i))
                .collect(),
        }
    }
}

struct Session {
    _audio_thread: crate::audio_scheduling::AudioThread,
    graph: Graph,
    plugins: crate::plugins::Rack,
    voice_changer: crate::voice::Rack,
    inputs: Vec<(u64, InputStream)>,
    application_inputs: Vec<(u64, crate::application_capture::ApplicationStream)>,
    application_errors: Arc<ArrayQueue<String>>,
    outputs: Vec<OutputStream>,
    listeners: Vec<Listener>,
    // Remember attempted destinations so unrelated fader changes do not retry failed devices.
    listen_routes: Vec<(u64, String)>,
    blocks: Vec<(u64, [Frame; BLOCK_SIZE])>,
    render_mappings: RenderMappings,
    counters: Arc<Counters>,
    project: Project,
    last_recovery: Instant,
    device_watcher: audio_io::DeviceWatcher,
    last_default_output: String,
    last_effect_recovery: Instant,
    voices: Vec<Voice>,
    recorder: piratewave_core::recording::Recorder,
    recording_voices: Vec<RecordingVoice>,
    message: String,
    load: f32,
    last_render: Instant,
    software_deadline: Option<Instant>,
}
impl Session {
    fn new(project: &Project) -> anyhow::Result<Self> {
        crate::logging::info(format_args!(
            "Starting audio session: {} nodes, {} routes",
            project.nodes.len(),
            project.connections.len()
        ));
        validate_hardware_routes(project).map_err(anyhow::Error::msg)?;
        let graph = Graph::new(project, &EffectRegistry::default()).map_err(anyhow::Error::msg)?;
        let mut plugins = crate::plugins::Rack::default();
        plugins.sync(project);
        let counters = Arc::new(Counters::default());
        let mut inputs = vec![];
        let mut application_inputs = vec![];
        let application_errors = Arc::new(ArrayQueue::new(32));
        let mut outputs = vec![];
        let mut warnings = std::mem::take(&mut plugins.warnings);
        for node in &project.nodes {
            match &node.kind {
                NodeKind::Output { device } => {
                    if device.as_ref().is_some_and(|s| s.is_empty()) {
                        continue;
                    }
                    match audio_io::open_output(
                        node.id,
                        device,
                        counters.clone(),
                        Arc::new(ArrayQueue::new(4)),
                    ) {
                        Ok(stream) => outputs.push(stream),
                        Err(error) => warnings.push(format!("{}: {error:#}", node.name)),
                    }
                }
                NodeKind::ApplicationCapture { target } => {
                    if let Some(target) = target {
                        match crate::application_capture::open(
                            target,
                            counters.clone(),
                            application_errors.clone(),
                        ) {
                            Ok(stream) => application_inputs.push((node.id, stream)),
                            Err(error) => warnings.push(format!("{}: {error:#}", node.name)),
                        }
                    } else {
                        warnings.push(format!("{}: Choose an application in Mixer", node.name));
                    }
                }
                NodeKind::Input { device, loopback } => {
                    match audio_io::open_input(
                        device,
                        *loopback,
                        counters.clone(),
                        Arc::new(ArrayQueue::new(4)),
                    ) {
                        Ok(stream) => inputs.push((node.id, stream)),
                        Err(error) => warnings.push(format!("{}: {error:#}", node.name)),
                    }
                }
                _ => {}
            }
        }
        let blocks = project
            .nodes
            .iter()
            .filter(|n| n.kind.is_source())
            .map(|n| (n.id, [[0.0; 2]; BLOCK_SIZE]))
            .collect();
        for warning in &warnings {
            crate::logging::warn(format_args!("Audio setup: {warning}"));
        }
        let mut session = Self {
            _audio_thread: crate::audio_scheduling::AudioThread::register(),
            voice_changer: crate::voice::Rack::new(project),
            graph,
            plugins,
            inputs,
            application_inputs,
            application_errors,
            outputs,
            listeners: vec![],
            listen_routes: vec![],
            blocks,
            render_mappings: RenderMappings::default(),
            counters,
            project: project.clone(),
            last_recovery: Instant::now(),
            device_watcher: audio_io::DeviceWatcher::new()?,
            last_default_output: audio_io::default_output_identity(),
            last_effect_recovery: Instant::now(),
            voices: vec![],
            recorder: Default::default(),
            recording_voices: vec![],
            message: if warnings.is_empty() {
                "Audio engine running".into()
            } else {
                warnings.join("; ")
            },
            load: 0.0,
            last_render: Instant::now(),
            software_deadline: None,
        };
        session.refresh_render_mappings();
        session.recorder.sync(project);
        session.sync_listeners(project);
        Ok(session)
    }
    fn refresh_render_mappings(&mut self) {
        self.render_mappings = RenderMappings::new(
            &self.graph,
            &self.blocks,
            self.inputs.iter().map(|(id, _)| *id),
            self.application_inputs.iter().map(|(id, _)| *id),
        );
    }
    fn sync_recording_effects(
        &mut self,
        project: &Project,
        registry: &EffectRegistry,
    ) -> Result<(), String> {
        self.recording_voices.retain(|v| {
            project.nodes.iter().any(|n| {
                n.id == v.node
                    && matches!(n.kind, NodeKind::Recording)
                    && n.recording.slots.iter().any(|s| s.id == v.slot)
            })
        });
        for voice in &mut self.recording_voices {
            let node = project.nodes.iter().find(|n| n.id == voice.node).unwrap();
            voice
                .effects
                .sync(&node.effects, node.effects_disabled, registry)?;
        }
        Ok(())
    }
    /// Add/remove only secondary streams; the primary clock and voices keep running.
    fn sync_listeners(&mut self, project: &Project) {
        let default = audio_io::default_output_identity();
        self.sync_listeners_available(project, &default, None);
    }
    fn sync_listeners_available(
        &mut self,
        project: &Project,
        default: &str,
        available: Option<&audio_io::DeviceSnapshot>,
    ) {
        let desired = listener_routes(project, default, |id| {
            self.outputs.iter().any(|o| o.node_id == id)
        });
        if desired == self.listen_routes {
            return;
        }
        let previous = std::mem::replace(&mut self.listen_routes, desired.clone());
        self.listeners.retain(|l| {
            desired.iter().any(|(id, endpoint)| {
                *id == l.stream.node_id && previous.contains(&(*id, endpoint.clone()))
            })
        });
        for (id, endpoint) in desired {
            if previous.contains(&(id, endpoint.clone())) {
                continue;
            }
            if available.is_some_and(|devices| !devices.available(&Some(endpoint.clone()), true)) {
                continue;
            }
            let errors = Arc::new(ArrayQueue::new(4));
            match audio_io::open_output(id, &Some(endpoint), self.counters.clone(), errors.clone()) {
                Ok(stream) => self.listeners.push(Listener { stream }),
                Err(error) => self.message = format!("Listen output for bus {id} unavailable: {error:#}. Primary output continues; retrying automatically."),
            }
        }
    }
    /// Each endpoint retries independently. Never rebuild the graph or reset playing clips.
    fn recover_devices(&mut self) {
        if self.last_recovery.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.last_recovery = Instant::now();
        self.plugins.recover();
        if self.last_effect_recovery.elapsed() >= Duration::from_secs(2) {
            self.graph.recover_effects();
            for voice in &mut self.recording_voices {
                voice.effects.recover();
            }
            self.last_effect_recovery = Instant::now();
        }
        self.inputs.retain_mut(|(id, stream)| {
            if let Some(error) = stream.health.failure() {
                crate::logging::warn(format_args!("Input {id} failed: {error}"));
                self.message = format!("Input {id} disconnected: {error}. Retrying automatically.");
                self.voice_changer.source_discontinuity(*id);
                false
            } else {
                true
            }
        });
        let available = self.device_watcher.latest().cloned();
        self.outputs.retain_mut(|stream| {
            let endpoint_changed = self
                .project
                .nodes
                .iter()
                .find(|n| n.id == stream.node_id)
                .is_some_and(|node| match &node.kind {
                    NodeKind::Output { device } => available.as_ref().is_some_and(|snapshot| {
                        snapshot.output_needs_reopen(device, &self.last_default_output)
                    }),
                    _ => false,
                });
            if let Some(error) = stream.health.failure().or_else(|| {
                endpoint_changed.then(|| "Output endpoint changed or became unavailable".into())
            }) {
                crate::logging::warn(format_args!("Output {} failed: {error}", stream.node_id));
                self.message = format!(
                    "Output {} disconnected: {error}. Retrying automatically.",
                    stream.node_id
                );
                false
            } else {
                true
            }
        });
        self.listeners.retain_mut(|listener| {
            let available = self
                .project
                .nodes
                .iter()
                .find(|node| node.id == listener.stream.node_id)
                .is_none_or(|node| {
                    available
                        .as_ref()
                        .is_none_or(|snapshot| snapshot.available(&node.listen_device, true))
                });
            available && listener.stream.health.failure().is_none()
        });
        // Discovery is asynchronous: absent devices must not trigger driver probes
        // on this worker. Healthy inputs and outputs continue while discovery runs.
        let project = self.project.clone();
        for node in &project.nodes {
            let errors = Arc::new(ArrayQueue::new(4));
            match &node.kind {
                NodeKind::Input { device, loopback }
                    if !self.inputs.iter().any(|(id, _)| *id == node.id)
                        && available
                            .as_ref()
                            .is_some_and(|d| d.available(device, *loopback)) =>
                {
                    if let Ok(stream) =
                        audio_io::open_input(device, *loopback, self.counters.clone(), errors)
                    {
                        self.inputs.push((node.id, stream));
                        self.voice_changer.source_discontinuity(node.id);
                        crate::logging::info(format_args!("Device reconnected: {}", node.name));
                        self.message = format!("{} reconnected; audio restored", node.name);
                    }
                }
                NodeKind::Output { device }
                    if !device.as_ref().is_some_and(|s| s.is_empty())
                        && !self.outputs.iter().any(|s| s.node_id == node.id)
                        && available
                            .as_ref()
                            .is_some_and(|d| d.available(device, true)) =>
                {
                    if let Ok(stream) =
                        audio_io::open_output(node.id, device, self.counters.clone(), errors)
                    {
                        self.outputs.push(stream);
                        crate::logging::info(format_args!("Device reconnected: {}", node.name));
                        self.message = format!("{} reconnected; audio restored", node.name);
                    }
                }
                NodeKind::ApplicationCapture {
                    target: Some(target),
                } if !self.application_inputs.iter().any(|(id, _)| *id == node.id) => {
                    if let Ok(stream) = crate::application_capture::open(
                        target,
                        self.counters.clone(),
                        self.application_errors.clone(),
                    ) {
                        self.application_inputs.push((node.id, stream));
                    }
                }
                _ => {}
            }
        }
        self.listen_routes
            .retain(|(id, _)| self.listeners.iter().any(|l| l.stream.node_id == *id));
        if let Some(available) = &available {
            self.last_default_output
                .clone_from(&available.default_output);
            self.sync_listeners_available(&project, &available.default_output, Some(available));
        }
        self.refresh_render_mappings();
        // Measure the retry interval from completion, not start. A slow recovery
        // must not immediately trigger another recovery and starve every render.
        self.last_recovery = Instant::now();
    }
    fn render_ready(&mut self) -> bool {
        if self.outputs.is_empty() {
            software_clock_due(&mut self.software_deadline, Instant::now())
        } else {
            self.software_deadline = None;
            self.outputs.iter().any(|o| o.queue.len() < TARGET_FRAMES)
        }
    }
    fn render(&mut self) {
        self.last_render = Instant::now();
        let start = Instant::now();
        while let Some(message) = self.application_errors.pop() {
            self.message = message;
        }
        for ((id, frames), &(input, application)) in
            self.blocks.iter_mut().zip(&self.render_mappings.sources)
        {
            if let Some(index) = input {
                let input = &mut self.inputs[index].1;
                let previous = input.resampler.underruns();
                for frame in frames.iter_mut() {
                    *frame = input.resampler.next_frame().unwrap_or([0.0; 2]);
                }
                if input.resampler.underruns() != previous {
                    self.voice_changer.source_discontinuity(*id);
                }
            } else if let Some(index) = application {
                let input = &mut self.application_inputs[index].1;
                let previous = input.resampler.underruns();
                for frame in frames.iter_mut() {
                    *frame = input.resampler.next_frame().unwrap_or([0.0; 2]);
                }
                if input.resampler.underruns() != previous {
                    self.voice_changer.source_discontinuity(*id);
                }
            } else {
                frames.fill([0.0; 2]);
            }
        }
        for voice in &mut self.voices {
            for offset in 0..BLOCK_SIZE {
                if let Some(frame) = voice.clip.frames.get(voice.position + offset) {
                    for &index in &self.render_mappings.soundboards {
                        let block = &mut self.blocks[index].1;
                        block[offset][0] += frame[0] * voice.volume;
                        block[offset][1] += frame[1] * voice.volume;
                    }
                }
            }
            voice.position += BLOCK_SIZE;
        }
        self.voices.retain(|v| v.position < v.clip.frames.len());
        self.graph.process_recording(
            &self.blocks,
            |node, frames| {
                if matches!(node.kind, NodeKind::VoiceChanger) {
                    self.voice_changer.process(node, frames);
                } else {
                    self.plugins.process(node, frames);
                }
                true
            },
            |node, _, preview, send| {
                for voice in self
                    .recording_voices
                    .iter_mut()
                    .filter(|v| v.node == node.id)
                {
                    let count = (voice.end - voice.position).min(BLOCK_SIZE);
                    let mut processed = [[0.; 2]; BLOCK_SIZE];
                    processed[..count].copy_from_slice(
                        &voice.clip.frames[voice.position..voice.position + count],
                    );
                    voice.position += count;
                    voice.effects.process(&mut processed);
                    for i in 0..count {
                        for c in 0..2 {
                            if voice.preview {
                                preview[i][c] += processed[i][c];
                            }
                            if voice.send {
                                send[i][c] += processed[i][c];
                            }
                        }
                    }
                }
            },
            |id, port, selected, frames| self.recorder.tap(id, port, selected, frames),
        );
        self.recorder.finish_block();
        self.recording_voices.retain(|v| v.position < v.end);
        if !self.plugins.warnings.is_empty() {
            self.message = std::mem::take(&mut self.plugins.warnings).join("; ");
        }
        for output in self
            .outputs
            .iter()
            .chain(self.listeners.iter().map(|l| &l.stream))
        {
            if let Some(&index) = self.render_mappings.outputs.get(&output.node_id) {
                for frame in &self.graph.nodes[index].frames {
                    if output.queue.push(*frame).is_err() {
                        self.counters.overflows.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
        let usage =
            start.elapsed().as_secs_f32() / (BLOCK_SIZE as f32 / SAMPLE_RATE as f32) * 100.0;
        self.load = self.load * 0.95 + usage * 0.05;
    }
    fn status(&mut self) -> Status {
        Status {
            devices: self.device_watcher.published_devices.clone(),
            running: true,
            ducking: self
                .graph
                .nodes
                .iter()
                .filter_map(|n| n.ducking.as_ref().map(|d| (n.config.id, d.status())))
                .collect(),
            recording_seconds: self
                .project
                .nodes
                .iter()
                .filter(|n| matches!(n.kind, NodeKind::Recording))
                .map(|n| (n.id, self.recorder.available(n.id)))
                .collect(),
            recording_buffers: self.recorder.histories.len(),
            recording_bytes: self
                .recorder
                .histories
                .values()
                .map(|h| piratewave_core::recording::history_bytes(h.capacity()))
                .sum(),
            plugins: self.plugins.statuses(),
            voice_changers: self.voice_changer.status().into_iter().collect(),
            peaks: self
                .graph
                .nodes
                .iter()
                .map(|n| (n.config.id, n.peak))
                .collect(),
            input_frames: self.counters.captured.load(Ordering::Relaxed),
            output_frames: self.counters.rendered.load(Ordering::Relaxed),
            underflows: self.counters.underflows.load(Ordering::Relaxed),
            input_underflows: self
                .inputs
                .iter()
                .map(|(_, i)| i.resampler.underruns())
                .sum::<u64>()
                + self
                    .application_inputs
                    .iter()
                    .map(|(_, i)| i.resampler.underruns())
                    .sum::<u64>(),
            overflows: self.counters.overflows.load(Ordering::Relaxed),
            render_load: self.load,
            message: if let Some(node) = self
                .graph
                .nodes
                .iter()
                .find(|n| n.extension.as_ref().is_some_and(|e| e.fault))
            {
                format!("{}: extension unavailable or faulted; outputs are silenced. Bypass uses only declared routes.",node.config.name)
            } else if let Some(node) = self.graph.nodes.iter().find(|n| n.has_effect_fault()) {
                format!(
                    "{}: a faulty effect was bypassed; retrying automatically. {}",
                    node.config.name, self.message
                )
            } else if let Some(node) = self
                .graph
                .nodes
                .iter()
                .find(|n| n.ducking.as_ref().is_some_and(|d| d.status().fallback))
            {
                format!("{}: Speech detector unavailable; using Level detection and retrying automatically",node.config.name)
            } else if let Some(voice) = self.recording_voices.iter().find(|v| v.effects.faulted()) {
                format!(
                    "Recording {}/{}: a faulty effect was bypassed; retrying automatically",
                    voice.node, voice.slot
                )
            } else {
                self.message.clone()
            },
            outputs: self.outputs.len() + self.listeners.len(),
            output_rates: self
                .outputs
                .iter()
                .chain(self.listeners.iter().map(|l| &l.stream))
                .map(|o| o.sample_rate)
                .collect(),
        }
    }
}
/// Listening mirrors an opened primary bus. Identical endpoints are emitted once,
/// including an explicit device that aliases the system default.
fn listener_routes(
    project: &Project,
    default: &str,
    primary_open: impl Fn(u64) -> bool,
) -> Vec<(u64, String)> {
    let identity = |device: &Option<String>| device.clone().unwrap_or_else(|| default.to_owned());
    project
        .nodes
        .iter()
        .filter_map(|node| {
            let NodeKind::Output { device } = &node.kind else {
                return None;
            };
            let destination = identity(&node.listen_device);
            (node.listen_enabled && primary_open(node.id) && destination != identity(device))
                .then_some((node.id, destination))
        })
        .collect()
}
fn publish(queue: &ArrayQueue<Status>, status: Status) {
    if queue.is_full() {
        queue.pop();
    }
    let _ = queue.push(status);
}
fn start_session(project: &Project, statuses: &ArrayQueue<Status>) -> Option<Session> {
    crate::training::audio_running(true);
    match Session::new(project) {
        Ok(mut session) => {
            publish(statuses, session.status());
            Some(session)
        }
        Err(error) => {
            crate::logging::error(format_args!("Audio session failed: {error:#}"));
            crate::training::audio_running(false);
            publish(
                statuses,
                Status {
                    message: format!("{error:#}"),
                    ..Default::default()
                },
            );
            None
        }
    }
}
/// Absolute deadlines keep recording-only graphs at 48 kHz despite sleep granularity.
fn software_clock_due(deadline: &mut Option<Instant>, now: Instant) -> bool {
    let next = deadline.get_or_insert(now);
    if now < *next {
        return false;
    }
    if now.duration_since(*next) > Duration::from_millis(100) {
        *next = now;
    }
    *next += Duration::from_secs_f64(BLOCK_SIZE as f64 / SAMPLE_RATE as f64);
    true
}
fn run(
    rx: mpsc::Receiver<Command>,
    statuses: Arc<ArrayQueue<Status>>,
    effect_audio: Arc<ArrayQueue<piratewave_core::graph::EffectMonitor>>,
) {
    let mut monitor_expiry = Instant::now();
    let _plugin_thread = crate::plugins::PluginThread::default();
    let registry = EffectRegistry::default();
    let mut session: Option<Session> = None;
    let mut last = Instant::now();
    let mut logged_message = String::new();
    let mut last_metrics = Instant::now();
    loop {
        while let Ok(command) = rx.try_recv() {
            match command {
                Command::MonitorEffect(target) => {
                    monitor_expiry = Instant::now() + Duration::from_millis(600);
                    if let Some(active) = &mut session {
                        match target {
                            Some((node, effect, token)) => {
                                if !active.graph.monitor.as_ref().is_some_and(|m| {
                                    (m.node, m.effect, m.token) == (node, effect, token)
                                }) {
                                    active.graph.monitor =
                                        Some(Box::new(piratewave_core::graph::EffectMonitor::new(
                                            node, effect, token,
                                        )));
                                }
                            }
                            None => active.graph.monitor = None,
                        }
                    }
                }
                Command::Shutdown => {
                    crate::training::audio_running(false);
                    return;
                }
                Command::RestartVoice => {
                    if let Some(active) = &session {
                        active.voice_changer.restart();
                    }
                }
                Command::Inspect(reply) => {
                    let status = session.as_mut().map(|s| s.status()).unwrap_or_default();
                    let _ = reply.try_send(status);
                }
                Command::Stop => {
                    crate::logging::info(format_args!("Audio stopped by user"));
                    crate::training::audio_running(false);
                    session = None;
                    publish(
                        &statuses,
                        Status {
                            message: "Audio stopped".into(),
                            ..Default::default()
                        },
                    );
                }
                Command::Start(project) => {
                    drop(session.take());
                    session = start_session(&project, &statuses);
                }
                Command::StartIfStopped(project) => {
                    if session.is_none() {
                        session = start_session(&project, &statuses);
                    }
                }
                Command::Toggle(project) => {
                    if session.is_some() {
                        crate::training::audio_running(false);
                        session = None;
                        publish(
                            &statuses,
                            Status {
                                message: "Audio stopped".into(),
                                ..Default::default()
                            },
                        );
                    } else {
                        session = start_session(&project, &statuses);
                    }
                }
                Command::Reconfigure(project) => {
                    if let Some(mut old) = session.take() {
                        let mut histories = std::mem::take(&mut old.recorder);
                        let voices = std::mem::take(&mut old.recording_voices);
                        drop(old);
                        session = start_session(&project, &statuses);
                        if let Some(active) = &mut session {
                            histories.sync(&project);
                            active.recorder = histories;
                            active.recording_voices = voices;
                            if let Err(error) = active.sync_recording_effects(&project, &registry) {
                                active.message = format!("Recording effects: {error}");
                            }
                        }
                    }
                }
                Command::Update(project) if session.is_none() => {
                    let _ = project;
                }
                Command::Update(project) => {
                    let result = validate_hardware_routes(&project).and_then(|()| {
                        if let Some(active) = &mut session {
                            active.graph.update(&project, &registry)?;
                            active.refresh_render_mappings();
                            active.voice_changer.sync(&project);
                            let plugin_update = Instant::now();
                            active.plugins.sync(&project);
                            // Loading a library can pause this worker; it is not a stalled device.
                            active.last_render += plugin_update.elapsed();
                            active.sync_listeners(&project);
                            active.recorder.sync(&project);
                            active.sync_recording_effects(&project, &registry)?;
                            active.project = project.clone();
                            Ok(())
                        } else {
                            Ok(())
                        }
                    });
                    if let Err(error) = result {
                        crate::logging::error(format_args!("Live audio edit rejected: {error}"));
                        if let Some(active) = &mut session {
                            active.message =
                                format!("Edit rejected; previous audio setup continues: {error}");
                        }
                    }
                }
                Command::Play { clip, volume } => {
                    if let Some(active) = &mut session {
                        if active.voices.len() < 32 {
                            active.voices.push(Voice {
                                clip,
                                position: 0,
                                volume: volume.clamp(0.0, 2.0),
                            });
                        } else {
                            active.message = "32-voice playback limit reached".into();
                        }
                    }
                }
                Command::Note {
                    id,
                    note,
                    velocity,
                    on,
                } => {
                    if let Some(active) = &mut session {
                        active.plugins.note(id, note, velocity, on);
                    }
                }
                Command::AllNotesOff(id) => {
                    if let Some(active) = &mut session {
                        active.plugins.all_notes_off(id);
                    }
                }
                Command::CaptureRecording {
                    node,
                    slot,
                    token,
                    jobs,
                    events,
                } => {
                    let plan = session
                        .as_ref()
                        .ok_or_else(|| "Start audio before capturing a recording".to_owned())
                        .and_then(|s| {
                            let seconds = s
                                .project
                                .nodes
                                .iter()
                                .find(|n| n.id == node && matches!(n.kind, NodeKind::Recording))
                                .and_then(|n| n.recording.slots.iter().find(|s| s.id == slot))
                                .ok_or("Recording slot no longer exists")?
                                .seconds;
                            s.recorder.capture(node, seconds)
                        });
                    let result = plan.and_then(|plan| {
                        jobs.try_send(crate::recording_io::Job::Capture {
                            plan,
                            node,
                            slot,
                            token,
                        })
                        .map_err(|_| "Recording file worker is busy; try again".to_owned())
                    });
                    if let Err(error) = result {
                        let _ = events.try_send(crate::recording_io::Event::Failed {
                            node,
                            slot,
                            token,
                            error,
                        });
                    }
                }
                Command::PlayRecording {
                    node,
                    slot,
                    clip,
                    start,
                    end,
                    preview,
                    send,
                } => {
                    if let Some(active) = &mut session {
                        if active.recording_voices.len() < 32
                            && active
                                .recording_voices
                                .iter()
                                .map(|v| v.clip.frames.len() * 8)
                                .sum::<usize>()
                                + clip.frames.len() * 8
                                <= 256 * 1024 * 1024
                            && active.project.nodes.iter().any(|n| {
                                n.id == node
                                    && matches!(n.kind, NodeKind::Recording)
                                    && n.recording.slots.iter().any(|s| s.id == slot)
                            })
                            && start.is_finite()
                            && end.is_finite()
                        {
                            let a = ((start.max(0.) * SAMPLE_RATE as f32).round() as usize)
                                .min(clip.frames.len());
                            let b = if end <= 0. {
                                clip.frames.len()
                            } else {
                                ((end * SAMPLE_RATE as f32).round() as usize).min(clip.frames.len())
                            };
                            if a < b {
                                let config =
                                    active.project.nodes.iter().find(|n| n.id == node).unwrap();
                                let effects =
                                    match piratewave_core::recording_effects::RecordingEffects::new(
                                        &config.effects,
                                        config.effects_disabled,
                                        &registry,
                                    ) {
                                        Ok(effects) => effects,
                                        Err(error) => {
                                            crate::logging::error(format_args!(
                                                "Recording effects: {error}"
                                            ));
                                            active.message = error;
                                            continue;
                                        }
                                    };
                                active.recording_voices.push(RecordingVoice {
                                    effects,
                                    node,
                                    slot,
                                    clip,
                                    position: a,
                                    end: b,
                                    preview,
                                    send,
                                });
                            }
                        }
                    }
                }
                Command::StopRecordingPlayback { node, slot } => {
                    if let Some(active) = &mut session {
                        active
                            .recording_voices
                            .retain(|v| v.node != node || slot.is_some_and(|s| s != v.slot));
                    }
                }
                Command::StopSounds => {
                    if let Some(active) = &mut session {
                        active.voices.clear();
                        active.recording_voices.clear();
                    }
                }
            }
        }
        if let Some(active) = &mut session {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| active.recover_devices()))
                .is_err()
            {
                // Recovery may have removed/reordered streams before unwinding.
                active.refresh_render_mappings();
                active.message =
                    "Device/plugin recovery fault; mixer continues and will retry".into();
            }
            if active.render_ready() {
                if active.graph.monitor.is_some() && Instant::now() >= monitor_expiry {
                    active.graph.monitor = None;
                }
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| active.render()))
                    .is_err()
                {
                    // Recover Rust processing faults; OS/native access violations cannot unwind.
                    crate::logging::error(format_args!("Recovering audio render panic"));
                    active.voices.clear();
                    if let Ok(graph) = Graph::new(&active.project, &registry) {
                        active.graph = graph;
                        active.refresh_render_mappings();
                    }
                    active.voice_changer.restart();
                    active.message =
                        "Recovered from an audio processing panic; playback cleared".into();
                }
                if let Some(monitor) = active.graph.monitor.as_ref().filter(|m| m.valid) {
                    // Diagnostics may drop frames under UI load; audio never waits.
                    let _ = effect_audio.push(**monitor);
                }
            } else {
                thread::sleep(Duration::from_micros(500));
            }
            if last.elapsed() >= Duration::from_millis(50) {
                crate::plugins::pump_messages();
                let status = active.status();
                if status.message != logged_message {
                    if active.graph.nodes.iter().any(|n| n.has_effect_fault()) {
                        crate::logging::error(format_args!(
                            "Audio effect fault: {}",
                            status.message
                        ));
                    } else {
                        crate::logging::info(format_args!("Audio status: {}", status.message));
                    }
                    logged_message = status.message.clone();
                }
                if last_metrics.elapsed() >= Duration::from_secs(10) {
                    crate::logging::info(format_args!("Audio health: outputs={}, load={:.2}%, input_frames={}, output_frames={}, underruns={}, dropped_frames={}",status.outputs,status.render_load,status.input_frames,status.output_frames,status.underflows,status.overflows));
                    last_metrics = Instant::now();
                }
                publish(&statuses, status);
                last = Instant::now();
            }
        } else {
            thread::sleep(Duration::from_millis(5));
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn render_mappings_follow_stream_recovery_and_graph_reordering() {
        use piratewave_core::project::Node;
        let mut p = Project {
            nodes: vec![
                Node::new(
                    1,
                    "Mic",
                    NodeKind::Input {
                        device: None,
                        loopback: false,
                    },
                    [0.; 2],
                ),
                Node::new(2, "Pads", NodeKind::Soundboard, [0.; 2]),
                Node::new(3, "More pads", NodeKind::Soundboard, [0.; 2]),
                Node::new(4, "Output", NodeKind::Output { device: None }, [0.; 2]),
            ],
            connections: vec![],
            ..Project::default()
        };
        let blocks = vec![
            (3, [[0.; 2]; BLOCK_SIZE]),
            (1, [[0.; 2]; BLOCK_SIZE]),
            (2, [[0.; 2]; BLOCK_SIZE]),
        ];
        let graph = Graph::new(&p, &EffectRegistry::default()).unwrap();
        let m = RenderMappings::new(&graph, &blocks, [99, 1].into_iter(), std::iter::empty());
        assert_eq!(m.soundboards, vec![0, 2]);
        assert_eq!(m.sources, vec![(None, None), (Some(1), None), (None, None)]);
        assert_eq!(m.outputs[&4], 3);
        p.nodes.reverse();
        let graph = Graph::new(&p, &EffectRegistry::default()).unwrap();
        let m = RenderMappings::new(&graph, &blocks, std::iter::empty(), [1].into_iter());
        assert_eq!(m.soundboards, vec![0, 2]);
        assert_eq!(m.sources[1], (None, Some(0)));
        assert_eq!(m.outputs[&4], 0);
        let m = RenderMappings::new(&graph, &blocks, [1].into_iter(), std::iter::empty());
        assert_eq!(m.sources[1], (Some(0), None));
    }

    use super::*;
    #[test]
    fn ducking_speech_detector_recognizes_bundled_speech_without_changing_audio() {
        use piratewave_core::ducking::{AutoDucking, Detection, DuckingConfig};
        let clip = crate::assets::load(std::path::Path::new("docs/audio/voice-clean.wav")).unwrap();
        let config = DuckingConfig {
            detection: Detection::Speech,
            ..Default::default()
        };
        let mut d = AutoDucking::new(&config);
        let mut speech_blocks = 0;
        let mut max_probability = 0_f32;
        for input in clip.frames.chunks(BLOCK_SIZE) {
            let mut p = [[0.; 2]; BLOCK_SIZE];
            p[..input.len()].copy_from_slice(input);
            let original = p;
            let mut b = [[0.1; 2]; BLOCK_SIZE];
            let mut mixed = [[0.; 2]; BLOCK_SIZE];
            d.process(&mut p, &mut b, &mut mixed, false);
            assert_eq!(p, original);
            max_probability = max_probability.max(d.status().speech_probability);
            speech_blocks += usize::from(d.status().active);
        }
        assert!(
            max_probability > 0.6 && speech_blocks > 20,
            "Speech detection: {max_probability}, {speech_blocks}"
        );
        let mut d = AutoDucking::new(&config);
        for _ in 0..400 {
            let mut p = [[0.; 2]; BLOCK_SIZE];
            let mut b = [[0.1; 2]; BLOCK_SIZE];
            let mut m = [[0.; 2]; BLOCK_SIZE];
            d.process(&mut p, &mut b, &mut m, false);
            assert!(!d.status().active);
            assert_eq!(b, [[0.1; 2]; BLOCK_SIZE]);
        }
    }
    #[test]
    fn recording_effects_process_slots_but_never_capture_or_passthrough_and_update_live() {
        use piratewave_core::{
            effects::EffectSpec,
            project::{Node, OutputPort},
            recording::RecordSlot,
            recording_effects::RecordingEffects,
        };
        let registry = EffectRegistry::default();
        let mut p = missing_device_project();
        let mut node = Node::new(4, "Recorder", NodeKind::Recording, [0.; 2]);
        node.recording.enabled = true;
        node.recording.slots.push(RecordSlot::new("Take"));
        let slot = node.recording.slots[0].id;
        let mut gain = EffectSpec::new(registry.get("gain").unwrap());
        gain.parameters.insert("gain".into(), -6.0206);
        node.effects.push(gain);
        p.nodes.extend([
            node,
            Node::new(5, "Preview", NodeKind::Processor, [0.; 2]),
            Node::new(6, "Send", NodeKind::Processor, [0.; 2]),
        ]);
        p.connect(2, 4).unwrap();
        p.connect_port(4, 5, OutputPort::Preview).unwrap();
        p.connect_port(4, 6, OutputPort::Send).unwrap();
        let mut s = Session::new(&p).unwrap();
        s.voices.push(Voice {
            clip: Clip {
                frames: Arc::new(vec![[0.2, 0.1]; 48000]),
            },
            position: 0,
            volume: 1.,
        });
        for _ in 0..100 {
            s.render();
        }
        assert!(s
            .recorder
            .capture(4, 0.1)
            .unwrap()
            .materialize()
            .unwrap()
            .iter()
            .all(|f| *f == [0.2, 0.1]));
        let frames = Arc::new(vec![[0.4, -0.2]; 2048]);
        s.recording_voices.push(RecordingVoice {
            node: 4,
            slot,
            clip: Clip {
                frames: frames.clone(),
            },
            position: 3,
            end: 3 + BLOCK_SIZE * 4,
            preview: true,
            send: true,
            effects: RecordingEffects::new(&p.nodes[3].effects, false, &registry).unwrap(),
        });
        for (enabled, bypass, db, expected) in [
            (true, false, -6.0206, 0.2),
            (false, false, -6.0206, 0.4),
            (true, true, -6.0206, 0.4),
            (true, false, 6.0206, 0.8),
        ] {
            let n = p.nodes.iter_mut().find(|n| n.id == 4).unwrap();
            n.effects[0].enabled = enabled;
            n.effects_disabled = bypass;
            n.effects[0].parameters.insert("gain".into(), db);
            s.graph.update(&p, &registry).unwrap();
            s.sync_recording_effects(&p, &registry).unwrap();
            s.render();
            let preview = &s
                .graph
                .nodes
                .iter()
                .find(|n| n.config.id == 5)
                .unwrap()
                .frames;
            let send = &s
                .graph
                .nodes
                .iter()
                .find(|n| n.config.id == 6)
                .unwrap()
                .frames;
            assert_eq!(preview, send);
            assert!(preview
                .iter()
                .all(|f| (f[0] - expected).abs() < 0.00001
                    && (f[1] + expected * 0.5).abs() < 0.00001));
            assert_eq!(
                s.graph
                    .nodes
                    .iter()
                    .find(|n| n.config.id == 4)
                    .unwrap()
                    .frames,
                [[0.2, 0.1]; BLOCK_SIZE]
            );
        }
        assert!(frames.iter().all(|f| *f == [0.4, -0.2]));
        assert!(s
            .recorder
            .capture(4, 0.1)
            .unwrap()
            .materialize()
            .unwrap()
            .iter()
            .all(|f| *f == [0.2, 0.1]));
        // Structural rack changes and graph rebuilds must not touch dry capture.
        p.nodes
            .iter_mut()
            .find(|n| n.id == 4)
            .unwrap()
            .effects
            .clear();
        s.graph.update(&p, &registry).unwrap();
        s.sync_recording_effects(&p, &registry).unwrap();
    }
    #[test]
    fn recording_only_clock_does_not_drift_with_millisecond_wakeups() {
        let start = Instant::now();
        let mut deadline = None;
        let mut blocks = 0;
        for ms in 0..10_000 {
            let now = start + Duration::from_millis(ms);
            while software_clock_due(&mut deadline, now) {
                blocks += 1;
            }
        }
        assert!(
            (3749..=3751).contains(&blocks),
            "{blocks} blocks in 10 seconds"
        );
        assert!(software_clock_due(
            &mut deadline,
            start + Duration::from_secs(30)
        ));
        assert!(!software_clock_due(
            &mut deadline,
            start + Duration::from_secs(30)
        ));
    }
    #[test]
    fn recording_render_captures_live_mix_and_keeps_preview_send_out_of_passthrough() {
        use piratewave_core::project::{Node, OutputPort};
        let mut p = missing_device_project();
        let mut recording = Node::new(4, "Recorder", NodeKind::Recording, [0.; 2]);
        recording.recording.enabled = true;
        recording
            .recording
            .slots
            .push(piratewave_core::recording::RecordSlot::new("Test"));
        let slot = recording.recording.slots[0].id;
        p.nodes.extend([
            recording,
            Node::new(5, "Preview", NodeKind::Processor, [0.; 2]),
            Node::new(6, "Send", NodeKind::Processor, [0.; 2]),
        ]);
        p.connect(2, 4).unwrap();
        p.connect_port(4, 5, OutputPort::Preview).unwrap();
        p.connect_port(4, 6, OutputPort::Send).unwrap();
        let mut s = Session::new(&p).unwrap();
        s.voices.push(Voice {
            clip: Clip {
                frames: Arc::new(vec![[0.2, 0.1]; 48000]),
            },
            position: 0,
            volume: 1.,
        });
        for _ in 0..100 {
            s.render();
        }
        let frames = s.recorder.capture(4, 0.1).unwrap().materialize().unwrap();
        assert_eq!(frames.len(), 4800);
        assert!(frames.iter().all(|f| *f == [0.2, 0.1]));
        s.recording_voices.push(RecordingVoice {
            effects: piratewave_core::recording_effects::RecordingEffects::new(
                &[],
                false,
                &EffectRegistry::default(),
            )
            .unwrap(),
            node: 4,
            slot,
            clip: Clip {
                frames: Arc::new(vec![[0.8; 2]; 256]),
            },
            position: 0,
            end: 256,
            preview: true,
            send: false,
        });
        s.render();
        assert_eq!(
            s.graph
                .nodes
                .iter()
                .find(|n| n.config.id == 4)
                .unwrap()
                .frames,
            [[0.2, 0.1]; BLOCK_SIZE]
        );
        assert_eq!(
            s.graph
                .nodes
                .iter()
                .find(|n| n.config.id == 5)
                .unwrap()
                .frames,
            [[0.8; 2]; BLOCK_SIZE]
        );
        assert_eq!(
            s.graph
                .nodes
                .iter()
                .find(|n| n.config.id == 6)
                .unwrap()
                .frames,
            [[0.; 2]; BLOCK_SIZE]
        );
        s.recording_voices.clear();
        s.recording_voices.push(RecordingVoice {
            effects: piratewave_core::recording_effects::RecordingEffects::new(
                &[],
                false,
                &EffectRegistry::default(),
            )
            .unwrap(),
            node: 4,
            slot,
            clip: Clip {
                frames: Arc::new(vec![[0.1; 2], [0.3; 2], [0.4; 2], [0.9; 2]]),
            },
            position: 1,
            end: 3,
            preview: true,
            send: true,
        });
        s.render();
        for id in [5, 6] {
            let node = s.graph.nodes.iter().find(|n| n.config.id == id).unwrap();
            assert_eq!(node.frames[0], [0.3; 2]);
            assert_eq!(node.frames[1], [0.4; 2]);
            assert_eq!(node.frames[2], [0.; 2]);
        }
    }
    fn missing_device_project() -> Project {
        use piratewave_core::project::{Connection, Node};
        Project {
            nodes: vec![
                Node::new(
                    1,
                    "Mic",
                    NodeKind::Input {
                        device: Some("missing-test-microphone::1".into()),
                        loopback: false,
                    },
                    [0.; 2],
                ),
                Node::new(2, "Pads", NodeKind::Soundboard, [0.; 2]),
                Node::new(
                    3,
                    "Bus",
                    NodeKind::Output {
                        device: Some("missing-test-output::1".into()),
                    },
                    [0.; 2],
                ),
            ],
            connections: vec![
                Connection {
                    input: Default::default(),
                    port: Default::default(),
                    from: 1,
                    to: 3,
                },
                Connection {
                    input: Default::default(),
                    port: Default::default(),
                    from: 2,
                    to: 3,
                },
            ],
            ..Project::default()
        }
    }
    #[test]
    fn missing_devices_keep_session_and_soundboard_alive_and_stop_stays_stopped() {
        let project = missing_device_project();
        let mut session = Session::new(&project).unwrap();
        assert!(session.inputs.is_empty() && session.outputs.is_empty());
        session.voices.push(Voice {
            clip: Clip {
                frames: Arc::new(vec![[0.1; 2]; 48000]),
            },
            position: 0,
            volume: 1.,
        });
        for _ in 0..10 {
            session.render();
        }
        assert_eq!(session.voices[0].position, 1280);
        assert!(session.status().running);
        session.last_recovery = Instant::now() - Duration::from_secs(1);
        session.recover_devices();
        assert_eq!(session.voices[0].position, 1280);
        let engine = Engine::new();
        engine.send(Command::Start(project)).unwrap();
        assert!(engine.snapshot().unwrap().running);
        engine.send(Command::Stop).unwrap();
        assert!(!engine.snapshot().unwrap().running);
        thread::sleep(Duration::from_millis(600));
        assert!(!engine.snapshot().unwrap().running);
    }
    #[test]
    #[ignore = "Requires default input/output hardware; injects only private test stream errors"]
    fn hardware_disconnect_reconnect_preserves_output_graph_and_playback() {
        let mut project = missing_device_project();
        project.nodes[0].kind = NodeKind::Input {
            device: None,
            loopback: false,
        };
        project.nodes[2].kind = NodeKind::Output { device: None };
        let mut s = Session::new(&project).unwrap();
        assert_eq!(s.inputs.len(), 1);
        assert_eq!(s.outputs.len(), 1);
        let output_queue = s.outputs[0].queue.clone();
        let graph = s.graph.nodes.as_ptr();
        s.voices.push(Voice {
            clip: Clip {
                frames: Arc::new(vec![[0.; 2]; 48000 * 10]),
            },
            position: 0,
            volume: 1.,
        });
        let drive = |s: &mut Session, millis: u64| {
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(millis) {
                if s.outputs.iter().any(|o| o.queue.len() < TARGET_FRAMES) {
                    s.render();
                } else {
                    thread::sleep(Duration::from_millis(1));
                }
            }
        };
        drive(&mut s, 150);
        let before = s.voices[0].position;
        s.project.nodes[0].kind = NodeKind::Input {
            device: Some("unplugged-test-mic::1".into()),
            loopback: false,
        };
        s.inputs[0]
            .1
            .health
            .inject_error("Injected device invalidation");
        s.last_recovery = Instant::now() - Duration::from_secs(1);
        s.recover_devices();
        assert!(s.inputs.is_empty());
        assert!(Arc::ptr_eq(&output_queue, &s.outputs[0].queue));
        assert_eq!(graph, s.graph.nodes.as_ptr());
        drive(&mut s, 200);
        assert!(s.voices[0].position > before);
        s.project.nodes[0].kind = NodeKind::Input {
            device: None,
            loopback: false,
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while s.inputs.is_empty() && Instant::now() < deadline {
            s.last_recovery = Instant::now() - Duration::from_secs(1);
            s.recover_devices();
            if s.inputs.is_empty() {
                drive(&mut s, 50);
            }
        }
        assert_eq!(s.inputs.len(), 1);
        drive(&mut s, 150);
        assert!(Arc::ptr_eq(&output_queue, &s.outputs[0].queue));
        assert_eq!(graph, s.graph.nodes.as_ptr());
        assert!(s.counters.captured.load(Ordering::Relaxed) > 0);
        assert!(s.counters.rendered.load(Ordering::Relaxed) > 0);
        let before = s.voices[0].position;
        s.outputs[0]
            .health
            .inject_error("Injected output invalidation");
        s.last_recovery = Instant::now() - Duration::from_secs(1);
        s.recover_devices();
        assert_eq!(s.outputs.len(), 1);
        assert!(!Arc::ptr_eq(&output_queue, &s.outputs[0].queue));
        assert_eq!(s.voices[0].position, before);
        assert_eq!(graph, s.graph.nodes.as_ptr());
        drive(&mut s, 150);
        assert!(s.voices[0].position > before);
    }
    #[test]
    fn listen_routes_require_primary_and_deduplicate_default_alias() {
        let mut p = Project::default();
        p.nodes[3].listen_enabled = true;
        assert!(listener_routes(&p, "default::1", |_| true).is_empty());
        p.nodes[3].listen_device = Some("default::1".into());
        assert!(listener_routes(&p, "default::1", |_| true).is_empty());
        p.nodes[3].listen_device = Some("headphones::1".into());
        assert_eq!(
            listener_routes(&p, "default::1", |_| true),
            vec![(4, "headphones::1".into())]
        );
        assert!(listener_routes(&p, "default::1", |_| false).is_empty());
        p.nodes[3].listen_enabled = false;
        assert!(listener_routes(&p, "default::1", |_| true).is_empty());
        assert_eq!(p.nodes[3].listen_device.as_deref(), Some("headphones::1"));
    }
    #[test]
    fn app_shortcuts_dispatch_to_authoritative_transport_commands() {
        use piratewave_core::shortcuts::AppAction as A;
        assert!(matches!(
            Command::for_app_action(A::StartAudio, Project::default()),
            Command::StartIfStopped(_)
        ));
        assert!(matches!(
            Command::for_app_action(A::StopAudio, Project::default()),
            Command::Stop
        ));
        assert!(matches!(
            Command::for_app_action(A::ToggleAudio, Project::default()),
            Command::Toggle(_)
        ));
        assert!(matches!(
            Command::for_app_action(A::StopSounds, Project::default()),
            Command::StopSounds
        ));
    }
}
