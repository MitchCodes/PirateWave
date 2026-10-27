//! Fail-silent graph adapter. File I/O, process control and inference are owned
//! by a supervisor thread and optional native worker, never the graph renderer.
#[cfg(windows)]
pub(crate) mod job;
pub mod local;
pub mod pack;
mod resample;
#[cfg(test)]
mod tests;
use crossbeam_queue::ArrayQueue;
use piratewave_core::{
    project::{Node, NodeKind, Project},
    voice::{VoiceBackend, VoiceConfig},
    Frame, BLOCK_SIZE,
};
use std::{
    io::{Read, Write},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
const HOP: usize = 416;
const CONTEXT: usize = 32;
const MAX_AGE: Duration = Duration::from_millis(250);
const LOADING: u8 = 0;
const PRIMING: u8 = 1;
const READY: u8 = 2;
const ERROR: u8 = 3;
const DISABLED: u8 = 4;
/// Explicit developer launch mode for the human acceptance checks. Never saved
/// in projects or startup settings, and never bypasses automated qualification.
pub fn qualification_mode() -> bool {
    std::env::args_os().any(|a| a == "--voice-qualification")
}
#[derive(Clone, Copy)]
struct Block {
    generation: u64,
    sequence: u64,
    born: Instant,
    frames: [Frame; BLOCK_SIZE],
    valid: usize,
}
struct Shared {
    input: ArrayQueue<Block>,
    output: ArrayQueue<Block>,
    messages: ArrayQueue<String>,
    generation: AtomicU64,
    state: AtomicU8,
    stop: AtomicBool,
    underruns: AtomicU64,
    resets: AtomicU64,
    desired: arc_swap::ArcSwapOption<Config>,
    worker_pid: AtomicU64,
    max_inference_us: AtomicU64,
    max_render_gap_us: AtomicU64,
    #[cfg(test)]
    test_pack: std::sync::OnceLock<std::path::PathBuf>,
}
#[derive(Clone, PartialEq)]
struct Config {
    id: u64,
    voice: VoiceConfig,
    active: bool,
    routes: Vec<u8>,
    sources: Vec<u64>,
}
pub struct Rack {
    shared: Arc<Shared>,
    config: Option<Config>,
    generation: u64,
    sequence: u64,
    expected: u64,
    current: Option<Block>,
    offset: usize,
    primed: bool,
    fade: f32,
    last: Frame,
    disabled: bool,
    reason: String,
    last_call: Option<Instant>,
}
impl Rack {
    pub fn new(project: &Project) -> Self {
        let shared = Arc::new(Shared {
            input: ArrayQueue::new(64),
            output: ArrayQueue::new(64),
            messages: ArrayQueue::new(8),
            generation: AtomicU64::new(1),
            state: AtomicU8::new(DISABLED),
            stop: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
            resets: AtomicU64::new(0),
            desired: arc_swap::ArcSwapOption::empty(),
            worker_pid: AtomicU64::new(0),
            max_inference_us: AtomicU64::new(0),
            max_render_gap_us: AtomicU64::new(0),
            #[cfg(test)]
            test_pack: std::sync::OnceLock::new(),
        });
        let worker = shared.clone();
        thread::Builder::new()
            .name("piratewave-voice-supervisor".into())
            .spawn(move || supervise(worker))
            .expect("Voice supervisor");
        let mut rack = Self {
            shared,
            config: None,
            generation: 0,
            sequence: 0,
            expected: 0,
            current: None,
            offset: 0,
            primed: false,
            fade: 0.0,
            last: [0.0; 2],
            disabled: false,
            reason: String::new(),
            last_call: None,
        };
        rack.sync(project);
        rack
    }
    pub fn sync(&mut self, project: &Project) {
        let selected = project
            .nodes
            .iter()
            .find(|n| {
                matches!(n.kind, NodeKind::VoiceChanger)
                    && !n.effects_disabled
                    && piratewave_core::voice::route_selected(project, n.id)
            })
            .or_else(|| {
                project
                    .nodes
                    .iter()
                    .find(|n| matches!(n.kind, NodeKind::VoiceChanger))
            });
        let Some(node) = selected else {
            self.config = None;
            self.shared.desired.store(None);
            self.shared.state.store(DISABLED, Ordering::Release);
            self.restart();
            return;
        };
        let mut reachable = vec![node.id];
        for _ in 0..project.nodes.len() {
            for edge in &project.connections {
                if reachable.contains(&edge.from) && !reachable.contains(&edge.to) {
                    reachable.push(edge.to);
                }
            }
        }
        let connected = project.nodes.iter().any(|n| {
            (matches!(n.kind, NodeKind::Output { .. })
                || (matches!(n.kind, NodeKind::Recording)
                    && n.recording.enabled
                    && !n.recording.slots.is_empty()))
                && reachable.contains(&n.id)
        });
        let mut upstream = vec![node.id];
        for _ in 0..project.nodes.len() {
            for edge in &project.connections {
                if upstream.contains(&edge.to) && !upstream.contains(&edge.from) {
                    upstream.push(edge.from);
                }
            }
        }
        let mut sources: Vec<_> = project
            .nodes
            .iter()
            .filter(|n| n.kind.is_source() && upstream.contains(&n.id))
            .map(|n| n.id)
            .collect();
        sources.sort_unstable();
        let mut voice = node.voice.clone().unwrap_or_default();
        voice.output_db = 0.0; // Output gain is applied smoothly by the graph.
                               // Only changes to the voice's incoming signal invalidate model state.
                               // Fan-out and unrelated routes must reuse the existing inference stream.
        let mut incoming: Vec<_> = project
            .connections
            .iter()
            .filter(|edge| upstream.contains(&edge.from) && upstream.contains(&edge.to))
            .collect();
        incoming.sort_unstable_by_key(|edge| (edge.from, edge.to));
        let mut ancestors: Vec<_> = project
            .nodes
            .iter()
            .filter(|n| upstream.contains(&n.id))
            .map(|n| (n.id, &n.kind))
            .collect();
        ancestors.sort_unstable_by_key(|(id, _)| *id);
        let routes = serde_json::to_vec(&(incoming, ancestors)).unwrap_or_default();
        let config = Config {
            id: node.id,
            voice,
            active: !node.effects_disabled
                && connected
                && piratewave_core::voice::route_selected(project, node.id),
            routes,
            sources,
        };
        if self.config.as_ref() != Some(&config) {
            if self.config.as_ref().is_none_or(|old| old.id != config.id) {
                // Transition history belongs to one node. An unrelated bypassed
                // node must never receive another node's converted samples.
                self.last = [0.0; 2];
                self.fade = 1.0;
                self.disabled = true;
            }
            self.shared.desired.store(Some(Arc::new(config.clone())));
            self.restart();
            self.shared.state.store(
                if config.active { LOADING } else { DISABLED },
                Ordering::Release,
            );
            self.config = Some(config);
        }
    }
    pub fn restart(&self) {
        self.shared.state.store(LOADING, Ordering::Release);
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        self.shared.resets.fetch_add(1, Ordering::Relaxed);
    }
    /// Capture discontinuities invalidate only an affected converted route.
    /// Called on the render worker, so this is atomics/lookup only.
    pub fn source_discontinuity(&self, source: u64) {
        if self
            .config
            .as_ref()
            .is_some_and(|c| c.active && c.sources.contains(&source))
        {
            self.restart();
        }
    }
    fn recover(&mut self) {
        self.shared.state.store(LOADING, Ordering::Release);
        self.restart();
        self.current = None;
        self.primed = false;
        self.fade = 0.0;
    }
    /// No allocations, locks, model work, process operations or thread joins.
    pub fn process(&mut self, node: &Node, frames: &mut [Frame]) {
        if node.effects_disabled {
            if self.config.as_ref().is_none_or(|c| c.id != node.id) {
                return; // Independent dry route; do not touch active voice state.
            }
            if !self.disabled {
                self.fade = 0.0;
                self.disabled = true;
            }
            for frame in frames {
                self.fade = (self.fade + 1.0 / 240.0).min(1.0);
                for (sample, last) in frame.iter_mut().zip(self.last) {
                    *sample = *sample * self.fade + last * (1.0 - self.fade);
                }
            }
            return;
        }
        self.disabled = false;
        if self
            .config
            .as_ref()
            .is_none_or(|c| c.id != node.id || !c.active)
        {
            frames.fill([0.0; 2]);
            return;
        }
        let generation = self.shared.generation.load(Ordering::Acquire);
        if generation != self.generation {
            self.generation = generation;
            self.sequence = 0;
            self.expected = 0;
            self.current = None;
            self.offset = 0;
            self.primed = false;
            self.fade = 0.0;
            self.last = [0.0; 2];
            // Old blocks have generation tags. The supervisor is the sole input
            // consumer, so render-side flushing touches only the output queue.
            while self.shared.output.pop().is_some() {}
        }
        let state = self.shared.state.load(Ordering::Acquire);
        if state != PRIMING && state != READY {
            self.last_call = None;
            frames.fill([0.0; 2]);
            return;
        }
        if frames.len() != BLOCK_SIZE {
            frames.fill([0.0; 2]);
            return;
        }
        let now = Instant::now();
        if let Some(previous) = self.last_call.replace(now) {
            self.shared.max_render_gap_us.fetch_max(
                now.duration_since(previous).as_micros() as u64,
                Ordering::Relaxed,
            );
        }
        let mut input = Block {
            generation,
            sequence: self.sequence,
            born: Instant::now(),
            frames: [[0.0; 2]; BLOCK_SIZE],
            valid: BLOCK_SIZE,
        };
        input.frames.copy_from_slice(frames);
        self.sequence += 1;
        if self.shared.input.push(input).is_err() {
            self.recover();
            frames.fill([0.0; 2]);
            return;
        }
        if !self.primed {
            // Windows can deliver/render more than a hop in a single burst.
            // Keep two reserved hops after the current block. A 1248-frame
            // hop occupies ten blocks; startup remains below 100ms on target.
            if self.shared.output.len() < 21 {
                frames.fill([0.0; 2]);
                return;
            }
            self.primed = true;
            self.shared.state.store(READY, Ordering::Release);
        }
        for frame in frames {
            if self.shared.generation.load(Ordering::Acquire) != generation {
                *frame = [0.0; 2];
                continue;
            }
            if self.current.as_ref().is_none_or(|b| self.offset == b.valid) {
                self.current = None;
                self.offset = 0;
                while let Some(block) = self.shared.output.pop() {
                    if block.generation != generation {
                        continue;
                    }
                    if block.sequence != self.expected || block.born.elapsed() > MAX_AGE {
                        self.recover();
                        break;
                    }
                    self.expected += 1;
                    self.current = Some(block);
                    break;
                }
            }
            if let Some(block) = &self.current {
                self.fade = (self.fade + 1.0 / 240.0).min(1.0);
                *frame = block.frames[self.offset].map(|s| s * self.fade);
                self.offset += 1;
                self.last = *frame;
            } else {
                *frame = [0.0; 2];
                self.fade = 0.0;
                if self.primed {
                    self.shared.underruns.fetch_add(1, Ordering::Relaxed);
                    self.primed = false;
                    self.shared.state.store(PRIMING, Ordering::Release);
                }
            }
        }
    }
    pub fn status(&mut self) -> Option<(u64, String)> {
        while let Some(message) = self.shared.messages.pop() {
            self.reason = message;
        }
        self.config.as_ref().map(|c| {
            let label = match self.shared.state.load(Ordering::Acquire) {
                LOADING => "Loading",
                PRIMING => "Recovering",
                READY if c.voice.backend == VoiceBackend::DirectMl => {
                    "Ready · DirectML transformer + CPU encoder/synthesis"
                }
                READY => "Ready · CPU / LibTorch 2.5.1",
                DISABLED => "Stopped (passthrough or disconnected)",
                5 => "Missing pack",
                _ => "Error",
            };
            (
                c.id,
                format!(
                    "{label}. {} · underruns {} · resets {} · max inference {:.1} ms · max render gap {:.1} ms",
                    self.reason,
                    self.shared.underruns.load(Ordering::Relaxed),
                    self.shared.resets.load(Ordering::Relaxed),
                    self.shared.max_inference_us.load(Ordering::Relaxed) as f64 / 1000.0,
                    self.shared.max_render_gap_us.load(Ordering::Relaxed) as f64 / 1000.0
                ),
            )
        })
    }
}
impl Drop for Rack {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        self.restart();
    }
}
fn message(shared: &Shared, text: String) {
    crate::logging::warn(format_args!("Voice worker: {text}"));
    if shared.messages.is_full() {
        shared.messages.pop();
    }
    let _ = shared.messages.push(text);
}
// Two bounded channel slots keep audio inline without a new box every hop.
#[allow(clippy::large_enum_variant)]
enum Reply {
    Ready,
    Audio([f32; HOP]),
    Failed,
}
struct Worker {
    child: Child,
    #[cfg(windows)]
    _job: job::Job,
    input: std::process::ChildStdin,
    replies: mpsc::Receiver<Reply>,
}
impl Worker {
    fn open(path: &std::path::Path, backend: VoiceBackend) -> anyhow::Result<Self> {
        Self::open_model(path, backend, &path.join("llvc-stream.pt"), None)
    }
    fn open_model(
        path: &std::path::Path,
        backend: VoiceBackend,
        model: &std::path::Path,
        worker: Option<std::path::PathBuf>,
    ) -> anyhow::Result<Self> {
        let executable = if backend == VoiceBackend::DirectMl {
            "PatchVoiceDmlWorker.exe"
        } else {
            "PatchVoiceWorker.exe"
        };
        let mut command = Command::new(worker.unwrap_or_else(|| path.join(executable)));
        let paths = std::env::join_paths(std::iter::once(path.to_path_buf()).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        ))?;
        command
            .arg(model)
            .env("PATH", paths)
            .current_dir(path)
            .env("KMP_BLOCKTIME", "0")
            .env("OMP_WAIT_POLICY", "PASSIVE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn()?;
        #[cfg(windows)]
        let job = match job::Job::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let input = child.stdin.take().unwrap();
        let mut output = child.stdout.take().unwrap();
        let (tx, replies) = mpsc::sync_channel(2);
        thread::spawn(move || {
            let mut ready = [0_u8; 8];
            if output.read_exact(&mut ready).is_err() || ready != *b"PATCHVC1" {
                let _ = tx.send(Reply::Failed);
                return;
            }
            if tx.send(Reply::Ready).is_err() {
                return;
            }
            loop {
                let mut data = [0_u8; HOP * 4];
                if output.read_exact(&mut data).is_err() {
                    let _ = tx.send(Reply::Failed);
                    return;
                }
                let values = std::array::from_fn(|i| {
                    f32::from_le_bytes(data[i * 4..i * 4 + 4].try_into().unwrap())
                });
                if tx.send(Reply::Audio(values)).is_err() {
                    return;
                }
            }
        });
        Ok(Self {
            child,
            #[cfg(windows)]
            _job: job,
            input,
            replies,
        })
    }
    fn receive(
        &self,
        shared: &Shared,
        generation: u64,
        timeout: Duration,
    ) -> anyhow::Result<Reply> {
        let start = Instant::now();
        loop {
            anyhow::ensure!(
                !shared.stop.load(Ordering::Acquire)
                    && shared.generation.load(Ordering::Acquire) == generation,
                "Voice reset"
            );
            anyhow::ensure!(
                start.elapsed() < timeout,
                "Voice worker timed out; retrying automatically"
            );
            match self.replies.recv_timeout(Duration::from_millis(10)) {
                Ok(v) => return Ok(v),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => anyhow::bail!("Voice worker exited"),
            }
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn supervise(shared: Arc<Shared>) {
    let mut generation = 0;
    let mut retry_delay = Duration::from_secs(1);
    while !shared.stop.load(Ordering::Acquire) {
        let current = shared.generation.load(Ordering::Acquire);
        if current == generation {
            thread::sleep(Duration::from_millis(2));
            continue;
        }
        generation = current;
        while shared.input.pop().is_some() {}
        let desired = shared.desired.load_full();
        let Some(config) = desired.as_ref().filter(|c| c.active) else {
            shared.state.store(DISABLED, Ordering::Release);
            continue;
        };
        shared.state.store(LOADING, Ordering::Release);
        crate::logging::info(format_args!(
            "Starting voice worker generation {generation}"
        ));
        let started = Instant::now();
        let result = run_generation(&shared, generation, config);
        shared.worker_pid.store(0, Ordering::Release);
        if shared.generation.load(Ordering::Acquire) == generation {
            if let Err(error) = result {
                crate::logging::error(format_args!(
                    "Voice worker generation {generation} failed: {error:#}"
                ));
                let text = format!("{error:#}");
                let missing = text.contains("Voice pack is missing")
                    || text.starts_with("Missing voice pack:");
                message(&shared, text);
                shared
                    .state
                    .store(if missing { 5 } else { ERROR }, Ordering::Release);
                if !missing {
                    if started.elapsed() > Duration::from_secs(60) {
                        retry_delay = Duration::from_secs(1);
                    }
                    let until = Instant::now() + retry_delay;
                    while Instant::now() < until
                        && !shared.stop.load(Ordering::Acquire)
                        && shared.generation.load(Ordering::Acquire) == generation
                    {
                        thread::sleep(Duration::from_millis(20));
                    }
                    // A user stop/configuration change always wins over automatic retries.
                    if !shared.stop.load(Ordering::Acquire) {
                        let _ = shared.generation.compare_exchange(
                            generation,
                            generation.wrapping_add(1),
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        );
                    }
                    retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
                }
            }
        }
    }
}
fn run_generation(shared: &Shared, generation: u64, config: &Config) -> anyhow::Result<()> {
    let custom = config.voice.pack != piratewave_core::voice::LLVC_PACK;
    let custom_model = if custom {
        anyhow::ensure!(
            config.voice.backend != VoiceBackend::DirectMl,
            "This custom voice has CPU validation only. Choose Auto / CPU."
        );
        Some(local::resolve(&config.voice.pack)?)
    } else {
        None
    };
    #[cfg(test)]
    let test_path = shared.test_pack.get().cloned();
    #[cfg(not(test))]
    let test_path: Option<std::path::PathBuf> = None;
    let path = if let Some(path) = test_path {
        pack::verify(&path)?;
        path
    } else {
        let path = if custom {
            crate::training::release::native_runtime()?
        } else if config.voice.backend != VoiceBackend::DirectMl {
            pack::load_cpu_voice()?
        } else {
            pack::load()?
        };
        if !custom {
            anyhow::ensure!(pack::cpu_matches(),"CPU profile is unqualified on this processor; run qualification before enabling this pack");
            let manifest = pack::manifest();
            pack::require_backend(&manifest, config.voice.backend)?;
        }
        path
    };
    let mut worker = if let Some(model) = custom_model {
        Worker::open_model(
            &path,
            VoiceBackend::Cpu,
            &model,
            Some(path.join("PatchVoiceWorker.exe")),
        )?
    } else {
        Worker::open(&path, config.voice.backend)?
    };
    shared
        .worker_pid
        .store(worker.child.id() as u64, Ordering::Release);
    anyhow::ensure!(
        matches!(
            worker.receive(shared, generation, Duration::from_secs(15))?,
            Reply::Ready
        ),
        "{}",
        if config.voice.backend == VoiceBackend::DirectMl {
            "DirectML worker failed to load. This profile requires the qualified RTX 4070 SUPER and matching GPU voice pack. Choose CPU / Auto or restart the voice."
        } else {
            "CPU voice worker failed to load. Reinstall the matching voice pack or restart the voice."
        }
    );
    if shared.generation.load(Ordering::Acquire) != generation {
        return Ok(());
    }
    message(
        shared,
        if qualification_mode() {
            "LLVC voice — Beta · qualification session; release acceptance pending".into()
        } else {
            "LLVC voice — Beta".into()
        },
    );
    let _audio_thread = crate::audio_scheduling::AudioThread::register();
    shared.state.store(PRIMING, Ordering::Release);
    let mut resampler = resample::Resampler::default();
    let mut packet = [0.0_f32; HOP + CONTEXT];
    let mut at = CONTEXT;
    let mut sequence = 0;
    let mut output_sequence = 0;
    let mut born = Instant::now();
    loop {
        if shared.stop.load(Ordering::Acquire)
            || shared.generation.load(Ordering::Acquire) != generation
        {
            return Ok(());
        }
        let Some(input) = shared.input.pop() else {
            thread::sleep(Duration::from_micros(250));
            continue;
        };
        if input.generation != generation {
            continue;
        }
        if input.sequence != sequence || input.born.elapsed() > MAX_AGE {
            shared
                .generation
                .compare_exchange(
                    generation,
                    generation + 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .ok();
            return Ok(());
        }
        sequence += 1;
        if at == CONTEXT {
            born = input.born;
        }
        for frame in input.frames {
            if let Some(sample) = resampler.down((frame[0] + frame[1]) * 0.5) {
                packet[at] = sample;
                at += 1;
                if at == packet.len() {
                    let mut bytes = [0_u8; (HOP + CONTEXT) * 4];
                    for (i, sample) in packet.iter().enumerate() {
                        bytes[i * 4..i * 4 + 4].copy_from_slice(&sample.to_le_bytes());
                    }
                    let inference_started = Instant::now();
                    worker.input.write_all(&bytes)?;
                    worker.input.flush()?;
                    let Reply::Audio(converted) = worker.receive(shared, generation, MAX_AGE)?
                    else {
                        anyhow::bail!("Voice worker failed during conversion");
                    };
                    shared.max_inference_us.fetch_max(
                        inference_started.elapsed().as_micros() as u64,
                        Ordering::Relaxed,
                    );
                    anyhow::ensure!(
                        converted.iter().all(|s| s.is_finite()),
                        "Nonfinite converted audio"
                    );
                    let mut block = Block {
                        generation,
                        sequence: output_sequence,
                        born,
                        frames: [[0.0; 2]; BLOCK_SIZE],
                        valid: 0,
                    };
                    for sample in converted {
                        for value in resampler.up(sample) {
                            block.frames[block.valid] = [value; 2];
                            block.valid += 1;
                            if block.valid == BLOCK_SIZE {
                                enqueue(shared, block)?;
                                output_sequence += 1;
                                block.sequence = output_sequence;
                                block.valid = 0;
                            }
                        }
                    }
                    if block.valid > 0 {
                        enqueue(shared, block)?;
                        output_sequence += 1;
                    }
                    packet.copy_within(HOP.., 0);
                    at = CONTEXT;
                    born = input.born;
                }
            }
        }
    }
}
fn enqueue(shared: &Shared, block: Block) -> anyhow::Result<()> {
    if shared.output.push(block).is_err() {
        if shared
            .generation
            .compare_exchange(
                block.generation,
                block.generation + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            shared.state.store(LOADING, Ordering::Release);
        }
        anyhow::bail!("Converted backlog flushed");
    }
    Ok(())
}
