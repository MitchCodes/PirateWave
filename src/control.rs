//! Local controller transport. This worker never runs on the audio render thread.
use piratewave_core::{
    control::{Entry, Selector},
    project::Project,
};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
pub const VERSION: u32 = 2;
pub const MAX_FRAME: usize = 16 * 1024 * 1024;
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Request {
    pub id: u64,
    pub op: String,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    #[serde(deserialize_with = "generation")]
    pub generation: u64,
    #[serde(default)]
    pub client: String,
    #[serde(default)]
    pub selector: Option<Selector>,
    #[serde(default)]
    pub ticks: i32,
    #[serde(default = "one")]
    pub sensitivity: f64,
}
// v1 C# clients use JSON integers; v2 JavaScript clients use decimal strings.
fn generation<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Generation {
        Number(u64),
        Text(String),
    }
    match Generation::deserialize(d)? {
        Generation::Number(n) => Ok(n),
        Generation::Text(s) => s.parse().map_err(serde::de::Error::custom),
    }
}
fn wire_state(state: &Snapshot, version: u32) -> serde_json::Value {
    let mut value = serde_json::to_value(state).expect("controller snapshot");
    if version == 2 {
        value["generation"] = state.generation.to_string().into();
    }
    value
}
fn one() -> f64 {
    1.
}
pub struct Pending {
    pub request: Request,
    pub live: Arc<AtomicBool>,
    pub reply: mpsc::SyncSender<Result<String, String>>,
}
#[derive(Clone)]
pub struct Source {
    pub project: Project,
    pub generation: u64,
    pub running: bool,
    pub notice: String,
    pub buffered: std::collections::HashMap<u64, f32>,
    pub recordings: std::collections::HashMap<(u64, u64), String>,
}
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Snapshot {
    pub generation: u64,
    pub project: String,
    pub entries: Vec<Entry>,
    pub notice: String,
}
pub struct Service {
    pub rx: mpsc::Receiver<Pending>,
    pub source: Arc<arc_swap::ArcSwapOption<Source>>,
    pub state: Arc<arc_swap::ArcSwap<Snapshot>>,
    pub message: Arc<std::sync::Mutex<String>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Service {
    pub fn start() -> Result<Self, String> {
        Self::start_with_suffix("")
    }
    fn start_with_suffix(_suffix: &str) -> Result<Self, String> {
        let (tx, rx) = mpsc::sync_channel(64);
        let source = Arc::new(arc_swap::ArcSwapOption::empty());
        let state = Arc::new(arc_swap::ArcSwap::from_pointee(Snapshot::default()));
        let message = Arc::new(std::sync::Mutex::new(
            "Waiting for external controllers".into(),
        ));
        let stop = Arc::new(AtomicBool::new(false));
        #[cfg(windows)]
        let server = windows::Server::create(_suffix)?;
        let (s, c, m, st) = (source.clone(), state.clone(), message.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("piratewave-controller".into())
            .spawn(move || {
                #[cfg(windows)]
                windows::serve(server, tx, s, c, m, st);
                #[cfg(not(windows))]
                {
                    let _ = (tx, s, c, m, st);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            rx,
            source,
            state,
            message,
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
pub fn snapshot(source: &Source, registry: &piratewave_core::effects::EffectRegistry) -> Snapshot {
    let mut entries = piratewave_core::control::catalog(&source.project, registry, source.running);
    for e in &mut entries {
        if e.selector.category == "node" && e.selector.node_kind == "recording" {
            if let Ok(node) = piratewave_core::control::node(&source.project, &e.selector) {
                e.state = format!(
                    "{} / {:.1}s buffered",
                    e.state,
                    source.buffered.get(&node.id).copied().unwrap_or(0.)
                );
            }
        }
        if e.selector.category == "recording_slot" {
            if let Ok(piratewave_core::shortcuts::ShortcutAction::RecordingSlot {
                id, slot, ..
            }) = piratewave_core::control::resolve(&source.project, &e.selector)
            {
                if let Some(outcome) = source.recordings.get(&(id, slot)) {
                    e.state = outcome.clone();
                }
            }
        }
    }
    Snapshot {
        generation: source.generation,
        project: source.project.name.clone(),
        entries,
        notice: source.notice.clone(),
    }
}
#[cfg(test)]
mod protocol_tests {
    use super::*;
    #[test]
    fn generations_are_lossless_and_v1_remains_compatible() {
        let generation = 1_792_746_915_180_750_500;
        for value in [
            serde_json::json!(generation),
            serde_json::json!(generation.to_string()),
        ] {
            let request: Request =
                serde_json::from_value(serde_json::json!({"id":1,"op":"state","generation":value}))
                    .unwrap();
            assert_eq!(request.generation, generation);
        }
        let state = Snapshot {
            generation,
            ..Snapshot::default()
        };
        assert_eq!(
            wire_state(&state, 1)["generation"].as_u64(),
            Some(generation)
        );
        assert_eq!(
            wire_state(&state, 2)["generation"].as_str(),
            Some(generation.to_string().as_str())
        );
        for value in [
            serde_json::json!("-1"),
            serde_json::json!(1.5),
            serde_json::json!("18446744073709551616"),
        ] {
            assert!(serde_json::from_value::<Request>(
                serde_json::json!({"id":1,"op":"execute","generation":value})
            )
            .is_err());
        }
    }
}
/// Isolated, opt-in native transport fixture. Never loads/saves user configuration
/// or opens audio hardware. The UI dispatcher is tested independently.
#[cfg(windows)]
pub fn diagnostic_host() -> anyhow::Result<()> {
    use piratewave_core::{
        project::{Node, NodeKind},
        recording::RecordSlot,
    };
    // Only this diagnostic mode reads a test ID. Production always uses the
    // fixed user-scoped endpoint, independent of environment variables.
    let test_id = std::env::var("PIRATEWAVE_CONTROLLER_TEST_ID").unwrap_or_default();
    anyhow::ensure!(
        test_id.len() <= 64 && test_id.bytes().all(|b| b.is_ascii_alphanumeric()),
        "Invalid controller test ID"
    );
    let suffix = if test_id.is_empty() {
        String::new()
    } else {
        format!(".test.{test_id}")
    };
    let service = Service::start_with_suffix(&suffix).map_err(anyhow::Error::msg)?;
    let registry = piratewave_core::effects::EffectRegistry::default();
    let mut project = Project {
        name: "Controller validation fixture".into(),
        ..Project::default()
    };
    let mut recorder = Node::new(91, "Replay", NodeKind::Recording, [0.; 2]);
    recorder
        .recording
        .slots
        .push(RecordSlot::new("Last phrase"));
    project.nodes.push(recorder);
    project
        .nodes
        .push(Node::new(90, "Source", NodeKind::Soundboard, [0.; 2]));
    project
        .nodes
        .push(Node::new(94, "Ducker", NodeKind::AutoDucking, [0.; 2]));
    let mut fx = Node::new(92, "FX", NodeKind::Processor, [0.; 2]);
    fx.effects.push(piratewave_core::effects::EffectSpec::new(
        registry.get("vocoder").unwrap(),
    ));
    project.nodes.push(fx);
    let mut vst = Node::new(93, "VST", NodeKind::VstEffect, [0.; 2]);
    vst.plugin = Some(piratewave_core::plugins::NodePlugin::new(
        piratewave_core::plugins::PluginDescriptor {
            path: "diagnostic-fixture.vst3".into(),
            class_id: "diagnostic".into(),
            format: piratewave_core::plugins::PluginFormat::Vst3,
            role: piratewave_core::plugins::PluginRole::Effect,
            name: "Fixture".into(),
            vendor: "PirateWave".into(),
            parameters: vec![piratewave_core::plugins::PluginParameter {
                id: 42,
                name: "Gain".into(),
                unit: String::new(),
                default: 0.5,
                steps: 0,
            }],
        },
    ));
    project.nodes.push(vst);
    for (index, e) in piratewave_core::extensions::catalog().iter().enumerate() {
        let mut n = Node::new(
            100 + index as u64,
            &e.descriptor.name,
            if e.descriptor.kind == piratewave_core::extensions::TypeKind::Node {
                NodeKind::Extension {
                    definition: Box::new(e.descriptor.clone()),
                }
            } else {
                NodeKind::Processor
            },
            [0.; 2],
        );
        if e.descriptor.kind == piratewave_core::extensions::TypeKind::Processor {
            n.effects
                .push(piratewave_core::extensions::effect_spec(&e.descriptor));
        }
        project.nodes.push(n);
    }
    let mut generation = 1_792_746_915_180_750_500;
    let mut accepted = 0;
    let publish = |project: &Project, generation, accepted| {
        service.source.store(Some(Arc::new(Source {
            project: project.clone(),
            generation,
            running: false,
            notice: format!("Accepted {accepted}"),
            buffered: Default::default(),
            recordings: Default::default(),
        })))
    };
    publish(&project, generation, accepted);
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(60) {
        if let Ok(pending) = service
            .rx
            .recv_timeout(std::time::Duration::from_millis(10))
        {
            if !pending.live.load(Ordering::Acquire) {
                continue;
            }
            let r = &pending.request;
            let result = (|| -> Result<String, String> {
                if r.generation != generation {
                    return Err("Project changed; refresh controller state".into());
                }
                let selector = r.selector.as_ref().ok_or("Missing selector")?;
                if r.op == "adjust" || r.op == "reset" {
                    if r.op == "reset" && r.version >= 2 {
                        piratewave_core::control::reset_parameter(
                            &mut project,
                            &registry,
                            selector,
                        )?;
                    } else {
                        piratewave_core::control::adjust(
                            &mut project,
                            &registry,
                            selector,
                            r.ticks,
                            r.sensitivity,
                            r.op == "reset",
                        )?;
                    }
                } else {
                    let action = piratewave_core::control::resolve(&project, selector)?;
                    match action {
                        piratewave_core::shortcuts::ShortcutAction::Extension {
                            id,
                            effect,
                            action,
                        } => piratewave_core::extensions::apply_action(
                            &mut project,
                            id,
                            effect,
                            action,
                        )?,
                        piratewave_core::shortcuts::ShortcutAction::App(
                            piratewave_core::shortcuts::AppAction::StartAudio,
                        ) => generation += 1,
                        piratewave_core::shortcuts::ShortcutAction::Node { id, action } => {
                            action.apply(project.nodes.iter_mut().find(|n| n.id == id).unwrap())?;
                        }
                        _ => {}
                    }
                }
                accepted += 1;
                publish(&project, generation, accepted);
                Ok("Accepted".into())
            })();
            let _ = pending.reply.try_send(result);
        }
    }
    Ok(())
}
#[cfg(windows)]
pub mod windows {
    use super::*;
    use std::{
        ptr,
        time::{Duration, Instant},
    };
    type Awaiting = (u64, mpsc::Receiver<Result<String, String>>, Instant);
    use windows_sys::Win32::{
        Foundation::*,
        Security::Authorization::*,
        Security::*,
        Storage::FileSystem::*,
        System::{Pipes::*, Threading::*, IO::*},
    };
    pub fn sid() -> Result<String, String> {
        unsafe {
            let mut token = ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let mut size = 0;
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut size);
            let mut data = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
            let ok =
                GetTokenInformation(token, TokenUser, data.as_mut_ptr().cast(), size, &mut size);
            CloseHandle(token);
            if ok == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let mut text = ptr::null_mut();
            if ConvertSidToStringSidW((*(data.as_ptr().cast::<TOKEN_USER>())).User.Sid, &mut text)
                == 0
            {
                return Err("Cannot determine controller user".into());
            }
            let mut len = 0;
            while *text.add(len) != 0 {
                len += 1;
            }
            let result = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
            LocalFree(text as _);
            Ok(result)
        }
    }
    pub struct Pipe(isize);
    impl Drop for Pipe {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0 as _);
            }
        }
    }
    impl Pipe {
        fn create(suffix: &str, first: bool) -> Result<Self, String> {
            unsafe {
                let sid = sid()?;
                let name: Vec<u16> = format!("\\\\.\\pipe\\PirateWave.Control.v1.{sid}{suffix}\0")
                    .encode_utf16()
                    .collect();
                let acl: Vec<u16> = format!("D:P(A;;GA;;;{sid})\0").encode_utf16().collect();
                let mut descriptor = ptr::null_mut();
                if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    acl.as_ptr(),
                    1,
                    &mut descriptor,
                    ptr::null_mut(),
                ) == 0
                {
                    return Err("Cannot secure controller pipe".into());
                }
                let security = SECURITY_ATTRIBUTES {
                    nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: descriptor,
                    bInheritHandle: 0,
                };
                let handle = CreateNamedPipeW(
                    name.as_ptr(),
                    PIPE_ACCESS_DUPLEX
                        | FILE_FLAG_OVERLAPPED
                        | if first {
                            FILE_FLAG_FIRST_PIPE_INSTANCE
                        } else {
                            0
                        },
                    PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    8,
                    65536,
                    65536,
                    0,
                    &security,
                );
                LocalFree(descriptor as _);
                if handle == INVALID_HANDLE_VALUE {
                    return Err(format!(
                        "Controller unavailable (another PirateWave instance may own it): {}",
                        std::io::Error::last_os_error()
                    ));
                }
                Ok(Self(handle as isize))
            }
        }
    }
    // Stable buffers and OVERLAPPED structures remain alive until completion or
    // cancellation. Each connection owns its operations; a slow peer cannot
    // stall the acceptor, publisher, another controller or the audio thread.
    struct Operation {
        pipe: isize,
        event: Pipe,
        overlapped: Box<OVERLAPPED>,
        pending: bool,
    }
    impl Operation {
        fn new(pipe: &Pipe) -> Result<Self, String> {
            let event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
            if event.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let mut overlapped: Box<OVERLAPPED> = Box::new(unsafe { std::mem::zeroed() });
            overlapped.hEvent = event;
            Ok(Self {
                pipe: pipe.0,
                event: Pipe(event as isize),
                overlapped,
                pending: false,
            })
        }
        fn poll(&mut self) -> Result<Option<u32>, String> {
            let mut count = 0;
            if unsafe { GetOverlappedResult(self.pipe as _, &*self.overlapped, &mut count, 0) } != 0
            {
                self.pending = false;
                return Ok(Some(count));
            }
            let error = unsafe { GetLastError() };
            if error == ERROR_IO_INCOMPLETE {
                return Ok(None);
            }
            self.pending = false;
            Err(format!("Controller I/O: {error}"))
        }
        fn wait(&mut self, stop: &AtomicBool, deadline: Option<Instant>) -> Result<u32, String> {
            loop {
                if stop.load(Ordering::Acquire) || deadline.is_some_and(|at| Instant::now() >= at) {
                    return Err("Controller I/O cancelled or timed out".into());
                }
                if let Some(count) = self.poll()? {
                    return Ok(count);
                }
                unsafe {
                    WaitForSingleObject(self.event.0 as _, 25);
                }
            }
        }
    }
    impl Drop for Operation {
        fn drop(&mut self) {
            if self.pending {
                unsafe {
                    CancelIoEx(self.pipe as _, &*self.overlapped);
                    let mut count = 0;
                    GetOverlappedResult(self.pipe as _, &*self.overlapped, &mut count, 1);
                }
            }
        }
    }
    struct Reader {
        operation: Operation,
        buffer: Box<[u8; 8192]>,
    }
    impl Reader {
        fn new(pipe: &Pipe) -> Result<Self, String> {
            Ok(Self {
                operation: Operation::new(pipe)?,
                buffer: Box::new([0; 8192]),
            })
        }
        fn append(&mut self, input: &mut Vec<u8>) -> Result<usize, String> {
            let op = &mut self.operation;
            if !op.pending {
                unsafe {
                    ResetEvent(op.event.0 as _);
                }
                let ok = unsafe {
                    ReadFile(
                        op.pipe as _,
                        self.buffer.as_mut_ptr(),
                        self.buffer.len() as u32,
                        ptr::null_mut(),
                        &mut *op.overlapped,
                    )
                };
                if ok == 0 && unsafe { GetLastError() } != ERROR_IO_PENDING {
                    return Err(format!(
                        "Controller read: {}",
                        std::io::Error::last_os_error()
                    ));
                }
                op.pending = true;
            }
            if let Some(count) = op.poll()? {
                if count == 0 {
                    return Err("Controller disconnected".into());
                }
                input.extend_from_slice(&self.buffer[..count as usize]);
                return Ok(count as usize);
            }
            Ok(0)
        }
    }
    fn connect(pipe: &Pipe, stop: &AtomicBool) -> Result<(), String> {
        let mut op = Operation::new(pipe)?;
        let ok = unsafe { ConnectNamedPipe(pipe.0 as _, &mut *op.overlapped) };
        if ok != 0 {
            return Ok(());
        }
        match unsafe { GetLastError() } {
            ERROR_PIPE_CONNECTED => Ok(()),
            ERROR_IO_PENDING => {
                op.pending = true;
                op.wait(stop, None).map(|_| ())
            }
            error => Err(format!("Controller accept: {error}")),
        }
    }
    fn write_frame(
        pipe: &Pipe,
        value: &serde_json::Value,
        stop: &AtomicBool,
    ) -> Result<(), String> {
        let json = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        if json.len() > MAX_FRAME {
            return Err("Controller response too large".into());
        }
        let mut bytes = (json.len() as u32).to_le_bytes().to_vec();
        bytes.extend(json);
        let mut op = Operation::new(pipe)?;
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut offset = 0;
        while offset < bytes.len() {
            unsafe {
                ResetEvent(op.event.0 as _);
            }
            let ok = unsafe {
                WriteFile(
                    pipe.0 as _,
                    bytes[offset..].as_ptr(),
                    (bytes.len() - offset) as u32,
                    ptr::null_mut(),
                    &mut *op.overlapped,
                )
            };
            if ok == 0 && unsafe { GetLastError() } != ERROR_IO_PENDING {
                return Err(format!(
                    "Controller write: {}",
                    std::io::Error::last_os_error()
                ));
            }
            op.pending = true;
            let written = op.wait(stop, Some(deadline))?;
            if written == 0 {
                return Err("Controller write disconnected".into());
            }
            offset += written as usize;
        }
        Ok(())
    }
    pub struct Server {
        pipes: Vec<Pipe>,
        _ownership: Pipe,
    }
    impl Server {
        pub fn create(suffix: &str) -> Result<Self, String> {
            // A named mutex is held open for the complete service lifetime, including
            // reconnects. Pipe first-instance ownership alone is not enough once
            // several instances are serving different controllers.
            let name: Vec<u16> = format!("Local\\PirateWave.Control.{}{}\0", sid()?, suffix)
                .encode_utf16()
                .collect();
            let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let ownership = Pipe(handle as isize);
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                return Err("Another PirateWave instance owns external controller support".into());
            }
            let mut pipes = Vec::new();
            for i in 0..8 {
                pipes.push(Pipe::create(suffix, i == 0)?);
            }
            Ok(Self {
                pipes,
                _ownership: ownership,
            })
        }
    }
    type Clients = Arc<std::sync::Mutex<std::collections::BTreeMap<usize, String>>>;
    fn status(clients: &Clients, message: &std::sync::Mutex<String>) {
        let clients = clients.lock().unwrap();
        *message.lock().unwrap() = if clients.is_empty() {
            "Waiting for external controllers".into()
        } else {
            format!(
                "Connected: {}",
                clients.values().cloned().collect::<Vec<_>>().join(", ")
            )
        };
    }
    pub fn serve(
        server: Server,
        tx: mpsc::SyncSender<Pending>,
        source: Arc<arc_swap::ArcSwapOption<Source>>,
        state: Arc<arc_swap::ArcSwap<Snapshot>>,
        message: Arc<std::sync::Mutex<String>>,
        stop: Arc<AtomicBool>,
    ) {
        let clients: Clients = Default::default();
        let mut workers = Vec::new();
        for (slot, pipe) in server.pipes.into_iter().enumerate() {
            let (tx, state, message, stop, clients) = (
                tx.clone(),
                state.clone(),
                message.clone(),
                stop.clone(),
                clients.clone(),
            );
            workers.push(std::thread::spawn(move || {
                connection(pipe, slot, tx, state, message, stop, clients)
            }));
        }
        let registry = piratewave_core::effects::EffectRegistry::default();
        let mut last = None;
        while !stop.load(Ordering::Acquire) {
            if let Some(current) = source.load_full() {
                if last.as_ref().is_none_or(|old| !Arc::ptr_eq(old, &current)) {
                    state.store(Arc::new(snapshot(&current, &registry)));
                    last = Some(current);
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        for worker in workers {
            let _ = worker.join();
        }
        drop(server._ownership);
    }
    fn connection(
        pipe: Pipe,
        slot: usize,
        tx: mpsc::SyncSender<Pending>,
        state: Arc<arc_swap::ArcSwap<Snapshot>>,
        message: Arc<std::sync::Mutex<String>>,
        stop: Arc<AtomicBool>,
        clients: Clients,
    ) {
        while !stop.load(Ordering::Acquire) {
            if let Err(error) = connect(&pipe, &stop) {
                if !stop.load(Ordering::Acquire) {
                    crate::logging::info(format_args!("{error}"));
                }
                break;
            }
            let live = Arc::new(AtomicBool::new(true));
            let mut input = vec![];
            let mut authenticated = false;
            let mut subscribed = false;
            let mut version = 1;
            let mut published = Instant::now() - Duration::from_secs(1);
            let mut pending: Option<Awaiting> = None;
            let mut last_activity = Instant::now();
            let outcome = (|| -> Result<(), String> {
                let mut reader = Reader::new(&pipe)?;
                loop {
                    if stop.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    if last_activity.elapsed() > Duration::from_secs(10) {
                        return Err("Controller heartbeat expired".into());
                    }
                    if published.elapsed() >= Duration::from_millis(100) {
                        if subscribed {
                            write_frame(
                                &pipe,
                                &serde_json::json!({"event":"state","state":wire_state(&state.load(), version)}),
                                &stop,
                            )?;
                        }
                        published = Instant::now();
                    }
                    if let Some((id, rx, at)) = &pending {
                        match rx.try_recv() {
                            Ok(result) => {
                                let response = match result {
                                    Ok(msg) => serde_json::json!({"id":id,"ok":true,"message":msg}),
                                    Err(error) => {
                                        serde_json::json!({"id":id,"ok":false,"error":error})
                                    }
                                };
                                write_frame(&pipe, &response, &stop)?;
                                pending = None;
                            }
                            Err(mpsc::TryRecvError::Disconnected) => {
                                return Err("PirateWave controller stopped".into())
                            }
                            Err(mpsc::TryRecvError::Empty) => {
                                if at.elapsed() > Duration::from_secs(2) {
                                    return Err("PirateWave controller busy".into());
                                }
                            }
                        }
                    }
                    if reader.append(&mut input)? > 0 {
                        last_activity = Instant::now();
                    }
                    if input.len() > MAX_FRAME + 4 {
                        return Err("Controller frame too large".into());
                    }
                    if input.len() >= 4 && pending.is_none() {
                        let size = u32::from_le_bytes(input[..4].try_into().unwrap()) as usize;
                        if size == 0 || size > MAX_FRAME {
                            return Err("Invalid controller frame".into());
                        }
                        if input.len() >= size + 4 {
                            let mut req: Request = serde_json::from_slice(&input[4..size + 4])
                                .map_err(|e| e.to_string())?;
                            input.drain(..size + 4);
                            if req.op == "hello" {
                                authenticated = req.version == 1 || req.version == VERSION;
                                if authenticated {
                                    version = req.version;
                                    let identity = match req.client.as_str() {
                                        "streamdeck" => "Stream Deck",
                                        "loupedeck" => "Loupedeck",
                                        _ => "Legacy controller",
                                    };
                                    clients.lock().unwrap().insert(slot, identity.into());
                                    status(&clients, &message);
                                }
                                write_frame(
                                    &pipe,
                                    &serde_json::json!({"id":req.id,"ok":authenticated,"version":if authenticated {version}else{VERSION},"error":if authenticated{""}else{"Incompatible protocol version"}}),
                                    &stop,
                                )?;
                            } else if !authenticated {
                                return Err("Controller handshake required".into());
                            } else if req.op == "subscribe" || req.op == "state" {
                                subscribed |= req.op == "subscribe";
                                write_frame(
                                    &pipe,
                                    &serde_json::json!({"id":req.id,"ok":true,"state":wire_state(&state.load(), version)}),
                                    &stop,
                                )?;
                            } else if req.op == "execute" || req.op == "adjust" || req.op == "reset"
                            {
                                req.version = version;
                                let (reply, rx) = mpsc::sync_channel(1);
                                let id = req.id;
                                if tx
                                    .try_send(Pending {
                                        request: req,
                                        live: live.clone(),
                                        reply,
                                    })
                                    .is_err()
                                {
                                    write_frame(
                                        &pipe,
                                        &serde_json::json!({"id":id,"ok":false,"error":"PirateWave controller queue full"}),
                                        &stop,
                                    )?;
                                } else {
                                    pending = Some((id, rx, Instant::now()));
                                }
                            } else {
                                write_frame(
                                    &pipe,
                                    &serde_json::json!({"id":req.id,"ok":false,"error":"Unsupported operation"}),
                                    &stop,
                                )?;
                            }
                        }
                    }
                    unsafe {
                        WaitForSingleObject(reader.operation.event.0 as _, 10);
                    }
                }
            })();
            live.store(false, Ordering::Release);
            unsafe {
                DisconnectNamedPipe(pipe.0 as _);
            }
            if let Err(error) = outcome {
                crate::logging::info(format_args!("Controller connection: {error}"));
            }
            clients.lock().unwrap().remove(&slot);
            status(&clients, &message);
        }
    }
}
