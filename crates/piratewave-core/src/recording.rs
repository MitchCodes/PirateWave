//! Persistent recording configuration and allocation-free shared rolling histories.
use crate::{
    project::{NodeKind, OutputPort, Project},
    Frame, BLOCK_SIZE, SAMPLE_RATE,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
pub const MAX_SECONDS: f32 = 600.;
const SNAPSHOT_RESERVE: usize = 188; // 0.5 s for off-thread copies at 48 kHz.
pub const MAX_HISTORY_BYTES: usize = 512 * 1024 * 1024;
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RecordingConfig {
    pub enabled: bool,
    pub slots: Vec<RecordSlot>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RecordSlot {
    pub id: u64,
    pub name: String,
    pub seconds: f32,
    #[serde(default)]
    pub asset: Option<String>,
    #[serde(default)]
    pub crop_start: f32,
    #[serde(default)]
    pub crop_end: f32,
    #[serde(default)]
    pub keybinds: BTreeMap<SlotAction, String>,
}
impl RecordSlot {
    pub fn new(name: &str) -> Self {
        Self {
            id: crate::effects::new_effect_id(),
            name: name.into(),
            seconds: 10.,
            asset: None,
            crop_start: 0.,
            crop_end: 0.,
            keybinds: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SlotAction {
    Capture,
    Preview,
    Send,
    PreviewAndSend,
    Delete,
}
impl SlotAction {
    pub const ALL: [Self; 5] = [
        Self::Capture,
        Self::Preview,
        Self::Send,
        Self::PreviewAndSend,
        Self::Delete,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Capture => "Capture recording",
            Self::Preview => "Preview recording",
            Self::Send => "Send recording",
            Self::PreviewAndSend => "Preview & send recording",
            Self::Delete => "Delete recording",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tap {
    pub stage: crate::workflows::CaptureStage,
    pub node: u64,
    pub port: OutputPort,
    pub switch_target: Option<u64>,
}
pub fn inputs(project: &Project, id: u64) -> Vec<Tap> {
    fn collect(project: &Project, id: u64, path: &mut Vec<u64>, out: &mut Vec<Tap>) {
        if path.contains(&id) {
            return;
        }
        path.push(id);
        for c in project.connections.iter().filter(|c| c.to == id) {
            let source = project.nodes.iter().find(|n| n.id == c.from);
            if c.port == OutputPort::Passthrough
                && source.is_some_and(|n| matches!(n.kind, NodeKind::Recording))
            {
                collect(project, c.from, path, out);
            } else {
                out.push(Tap {
                    stage: crate::workflows::CaptureStage::Processed,
                    node: c.from,
                    port: c.port,
                    switch_target: source
                        .filter(|n| matches!(n.kind, NodeKind::Switch))
                        .map(|_| c.to),
                });
            }
        }
        path.pop();
    }
    let mut inputs = vec![];
    collect(project, id, &mut vec![], &mut inputs);
    inputs
}
pub fn requirements(project: &Project) -> BTreeMap<Tap, usize> {
    let mut plan = BTreeMap::new();
    for node in project.nodes.iter().filter(|n| {
        matches!(n.kind, NodeKind::Recording)
            && n.recording.enabled
            && !n.recording.slots.is_empty()
    }) {
        let frames = (node
            .recording
            .slots
            .iter()
            .map(|s| s.seconds)
            .fold(0., f32::max)
            * SAMPLE_RATE as f32)
            .ceil() as usize;
        for tap in inputs(project, node.id) {
            let need = plan.entry(tap).or_insert(0);
            *need = (*need).max(frames);
        }
    }
    plan
}
pub fn history_bytes(frames: usize) -> usize {
    (frames.div_ceil(BLOCK_SIZE) + SNAPSHOT_RESERVE) * (BLOCK_SIZE + 1) * 8
}
pub fn validate(project: &Project) -> Result<(), String> {
    for n in &project.nodes {
        if !matches!(n.kind, NodeKind::Recording) {
            if n.recording != RecordingConfig::default() {
                return Err("Recording settings belong to a recording node".into());
            }
            continue;
        }
        if n.recording.slots.len() > 64 {
            return Err("A recording node supports up to 64 slots".into());
        }
        let mut ids = std::collections::HashSet::new();
        for s in &n.recording.slots {
            if !ids.insert(s.id)
                || !s.seconds.is_finite()
                || !(0.1..=MAX_SECONDS).contains(&s.seconds)
                || !s.crop_start.is_finite()
                || !s.crop_end.is_finite()
                || s.crop_start < 0.
                || s.crop_end < 0.
                || s.crop_start > MAX_SECONDS
                || s.crop_end > MAX_SECONDS
                || (s.crop_end > 0. && s.crop_end <= s.crop_start)
                || s.keybinds.values().any(|k| k.len() > 256)
            {
                return Err("Invalid recording slot settings".into());
            }
        }
    }
    if requirements(project)
        .values()
        .map(|&n| history_bytes(n))
        .sum::<usize>()
        > MAX_HISTORY_BYTES
    {
        return Err(
            "Active recording histories exceed the 512 MiB limit; shorten slots or stop recorders"
                .into(),
        );
    }
    Ok(())
}
struct AtomicBlock {
    version: AtomicU64,
    frames: [AtomicU64; BLOCK_SIZE],
}
pub struct History {
    blocks: Box<[AtomicBlock]>,
    pub origin: u64,
    published: AtomicU64,
    budget: Option<Arc<std::sync::atomic::AtomicUsize>>,
}
impl History {
    pub fn new(frames: usize, origin: u64) -> Self {
        Self {
            blocks: (0..frames.div_ceil(BLOCK_SIZE) + SNAPSHOT_RESERVE)
                .map(|_| AtomicBlock {
                    version: AtomicU64::new(u64::MAX),
                    frames: std::array::from_fn(|_| AtomicU64::new(0)),
                })
                .collect(),
            origin,
            published: AtomicU64::new(origin),
            budget: None,
        }
    }
    fn with_budget(
        frames: usize,
        origin: u64,
        budget: Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<Self, String> {
        let bytes = history_bytes(frames);
        let mut current = budget.load(Ordering::Acquire);
        loop {
            let next = current
                .checked_add(bytes)
                .filter(|n| *n <= MAX_HISTORY_BYTES)
                .ok_or_else(|| {
                    "Recording histories still held by pending captures reach the 512 MiB budget"
                        .to_owned()
                })?;
            match budget.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => break,
                Err(value) => current = value,
            }
        }
        let mut history = Self::new(frames, origin);
        history.budget = Some(budget);
        Ok(history)
    }
    pub fn capacity(&self) -> usize {
        (self.blocks.len() - SNAPSHOT_RESERVE) * BLOCK_SIZE
    }
    pub fn end(&self) -> u64 {
        self.published.load(Ordering::Acquire)
    }
    pub fn write(&self, index: u64, frames: &[Frame; BLOCK_SIZE]) {
        let block = &self.blocks[index as usize % self.blocks.len()];
        block.version.store(index * 2 + 1, Ordering::SeqCst);
        for (dst, frame) in block.frames.iter().zip(frames) {
            dst.store(
                u64::from(frame[0].to_bits()) | u64::from(frame[1].to_bits()) << 32,
                Ordering::Relaxed,
            );
        }
        block.version.store(index * 2 + 2, Ordering::Release);
        self.published.store(index + 1, Ordering::Release);
    }
    pub fn read_block(&self, index: u64) -> Result<[Frame; BLOCK_SIZE], String> {
        let b = &self.blocks[index as usize % self.blocks.len()];
        let stamp = index * 2 + 2;
        if b.version.load(Ordering::Acquire) != stamp {
            return Err("Recording history was overwritten during capture".into());
        }
        let result = std::array::from_fn(|i| {
            let bits = b.frames[i].load(Ordering::Relaxed);
            [
                f32::from_bits(bits as u32),
                f32::from_bits((bits >> 32) as u32),
            ]
        });
        std::sync::atomic::fence(Ordering::Acquire);
        if b.version.load(Ordering::Acquire) != stamp {
            return Err("Recording history changed during capture".into());
        }
        Ok(result)
    }
    pub fn read(&self, start: u64, end: u64) -> Result<Vec<Frame>, String> {
        if start > end || end - start > self.blocks.len() as u64 {
            return Err("Invalid recording snapshot range".into());
        }
        let mut frames = Vec::with_capacity((end - start) as usize * BLOCK_SIZE);
        for index in start..end {
            let b = &self.blocks[index as usize % self.blocks.len()];
            let stamp = index * 2 + 2;
            if b.version.load(Ordering::Acquire) != stamp {
                return Err("Recording history changed during capture; try again".into());
            }
            for sample in &b.frames {
                let bits = sample.load(Ordering::Relaxed);
                frames.push([
                    f32::from_bits(bits as u32),
                    f32::from_bits((bits >> 32) as u32),
                ]);
            }
            std::sync::atomic::fence(Ordering::Acquire);
            if b.version.load(Ordering::Acquire) != stamp {
                return Err("Recording history was overwritten during capture; try again".into());
            }
        }
        Ok(frames)
    }
}
impl Drop for History {
    fn drop(&mut self) {
        if let Some(budget) = &self.budget {
            budget.fetch_sub(history_bytes(self.capacity()), Ordering::AcqRel);
        }
    }
}
struct Active {
    since: u64,
    inputs: Vec<Tap>,
}
struct TapHistory {
    switch_target: Option<u64>,
    history: Arc<History>,
}
#[derive(Default)]
pub struct Recorder {
    pub histories: BTreeMap<Tap, Arc<History>>,
    active: HashMap<u64, Active>,
    // Rebuilt only during sync; render taps visit just their own histories.
    tap_histories: HashMap<(u64, OutputPort, crate::workflows::CaptureStage), Vec<TapHistory>>,
    session_armed: std::collections::BTreeSet<u64>,
    budget: Arc<std::sync::atomic::AtomicUsize>,
    pub error: Option<String>,
    pub block: u64,
}
impl Recorder {
    fn session_requirements(
        &self,
        project: &Project,
        armed: &std::collections::BTreeSet<u64>,
    ) -> BTreeMap<Tap, usize> {
        let mut plan = requirements(project);
        for session in project
            .workflows
            .sessions
            .iter()
            .filter(|s| armed.contains(&s.id))
        {
            for track in session.tracks.iter().filter(|t| t.armed) {
                let tap = Tap {
                    node: track.point.node,
                    port: track.point.port,
                    stage: track.point.stage,
                    switch_target: None,
                };
                let frames = (session.history_seconds * SAMPLE_RATE as f32).ceil() as usize;
                let need = plan.entry(tap).or_insert(0);
                *need = (*need).max(frames);
            }
        }
        plan
    }
    pub fn allocated_bytes(&self) -> usize {
        self.budget.load(Ordering::Acquire)
    }
    pub fn with_budget(budget: Arc<std::sync::atomic::AtomicUsize>) -> Self {
        Self {
            budget,
            ..Self::default()
        }
    }
    fn planned_bytes(&self, requirements: &BTreeMap<Tap, usize>) -> usize {
        let own = self
            .histories
            .values()
            .map(|h| history_bytes(h.capacity()))
            .sum::<usize>();
        let held = self.budget.load(Ordering::Acquire).saturating_sub(own);
        let replacing =
            self.histories
                .iter()
                .filter(|(tap, h)| {
                    requirements.get(tap).is_none_or(|frames| {
                        frames.div_ceil(BLOCK_SIZE) * BLOCK_SIZE != h.capacity()
                    }) && Arc::strong_count(h) > 2
                })
                .map(|(_, h)| history_bytes(h.capacity()))
                .sum::<usize>();
        held + replacing
            + requirements
                .values()
                .map(|n| history_bytes(*n))
                .sum::<usize>()
    }
    pub fn memory_for_sessions(
        project: &Project,
        armed: &std::collections::BTreeSet<u64>,
    ) -> usize {
        Self::default()
            .session_requirements(project, armed)
            .values()
            .map(|n| history_bytes(*n))
            .sum()
    }
    pub fn validate_memory(&self, project: &Project) -> Result<(), String> {
        if self.planned_bytes(&self.session_requirements(project, &self.session_armed))
            > MAX_HISTORY_BYTES
        {
            Err("Shared histories, including pending captures, exceed 512 MiB; shorten histories or wait for exports".into())
        } else {
            Ok(())
        }
    }
    pub fn session_memory(&self, project: &Project, id: u64, armed: bool) -> usize {
        let mut selected = self.session_armed.clone();
        if armed {
            selected.insert(id);
        } else {
            selected.remove(&id);
        }
        self.planned_bytes(&self.session_requirements(project, &selected))
    }
    pub fn arm_session(&mut self, project: &Project, id: u64, armed: bool) -> Result<(), String> {
        let s = project
            .workflows
            .sessions
            .iter()
            .find(|s| s.id == id)
            .ok_or("Session is missing")?;
        if armed {
            if !s.tracks.iter().any(|t| t.armed) {
                return Err("Select at least one track".into());
            }
            for t in s.tracks.iter().filter(|t| t.armed) {
                t.point.validate(project)?;
            }
            if self.session_memory(project, id, true) > MAX_HISTORY_BYTES {
                return Err(
                    "Shared recording histories exceed 512 MiB; shorten history or disarm tracks"
                        .into(),
                );
            }
            self.session_armed.insert(id);
        } else {
            self.session_armed.remove(&id);
        }
        self.sync(project);
        Ok(())
    }
    pub fn snapshot_session(&self, project: &Project, id: u64) -> Result<SessionSnapshot, String> {
        if !self.session_armed.contains(&id) {
            return Err("Arm session history first".into());
        }
        let s = project
            .workflows
            .sessions
            .iter()
            .find(|s| s.id == id)
            .ok_or("Session is missing")?;
        let mut tracks = Vec::new();
        let mut start = self.block.saturating_sub(
            (s.history_seconds * SAMPLE_RATE as f32 / BLOCK_SIZE as f32).ceil() as u64,
        );
        for track in s.tracks.iter().filter(|t| t.armed) {
            let tap = Tap {
                node: track.point.node,
                port: track.point.port,
                stage: track.point.stage,
                switch_target: None,
            };
            let history = self
                .histories
                .get(&tap)
                .ok_or("Track history is unavailable")?;
            start = start.max(history.origin).max(
                self.block
                    .saturating_sub((history.capacity() / BLOCK_SIZE) as u64),
            );
            tracks.push((track.clone(), Arc::clone(history)));
        }
        if tracks.is_empty() || start >= self.block {
            return Err("Wait for recording history to fill".into());
        }
        Ok(SessionSnapshot {
            tracks,
            start,
            end: self.block,
        })
    }
    pub fn sync(&mut self, project: &Project) {
        self.session_armed
            .retain(|id| project.workflows.sessions.iter().any(|s| s.id == *id));
        let requirements = self.session_requirements(project, &self.session_armed);
        self.error = None;
        self.tap_histories.clear();
        self.histories.retain(|tap, h| {
            requirements
                .get(tap)
                .is_some_and(|frames| h.capacity() == frames.div_ceil(BLOCK_SIZE) * BLOCK_SIZE)
        });
        for (tap, frames) in requirements {
            if !self.histories.contains_key(&tap) {
                match History::with_budget(frames, self.block, self.budget.clone()) {
                    Ok(h) => {
                        self.histories.insert(tap, Arc::new(h));
                    }
                    Err(e) => {
                        self.error = Some(e);
                        break;
                    }
                }
            }
        }
        for (tap, history) in &self.histories {
            self.tap_histories
                .entry((tap.node, tap.port, tap.stage))
                .or_default()
                .push(TapHistory {
                    switch_target: tap.switch_target,
                    history: Arc::clone(history),
                });
        }
        self.active.retain(|id, _| {
            project.nodes.iter().any(|n| {
                n.id == *id
                    && matches!(n.kind, NodeKind::Recording)
                    && n.recording.enabled
                    && !n.recording.slots.is_empty()
            })
        });
        for n in project.nodes.iter().filter(|n| {
            matches!(n.kind, NodeKind::Recording)
                && n.recording.enabled
                && !n.recording.slots.is_empty()
        }) {
            let taps = inputs(project, n.id);
            if self.active.get(&n.id).is_none_or(|a| a.inputs != taps) {
                self.active.insert(
                    n.id,
                    Active {
                        since: self.block,
                        inputs: taps,
                    },
                );
            }
        }
    }
    pub fn tap(
        &self,
        node: u64,
        port: OutputPort,
        selected: Option<u64>,
        frames: &[Frame; BLOCK_SIZE],
    ) {
        self.tap_stage(
            node,
            port,
            crate::workflows::CaptureStage::Processed,
            selected,
            frames,
        );
    }
    pub fn tap_stage(
        &self,
        node: u64,
        port: OutputPort,
        stage: crate::workflows::CaptureStage,
        selected: Option<u64>,
        frames: &[Frame; BLOCK_SIZE],
    ) {
        if let Some(histories) = self.tap_histories.get(&(node, port, stage)) {
            for TapHistory {
                switch_target,
                history,
            } in histories
            {
                if switch_target.is_none() || *switch_target == selected {
                    history.write(self.block, frames);
                } else {
                    history.write(self.block, &[[0.; 2]; BLOCK_SIZE]);
                }
            }
        }
    }
    /// Precise route tap used during switch crossfades; never overwrites another destination.
    pub fn tap_route(
        &self,
        node: u64,
        port: OutputPort,
        destination: u64,
        frames: &[Frame; BLOCK_SIZE],
    ) {
        if let Some(histories) =
            self.tap_histories
                .get(&(node, port, crate::workflows::CaptureStage::Processed))
        {
            for h in histories
                .iter()
                .filter(|h| h.switch_target == Some(destination))
            {
                h.history.write(self.block, frames);
            }
        }
    }
    pub fn finish_block(&mut self) {
        self.block += 1;
    }
    pub fn capture(&self, node: u64, seconds: f32) -> Result<Capture, String> {
        let active = self
            .active
            .get(&node)
            .ok_or("Start recording and add a slot first")?;
        let sources: Vec<_> = active
            .inputs
            .iter()
            .filter_map(|tap| self.histories.get(tap).cloned())
            .collect();
        if sources.is_empty() {
            return Err("Connect an audio input to the recording node".into());
        }
        Ok(Capture {
            sources,
            since: active.since,
            seconds,
            end: self.block,
        })
    }
    pub fn available(&self, node: u64) -> f32 {
        self.active.get(&node).map_or(0., |a| {
            let start = a
                .inputs
                .iter()
                .filter_map(|tap| self.histories.get(tap).map(|h| h.origin))
                .max()
                .unwrap_or(self.block)
                .max(a.since);
            let capacity = a
                .inputs
                .iter()
                .filter_map(|tap| self.histories.get(tap).map(|h| h.capacity()))
                .min()
                .unwrap_or(0);
            ((self.block - start) as usize * BLOCK_SIZE).min(capacity) as f32 / SAMPLE_RATE as f32
        })
    }
}
pub struct Capture {
    sources: Vec<Arc<History>>,
    since: u64,
    seconds: f32,
    end: u64,
}
impl Capture {
    pub fn materialize(&self) -> Result<Vec<Frame>, String> {
        for _ in 0..3 {
            let end = self.end;
            let wanted = (self.seconds * SAMPLE_RATE as f32).round() as usize;
            let blocks = wanted.div_ceil(BLOCK_SIZE) as u64;
            let start = end
                .saturating_sub(blocks)
                .max(self.since)
                .max(self.sources.iter().map(|h| h.origin).max().unwrap_or(end));
            if start >= end {
                return Err("Recording history is still empty".into());
            }
            let mut mixed = vec![[0.; 2]; (end - start) as usize * BLOCK_SIZE];
            let mut success = true;
            for source in &self.sources {
                match source.read(start, end) {
                    Ok(frames) => {
                        for (m, f) in mixed.iter_mut().zip(frames) {
                            for c in 0..2 {
                                m[c] += f[c];
                            }
                        }
                    }
                    Err(_) => {
                        success = false;
                        break;
                    }
                }
            }
            if success {
                let excess = mixed.len().saturating_sub(wanted);
                if excess > 0 {
                    mixed.drain(..excess);
                }
                for frame in &mut mixed {
                    for sample in frame {
                        if !sample.is_finite() {
                            *sample = 0.;
                        }
                    }
                }
                return Ok(mixed);
            }
        }
        Err("Recording history was busy; capture again".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Connection, Node};
    fn project() -> Project {
        let mut p = Project {
            nodes: vec![Node::new(1, "Mic", NodeKind::Soundboard, [0.; 2])],
            connections: vec![],
            ..Project::default()
        };
        for id in 2..6 {
            let mut node = Node::new(id, "Recorder", NodeKind::Recording, [0.; 2]);
            node.recording.enabled = true;
            for n in 0..5 {
                let mut slot = RecordSlot::new(&format!("Slot {n}"));
                slot.seconds = if id == 5 && n == 4 { 60. } else { 10. };
                node.recording.slots.push(slot);
            }
            p.nodes.push(node);
            p.connections.push(Connection {
                input: Default::default(),
                from: 1,
                to: id,
                port: OutputPort::Passthrough,
            });
        }
        p
    }
    #[test]
    fn indexed_taps_refresh_after_resize_stop_and_recreation() {
        let mut p = project();
        let mut recorder = Recorder::default();
        recorder.sync(&p);
        let first = Arc::clone(recorder.histories.values().next().unwrap());
        recorder.tap(1, OutputPort::Passthrough, None, &[[0.2, -0.1]; BLOCK_SIZE]);
        recorder.finish_block();
        assert_eq!(first.end(), 1);
        for node in &mut p.nodes {
            for slot in &mut node.recording.slots {
                slot.seconds = 0.1;
            }
        }
        recorder.sync(&p);
        let resized = Arc::clone(recorder.histories.values().next().unwrap());
        assert!(!Arc::ptr_eq(&first, &resized));
        recorder.tap(1, OutputPort::Passthrough, None, &[[0.4; 2]; BLOCK_SIZE]);
        recorder.finish_block();
        assert_eq!(first.end(), 1);
        assert_eq!(resized.read(1, 2).unwrap(), vec![[0.4; 2]; BLOCK_SIZE]);
        for node in &mut p.nodes {
            node.recording.enabled = false;
        }
        recorder.sync(&p);
        assert!(recorder.tap_histories.is_empty());
        recorder.tap(1, OutputPort::Passthrough, None, &[[0.8; 2]; BLOCK_SIZE]);
        recorder.finish_block();
        assert_eq!(resized.end(), 2);
        p.nodes[1].recording.enabled = true;
        recorder.sync(&p);
        recorder.tap(1, OutputPort::Passthrough, None, &[[0.6; 2]; BLOCK_SIZE]);
        recorder.finish_block();
        assert_eq!(
            recorder
                .histories
                .values()
                .next()
                .unwrap()
                .read(3, 4)
                .unwrap(),
            vec![[0.6; 2]; BLOCK_SIZE]
        );
    }
    #[test]
    fn recording_passthrough_chains_share_the_original_input_buffer() {
        let mut p = project();
        p.connections.retain(|c| c.to == 2);
        for id in 3..6 {
            p.connect(2, id).unwrap();
        }
        let mut recorder = Recorder::default();
        recorder.sync(&p);
        assert_eq!(recorder.histories.len(), 1);
        assert_eq!(recorder.histories.keys().next().unwrap().node, 1);
    }
    #[test]
    fn twenty_slots_four_recorders_share_one_maximum_input_history() {
        let mut p = project();
        let mut recorder = Recorder::default();
        recorder.sync(&p);
        assert_eq!(recorder.histories.len(), 1);
        assert_eq!(
            recorder.histories.values().next().unwrap().capacity(),
            60 * 48000
        );
        assert!(
            recorder
                .histories
                .values()
                .map(|h| history_bytes(h.capacity()))
                .sum::<usize>()
                < 24 * 1024 * 1024
        );
        for node in &mut p.nodes {
            node.recording.enabled = false;
        }
        recorder.sync(&p);
        assert!(recorder.histories.is_empty());
    }
    #[test]
    fn capture_is_anchored_to_press_time_and_mixes_inputs() {
        let mut p = project();
        p.nodes.truncate(2);
        p.connections.truncate(1);
        p.nodes
            .push(Node::new(9, "Other", NodeKind::Soundboard, [0.; 2]));
        p.connections.push(Connection {
            input: Default::default(),
            from: 9,
            to: 2,
            port: OutputPort::Passthrough,
        });
        let mut recorder = Recorder::default();
        recorder.sync(&p);
        for _ in 0..100 {
            recorder.tap(1, OutputPort::Passthrough, None, &[[0.2, 0.1]; BLOCK_SIZE]);
            recorder.tap(9, OutputPort::Passthrough, None, &[[0.3, -0.1]; BLOCK_SIZE]);
            recorder.finish_block();
        }
        let capture = recorder.capture(2, 0.1).unwrap();
        for _ in 0..50 {
            recorder.tap(1, OutputPort::Passthrough, None, &[[0.9; 2]; BLOCK_SIZE]);
            recorder.tap(9, OutputPort::Passthrough, None, &[[0.9; 2]; BLOCK_SIZE]);
            recorder.finish_block();
        }
        let audio = capture.materialize().unwrap();
        assert_eq!(audio.len(), 4800);
        assert!(audio
            .iter()
            .all(|f| (f[0] - 0.5).abs() < 1e-6 && f[1] == 0.));
    }
    #[test]
    fn start_stop_epochs_do_not_reuse_another_recorders_old_history() {
        let mut p = project();
        p.nodes[1].recording.enabled = false;
        let mut recorder = Recorder::default();
        recorder.sync(&p);
        for _ in 0..20 {
            recorder.tap(1, OutputPort::Passthrough, None, &[[0.1; 2]; BLOCK_SIZE]);
            recorder.finish_block();
        }
        p.nodes[1].recording.enabled = true;
        recorder.sync(&p);
        assert!(recorder.capture(2, 1.).unwrap().materialize().is_err());
        for _ in 0..3 {
            recorder.tap(1, OutputPort::Passthrough, None, &[[0.4; 2]; BLOCK_SIZE]);
            recorder.finish_block();
        }
        let audio = recorder.capture(2, 10.).unwrap().materialize().unwrap();
        assert_eq!(audio.len(), 3 * BLOCK_SIZE);
        assert!(audio.iter().all(|f| *f == [0.4; 2]));
    }
    #[test]
    fn switch_taps_record_silence_on_inactive_branches() {
        let mut p = project();
        p.nodes[0].kind = NodeKind::Switch;
        p.nodes[0].switch_output = Some(2);
        let mut recorder = Recorder::default();
        recorder.sync(&p);
        for _ in 0..4 {
            recorder.tap(1, OutputPort::Passthrough, Some(2), &[[0.3; 2]; BLOCK_SIZE]);
            recorder.finish_block();
        }
        assert!(recorder
            .capture(2, 1.)
            .unwrap()
            .materialize()
            .unwrap()
            .iter()
            .all(|f| *f == [0.3; 2]));
        assert!(recorder
            .capture(3, 1.)
            .unwrap()
            .materialize()
            .unwrap()
            .iter()
            .all(|f| *f == [0.; 2]));
    }
}

/// Immutable shared endpoints and one interval. Workers stream blocks and validate
/// sequence stamps; overwritten ranges fail instead of producing misaligned takes.
pub struct SessionSnapshot {
    pub tracks: Vec<(crate::workflows::Track, Arc<History>)>,
    pub start: u64,
    pub end: u64,
}
#[cfg(test)]
mod workflow_tests {
    use super::*;
    use crate::{
        effects::{EffectRegistry, EffectSpec},
        graph::Graph,
        project::{Node, NodeKind, OutputPort, Project},
        workflows::{CapturePoint, CaptureStage, SessionConfig, Track},
    };
    fn track(id: u64, node: u64, stage: CaptureStage) -> Track {
        Track {
            id,
            name: format!("Track {id}"),
            point: CapturePoint {
                node,
                port: OutputPort::Passthrough,
                stage,
            },
            armed: true,
        }
    }
    fn project(tracks: Vec<Track>, seconds: f32) -> Project {
        let mut p = Project {
            nodes: vec![Node::new(1, "Source", NodeKind::Soundboard, [0.; 2])],
            connections: vec![],
            ..Project::default()
        };
        p.workflows.sessions.push(SessionConfig {
            id: 10,
            name: "Session".into(),
            history_seconds: seconds,
            tracks,
            take_ids: vec![],
            keybinds: Default::default(),
        });
        p
    }
    #[test]
    fn shared_raw_and_processed_histories_keep_one_common_interval_across_wrap_and_new_tracks() {
        let mut p = project(
            vec![
                track(1, 1, CaptureStage::Raw),
                track(2, 1, CaptureStage::Processed),
            ],
            0.1,
        );
        let registry = EffectRegistry::default();
        let mut gain = EffectSpec::new(registry.get("gain").unwrap());
        gain.parameters.insert("gain".into(), 6.);
        p.nodes[0].effects.push(gain);
        p.nodes[0].mute = true;
        let mut graph = Graph::new(&p, &registry).unwrap();
        graph.workflow_taps = true;
        graph.nodes[0].capture_raw = true;
        let mut recorder = Recorder::default();
        recorder.arm_session(&p, 10, true).unwrap();
        for _ in 0..400 {
            graph.process_workflows(
                &[(1, [[0.25; 2]; BLOCK_SIZE])],
                |_, _| false,
                |_, _, _, _| {},
                |id, frames| {
                    recorder.tap_stage(id, OutputPort::Passthrough, CaptureStage::Raw, None, frames)
                },
                |id, port, target, frames| recorder.tap(id, port, target, frames),
            );
            recorder.finish_block();
        }
        let snapshot = recorder.snapshot_session(&p, 10).unwrap();
        assert_eq!(snapshot.end, 400);
        assert_eq!(snapshot.start, 362);
        for (_, history) in &snapshot.tracks {
            let frames = history.read_block(snapshot.end - 1).unwrap();
            if Arc::ptr_eq(history, &snapshot.tracks[0].1) {
                assert_eq!(frames[0], [0.25; 2]);
            } else {
                assert_eq!(frames[0], [0.; 2]);
            }
        }
        // Adding a new capture point cannot silently select a different interval for that track.
        p.nodes
            .push(Node::new(2, "Second", NodeKind::Soundboard, [0.; 2]));
        p.workflows.sessions[0]
            .tracks
            .push(track(3, 2, CaptureStage::Processed));
        recorder.sync(&p);
        for _ in 0..8 {
            recorder.tap(1, OutputPort::Passthrough, None, &[[0.1; 2]; BLOCK_SIZE]);
            recorder.tap(2, OutputPort::Passthrough, None, &[[0.2; 2]; BLOCK_SIZE]);
            recorder.tap_stage(
                1,
                OutputPort::Passthrough,
                CaptureStage::Raw,
                None,
                &[[0.3; 2]; BLOCK_SIZE],
            );
            recorder.finish_block();
        }
        let snapshot = recorder.snapshot_session(&p, 10).unwrap();
        assert_eq!((snapshot.start, snapshot.end), (400, 408));
    }
    #[test]
    fn duplicate_track_points_share_memory_and_excessive_arming_is_rejected_before_allocation() {
        let p = project(
            (1..=32)
                .map(|id| track(id, 1, CaptureStage::Processed))
                .collect(),
            60.,
        );
        let mut recorder = Recorder::default();
        assert_eq!(
            recorder.session_memory(&p, 10, true),
            history_bytes(48000 * 60)
        );
        recorder.arm_session(&p, 10, true).unwrap();
        assert_eq!(recorder.histories.len(), 1);
        let mut large = project(
            (1..=32)
                .map(|id| track(id, id, CaptureStage::Raw))
                .collect(),
            600.,
        );
        large.nodes = (1..=32)
            .map(|id| Node::new(id, &format!("Source {id}"), NodeKind::Soundboard, [0.; 2]))
            .collect();
        let mut empty = Recorder::default();
        assert!(empty
            .arm_session(&large, 10, true)
            .unwrap_err()
            .contains("512 MiB"));
        assert!(empty.histories.is_empty());
    }
}
#[cfg(test)]
mod history_budget_tests {
    use super::*;
    use crate::{
        project::{Node, NodeKind},
        workflows::{CapturePoint, CaptureStage, SessionConfig, Track},
    };
    #[test]
    fn pending_capture_budget_follows_history_across_workspace_recorders() {
        let budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut p = Project {
            nodes: vec![Node::new(1, "Source", NodeKind::Soundboard, [0.; 2])],
            connections: vec![],
            ..Project::default()
        };
        p.workflows.sessions.push(SessionConfig {
            id: 9,
            name: "Session".into(),
            history_seconds: 1.,
            tracks: vec![Track {
                id: 8,
                name: "Track".into(),
                point: CapturePoint {
                    node: 1,
                    port: OutputPort::Passthrough,
                    stage: CaptureStage::Processed,
                },
                armed: true,
            }],
            take_ids: vec![],
            keybinds: Default::default(),
        });
        let mut first = Recorder::with_budget(budget.clone());
        first.arm_session(&p, 9, true).unwrap();
        first.tap(1, OutputPort::Passthrough, None, &[[0.1; 2]; BLOCK_SIZE]);
        first.finish_block();
        let held = first.snapshot_session(&p, 9).unwrap();
        let bytes = history_bytes(48000);
        assert_eq!(budget.load(Ordering::Acquire), bytes);
        drop(first);
        assert_eq!(budget.load(Ordering::Acquire), bytes);
        let mut second = Recorder::with_budget(budget.clone());
        assert_eq!(second.session_memory(&p, 9, true), bytes * 2);
        second.arm_session(&p, 9, true).unwrap();
        assert_eq!(budget.load(Ordering::Acquire), bytes * 2);
        drop(held);
        assert_eq!(budget.load(Ordering::Acquire), bytes);
        drop(second);
        assert_eq!(budget.load(Ordering::Acquire), 0);
        let full = Arc::new(std::sync::atomic::AtomicUsize::new(MAX_HISTORY_BYTES));
        assert!(History::with_budget(128, 0, full.clone()).is_err());
        assert_eq!(full.load(Ordering::Acquire), MAX_HISTORY_BYTES);
    }
}
