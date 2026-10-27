//! Session media I/O. All files, encoding, waveforms and metadata live off render.
use anyhow::{ensure, Context, Result};
use crossbeam_queue::ArrayQueue;
use piratewave_core::{
    automation::Context as ActionContext, recording::SessionSnapshot, workflows::Track, Frame,
    BLOCK_SIZE, SAMPLE_RATE,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread,
};
pub const MAX_TRACKS: usize = 32;
const BUFFER: usize = 64 * 1024;
const HEADER: u64 = 92;
const POOL_BYTES: usize = 27 * 1024 * 1024;
pub struct Packet {
    pub frames: [[Frame; BLOCK_SIZE]; MAX_TRACKS],
    pub frame: u64,
    pub discontinuity: bool,
}
/// Compiled off the render path. The render method only copies into owned pool slots.
pub struct Producer {
    pub pool: Arc<Pool>,
    points: Vec<(usize, piratewave_core::workflows::CapturePoint)>,
}
impl Producer {
    pub fn new(
        pool: Arc<Pool>,
        tracks: &[Track],
        graph: &piratewave_core::graph::Graph,
    ) -> Result<Self> {
        ensure!(
            !tracks.is_empty() && tracks.len() <= MAX_TRACKS,
            "Invalid recording track count"
        );
        let points = tracks
            .iter()
            .map(|t| {
                let index = graph
                    .nodes
                    .iter()
                    .position(|n| n.config.id == t.point.node)
                    .context("Recording target disappeared")?;
                ensure!(
                    t.point.stage == piratewave_core::workflows::CaptureStage::Raw
                        || graph.nodes[index].output(t.point.port).is_some(),
                    "Recording output disappeared"
                );
                Ok((index, t.point))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { pool, points })
    }
    pub fn remap(&mut self, graph: &piratewave_core::graph::Graph) -> Result<()> {
        for (index, point) in &mut self.points {
            *index = graph
                .nodes
                .iter()
                .position(|n| n.config.id == point.node)
                .context("Recording source was removed")?;
            ensure!(
                point.stage == piratewave_core::workflows::CaptureStage::Raw
                    || graph.nodes[*index].output(point.port).is_some(),
                "Recording port was removed"
            );
        }
        Ok(())
    }
    pub fn mark_raw(&self, graph: &mut piratewave_core::graph::Graph) {
        for (index, point) in &self.points {
            if point.stage == piratewave_core::workflows::CaptureStage::Raw {
                graph.nodes[*index].capture_raw = true;
            }
        }
    }
    pub fn copy_block(
        &self,
        graph: &piratewave_core::graph::Graph,
        frame: u64,
        discontinuity: bool,
    ) {
        if self.pool.stopped.load(Ordering::Acquire) {
            return;
        }
        let Some(mut packet) = self.pool.free.pop() else {
            self.pool.fault.store(1, Ordering::Release);
            self.pool.stop();
            return;
        };
        packet.frame = frame;
        packet.discontinuity = discontinuity;
        for (track, (index, point)) in self.points.iter().enumerate() {
            let node = &graph.nodes[*index];
            if point.stage == piratewave_core::workflows::CaptureStage::Raw {
                packet.frames[track].copy_from_slice(&node.raw_frames);
            } else if let Some(frames) = node.output(point.port) {
                packet.frames[track].copy_from_slice(frames);
            } else {
                packet.frames[track].fill([0.; 2]);
            }
        }
        self.pool
            .start
            .compare_exchange(u64::MAX, frame, Ordering::AcqRel, Ordering::Acquire)
            .ok();
        self.pool
            .end
            .store(frame + BLOCK_SIZE as u64, Ordering::Release);
        if let Err(packet) = self.pool.pending.push(packet) {
            let _ = self.pool.free.push(packet);
            self.pool.fault.store(1, Ordering::Release);
            self.pool.stop();
        }
    }
}
impl Drop for Producer {
    fn drop(&mut self) {
        self.pool.stop();
    }
}
pub struct Pool {
    marker_count: AtomicUsize,
    pub markers: ArrayQueue<Marker>,
    pub free: ArrayQueue<Box<Packet>>,
    pub pending: ArrayQueue<Box<Packet>>,
    pub stopped: AtomicBool,
    pub fault: AtomicU8,
    pub start: AtomicU64,
    pub end: AtomicU64,
}
impl Pool {
    fn new() -> Self {
        let count = POOL_BYTES / std::mem::size_of::<Packet>();
        let free = ArrayQueue::new(count);
        for _ in 0..count {
            let _ = free.push(Box::new(Packet {
                frames: [[[0.; 2]; BLOCK_SIZE]; MAX_TRACKS],
                frame: 0,
                discontinuity: false,
            }));
        }
        Self {
            marker_count: AtomicUsize::new(0),
            markers: ArrayQueue::new(256),
            free,
            pending: ArrayQueue::new(count),
            stopped: AtomicBool::new(false),
            fault: AtomicU8::new(0),
            start: AtomicU64::new(u64::MAX),
            end: AtomicU64::new(0),
        }
    }
    pub fn add_marker(&self, marker: Marker) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) || self.start.load(Ordering::Acquire) == u64::MAX {
            return Err("Wait until this take is recording before adding a marker".into());
        }
        if marker.name.trim().is_empty() || marker.name.len() > 256 {
            return Err("Marker name must contain 1-256 characters".into());
        }
        let count = self.marker_count.fetch_add(1, Ordering::AcqRel);
        if count >= 10000 {
            self.marker_count.fetch_sub(1, Ordering::AcqRel);
            return Err("This take already has 10,000 markers".into());
        }
        self.markers.push(marker).map_err(|_| {
            self.marker_count.fetch_sub(1, Ordering::AcqRel);
            "Marker queue is full; try again".into()
        })
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Marker {
    pub frame: u64,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TakeTrack {
    pub track: Track,
    pub file: String,
    #[serde(default)]
    pub waveform: Vec<[f32; 2]>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Take {
    pub schema: u32,
    pub id: String,
    pub workspace: u64,
    pub session: u64,
    pub name: String,
    pub tracks: Vec<TakeTrack>,
    pub start: u64,
    pub end: u64,
    pub complete: bool,
    pub error: Option<String>,
    pub markers: Vec<Marker>,
    pub discontinuities: Vec<u64>,
}
pub enum Event {
    Listed {
        context: ActionContext,
        takes: Vec<Take>,
    },
    Updated {
        context: ActionContext,
        take: Take,
    },
    Prepared {
        context: ActionContext,
        take: Take,
        pool: Arc<Pool>,
    },
    Finished {
        context: ActionContext,
        take: Take,
    },
    Failed {
        context: ActionContext,
        session: Option<u64>,
        error: String,
    },
    Exported {
        context: ActionContext,
        path: PathBuf,
    },
}
pub enum Task {
    List {
        context: ActionContext,
        ids: Vec<String>,
    },
    Rename {
        context: ActionContext,
        id: String,
        name: String,
    },
    Capture {
        context: ActionContext,
        workspace: u64,
        session: u64,
        name: String,
        snapshot: SessionSnapshot,
    },
    Export {
        context: ActionContext,
        id: String,
        destination: PathBuf,
        start: u64,
        end: u64,
    },
    Recover {
        context: ActionContext,
        id: String,
    },
    Delete {
        context: ActionContext,
        id: String,
    },
}
pub struct Service {
    pub events: mpsc::Receiver<Event>,
    pub tasks: mpsc::SyncSender<Task>,
    sender: mpsc::SyncSender<Event>,
    live: Arc<AtomicBool>,
    root: PathBuf,
    shutdown: Arc<AtomicBool>,
    pub generation: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    active_pool: Arc<arc_swap::ArcSwapOption<Pool>>,
    previous_pool: Arc<std::sync::Mutex<std::sync::Weak<Pool>>>,
}
impl Default for Service {
    fn default() -> Self {
        Self::new(crate::storage::default_path().with_file_name("takes"))
    }
}
impl Service {
    pub fn new(root: PathBuf) -> Self {
        let generation = Arc::new(AtomicU64::new(0));
        let current_generation = generation.clone();
        let (sender, events) = mpsc::sync_channel(64);
        let (tasks, rx) = mpsc::sync_channel(8);
        let dest = sender.clone();
        let folder = root.clone();
        thread::Builder::new()
            .name("piratewave-take-files".into())
            .spawn(move || {
                while let Ok(task) = rx.recv() {
                    let context = match &task {
                        Task::List { context, .. }
                        | Task::Rename { context, .. }
                        | Task::Capture { context, .. }
                        | Task::Export { context, .. }
                        | Task::Recover { context, .. }
                        | Task::Delete { context, .. } => *context,
                    };
                    let session = match &task {
                        Task::Capture { session, .. } => Some(*session),
                        _ => None,
                    };
                    let result = match task {
                        Task::List { ids, .. } => {
                            let mut takes = Vec::new();
                            let mut selected = ids.into_iter().take(64).collect::<Vec<_>>();
                            if let Ok(entries) = fs::read_dir(&folder) {
                                for entry in entries.flatten() {
                                    if selected.len() >= 128 {
                                        break;
                                    }
                                    if entry.path().join("incomplete.json").exists() {
                                        if let Some(id) = entry.file_name().to_str() {
                                            if !selected.iter().any(|s| s == id) {
                                                selected.push(id.to_owned());
                                            }
                                        }
                                    }
                                }
                            }
                            for id in selected {
                                if let Ok(mut take) = load(&folder, &id) {
                                    for track in &mut take.tracks {
                                        track.waveform.clear();
                                    }
                                    take.markers.clear();
                                    take.discontinuities.clear();
                                    takes.push(take);
                                }
                            }
                            Ok(Event::Listed { context, takes })
                        }
                        Task::Capture {
                            workspace,
                            session,
                            name,
                            snapshot,
                            ..
                        } => capture(
                            &folder,
                            context,
                            workspace,
                            session,
                            name,
                            snapshot,
                            &current_generation,
                        )
                        .map(|take| Event::Finished { context, take }),
                        Task::Export {
                            id,
                            destination,
                            start,
                            end,
                            ..
                        } => export(&folder, &id, &destination, start, end).map(|()| {
                            Event::Exported {
                                context,
                                path: destination,
                            }
                        }),
                        Task::Rename { id, name, .. } => {
                            rename(&folder, &id, name).map(|take| Event::Updated { context, take })
                        }
                        Task::Recover { id, .. } => {
                            recover(&folder, &id).map(|take| Event::Updated { context, take })
                        }
                        Task::Delete { id, .. } => delete(&folder, &id).map(|()| Event::Exported {
                            context,
                            path: folder.join(id),
                        }),
                    };
                    let event = result.unwrap_or_else(|error| Event::Failed {
                        context,
                        session,
                        error: format!("{error:#}"),
                    });
                    if dest.send(event).is_err() {
                        break;
                    }
                }
            })
            .expect("take file worker");
        Self {
            events,
            tasks,
            sender,
            live: Arc::new(AtomicBool::new(false)),
            root,
            shutdown: Arc::new(AtomicBool::new(false)),
            generation,
            cancel: Arc::new(AtomicBool::new(false)),
            active_pool: Arc::new(arc_swap::ArcSwapOption::empty()),
            previous_pool: Arc::new(std::sync::Mutex::new(std::sync::Weak::new())),
        }
    }
    pub fn start(
        &self,
        context: ActionContext,
        workspace: u64,
        session: u64,
        name: String,
        tracks: Vec<Track>,
    ) -> Result<()> {
        ensure!(
            !name.trim().is_empty() && name.len() <= 256,
            "Take name must contain 1-256 characters"
        );
        ensure!(
            !tracks.is_empty() && tracks.len() <= MAX_TRACKS,
            "Select between 1 and 32 tracks"
        );
        ensure!(
            !self.live.swap(true, Ordering::AcqRel),
            "Another live take is already recording or finalizing"
        );
        self.cancel.store(false, Ordering::Release);
        let cancel = self.cancel.clone();
        let active_pool = self.active_pool.clone();
        let previous_pool = self.previous_pool.clone();
        let sender = self.sender.clone();
        let root = self.root.clone();
        let busy = self.live.clone();
        let shutdown = self.shutdown.clone();
        let result = thread::Builder::new()
            .name("piratewave-live-take".into())
            .spawn(move || {
                let result = (|| -> Result<Take> {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                    loop {
                        let available = previous_pool
                            .lock()
                            .map_err(|_| anyhow::anyhow!("Recording pool guard failed"))?
                            .strong_count()
                            == 0;
                        if available {
                            break;
                        }
                        ensure!(
                            !cancel.load(Ordering::Acquire) && !shutdown.load(Ordering::Acquire),
                            "Recording start cancelled"
                        );
                        ensure!(
                            std::time::Instant::now() < deadline,
                            "Previous take is still releasing transfer buffers; retry recording"
                        );
                        thread::sleep(std::time::Duration::from_millis(2));
                    }
                    ensure!(!cancel.load(Ordering::Acquire), "Recording start cancelled");
                    let pool = Arc::new(Pool::new());
                    *previous_pool
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Recording pool guard failed"))? =
                        Arc::downgrade(&pool);
                    active_pool.store(Some(pool.clone()));
                    let _guard = LiveGuard(pool.clone());
                    let mut take = new_take(workspace, session, name, tracks);
                    let mut writers = open_take(&root, &take)?;
                    ensure!(!cancel.load(Ordering::Acquire), "Recording start cancelled");
                    // Publication happens only after all files and the incomplete manifest exist.
                    if sender
                        .send(Event::Prepared {
                            context,
                            take: take.clone(),
                            pool: pool.clone(),
                        })
                        .is_err()
                    {
                        return Err(anyhow::anyhow!("Recording owner closed"));
                    }
                    let mut checkpoint = std::time::Instant::now();
                    let mut expected = None;
                    let mut previous_discontinuity = false;
                    loop {
                        while let Some(marker) = pool.markers.pop() {
                            if take.markers.len() < 10000 {
                                take.markers.push(marker);
                            }
                        }
                        if checkpoint.elapsed() > std::time::Duration::from_secs(1) {
                            save(&folder(&root, &take.id)?.join("incomplete.json"), &take)?;
                            checkpoint = std::time::Instant::now();
                        }
                        if shutdown.load(Ordering::Acquire) {
                            pool.fault.store(3, Ordering::Release);
                            pool.stop();
                        }
                        if let Some(packet) = pool.pending.pop() {
                            let expected = expected.get_or_insert(packet.frame);
                            ensure!(
                                *expected == packet.frame,
                                "Live recording transfer lost synchronization"
                            );
                            if take.start == 0 && take.end == 0 {
                                take.start = packet.frame;
                            }
                            for (index, writer) in writers.iter_mut().enumerate() {
                                writer.write_frames(&packet.frames[index])?;
                            }
                            if packet.discontinuity
                                && !previous_discontinuity
                                && take.discontinuities.len() < 10000
                            {
                                take.discontinuities.push(packet.frame);
                            }
                            previous_discontinuity = packet.discontinuity;
                            *expected += BLOCK_SIZE as u64;
                            take.end = *expected;
                            let _ = pool.free.push(packet);
                        } else if pool.stopped.load(Ordering::Acquire) {
                            break;
                        } else {
                            thread::sleep(std::time::Duration::from_millis(1));
                        }
                        if pool.fault.load(Ordering::Acquire) != 0 {
                            pool.stop();
                        }
                    }
                    for writer in &mut writers {
                        writer.finish()?;
                    }
                    take.complete = pool.fault.load(Ordering::Acquire) == 0;
                    if !take.complete {
                        take.error=Some(match pool.fault.load(Ordering::Acquire){1=>"Recording transfer queue filled; the take stopped without blocking audio",4=>"A recording capture point was removed; available audio was preserved",_=>"Recording was interrupted; available audio was preserved"}.into());
                    }
                    finish_take(&root, &mut take)?;
                    Ok(take)
                })();
                active_pool.store(None);
                busy.store(false, Ordering::Release);
                let event = result
                    .map(|take| Event::Finished { context, take })
                    .unwrap_or_else(|error| Event::Failed {
                        context,
                        session: Some(session),
                        error: format!("Recording failed: {error:#}"),
                    });
                let _ = sender.send(event);
            });
        if let Err(error) = result {
            self.live.store(false, Ordering::Release);
            return Err(error.into());
        }
        Ok(())
    }
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(pool) = self.active_pool.load_full() {
            pool.stop();
        }
    }
    pub fn busy(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }
    pub fn load(&self, id: &str) -> Result<Take> {
        load(&self.root, id)
    }
}
pub fn new_take(workspace: u64, session: u64, name: String, tracks: Vec<Track>) -> Take {
    Take {
        schema: 1,
        id: format!(
            "take-{}",
            piratewave_core::recording::RecordSlot::new("").id
        ),
        workspace,
        session,
        name,
        tracks: tracks
            .into_iter()
            .enumerate()
            .map(|(i, track)| TakeTrack {
                track,
                file: format!("track-{:02}.wav", i + 1),
                waveform: vec![],
            })
            .collect(),
        start: 0,
        end: 0,
        complete: false,
        error: None,
        markers: vec![],
        discontinuities: vec![],
    }
}
fn folder(root: &Path, id: &str) -> Result<PathBuf> {
    ensure!(
        id.starts_with("take-")
            && id.len() <= 64
            && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "Invalid take identity"
    );
    Ok(root.join(id))
}
fn save(path: &Path, take: &Take) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(take)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}
fn open_take(root: &Path, take: &Take) -> Result<Vec<FloatWriter>> {
    let folder = folder(root, &take.id)?;
    fs::create_dir_all(root)?;
    fs::create_dir(&folder)?;
    save(&folder.join("incomplete.json"), take)?;
    take.tracks
        .iter()
        .map(|track| FloatWriter::create(&folder.join(&track.file)))
        .collect()
}
fn finish_take(root: &Path, take: &mut Take) -> Result<()> {
    let dir = folder(root, &take.id)?;
    for track in &mut take.tracks {
        track.waveform = waveform(&dir.join(&track.file))?;
    }
    save(&dir.join("take.json"), take)?;
    if take.complete && dir.join("incomplete.json").exists() {
        fs::remove_file(dir.join("incomplete.json"))?;
    }
    Ok(())
}
fn capture(
    root: &Path,
    context: ActionContext,
    workspace: u64,
    session: u64,
    name: String,
    snapshot: SessionSnapshot,
    generation: &AtomicU64,
) -> Result<Take> {
    let mut take = new_take(
        workspace,
        session,
        name,
        snapshot.tracks.iter().map(|(t, _)| t.clone()).collect(),
    );
    take.start = snapshot.start * BLOCK_SIZE as u64;
    take.end = snapshot.end * BLOCK_SIZE as u64;
    let mut writers = open_take(root, &take)?;
    let result = (|| -> Result<()> {
        for index in snapshot.start..snapshot.end {
            ensure!(
                generation.load(Ordering::Acquire) == context.generation,
                "Workspace changed; capture cancelled"
            );
            for ((_, history), writer) in snapshot.tracks.iter().zip(&mut writers) {
                writer.write_frames(&history.read_block(index).map_err(anyhow::Error::msg)?)?;
            }
        }
        for writer in &mut writers {
            writer.finish()?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        take.error = Some(format!("Capture failed: {error:#}"));
        save(&folder(root, &take.id)?.join("incomplete.json"), &take)?;
        return Err(error);
    }
    take.complete = true;
    finish_take(root, &mut take)?;
    Ok(take)
}
fn load(root: &Path, id: &str) -> Result<Take> {
    let dir = folder(root, id)?;
    let path = if dir.join("take.json").exists() {
        dir.join("take.json")
    } else {
        dir.join("incomplete.json")
    };
    ensure!(
        fs::metadata(&path)?.len() <= 4 * 1024 * 1024,
        "Take manifest is too large"
    );
    let take: Take = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        take.id == id && take.schema == 1 && take.tracks.len() <= MAX_TRACKS,
        "Invalid take manifest"
    );
    ensure!(
        take.tracks
            .iter()
            .enumerate()
            .all(|(i, t)| t.file == format!("track-{:02}.wav", i + 1))
            && take.name.len() <= 256
            && take.markers.len() <= 10000
            && take.discontinuities.len() <= 10000,
        "Invalid take contents"
    );
    Ok(take)
}
fn delete(root: &Path, id: &str) -> Result<()> {
    let dir = folder(root, id)?.canonicalize()?;
    ensure!(
        dir.parent() == Some(root.canonicalize()?.as_path()),
        "Take is outside storage"
    );
    fs::remove_dir_all(dir)?;
    Ok(())
}
fn recover(root: &Path, id: &str) -> Result<Take> {
    let mut take = load(root, id)?;
    let dir = folder(root, id)?;
    let frames = take
        .tracks
        .iter()
        .map(|t| fs::metadata(dir.join(&t.file)).map(|m| m.len().saturating_sub(HEADER) / 8))
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .min()
        .unwrap_or(0);
    ensure!(frames > 0, "No recoverable audio in this take");
    for track in &take.tracks {
        let path = dir.join(&track.file);
        let mut file = fs::OpenOptions::new().read(true).write(true).open(path)?;
        file.set_len(HEADER + frames * 8)?;
        write_header(&mut file, frames * 8)?;
        file.sync_all()?;
    }
    take.end = take.start + frames;
    take.complete = true;
    take.error = Some(
        "Recovered after interruption; tracks trimmed to their common available duration".into(),
    );
    finish_take(root, &mut take)?;
    Ok(take)
}
fn rename(root: &Path, id: &str, name: String) -> Result<Take> {
    ensure!(
        !name.trim().is_empty() && name.len() <= 256,
        "Take name must contain 1-256 characters"
    );
    let mut take = load(root, id)?;
    ensure!(take.complete, "Finish or recover the take before renaming");
    take.name = name;
    save(&folder(root, id)?.join("take.json"), &take)?;
    Ok(take)
}
fn export(root: &Path, id: &str, destination: &Path, start: u64, end: u64) -> Result<()> {
    let mut take = load(root, id)?;
    ensure!(take.complete, "Recover the take before exporting");
    let available = take.end.saturating_sub(take.start);
    let end = end.min(available);
    ensure!(start < end, "Choose a nonempty selection");
    // Never overwrite or partially mix an existing export directory.
    fs::create_dir(destination).context("Choose a new export folder")?;
    for track in &take.tracks {
        let mut source = File::open(folder(root, id)?.join(&track.file))?;
        source.seek(SeekFrom::Start(HEADER + start * 8))?;
        let mut writer = FloatWriter::create(&destination.join(&track.file))?;
        let mut remaining = end - start;
        let mut bytes = [0u8; BLOCK_SIZE * 8];
        while remaining > 0 {
            let frames = remaining.min(BLOCK_SIZE as u64) as usize;
            source.read_exact(&mut bytes[..frames * 8])?;
            writer.writer.write_all(&bytes[..frames * 8])?;
            writer.bytes += (frames * 8) as u64;
            remaining -= frames as u64;
        }
        writer.finish()?;
    }
    take.start += start;
    take.end = take.start + (end - start);
    take.markers
        .retain(|m| m.frame >= take.start && m.frame <= take.end);
    take.discontinuities
        .retain(|f| *f >= take.start && *f < take.end);
    save(&destination.join("take.json"), &take)?;
    Ok(())
}
pub struct FloatWriter {
    writer: BufWriter<File>,
    bytes: u64,
}
impl FloatWriter {
    fn create(path: &Path) -> Result<Self> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .open(path)?;
        write_header(&mut file, 0)?;
        file.seek(SeekFrom::Start(HEADER))?;
        Ok(Self {
            writer: BufWriter::with_capacity(BUFFER, file),
            bytes: 0,
        })
    }
    fn write_frames(&mut self, frames: &[Frame]) -> Result<()> {
        let mut bytes = [0u8; BLOCK_SIZE * 8];
        for chunk in frames.chunks(BLOCK_SIZE) {
            for (i, frame) in chunk.iter().enumerate() {
                for (c, sample) in frame.iter().enumerate() {
                    bytes[i * 8 + c * 4..i * 8 + c * 4 + 4].copy_from_slice(
                        &if sample.is_finite() { *sample } else { 0. }.to_le_bytes(),
                    );
                }
            }
            self.writer.write_all(&bytes[..chunk.len() * 8])?;
            self.bytes += (chunk.len() * 8) as u64;
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        self.writer.flush()?;
        write_header(self.writer.get_mut(), self.bytes)?;
        self.writer.get_ref().sync_all()?;
        Ok(())
    }
}
fn write_header(file: &mut File, bytes: u64) -> Result<()> {
    let rf64 = bytes + HEADER - 8 > u32::MAX as u64;
    let mut header = [0u8; 92];
    header[0..4].copy_from_slice(if rf64 { b"RF64" } else { b"RIFF" });
    header[4..8].copy_from_slice(
        &if rf64 {
            u32::MAX
        } else {
            (bytes + HEADER - 8) as u32
        }
        .to_le_bytes(),
    );
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(if rf64 { b"ds64" } else { b"JUNK" });
    header[16..20].copy_from_slice(&28u32.to_le_bytes());
    if rf64 {
        header[20..28].copy_from_slice(&(bytes + HEADER - 8).to_le_bytes());
        header[28..36].copy_from_slice(&bytes.to_le_bytes());
        header[36..44].copy_from_slice(&(bytes / 8).to_le_bytes());
    }
    header[48..52].copy_from_slice(b"fmt ");
    header[52..56].copy_from_slice(&16u32.to_le_bytes());
    header[56..58].copy_from_slice(&3u16.to_le_bytes());
    header[58..60].copy_from_slice(&2u16.to_le_bytes());
    header[60..64].copy_from_slice(&SAMPLE_RATE.to_le_bytes());
    header[64..68].copy_from_slice(&(SAMPLE_RATE * 8).to_le_bytes());
    header[68..70].copy_from_slice(&8u16.to_le_bytes());
    header[70..72].copy_from_slice(&32u16.to_le_bytes());
    header[72..76].copy_from_slice(b"fact");
    header[76..80].copy_from_slice(&4u32.to_le_bytes());
    header[80..84].copy_from_slice(&((bytes / 8).min(u32::MAX as u64) as u32).to_le_bytes());
    header[84..88].copy_from_slice(b"data");
    header[88..92].copy_from_slice(&if rf64 { u32::MAX } else { bytes as u32 }.to_le_bytes());
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header)?;
    Ok(())
}
fn waveform(path: &Path) -> Result<Vec<[f32; 2]>> {
    let mut file = File::open(path)?;
    let count = file.metadata()?.len().saturating_sub(HEADER) / 8;
    file.seek(SeekFrom::Start(HEADER))?;
    let bucket = count.div_ceil(512).max(1);
    let mut result = vec![[0f32; 2]; count.div_ceil(bucket) as usize];
    let mut bytes = [0u8; 8192];
    let mut index = 0u64;
    while index < count {
        let frames = (count - index).min(1024) as usize;
        file.read_exact(&mut bytes[..frames * 8])?;
        for frame in bytes[..frames * 8].as_chunks::<8>().0 {
            let a = f32::from_le_bytes(frame[..4].try_into().unwrap());
            let b = f32::from_le_bytes(frame[4..].try_into().unwrap());
            let bin = &mut result[(index / bucket) as usize];
            bin[0] = bin[0].min(a).min(b);
            bin[1] = bin[1].max(a).max(b);
            index += 1;
        }
    }
    Ok(result)
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
        self.shutdown.store(true, Ordering::Release);
    }
}
struct LiveGuard(Arc<Pool>);
impl Drop for LiveGuard {
    fn drop(&mut self) {
        if !self.0.stopped.load(Ordering::Acquire) {
            self.0.fault.store(3, Ordering::Release);
            self.0.stop();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "piratewave-take-test-{}",
            piratewave_core::recording::RecordSlot::new("").id
        ))
    }
    fn track(id: u64) -> Track {
        Track {
            id,
            name: format!("Track {id}"),
            armed: true,
            point: piratewave_core::workflows::CapturePoint {
                node: 1,
                port: piratewave_core::project::OutputPort::Passthrough,
                stage: piratewave_core::workflows::CaptureStage::Processed,
            },
        }
    }
    #[test]
    fn float_wav_and_rf64_headers_have_correct_sizes_and_frame_counts() {
        let root = root();
        fs::create_dir(&root).unwrap();
        let path = root.join("audio.wav");
        let mut writer = FloatWriter::create(&path).unwrap();
        writer.write_frames(&[[0.25, -0.5]; BLOCK_SIZE]).unwrap();
        writer.finish().unwrap();
        drop(writer);
        let mut wav = hound::WavReader::open(&path).unwrap();
        assert_eq!(wav.spec().sample_format, hound::SampleFormat::Float);
        assert_eq!(wav.duration(), BLOCK_SIZE as u32);
        assert_eq!(wav.samples::<f32>().next().unwrap().unwrap(), 0.25);
        let mut file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        let bytes = (u32::MAX as u64) + 1;
        write_header(&mut file, bytes).unwrap();
        drop(file);
        let bytes = fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"RF64");
        assert_eq!(&bytes[12..16], b"ds64");
        assert_eq!(
            u64::from_le_bytes(bytes[28..36].try_into().unwrap()),
            (u32::MAX as u64) + 1
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn recovery_trims_all_tracks_and_export_keeps_original_immutable() {
        let root = root();
        let mut take = new_take(1, 2, "Interrupted".into(), vec![track(1), track(2)]);
        take.start = 512;
        let mut writers = open_take(&root, &take).unwrap();
        writers[0]
            .write_frames(&[[0.25; 2]; BLOCK_SIZE * 2])
            .unwrap();
        writers[1].write_frames(&[[0.5; 2]; BLOCK_SIZE]).unwrap();
        for w in &mut writers {
            w.writer.flush().unwrap();
        }
        drop(writers);
        take = recover(&root, &take.id).unwrap();
        assert!(take.complete);
        assert_eq!(take.end, 640);
        for t in &take.tracks {
            assert_eq!(
                fs::metadata(root.join(&take.id).join(&t.file))
                    .unwrap()
                    .len(),
                HEADER + BLOCK_SIZE as u64 * 8
            );
        }
        let destination = root.join("export");
        export(&root, &take.id, &destination, 10, 100).unwrap();
        let exported: Take =
            serde_json::from_slice(&fs::read(destination.join("take.json")).unwrap()).unwrap();
        assert_eq!(exported.end - exported.start, 90);
        assert_eq!(load(&root, &take.id).unwrap().end, 640);
        assert!(folder(&root, "take-../../escape").is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn producer_copies_raw_and_processed_on_one_boundary_and_stops_on_exhaustion() {
        let registry = piratewave_core::effects::EffectRegistry::default();
        let project = piratewave_core::project::Project::default();
        let mut graph = piratewave_core::graph::Graph::new(&project, &registry).unwrap();
        let id = graph.nodes[0].config.id;
        graph.nodes[0].raw_frames.fill([0.2; 2]);
        graph.nodes[0].frames.fill([0.1; 2]);
        let mut a = track(1);
        a.point.node = id;
        let mut b = a.clone();
        b.point.stage = piratewave_core::workflows::CaptureStage::Raw;
        let pool = Arc::new(Pool::new());
        let producer = Producer::new(pool.clone(), &[a, b], &graph).unwrap();
        producer.copy_block(&graph, 1024, false);
        let packet = pool.pending.pop().unwrap();
        assert_eq!(packet.frame, 1024);
        assert_eq!(packet.frames[0][0], [0.1; 2]);
        assert_eq!(packet.frames[1][0], [0.2; 2]);
        let mut held = Vec::new();
        while let Some(p) = pool.free.pop() {
            held.push(p);
        }
        producer.copy_block(&graph, 1152, false);
        assert!(pool.stopped.load(Ordering::Acquire));
        assert_eq!(pool.fault.load(Ordering::Acquire), 1);
    }
}
/// Streaming session preview. Device creation and file reads stay on this worker.
pub struct Preview {
    pub stop: Arc<AtomicBool>,
    pub frame: Arc<AtomicU64>,
    pub done: Arc<AtomicBool>,
    pub result: mpsc::Receiver<Result<()>>,
}
impl Drop for Preview {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl Service {
    pub fn preview(
        &self,
        id: &str,
        selected: &[u64],
        start: u64,
        end: u64,
        device: String,
    ) -> Result<Preview> {
        ensure!(!device.is_empty(), "Choose a monitoring device");
        let take = load(&self.root, id)?;
        ensure!(take.complete, "Recover this take before previewing");
        let dir = folder(&self.root, id)?;
        let end = end.min(take.end.saturating_sub(take.start));
        ensure!(start < end, "Choose a nonempty selection");
        let tracks: Vec<_> = take
            .tracks
            .iter()
            .filter(|t| selected.contains(&t.track.id))
            .map(|t| dir.join(&t.file))
            .collect();
        ensure!(!tracks.is_empty(), "Select at least one preview track");
        let stop = Arc::new(AtomicBool::new(false));
        let frame = Arc::new(AtomicU64::new(start));
        let done = Arc::new(AtomicBool::new(false));
        let (tx, result) = mpsc::sync_channel(1);
        let cancel = stop.clone();
        let position = frame.clone();
        let completed = done.clone();
        thread::Builder::new()
            .name("piratewave-take-preview".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    let mut output = crate::audio_io::open_output(
                        0,
                        &Some(device),
                        Arc::new(crate::audio_io::Counters::default()),
                        Arc::new(ArrayQueue::new(4)),
                    )?;
                    let mut files = tracks
                        .into_iter()
                        .map(|path| {
                            let mut f = File::open(path)?;
                            f.seek(SeekFrom::Start(HEADER + start * 8))?;
                            Ok(f)
                        })
                        .collect::<Result<Vec<_>>>()?;
                    let mut offset = start;
                    let mut bytes = [0u8; BLOCK_SIZE * 8];
                    while offset < end && !cancel.load(Ordering::Acquire) {
                        if let Some(error) = output.health.failure() {
                            anyhow::bail!("Preview device unavailable: {error}");
                        }
                        if output.queue.len() > crate::audio_io::TARGET_FRAMES {
                            thread::sleep(std::time::Duration::from_millis(2));
                            continue;
                        }
                        let count = (end - offset).min(BLOCK_SIZE as u64) as usize;
                        let mut mixed = [[0f32; 2]; BLOCK_SIZE];
                        for file in &mut files {
                            file.read_exact(&mut bytes[..count * 8])?;
                            for (frame, raw) in
                                mixed.iter_mut().zip(bytes[..count * 8].as_chunks::<8>().0)
                            {
                                for c in 0..2 {
                                    let value = f32::from_le_bytes(
                                        raw[c * 4..c * 4 + 4].try_into().unwrap(),
                                    );
                                    if value.is_finite() {
                                        frame[c] += value;
                                    }
                                }
                            }
                        }
                        for frame in &mixed[..count] {
                            output
                                .queue
                                .push([frame[0].clamp(-1., 1.), frame[1].clamp(-1., 1.)])
                                .map_err(|_| anyhow::anyhow!("Preview output queue full"))?;
                        }
                        offset += count as u64;
                        position.store(
                            offset.saturating_sub(output.queue.len() as u64).max(start),
                            Ordering::Release,
                        );
                    }
                    if !cancel.load(Ordering::Acquire) {
                        let deadline =
                            std::time::Instant::now() + std::time::Duration::from_secs(2);
                        while !output.queue.is_empty()
                            && std::time::Instant::now() < deadline
                            && !cancel.load(Ordering::Acquire)
                        {
                            thread::sleep(std::time::Duration::from_millis(2));
                        }
                        position.store(end, Ordering::Release);
                    }
                    Ok(())
                })();
                completed.store(true, Ordering::Release);
                let _ = tx.send(result);
            })?;
        Ok(Preview {
            stop,
            frame,
            done,
            result,
        })
    }
}
#[cfg(test)]
mod live_tests {
    use super::*;
    use piratewave_core::project::Project;
    #[test]
    fn thirty_two_track_writer_preserves_common_boundaries_samples_and_markers() {
        let root = std::env::temp_dir().join(format!(
            "piratewave-live-test-{}",
            piratewave_core::recording::RecordSlot::new("").id
        ));
        let service = Service::new(root.clone());
        let context = ActionContext {
            generation: 77,
            operation: 123,
            ..Default::default()
        };
        let project = Project {
            nodes: (1..=32)
                .map(|id| {
                    piratewave_core::project::Node::new(
                        id,
                        &format!("Source {id}"),
                        piratewave_core::project::NodeKind::Soundboard,
                        [0.; 2],
                    )
                })
                .collect(),
            connections: vec![],
            ..Project::default()
        };
        let mut graph = piratewave_core::graph::Graph::new(
            &project,
            &piratewave_core::effects::EffectRegistry::default(),
        )
        .unwrap();
        for (i, node) in graph.nodes.iter_mut().enumerate() {
            node.frames.fill([(i + 1) as f32 / 100.; 2]);
        }
        let tracks: Vec<_> = project
            .nodes
            .iter()
            .map(|n| Track {
                id: n.id,
                name: n.name.clone(),
                armed: true,
                point: piratewave_core::workflows::CapturePoint {
                    node: n.id,
                    port: piratewave_core::project::OutputPort::Passthrough,
                    stage: piratewave_core::workflows::CaptureStage::Processed,
                },
            })
            .collect();
        service
            .start(context, 1, 2, "Live alignment".into(), tracks.clone())
            .unwrap();
        let Event::Prepared { pool, .. } = service
            .events
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
        else {
            panic!("Take preparation failed");
        };
        let producer = Producer::new(pool.clone(), &tracks, &graph).unwrap();
        for index in 0..100 {
            producer.copy_block(&graph, 2048 + index * BLOCK_SIZE as u64, index >= 50);
            if index == 50 {
                pool.markers
                    .push(Marker {
                        frame: 2048 + 50 * BLOCK_SIZE as u64,
                        name: "Halfway".into(),
                    })
                    .unwrap();
            }
        }
        pool.stop();
        let Event::Finished {
            context: done,
            take,
        } = service
            .events
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
        else {
            panic!("Take did not finish");
        };
        assert_eq!(done, context);
        assert!(take.complete);
        assert_eq!((take.start, take.end), (2048, 14848));
        assert_eq!(take.markers.len(), 1);
        assert_eq!(take.discontinuities, vec![8448]);
        for (i, track) in take.tracks.iter().enumerate() {
            let mut wav = hound::WavReader::open(root.join(&take.id).join(&track.file)).unwrap();
            assert_eq!(wav.duration(), 12800);
            assert_eq!(
                wav.samples::<f32>().next().unwrap().unwrap(),
                (i + 1) as f32 / 100.
            );
        }
        drop(producer);
        drop(pool);
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }
}
#[cfg(test)]
mod sustained_stress {
    use super::*;
    use piratewave_core::{
        automation::Facts,
        effects::{EffectRegistry, EffectSpec},
        graph::Graph,
        project::{Node, NodeKind, OutputPort, Project},
        workflows::{CapturePoint, CaptureStage, Control, Rule, Scene, Step, Track, Trigger},
    };
    #[test]
    #[ignore = "60-second realtime disk/automation stress; run explicitly"]
    fn thirty_two_tracks_scenes_and_armed_automation_for_sixty_seconds() {
        let root = std::env::temp_dir().join(format!(
            "piratewave-sustained-{}",
            piratewave_core::recording::RecordSlot::new("").id
        ));
        let service = Service::new(root.clone());
        let registry = EffectRegistry::default();
        let mut p = Project {
            nodes: (1..=16)
                .map(|id| {
                    let mut n =
                        Node::new(id, &format!("Source {id}"), NodeKind::Soundboard, [0.; 2]);
                    n.effects = vec![
                        EffectSpec::new(registry.get("gain").unwrap()),
                        EffectSpec::new(registry.get("compressor").unwrap()),
                    ];
                    n
                })
                .collect(),
            connections: vec![],
            ..Project::default()
        };
        for id in 1..=8 {
            p.workflows.rules.push(Rule {
                id: 1000 + id,
                name: format!("Timer {id}"),
                enabled: true,
                trigger: Trigger::Timer {
                    interval_ms: 100,
                    repeating: true,
                },
                match_all: true,
                conditions: vec![],
                actions: vec![Step::Notify {
                    text: "Stress event".into(),
                }],
                cooldown_ms: 0,
                keybind: String::new(),
            });
        }
        let tracks: Vec<_> = (0..32)
            .map(|i| Track {
                id: 100 + i,
                name: format!("Track {i}"),
                armed: true,
                point: CapturePoint {
                    node: 1 + i / 2,
                    port: OutputPort::Passthrough,
                    stage: if i % 2 == 0 {
                        CaptureStage::Raw
                    } else {
                        CaptureStage::Processed
                    },
                },
            })
            .collect();
        let context = ActionContext {
            generation: 57,
            operation: 58,
            ..Default::default()
        };
        let mut graph = Graph::new(&p, &registry).unwrap();
        graph.workflow_taps = true;
        for n in &mut graph.nodes {
            n.capture_raw = true;
        }
        let mut scene = Scene::capture(&p, &registry, "Stress scene".into());
        scene.duration_ms = 10000;
        for s in &mut scene.settings {
            if let Control::Gain { value } = &mut s.value {
                *value = -20.;
            }
        }
        graph.recall_scene(&scene, &p, &registry).unwrap();
        let controls = crate::workflow_runtime::Worker::default();
        let shared = Arc::new(p.clone());
        controls
            .send(crate::workflow_runtime::Command::Arm {
                project: shared.clone(),
                generation: 57,
                facts: Facts {
                    running: true,
                    ..Default::default()
                },
            })
            .unwrap();
        service
            .start(context, 1, 2, "Sustained stress".into(), tracks.clone())
            .unwrap();
        let Event::Prepared { pool, .. } = service
            .events
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
        else {
            panic!("Preparation failed")
        };
        let producer = Producer::new(pool.clone(), &tracks, &graph).unwrap();
        let inputs: Vec<_> = p
            .nodes
            .iter()
            .map(|n| (n.id, [[0.1; 2]; BLOCK_SIZE]))
            .collect();
        let start = std::time::Instant::now();
        let mut peak_queue = 0;
        let mut updates = 0;
        for block in 0..22500u64 {
            let due = start + std::time::Duration::from_secs_f64(block as f64 * 128. / 48000.);
            let now = std::time::Instant::now();
            if due > now {
                thread::sleep(due - now);
            }
            while let Ok(ready) = controls.ready.try_recv() {
                controls
                    .send(crate::workflow_runtime::Command::Acknowledge {
                        operation: ready.action.context.operation,
                        result: Ok(()),
                    })
                    .unwrap();
                updates += 1;
            }
            while let Ok(status) = controls.statuses.try_recv() {
                assert!(
                    status.error.is_none(),
                    "Automation failed: {:?}",
                    status.error
                );
            }
            graph.process_workflows(
                &inputs,
                |_, _| false,
                |_, _, _, _| {},
                |_, _| {},
                |_, _, _, _| {},
            );
            producer.copy_block(&graph, block * 128, false);
            peak_queue = peak_queue.max(pool.pending.len());
            assert_eq!(
                pool.fault.load(Ordering::Acquire),
                0,
                "Transfer queue exhausted"
            );
        }
        pool.stop();
        let Event::Finished { take, .. } = service
            .events
            .recv_timeout(std::time::Duration::from_secs(20))
            .unwrap()
        else {
            panic!("Finalization failed")
        };
        assert!(take.complete);
        assert_eq!((take.start, take.end), (0, 2880000));
        assert!(updates > 4000);
        assert_eq!(take.tracks.len(), 32);
        for track in &take.tracks {
            let wav = hound::WavReader::open(root.join(&take.id).join(&track.file)).unwrap();
            assert_eq!(wav.duration(), 2880000);
        }
        println!("60 seconds, 32 tracks, 737280000 audio bytes, {updates} acknowledged automation steps, peak queue {peak_queue}/{} packets; no lost frames",pool.pending.capacity());
        drop(producer);
        drop(pool);
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }
}
