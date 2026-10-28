//! OBS WebSocket v5 control plane. No audio data, driver handles or render work.
use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tungstenite::{protocol::WebSocketConfig, Message, WebSocket};
const MAX_MESSAGE: usize = 1024 * 1024;
const MAX_PENDING: usize = 32;
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub connected: bool,
    pub epoch: u64,
    pub message: String,
    pub version: String,
    pub scenes: Vec<String>,
    pub current_scene: String,
    pub inputs: Vec<String>,
    pub selected_input: String,
    pub input_muted: Option<bool>,
    pub input_db: Option<f64>,
    pub streaming: Option<bool>,
    pub recording: Option<bool>,
    pub paused: bool,
}
#[derive(Clone, Debug)]
pub enum Action {
    Refresh,
    SelectInput(String),
    ProgramScene(String),
    StartRecord,
    StopRecord,
    PauseRecord(bool),
    InputMute(String, bool),
    InputVolume(String, f64),
    ArmBarrier,
}
impl Action {
    fn validate(&self) -> Result<()> {
        let name = match self {
            Self::SelectInput(n)
            | Self::ProgramScene(n)
            | Self::InputMute(n, _)
            | Self::InputVolume(n, _) => Some(n),
            _ => None,
        };
        ensure!(
            name.is_none_or(|n| !n.trim().is_empty() && n.len() <= 512),
            "Select an OBS scene/source first"
        );
        if let Self::InputVolume(_, v) = self {
            ensure!(
                v.is_finite() && (-100.0..=0.0).contains(v),
                "OBS volume must be -100 to 0 dB"
            );
        }
        Ok(())
    }
}
#[derive(Debug)]
pub enum Event {
    ProgramScene {
        epoch: u64,
        generation: u64,
        name: String,
    },
    Barrier {
        epoch: u64,
        generation: u64,
        token: u64,
        name: String,
    },
    Completed {
        epoch: u64,
        generation: u64,
        result: Result<(), String>,
    },
}
struct Envelope {
    epoch: u64,
    generation: u64,
    token: u64,
    action: Action,
}
pub struct Service {
    pub state: Arc<arc_swap::ArcSwap<State>>,
    pub events: mpsc::Receiver<Event>,
    pub workspace: Arc<AtomicU64>,
    tx: mpsc::SyncSender<Envelope>,
    stop: Arc<AtomicBool>,
    online: Arc<AtomicBool>,
}
impl Service {
    pub fn start(port: u16, password: String, generation: u64) -> Result<Self> {
        ensure!(port != 0, "OBS port must be 1–65535");
        ensure!(password.len() <= 4096, "OBS password too long");
        let state = Arc::new(arc_swap::ArcSwap::from_pointee(State {
            message: "Connecting to OBS…".into(),
            ..Default::default()
        }));
        let workspace = Arc::new(AtomicU64::new(generation));
        let stop = Arc::new(AtomicBool::new(false));
        let online = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(64);
        let (events, event_rx) = mpsc::sync_channel(64);
        let (shared, w, s, o) = (
            state.clone(),
            workspace.clone(),
            stop.clone(),
            online.clone(),
        );
        thread::Builder::new()
            .name("piratewave-obs".into())
            .spawn(move || run(port, password, shared, w, s, o, rx, events))?;
        Ok(Self {
            state,
            events: event_rx,
            workspace,
            tx,
            stop,
            online,
        })
    }
    pub fn is_connected(&self) -> bool {
        self.online.load(Ordering::Acquire)
    }
    pub fn send(&self, action: Action, generation: u64) -> Result<u64, String> {
        action.validate().map_err(|e| e.to_string())?;
        if !self.online.load(Ordering::Acquire) {
            return Err("OBS is offline; command was not queued".into());
        }
        let token = piratewave_core::workflows::new_id();
        self.tx
            .try_send(Envelope {
                epoch: self.state.load().epoch,
                generation,
                token,
                action,
            })
            .map_err(|_| "OBS command queue is full; try again".to_owned())?;
        Ok(token)
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.online.store(false, Ordering::Release);
        self.stop.store(true, Ordering::Release);
    }
}
fn authentication(password: &str, salt: &str, challenge: &str) -> String {
    let secret = STANDARD.encode(Sha256::digest(format!("{password}{salt}")));
    STANDARD.encode(Sha256::digest(format!("{secret}{challenge}")))
}
fn read_json(ws: &mut WebSocket<TcpStream>) -> Result<Option<Value>> {
    match ws.read() {
        Ok(Message::Text(t)) => Ok(Some(serde_json::from_str(&t).context("Invalid OBS JSON")?)),
        Ok(Message::Close(frame)) => {
            let code = frame.map_or(1000, |f| u16::from(f.code));
            match code {
                4009 => bail!("OBS authentication failed; check the password and reconnect"),
                4010 => bail!("OBS protocol incompatible; OBS WebSocket v5 / RPC 1 is required"),
                4011 => bail!("OBS revoked this connection; reconnect manually"),
                _ => bail!("OBS closed the connection (code {code})"),
            }
        }
        Ok(Message::Binary(_)) => bail!("OBS sent unsupported binary data"),
        Ok(_) => {
            ws.flush()?;
            Ok(None)
        }
        Err(tungstenite::Error::Io(e))
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(e).context("OBS connection lost"),
    }
}
fn receive_handshake(ws: &mut WebSocket<TcpStream>, stop: &AtomicBool) -> Result<Value> {
    let start = Instant::now();
    while !stop.load(Ordering::Acquire) && start.elapsed() < Duration::from_secs(3) {
        if let Some(v) = read_json(ws)? {
            return Ok(v);
        }
    }
    bail!("OBS handshake timed out")
}
fn connect(port: u16, password: &str, stop: &AtomicBool) -> Result<WebSocket<TcpStream>> {
    let socket = TcpStream::connect_timeout(
        &SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        Duration::from_millis(500),
    )?;
    socket.set_nodelay(true)?;
    socket.set_read_timeout(Some(Duration::from_millis(250)))?;
    socket.set_write_timeout(Some(Duration::from_millis(250)))?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE))
        .write_buffer_size(0)
        .max_write_buffer_size(MAX_MESSAGE);
    let (mut ws, _) = tungstenite::client::client_with_config(
        format!("ws://127.0.0.1:{port}"),
        socket,
        Some(config),
    )
    .map_err(|_| anyhow::anyhow!("OBS WebSocket handshake failed"))?;
    let hello = receive_handshake(&mut ws, stop)?;
    ensure!(
        hello["op"] == 0 && hello["d"]["rpcVersion"].as_u64().is_some_and(|v| v >= 1),
        "OBS WebSocket v5 / RPC 1 is required"
    );
    let mut identify = json!({"rpcVersion":1,"eventSubscriptions":1|4|8|64});
    if let Some(auth) = hello["d"].get("authentication") {
        ensure!(
            !password.is_empty(),
            "OBS requires a password; enter it and reconnect"
        );
        let salt = auth["salt"].as_str().context("Missing OBS salt")?;
        let challenge = auth["challenge"]
            .as_str()
            .context("Missing OBS challenge")?;
        identify["authentication"] = authentication(password, salt, challenge).into();
    }
    ws.send(Message::text(json!({"op":1,"d":identify}).to_string()))?;
    let identified = receive_handshake(&mut ws, stop)?;
    ensure!(
        identified["op"] == 2 && identified["d"]["negotiatedRpcVersion"] == 1,
        "OBS identification failed; incompatible RPC version"
    );
    ws.get_mut()
        .set_read_timeout(Some(Duration::from_millis(40)))?;
    Ok(ws)
}
struct Pending {
    kind: String,
    source: String,
    generation: u64,
    token: u64,
    barrier: bool,
    user: bool,
    at: Instant,
}
struct Session<'a> {
    ws: WebSocket<TcpStream>,
    model: State,
    requests: HashMap<String, Pending>,
    next: u64,
    query: VecDeque<(String, Value)>,
    refresh: bool,
    shared: &'a arc_swap::ArcSwap<State>,
    events: &'a mpsc::SyncSender<Event>,
    workspace: &'a AtomicU64,
}
fn emit(events: &mpsc::SyncSender<Event>, event: Event) -> Result<()> {
    events
        .try_send(event)
        .map_err(|_| anyhow::anyhow!("OBS event queue saturated; scene linking disarmed"))
}
fn names(data: &Value, array: &str, name: &str) -> Vec<String> {
    data[array]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v[name].as_str())
        .filter(|s| s.len() <= 512)
        .take(256)
        .map(str::to_owned)
        .collect()
}
impl Session<'_> {
    fn request(
        &mut self,
        kind: &str,
        data: Value,
        user: bool,
        barrier: bool,
        generation: u64,
        token: u64,
    ) -> Result<()> {
        ensure!(
            self.requests.len() < MAX_PENDING,
            "Too many pending OBS requests"
        );
        self.next += 1;
        let id = format!("{}:{}", self.model.epoch, self.next);
        let source = data["inputName"].as_str().unwrap_or_default().to_owned();
        self.ws.send(Message::text(
            json!({"op":6,"d":{"requestType":kind,"requestId":id,"requestData":data}}).to_string(),
        ))?;
        self.requests.insert(
            id,
            Pending {
                kind: kind.into(),
                source,
                generation,
                token,
                barrier,
                user,
                at: Instant::now(),
            },
        );
        Ok(())
    }
    fn queue_refresh(&mut self) {
        for kind in [
            "GetVersion",
            "GetSceneList",
            "GetInputList",
            "GetStreamStatus",
            "GetRecordStatus",
        ] {
            self.queue_query(kind, json!({}));
        }
        self.input_queries();
    }
    fn queue_query(&mut self, kind: &str, data: Value) {
        if self.query.len() < 16
            && !self.query.iter().any(|(k, d)| k == kind && d == &data)
            && !self.requests.values().any(|p| {
                p.kind == kind && p.source == data["inputName"].as_str().unwrap_or_default()
            })
        {
            self.query.push_back((kind.into(), data));
        }
    }
    fn input_queries(&mut self) {
        if !self.model.selected_input.is_empty() {
            let data = json!({"inputName":self.model.selected_input});
            self.queue_query("GetInputMute", data.clone());
            self.queue_query("GetInputVolume", data);
        }
    }
    fn action(&mut self, command: Envelope) -> Result<()> {
        if command.generation != self.workspace.load(Ordering::Acquire)
            || command.epoch != self.model.epoch
        {
            return Ok(());
        }
        let (kind, data, barrier) = match command.action {
            Action::Refresh => {
                self.refresh = true;
                return Ok(());
            }
            Action::SelectInput(name) => {
                self.model.selected_input = name;
                self.model.input_muted = None;
                self.model.input_db = None;
                self.input_queries();
                return Ok(());
            }
            Action::ProgramScene(name) => {
                ("SetCurrentProgramScene", json!({"sceneName":name}), false)
            }
            Action::StartRecord => ("StartRecord", json!({}), false),
            Action::StopRecord => ("StopRecord", json!({}), false),
            Action::PauseRecord(paused) => (
                if paused {
                    "PauseRecord"
                } else {
                    "ResumeRecord"
                },
                json!({}),
                false,
            ),
            Action::InputMute(name, muted) => (
                "SetInputMute",
                json!({"inputName":name,"inputMuted":muted}),
                false,
            ),
            Action::InputVolume(name, db) => (
                "SetInputVolume",
                json!({"inputName":name,"inputVolumeDb":db}),
                false,
            ),
            Action::ArmBarrier => ("GetCurrentProgramScene", json!({}), true),
        };
        self.request(kind, data, true, barrier, command.generation, command.token)
    }
    fn response(&mut self, d: &Value) -> Result<()> {
        let Some(p) = d["requestId"]
            .as_str()
            .and_then(|id| self.requests.remove(id))
        else {
            return Ok(());
        };
        ensure!(d["requestType"] == p.kind, "OBS response type mismatch");
        let success = d["requestStatus"]["result"].as_bool().unwrap_or(false);
        let data = &d["responseData"];
        if !success {
            if p.user {
                emit(
                    self.events,
                    Event::Completed {
                        epoch: self.model.epoch,
                        generation: p.generation,
                        result: Err(format!(
                            "OBS {} failed (code {}): {}",
                            p.kind,
                            d["requestStatus"]["code"],
                            d["requestStatus"]["comment"]
                                .as_str()
                                .unwrap_or("request rejected")
                                .chars()
                                .take(256)
                                .collect::<String>()
                        )),
                    },
                )?;
            }
            return Ok(());
        }
        match p.kind.as_str() {
            "GetVersion" => {
                self.model.version = data["obsVersion"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(128)
                    .collect()
            }
            "GetSceneList" => {
                self.model.scenes = names(data, "scenes", "sceneName");
                self.model.current_scene = data["currentProgramSceneName"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(512)
                    .collect();
            }
            "GetInputList" => self.model.inputs = names(data, "inputs", "inputName"),
            "GetCurrentProgramScene" => {
                self.model.current_scene = data["currentProgramSceneName"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(512)
                    .collect()
            }
            "GetStreamStatus" => self.model.streaming = data["outputActive"].as_bool(),
            "GetRecordStatus" => {
                self.model.recording = data["outputActive"].as_bool();
                self.model.paused = data["outputPaused"].as_bool().unwrap_or(false);
            }
            "GetInputMute" if p.source == self.model.selected_input => {
                self.model.input_muted = data["inputMuted"].as_bool()
            }
            "GetInputVolume" if p.source == self.model.selected_input => {
                self.model.input_db = data["inputVolumeDb"].as_f64().filter(|v| v.is_finite())
            }
            _ => {}
        }
        if p.barrier {
            emit(
                self.events,
                Event::Barrier {
                    epoch: self.model.epoch,
                    generation: p.generation,
                    token: p.token,
                    name: self.model.current_scene.clone(),
                },
            )?;
        } else if p.user {
            emit(
                self.events,
                Event::Completed {
                    epoch: self.model.epoch,
                    generation: p.generation,
                    result: Ok(()),
                },
            )?;
            self.refresh = true;
        }
        Ok(())
    }
    fn event(&mut self, d: &Value) -> Result<()> {
        let data = &d["eventData"];
        match d["eventType"].as_str().unwrap_or_default() {
            "ExitStarted" => bail!("OBS is shutting down"),
            "CurrentProgramSceneChanged" => {
                let name = data["sceneName"]
                    .as_str()
                    .context("OBS scene event has no name")?;
                ensure!(name.len() <= 512, "OBS scene name too long");
                self.model.current_scene = name.into();
                emit(
                    self.events,
                    Event::ProgramScene {
                        epoch: self.model.epoch,
                        generation: self.workspace.load(Ordering::Acquire),
                        name: name.into(),
                    },
                )?;
            }
            "SceneCreated"
            | "SceneRemoved"
            | "SceneNameChanged"
            | "SceneListChanged"
            | "CurrentSceneCollectionChanged"
            | "InputCreated"
            | "InputRemoved"
            | "InputNameChanged" => self.refresh = true,
            "StreamStateChanged" => self.model.streaming = data["outputActive"].as_bool(),
            "RecordStateChanged" => {
                self.model.recording = data["outputActive"].as_bool();
                self.model.paused =
                    data["outputState"].as_str() == Some("OBS_WEBSOCKET_OUTPUT_PAUSED");
            }
            "InputMuteStateChanged" if data["inputName"] == self.model.selected_input => {
                self.model.input_muted = data["inputMuted"].as_bool()
            }
            "InputVolumeChanged" if data["inputName"] == self.model.selected_input => {
                self.model.input_db = data["inputVolumeDb"].as_f64().filter(|v| v.is_finite())
            }
            _ => {}
        }
        Ok(())
    }
    fn serve(&mut self, rx: &mpsc::Receiver<Envelope>, stop: &AtomicBool) -> Result<()> {
        let mut poll = Instant::now();
        let mut published = Instant::now();
        self.queue_refresh();
        while !stop.load(Ordering::Acquire) {
            if poll.elapsed() >= Duration::from_secs(2) || self.refresh {
                self.queue_refresh();
                self.refresh = false;
                poll = Instant::now();
            }
            // User commands precede periodic queries, with bounded in-flight work.
            if self.requests.len() < MAX_PENDING - 1 {
                match rx.try_recv() {
                    Ok(c) => self.action(c)?,
                    Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                    Err(_) => {}
                }
            }
            if self.requests.len() < MAX_PENDING - 1 {
                if let Some((kind, data)) = self.query.pop_front() {
                    self.request(
                        &kind,
                        data,
                        false,
                        false,
                        self.workspace.load(Ordering::Acquire),
                        0,
                    )?;
                }
            }
            if let Some(v) = read_json(&mut self.ws)? {
                match v["op"].as_u64() {
                    Some(5) => self.event(&v["d"])?,
                    Some(7) => self.response(&v["d"])?,
                    _ => {}
                }
            }
            ensure!(
                !self
                    .requests
                    .values()
                    .any(|p| p.at.elapsed() > Duration::from_secs(3)),
                "OBS request timed out; pending commands discarded"
            );
            if published.elapsed() >= Duration::from_millis(50) {
                if **self.shared.load() != self.model {
                    self.shared.store(Arc::new(self.model.clone()));
                }
                published = Instant::now();
            }
        }
        let _ = self.ws.close(None);
        Ok(())
    }
}
#[allow(clippy::too_many_arguments)]
fn run(
    port: u16,
    password: String,
    shared: Arc<arc_swap::ArcSwap<State>>,
    workspace: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    online: Arc<AtomicBool>,
    rx: mpsc::Receiver<Envelope>,
    events: mpsc::SyncSender<Event>,
) {
    let mut epoch = 0;
    let mut backoff = Duration::from_secs(1);
    while !stop.load(Ordering::Acquire) {
        while rx.try_recv().is_ok() {} // Never replay old commands.
        epoch += 1;
        let mut was_connected = false;
        let result = connect(port, &password, &stop).and_then(|ws| {
            was_connected = true;
            backoff = Duration::from_secs(1);
            let model = State {
                connected: true,
                epoch,
                message: "Connected to OBS".into(),
                ..Default::default()
            };
            shared.store(Arc::new(model.clone()));
            online.store(true, Ordering::Release);
            crate::logging::info(format_args!("OBS control connection established"));
            Session {
                ws,
                model,
                requests: HashMap::new(),
                next: 0,
                query: VecDeque::new(),
                refresh: false,
                shared: &shared,
                events: &events,
                workspace: &workspace,
            }
            .serve(&rx, &stop)
        });
        online.store(false, Ordering::Release);
        while rx.try_recv().is_ok() {}
        if stop.load(Ordering::Acquire) {
            break;
        }
        let error = result
            .err()
            .map_or("OBS disconnected".into(), |e| e.to_string());
        if was_connected {
            crate::logging::warn(format_args!(
                "OBS disconnected; pending commands discarded: {error}"
            ));
        }
        let fatal = error.contains("authentication")
            || error.contains("password")
            || error.contains("incompatible")
            || error.contains("RPC 1")
            || error.contains("revoked");
        shared.store(Arc::new(State {
            epoch,
            message: if fatal {
                error.clone()
            } else {
                format!("{error}. Reconnecting…")
            },
            ..Default::default()
        }));
        if fatal {
            crate::logging::error(format_args!("OBS connection requires attention: {error}"));
            break;
        }
        let until = Instant::now() + backoff;
        while !stop.load(Ordering::Acquire) && Instant::now() < until {
            thread::sleep(Duration::from_millis(50));
        }
        backoff = (backoff * 2).min(Duration::from_secs(10));
    }
    online.store(false, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, sync::Mutex};
    const SALT: &str = "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=";
    const CHALLENGE: &str = "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=";
    enum Wire {
        Event(Value),
        Drop,
        Silence,
    }
    struct Fixture {
        port: u16,
        tx: mpsc::Sender<Wire>,
        actions: Arc<Mutex<Vec<(String, Value)>>>,
        connections: Arc<AtomicU64>,
        stop: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
    }
    impl Fixture {
        fn new(auth: bool, rpc: u64) -> Self {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            listener.set_nonblocking(true).unwrap();
            let (tx, rx) = mpsc::channel();
            let actions = Arc::new(Mutex::new(vec![]));
            let seen = actions.clone();
            let stop = Arc::new(AtomicBool::new(false));
            let s = stop.clone();
            let connections = Arc::new(AtomicU64::new(0));
            let count = connections.clone();
            let thread = thread::spawn(move || {
                while !s.load(Ordering::Acquire) {
                    let socket = match listener.accept() {
                        Ok((socket, _)) => socket,
                        Err(_) => {
                            thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                    };
                    count.fetch_add(1, Ordering::AcqRel);
                    socket
                        .set_read_timeout(Some(Duration::from_millis(40)))
                        .unwrap();
                    socket
                        .set_write_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let Ok(mut ws) = tungstenite::accept(socket) else {
                        continue;
                    };
                    let mut hello =
                        json!({"op":0,"d":{"rpcVersion":rpc,"obsWebSocketVersion":"5.0.0"}});
                    if auth {
                        hello["d"]["authentication"] = json!({"salt":SALT,"challenge":CHALLENGE});
                    }
                    if ws.send(Message::text(hello.to_string())).is_err() {
                        continue;
                    }
                    let identified = loop {
                        match ws.read() {
                            Ok(Message::Text(t)) => break serde_json::from_str::<Value>(&t).ok(),
                            Err(tungstenite::Error::Io(e))
                                if matches!(
                                    e.kind(),
                                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                                ) =>
                            {
                                if s.load(Ordering::Acquire) {
                                    return;
                                }
                            }
                            _ => break None,
                        }
                    };
                    let Some(identify) = identified else {
                        continue;
                    };
                    assert_eq!(identify["op"], 1);
                    assert_eq!(identify["d"]["rpcVersion"], 1);
                    assert_eq!(identify["d"]["eventSubscriptions"], 1 | 4 | 8 | 64);
                    if auth
                        && identify["d"]["authentication"]
                            != "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4="
                    {
                        let _ = ws.close(Some(tungstenite::protocol::CloseFrame {
                            code: 4009.into(),
                            reason: "Authentication failed".into(),
                        }));
                        continue;
                    }
                    if ws
                        .send(Message::text(
                            json!({"op":2,"d":{"negotiatedRpcVersion":1}}).to_string(),
                        ))
                        .is_err()
                    {
                        continue;
                    }
                    let mut scene = "Starting Soon".to_owned();
                    let mut record = false;
                    let mut paused = false;
                    let mut muted = false;
                    let mut volume = -6.;
                    let mut silent = false;
                    while !s.load(Ordering::Acquire) {
                        let mut drop_connection = false;
                        while let Ok(wire) = rx.try_recv() {
                            match wire {
                                Wire::Silence => silent = true,
                                Wire::Drop => {
                                    drop_connection = true;
                                    break;
                                }
                                Wire::Event(e) => {
                                    if e["d"]["eventType"] == "CurrentProgramSceneChanged" {
                                        scene = e["d"]["eventData"]["sceneName"]
                                            .as_str()
                                            .unwrap()
                                            .into();
                                    }
                                    if ws.send(Message::text(e.to_string())).is_err() {
                                        drop_connection = true;
                                        break;
                                    }
                                }
                            }
                        }
                        if drop_connection {
                            break;
                        }
                        let request = match ws.read() {
                            Ok(Message::Text(t)) => serde_json::from_str::<Value>(&t).unwrap(),
                            Err(tungstenite::Error::Io(e))
                                if matches!(
                                    e.kind(),
                                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                                ) =>
                            {
                                continue
                            }
                            _ => break,
                        };
                        if request["op"] != 6 {
                            continue;
                        }
                        let d = &request["d"];
                        let kind = d["requestType"].as_str().unwrap();
                        let data = &d["requestData"];
                        let mut result = true;
                        let response = match kind {
                            "GetVersion" => json!({"obsVersion":"32.fixture"}),
                            "GetSceneList" => {
                                json!({"scenes":[{"sceneName":"Starting Soon"},{"sceneName":"Live"}],"currentProgramSceneName":scene})
                            }
                            "GetCurrentProgramScene" => json!({"currentProgramSceneName":scene}),
                            "GetInputList" => {
                                json!({"inputs":[{"inputName":"PirateWave cable"},{"inputName":"Image"}]})
                            }
                            "GetStreamStatus" => json!({"outputActive":false}),
                            "GetRecordStatus" => {
                                json!({"outputActive":record,"outputPaused":paused})
                            }
                            "GetInputMute" => {
                                result = data["inputName"] != "Image";
                                json!({"inputMuted":muted})
                            }
                            "GetInputVolume" => {
                                result = data["inputName"] != "Image";
                                json!({"inputVolumeDb":volume})
                            }
                            "SetCurrentProgramScene" => {
                                scene = data["sceneName"].as_str().unwrap().into();
                                json!({})
                            }
                            "StartRecord" => {
                                record = true;
                                json!({})
                            }
                            "StopRecord" => {
                                record = false;
                                paused = false;
                                json!({})
                            }
                            "PauseRecord" => {
                                paused = true;
                                json!({})
                            }
                            "ResumeRecord" => {
                                paused = false;
                                json!({})
                            }
                            "SetInputMute" => {
                                muted = data["inputMuted"].as_bool().unwrap();
                                json!({})
                            }
                            "SetInputVolume" => {
                                volume = data["inputVolumeDb"].as_f64().unwrap();
                                json!({})
                            }
                            _ => {
                                result = false;
                                json!({})
                            }
                        };
                        if !kind.starts_with("Get") {
                            seen.lock().unwrap().push((kind.into(), data.clone()));
                        }
                        let reply = json!({"op":7,"d":{"requestType":kind,"requestId":d["requestId"],"requestStatus":{"result":result,"code":if result{100}else{604},"comment":"Unsupported source audio"},"responseData":response}});
                        if silent {
                            continue;
                        }
                        if ws.send(Message::text(reply.to_string())).is_err() {
                            break;
                        }
                    }
                }
            });
            Self {
                port,
                tx,
                actions,
                connections,
                stop,
                thread: Some(thread),
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(t) = self.thread.take() {
                t.join().unwrap();
            }
        }
    }
    fn until(mut f: impl FnMut() -> bool) {
        let start = Instant::now();
        while !f() {
            assert!(
                start.elapsed() < Duration::from_secs(8),
                "OBS fixture timed out"
            );
            thread::sleep(Duration::from_millis(15));
        }
    }
    #[test]
    fn authentication_matches_independent_sha256_base64_vector() {
        assert_eq!(
            authentication("supersecretpassword", SALT, CHALLENGE),
            "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4="
        );
    }
    #[test]
    fn real_websocket_auth_queries_controls_events_and_reconnect_without_replay() {
        let fixture = Fixture::new(true, 1);
        let service = Service::start(fixture.port, "supersecretpassword".into(), 77).unwrap();
        until(|| {
            service.state.load().version == "32.fixture"
                && service.state.load().scenes.len() == 2
                && service.state.load().recording == Some(false)
        });
        service
            .send(Action::SelectInput("PirateWave cable".into()), 77)
            .unwrap();
        until(|| service.state.load().input_db == Some(-6.));
        for (action, kind) in [
            (
                Action::ProgramScene("Live".into()),
                "SetCurrentProgramScene",
            ),
            (Action::StartRecord, "StartRecord"),
            (Action::PauseRecord(true), "PauseRecord"),
            (
                Action::InputMute("PirateWave cable".into(), true),
                "SetInputMute",
            ),
            (
                Action::InputVolume("PirateWave cable".into(), -12.),
                "SetInputVolume",
            ),
        ] {
            service.send(action, 77).unwrap();
            until(|| {
                fixture
                    .actions
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(k, _)| k == kind)
            });
        }
        until(|| {
            service.state.load().recording == Some(true)
                && service.state.load().paused
                && service.state.load().input_db == Some(-12.)
                && service.state.load().input_muted == Some(true)
        });
        let token = service.send(Action::ArmBarrier, 77).unwrap();
        let mut baseline = false;
        until(|| {
            while let Ok(e) = service.events.try_recv() {
                if matches!(e,Event::Barrier{token:t,..}if t==token) {
                    baseline = true;
                }
            }
            baseline
        });
        fixture.tx.send(Wire::Event(json!({"op":5,"d":{"eventType":"CurrentProgramSceneChanged","eventData":{"sceneName":"Starting Soon"}}}))).unwrap();
        let mut changed = false;
        until(|| {
            while let Ok(e) = service.events.try_recv() {
                if matches!(e,Event::ProgramScene{generation:77,name,..}if name=="Starting Soon") {
                    changed = true;
                }
            }
            changed
        });
        // Commands queued with a previous workspace stamp are never sent.
        service.workspace.store(78, Ordering::Release);
        let before = fixture.actions.lock().unwrap().len();
        service.send(Action::StopRecord, 77).unwrap();
        thread::sleep(Duration::from_millis(120));
        assert_eq!(fixture.actions.lock().unwrap().len(), before);
        let epoch = service.state.load().epoch;
        fixture.tx.send(Wire::Drop).unwrap();
        until(|| !service.is_connected());
        assert!(service.send(Action::StartRecord, 78).is_err());
        until(|| {
            service.is_connected()
                && service.state.load().epoch > epoch
                && service.state.load().version == "32.fixture"
        });
        assert_eq!(fixture.actions.lock().unwrap().len(), before);
        assert!(fixture.connections.load(Ordering::Acquire) >= 2);
        service
            .send(Action::SelectInput("Image".into()), 78)
            .unwrap();
        until(|| service.state.load().selected_input == "Image");
        assert_eq!(service.state.load().input_db, None);
    }
    #[test]
    fn authentication_failure_and_revoked_connections_do_not_retry() {
        let fixture = Fixture::new(true, 1);
        let service = Service::start(fixture.port, "wrong".into(), 1).unwrap();
        until(|| {
            service
                .state
                .load()
                .message
                .contains("authentication failed")
        });
        thread::sleep(Duration::from_millis(150));
        assert_eq!(fixture.connections.load(Ordering::Acquire), 1);
        assert!(!service.is_connected());
        let fixture2 = Fixture::new(false, 1);
        let service2 = Service::start(fixture2.port, String::new(), 1).unwrap();
        until(|| service2.is_connected());
        // Closing code is tested directly on a real WebSocket pair.
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut ws = tungstenite::accept(listener.accept().unwrap().0).unwrap();
            ws.close(Some(tungstenite::protocol::CloseFrame {
                code: 4011.into(),
                reason: "Revoked".into(),
            }))
            .unwrap();
        });
        let socket = TcpStream::connect(addr).unwrap();
        let (mut ws, _) = tungstenite::client::client(format!("ws://{addr}"), socket).unwrap();
        assert!(read_json(&mut ws)
            .unwrap_err()
            .to_string()
            .contains("revoked"));
        server.join().unwrap();
    }

    #[test]
    fn request_timeout_reconnects_without_replaying_unacknowledged_mutations() {
        let fixture = Fixture::new(false, 1);
        let service = Service::start(fixture.port, String::new(), 1).unwrap();
        until(|| service.state.load().recording == Some(false));
        let epoch = service.state.load().epoch;
        fixture.tx.send(Wire::Silence).unwrap();
        thread::sleep(Duration::from_millis(80));
        service.send(Action::StartRecord, 1).unwrap();
        until(|| {
            fixture
                .actions
                .lock()
                .unwrap()
                .iter()
                .any(|(k, _)| k == "StartRecord")
        });
        until(|| !service.is_connected() && service.state.load().message.contains("timed out"));
        until(|| {
            service.is_connected()
                && service.state.load().epoch > epoch
                && service.state.load().version == "32.fixture"
        });
        assert_eq!(
            fixture
                .actions
                .lock()
                .unwrap()
                .iter()
                .filter(|(k, _)| k == "StartRecord")
                .count(),
            1
        );
    }
    #[test]
    fn incompatible_rpc_and_full_command_queue_fail_explicitly() {
        let fixture = Fixture::new(false, 0);
        let service = Service::start(fixture.port, String::new(), 1).unwrap();
        until(|| service.state.load().message.contains("RPC 1"));
        assert!(!service.is_connected());
        let (tx, rx) = mpsc::sync_channel(64);
        let (_, events) = mpsc::sync_channel(64);
        let offline = Service {
            state: Arc::new(arc_swap::ArcSwap::from_pointee(State {
                connected: true,
                epoch: 9,
                ..Default::default()
            })),
            events,
            workspace: Arc::new(AtomicU64::new(5)),
            tx,
            stop: Arc::new(AtomicBool::new(false)),
            online: Arc::new(AtomicBool::new(true)),
        };
        for _ in 0..64 {
            offline.send(Action::StartRecord, 5).unwrap();
        }
        assert!(offline
            .send(Action::StopRecord, 5)
            .unwrap_err()
            .contains("full"));
        assert_eq!(rx.try_iter().count(), 64);
    }
    #[test]
    fn event_bounds_and_action_validation_are_explicit() {
        let (tx, _rx) = mpsc::sync_channel(64);
        for _ in 0..64 {
            emit(
                &tx,
                Event::Completed {
                    epoch: 1,
                    generation: 1,
                    result: Ok(()),
                },
            )
            .unwrap();
        }
        assert!(emit(
            &tx,
            Event::Completed {
                epoch: 1,
                generation: 1,
                result: Ok(())
            }
        )
        .is_err());
        assert!(Action::InputVolume("Source".into(), f64::NAN)
            .validate()
            .is_err());
        assert!(Action::InputVolume("Source".into(), 1.).validate().is_err());
        assert!(Action::ProgramScene(String::new()).validate().is_err());
        assert!(Service::start(0, String::new(), 1).is_err());
    }
}
