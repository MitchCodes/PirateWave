use crate::{
    effects::{db_to_gain, AudioEffect, EffectRegistry},
    project::{InputPort, Node, NodeKind, OutputPort, Project},
    Frame, BLOCK_SIZE, SAMPLE_RATE,
};
use std::collections::{HashMap, HashSet, VecDeque};

pub fn validate(project: &Project) -> Result<Vec<usize>, String> {
    if project.version != 1 {
        return Err(format!("Unsupported project version {}", project.version));
    }
    if project.nodes.len() > 64 {
        return Err("A project supports up to 64 audio nodes".into());
    }
    crate::recording::validate(project)?;
    if project.nodes.iter().map(|n| n.effects.len()).sum::<usize>() > 128 {
        return Err("A project supports up to 128 total effect instances".into());
    }
    let mut ids = HashMap::new();
    if project
        .nodes
        .iter()
        .filter(|n| {
            matches!(n.kind, NodeKind::VoiceChanger)
                && !n.effects_disabled
                && crate::voice::route_selected(project, n.id)
        })
        .count()
        > 1
    {
        return Err(
            "Only one Voice Changer can be enabled. Disable the current voice node first.".into(),
        );
    }
    for (i, node) in project.nodes.iter().enumerate() {
        if ids.insert(node.id, i).is_some() {
            return Err("Duplicate node ID".into());
        }
        if !node.gain_db.is_finite()
            || !(-60.0..=24.0).contains(&node.gain_db)
            || !node.pan.is_finite()
            || !(-1.0..=1.0).contains(&node.pan)
            || !node.position.iter().all(|v| v.is_finite())
        {
            return Err(format!("Invalid settings for {}", node.name));
        }
        crate::extensions::validate_node(node)?;
        if let NodeKind::ApplicationCapture {
            target: Some(target),
        } = &node.kind
        {
            target.validate()?;
        }
        if let Some(plugin) = &node.plugin {
            plugin.validate()?;
            let expected = if matches!(node.kind, NodeKind::VstInstrument) {
                crate::plugins::PluginRole::Instrument
            } else {
                crate::plugins::PluginRole::Effect
            };
            if !node.kind.is_plugin() || plugin.descriptor.role != expected {
                return Err("Plugin type does not match its node".into());
            }
        }
        if let Some(voice) = &node.voice {
            voice.validate()?;
            if !matches!(node.kind, NodeKind::VoiceChanger) {
                return Err("Voice configuration belongs to a Voice Changer node".into());
            }
        }
        if matches!(node.kind, NodeKind::VoiceChanger) && !node.effects.is_empty() {
            return Err(
                "Place PirateWave effects in a separate processor before or after Voice Changer"
                    .into(),
            );
        }
        if matches!(node.kind, NodeKind::AutoDucking) {
            node.ducking.validate()?;
            if !node.effects.is_empty() {
                return Err("Place effects before or after Auto Ducking".into());
            }
        }
        let mut effect_ids = HashSet::new();
        for effect in &node.effects {
            if !effect_ids.insert(effect.id) || effect.keybinds.values().any(|key| key.len() > 256)
            {
                return Err("Duplicate effect identity or invalid effect shortcut".into());
            }
        }
        if matches!(node.kind, NodeKind::Switch) {
            if !node.effects.is_empty() {
                return Err("Place effects after the switch".into());
            }
            if node.switch_output.is_some_and(|to| {
                !project
                    .connections
                    .iter()
                    .any(|c| c.from == node.id && c.to == to)
            }) || node.switch_keybinds.iter().any(|(to, key)| {
                key.len() > 256
                    || !project
                        .connections
                        .iter()
                        .any(|c| c.from == node.id && c.to == *to)
            }) {
                return Err("Switch destination is not connected".into());
            }
        } else if node.switch_output.is_some() || !node.switch_keybinds.is_empty() {
            return Err("Switch settings belong to a switch node".into());
        }
        if node.effects.len() > 16 {
            return Err("A node supports up to 16 effects".into());
        }
        if node.keybinds.iter().any(|(action, key)| {
            !crate::shortcuts::NodeAction::for_kind(&node.kind).contains(action) || key.len() > 256
        }) {
            return Err(format!("Invalid keybinds for {}", node.name));
        }
    }
    if project.sounds.len() > 128 {
        return Err("A project supports up to 128 sound pads".into());
    }
    let mut sound_ids = HashSet::new();
    for s in &project.sounds {
        if !sound_ids.insert(s.id) || !s.volume.is_finite() || !(0.0..=2.0).contains(&s.volume) {
            return Err("Invalid sound pad".into());
        }
    }
    let mut degrees = vec![0; project.nodes.len()];
    let mut edges = vec![Vec::new(); project.nodes.len()];
    let mut seen = HashSet::new();
    for c in &project.connections {
        let &from = ids.get(&c.from).ok_or("Connection source is missing")?;
        let &to = ids.get(&c.to).ok_or("Connection destination is missing")?;
        if !seen.insert((from, to, c.port, c.input)) {
            return Err("Duplicate connection".into());
        }
        if matches!(project.nodes[from].kind, NodeKind::Output { .. })
            || project.nodes[to].kind.is_source()
        {
            return Err("Connect inputs to processors or output buses".into());
        }
        if !project.nodes[from].kind.outputs().contains(&c.port)
            || !project.nodes[to].kind.inputs().contains(&c.input)
        {
            return Err("Invalid source output or destination input port".into());
        }
        degrees[to] += 1;
        edges[from].push(to);
    }
    let mut ready: VecDeque<_> = degrees
        .iter()
        .enumerate()
        .filter_map(|(i, &d)| (d == 0).then_some(i))
        .collect();
    let mut order = vec![];
    while let Some(i) = ready.pop_front() {
        order.push(i);
        for &next in &edges[i] {
            degrees[next] -= 1;
            if degrees[next] == 0 {
                ready.push_back(next)
            }
        }
    }
    if order.len() != project.nodes.len() {
        return Err("This connection would create an audio feedback loop".into());
    }
    Ok(order)
}
/// Detects digital feedback through a WASAPI loopback source and a render endpoint.
/// `identity` resolves default-device aliases to a stable endpoint identity.
pub fn validate_loopback_routes(
    project: &Project,
    identity: impl Fn(&Option<String>) -> String,
) -> Result<(), String> {
    for source in &project.nodes {
        let NodeKind::Input {
            device,
            loopback: true,
        } = &source.kind
        else {
            continue;
        };
        if source.mute {
            continue;
        }
        let endpoint = identity(device);
        let mut stack = vec![source.id];
        let mut visited = HashSet::new();
        while let Some(id) = stack.pop() {
            if !visited.insert(id) {
                continue;
            }
            let Some(node) = project.nodes.iter().find(|n| n.id == id) else {
                continue;
            };
            if node.mute && !node.kind.is_processor() {
                continue;
            }
            if let NodeKind::Output { device } = &node.kind {
                if (!device.as_ref().is_some_and(|s| s.is_empty()) && identity(device) == endpoint)
                    || (node.listen_enabled && identity(&node.listen_device) == endpoint)
                {
                    return Err(format!("{} routes back into its own captured playback device through {}. Choose a different output or mute this capture.",source.name,node.name));
                }
            }
            stack.extend(
                project
                    .connections
                    .iter()
                    .filter(|c| {
                        c.from == id
                            && (!matches!(node.kind, NodeKind::Recording)
                                || c.port == OutputPort::Passthrough)
                    })
                    .map(|c| c.to),
            );
        }
    }
    Ok(())
}
pub struct RuntimeNode {
    pub config: Node,
    pub frames: [Frame; BLOCK_SIZE],
    pub peak: f32,
    pub preview: [Frame; BLOCK_SIZE],
    pub send: [Frame; BLOCK_SIZE],
    pub ducking: Option<Box<crate::ducking::AutoDucking>>,
    ducking_inputs: [bool; 2],
    priority: [Frame; BLOCK_SIZE],
    ducked: [Frame; BLOCK_SIZE],
    pub extension: Option<Box<crate::extensions::Runtime>>,
    effects: Vec<Box<dyn AudioEffect>>,
    gains: Frame,
}
impl RuntimeNode {
    pub fn has_effect_fault(&self) -> bool {
        self.effects.iter().any(|e| e.faulted()) || self.extension.as_ref().is_some_and(|e| e.fault)
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum RenderOutput {
    Passthrough,
    Preview,
    Send,
    Priority,
    Ducked,
    Extension(u8),
}
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum RenderInput {
    Main,
    Priority,
    Ducked,
    Extension(u8),
}
/// Fixed-size diagnostic tap. Captures one effect, never changes the signal.
/// The host owns visibility, expiry and bounded publication outside rendering.
#[derive(Clone, Copy)]
pub struct EffectMonitor {
    pub node: u64,
    pub effect: u64,
    pub token: u64,
    pub before: [Frame; BLOCK_SIZE],
    pub after: [Frame; BLOCK_SIZE],
    pub valid: bool,
    pub bypassed: bool,
}
impl EffectMonitor {
    pub fn new(node: u64, effect: u64, token: u64) -> Self {
        Self {
            node,
            effect,
            token,
            before: [[0.; 2]; BLOCK_SIZE],
            after: [[0.; 2]; BLOCK_SIZE],
            valid: false,
            bypassed: false,
        }
    }
}
pub struct Graph {
    pub monitor: Option<Box<EffectMonitor>>,
    has_extensions: bool,
    pub nodes: Vec<RuntimeNode>,
    order: Vec<usize>,
    edges: Vec<Vec<(usize, RenderOutput, RenderInput)>>,
}
impl Graph {
    pub fn new(project: &Project, registry: &EffectRegistry) -> Result<Self, String> {
        let order = validate(project)?;
        for node in &project.nodes {
            for spec in &node.effects {
                registry.validate(spec)?;
            }
        }
        let mut nodes = vec![];
        let solo = project.nodes.iter().any(|n| n.kind.is_source() && n.solo);
        for node in &project.nodes {
            let effects = node
                .effects
                .iter()
                .filter(|_| !matches!(node.kind, NodeKind::Recording))
                .map(|spec| registry.create(spec, SAMPLE_RATE as f32))
                .collect::<Result<Vec<_>, _>>()?;
            nodes.push(RuntimeNode {
                extension: matches!(node.kind, NodeKind::Extension { .. })
                    .then(|| Box::new(crate::extensions::Runtime::new(node))),
                config: node.clone(),
                frames: [[0.0; 2]; BLOCK_SIZE],
                peak: 0.0,
                preview: [[0.; 2]; BLOCK_SIZE],
                send: [[0.; 2]; BLOCK_SIZE],
                ducking: matches!(node.kind, NodeKind::AutoDucking)
                    .then(|| Box::new(crate::ducking::AutoDucking::new(&node.ducking))),
                ducking_inputs: [false; 2],
                priority: [[0.; 2]; BLOCK_SIZE],
                ducked: [[0.; 2]; BLOCK_SIZE],
                effects,
                gains: node_gains(node, solo),
            });
        }
        let mut edges = vec![vec![]; nodes.len()];
        for c in &project.connections {
            let from = nodes.iter().position(|n| n.config.id == c.from).unwrap();
            let to = nodes.iter().position(|n| n.config.id == c.to).unwrap();
            match c.input {
                InputPort::Priority => nodes[to].ducking_inputs[0] = true,
                InputPort::Ducked => nodes[to].ducking_inputs[1] = true,
                InputPort::Main | InputPort::Extension(_) => {}
            }
            edges[from].push((
                to,
                compile_output(&project.nodes[from], c.port),
                compile_input(&project.nodes[to], c.input),
            ));
        }
        Ok(Self {
            monitor: None,
            has_extensions: project
                .nodes
                .iter()
                .any(|n| matches!(n.kind, NodeKind::Extension { .. })),
            nodes,
            order,
            edges,
        })
    }
    /// Called on the control plane at a bounded interval, never from a device callback.
    pub fn recover_effects(&mut self) {
        for node in &mut self.nodes {
            if let Some(e) = &mut node.extension {
                if e.fault {
                    e.reset();
                }
            }
            if let Some(ducking) = &mut node.ducking {
                ducking.recover();
            }
            for effect in &mut node.effects {
                if effect.faulted() {
                    effect.reset();
                }
            }
        }
    }
    /// Validates first, then preserves processor history for unchanged effect types.
    pub fn update(&mut self, project: &Project, registry: &EffectRegistry) -> Result<(), String> {
        validate(project)?;
        for node in &project.nodes {
            for spec in &node.effects {
                registry.validate(spec)?;
            }
        }
        let same_nodes = self.nodes.len() == project.nodes.len()
            && self.nodes.iter().zip(&project.nodes).all(|(old, new)| {
                old.config.id == new.id
                    && old.config.kind == new.kind
                    && old.config.effects.len() == new.effects.len()
                    && old
                        .config
                        .effects
                        .iter()
                        .zip(&new.effects)
                        .all(|(a, b)| a.kind == b.kind && a.id == b.id)
            });
        let same_edges = same_nodes
            && project.connections.len() == self.edges.iter().map(Vec::len).sum::<usize>()
            && project.connections.iter().all(|c| {
                let from = self.nodes.iter().position(|n| n.config.id == c.from);
                let to = self.nodes.iter().position(|n| n.config.id == c.to);
                match (from, to) {
                    (Some(a), Some(b)) => self.edges[a].contains(&(
                        b,
                        compile_output(&self.nodes[a].config, c.port),
                        compile_input(&self.nodes[b].config, c.input),
                    )),
                    _ => false,
                }
            });
        if same_nodes && same_edges {
            for (old, new) in self.nodes.iter_mut().zip(&project.nodes) {
                if let Some(extension) = &mut old.extension {
                    if old.config.extension.state != new.extension.state {
                        **extension = crate::extensions::Runtime::new(new);
                    } else {
                        let _ = extension.configure(new);
                        if old.config.effects_disabled != new.effects_disabled {
                            extension.reset();
                        }
                    }
                }
                if let Some(ducking) = &mut old.ducking {
                    ducking.configure(&new.ducking);
                }
                if matches!(new.kind, NodeKind::Recording) {
                    old.config = new.clone();
                    continue;
                }
                for (i, spec) in new.effects.iter().enumerate() {
                    if old.config.effects[i].extension != spec.extension
                        || old.config.effects[i].extension_descriptor != spec.extension_descriptor
                    {
                        old.effects[i] = registry.create(spec, SAMPLE_RATE as f32)?;
                    } else {
                        registry.configure(old.effects[i].as_mut(), spec)?;
                    }
                    if old.config.effects[i].enabled != spec.enabled
                        || old.config.effects_disabled != new.effects_disabled
                    {
                        old.effects[i].reset();
                    }
                }
                old.config = new.clone();
            }
            return Ok(());
        }
        let mut next = Self::new(project, registry)?;
        for new_node in &mut next.nodes {
            if matches!(new_node.config.kind, NodeKind::Recording) {
                continue;
            }
            if let Some(old) = self
                .nodes
                .iter_mut()
                .find(|n| n.config.id == new_node.config.id)
            {
                if matches!(new_node.config.kind, NodeKind::AutoDucking) && old.ducking.is_some() {
                    std::mem::swap(&mut new_node.ducking, &mut old.ducking);
                    new_node
                        .ducking
                        .as_mut()
                        .unwrap()
                        .configure(&new_node.config.ducking);
                }
                if old.config.kind == new_node.config.kind
                    && old.config.extension.state == new_node.config.extension.state
                {
                    std::mem::swap(&mut new_node.extension, &mut old.extension);
                    if let Some(e) = &mut new_node.extension {
                        let _ = e.configure(&new_node.config);
                        if old.config.effects_disabled != new_node.config.effects_disabled {
                            e.reset();
                        }
                    }
                }
                new_node.gains = old.gains;
                for (i, spec) in new_node.config.effects.iter().enumerate() {
                    if let Some(old_index) = old.config.effects.iter().position(|s| {
                        s.id == spec.id
                            && s.kind == spec.kind
                            && s.extension == spec.extension
                            && s.extension_descriptor == spec.extension_descriptor
                    }) {
                        std::mem::swap(&mut new_node.effects[i], &mut old.effects[old_index]);
                        registry.configure(new_node.effects[i].as_mut(), spec)?;
                        if old.config.effects[old_index].enabled != spec.enabled
                            || old.config.effects_disabled != new_node.config.effects_disabled
                        {
                            new_node.effects[i].reset();
                        }
                    }
                }
            }
        }
        *self = next;
        Ok(())
    }
    pub fn process(&mut self, inputs: &[(u64, [Frame; BLOCK_SIZE])]) {
        self.process_with_plugins(inputs, |_, _| {});
    }
    /// Native hosting is injected at each node in topological order, before its built-in chain and channel gain.
    pub fn process_with_plugins(
        &mut self,
        inputs: &[(u64, [Frame; BLOCK_SIZE])],
        mut process_plugin: impl FnMut(&Node, &mut [Frame]),
    ) {
        self.process_with_external(inputs, |node, frames| {
            if matches!(node.kind, NodeKind::VoiceChanger) {
                false
            } else {
                process_plugin(node, frames);
                true
            }
        });
    }
    /// Return true only when the host handled the node. Unhandled enabled voice
    /// nodes are silenced here, including callers using the legacy plugin API.
    pub fn process_with_external(
        &mut self,
        inputs: &[(u64, [Frame; BLOCK_SIZE])],
        process_external: impl FnMut(&Node, &mut [Frame]) -> bool,
    ) {
        self.process_recording(inputs, process_external, |_, _, _, _| {}, |_, _, _, _| {});
    }
    pub fn process_recording(
        &mut self,
        inputs: &[(u64, [Frame; BLOCK_SIZE])],
        process_external: impl FnMut(&Node, &mut [Frame]) -> bool,
        playback: impl FnMut(
            &Node,
            &[Frame; BLOCK_SIZE],
            &mut [Frame; BLOCK_SIZE],
            &mut [Frame; BLOCK_SIZE],
        ),
        tap: impl FnMut(u64, OutputPort, Option<u64>, &[Frame; BLOCK_SIZE]),
    ) {
        if self.has_extensions {
            self.process_recording_impl::<true>(inputs, process_external, playback, tap);
        } else {
            self.process_recording_impl::<false>(inputs, process_external, playback, tap);
        }
    }
    fn process_recording_impl<const EXT: bool>(
        &mut self,
        inputs: &[(u64, [Frame; BLOCK_SIZE])],
        mut process_external: impl FnMut(&Node, &mut [Frame]) -> bool,
        mut playback: impl FnMut(
            &Node,
            &[Frame; BLOCK_SIZE],
            &mut [Frame; BLOCK_SIZE],
            &mut [Frame; BLOCK_SIZE],
        ),
        mut tap: impl FnMut(u64, OutputPort, Option<u64>, &[Frame; BLOCK_SIZE]),
    ) {
        if let Some(monitor) = &mut self.monitor {
            monitor.valid = false;
        }
        for node in &mut self.nodes {
            if EXT {
                if let Some(e) = &mut node.extension {
                    for input in &mut e.inputs {
                        input.fill([0.; 2]);
                    }
                }
            }
            // Source frames are overwritten below, including missing-input silence.
            if !node.config.kind.is_source() {
                node.frames.fill([0.; 2]);
            }
            if matches!(node.config.kind, NodeKind::Recording) {
                node.preview.fill([0.; 2]);
                node.send.fill([0.; 2]);
            }
            if node.ducking.is_some() {
                node.priority.fill([0.; 2]);
                node.ducked.fill([0.; 2]);
            }
        }
        let solo = self
            .nodes
            .iter()
            .any(|n| n.config.kind.is_source() && n.config.solo);
        for &i in &self.order {
            let node = &mut self.nodes[i];
            if node.config.kind.is_source() {
                if let Some((_, frames)) = inputs.iter().find(|(id, _)| *id == node.config.id) {
                    node.frames = *frames;
                } else {
                    node.frames.fill([0.; 2]);
                }
            }
            if matches!(node.config.kind, NodeKind::VoiceChanger)
                || (node.config.kind.is_plugin()
                    && !(node.config.kind.is_processor() && node.config.effects_disabled))
            {
                let handled = process_external(&node.config, &mut node.frames);
                if !handled
                    && matches!(node.config.kind, NodeKind::VoiceChanger)
                    && !node.config.effects_disabled
                {
                    node.frames.fill([0.0; 2]);
                }
            }
            if EXT {
                if let Some(e) = &mut node.extension {
                    e.process(node.config.effects_disabled);
                    if let Some(first) = e.outputs.first() {
                        node.frames = *first;
                    }
                }
            }
            for (effect, spec) in node.effects.iter_mut().zip(&node.config.effects) {
                if let Some(m) = self
                    .monitor
                    .as_mut()
                    .filter(|m| m.node == node.config.id && m.effect == spec.id)
                {
                    m.before = node.frames;
                }
                if spec.enabled
                    && !(node.config.kind.is_processor() && node.config.effects_disabled)
                {
                    effect.process(&mut node.frames);
                }
                if let Some(m) = self
                    .monitor
                    .as_mut()
                    .filter(|m| m.node == node.config.id && m.effect == spec.id)
                {
                    m.after = node.frames;
                    m.valid = true;
                    m.bypassed = !spec.enabled
                        || (node.config.kind.is_processor() && node.config.effects_disabled);
                }
            }
            if let Some(ducking) = &mut node.ducking {
                ducking.process_connected(
                    &mut node.priority,
                    &mut node.ducked,
                    &mut node.frames,
                    node.config.effects_disabled,
                    node.ducking_inputs,
                );
            }
            if matches!(node.config.kind, NodeKind::Recording) {
                playback(
                    &node.config,
                    &node.frames,
                    &mut node.preview,
                    &mut node.send,
                );
            }
            let target = node_gains(&node.config, solo);
            let step = [
                (target[0] - node.gains[0]) / BLOCK_SIZE as f32,
                (target[1] - node.gains[1]) / BLOCK_SIZE as f32,
            ];
            let is_output = matches!(node.config.kind, NodeKind::Output { .. });
            let unity = node.gains == [1.; 2] && target == [1.; 2];
            node.peak = 0.0;
            for frame in &mut node.frames {
                if !unity {
                    node.gains[0] += step[0];
                    node.gains[1] += step[1];
                    frame[0] *= node.gains[0];
                    frame[1] *= node.gains[1];
                }
                for sample in frame {
                    if !sample.is_finite() {
                        *sample = 0.0;
                    }
                    node.peak = node.peak.max(sample.abs());
                    if is_output {
                        *sample = sample.clamp(-1.0, 1.0);
                    }
                }
            }
            node.gains = target;
            let selected = if matches!(node.config.kind, NodeKind::Switch) {
                node.config.switch_output.or_else(|| {
                    self.edges[i]
                        .first()
                        .map(|&(d, _, _)| self.nodes[d].config.id)
                })
            } else {
                None
            };
            let is_switch = matches!(self.nodes[i].config.kind, NodeKind::Switch);
            tap(
                self.nodes[i].config.id,
                OutputPort::Passthrough,
                selected,
                &self.nodes[i].frames,
            );
            if matches!(self.nodes[i].config.kind, NodeKind::Recording) {
                tap(
                    self.nodes[i].config.id,
                    OutputPort::Preview,
                    None,
                    &self.nodes[i].preview,
                );
                tap(
                    self.nodes[i].config.id,
                    OutputPort::Send,
                    None,
                    &self.nodes[i].send,
                );
            }
            if matches!(self.nodes[i].config.kind, NodeKind::AutoDucking) {
                tap(
                    self.nodes[i].config.id,
                    OutputPort::Priority,
                    None,
                    &self.nodes[i].priority,
                );
                tap(
                    self.nodes[i].config.id,
                    OutputPort::Ducked,
                    None,
                    &self.nodes[i].ducked,
                );
            }
            if EXT {
                if let Some(e) = &self.nodes[i].extension {
                    for (id, frames) in e.output_ids.iter().zip(&e.outputs) {
                        tap(
                            self.nodes[i].config.id,
                            OutputPort::Extension(*id),
                            None,
                            frames,
                        );
                    }
                }
            }
            for &(destination, port, input) in &self.edges[i] {
                if is_switch && Some(self.nodes[destination].config.id) != selected {
                    continue;
                }
                // Validation rejects cycles/self routes; borrow disjoint nodes to avoid
                // materializing a block-sized copy for each fan-out connection.
                let (source, destination_node) = if i < destination {
                    let (before, after) = self.nodes.split_at_mut(destination);
                    (&before[i], &mut after[0])
                } else {
                    let (before, after) = self.nodes.split_at_mut(i);
                    (&after[0], &mut before[destination])
                };
                let routed = match port {
                    RenderOutput::Extension(index) => {
                        &source.extension.as_ref().unwrap().outputs[index as usize]
                    }
                    RenderOutput::Passthrough => &source.frames,
                    RenderOutput::Preview => &source.preview,
                    RenderOutput::Send => &source.send,
                    RenderOutput::Priority => &source.priority,
                    RenderOutput::Ducked => &source.ducked,
                };
                let target = match input {
                    RenderInput::Extension(index) => {
                        &mut destination_node.extension.as_mut().unwrap().inputs[index as usize]
                    }
                    RenderInput::Main => &mut destination_node.frames,
                    RenderInput::Priority => &mut destination_node.priority,
                    RenderInput::Ducked => &mut destination_node.ducked,
                };
                for (dst, src) in target.iter_mut().zip(routed) {
                    dst[0] += src[0];
                    dst[1] += src[1];
                }
            }
        }
    }
}

pub fn validate_with_registry(p: &Project, r: &EffectRegistry) -> Result<(), String> {
    validate(p)?;
    for n in &p.nodes {
        for e in &n.effects {
            r.validate(e)?;
        }
    }
    Ok(())
}
fn compile_output(n: &Node, p: OutputPort) -> RenderOutput {
    match p {
        OutputPort::Passthrough => RenderOutput::Passthrough,
        OutputPort::Preview => RenderOutput::Preview,
        OutputPort::Send => RenderOutput::Send,
        OutputPort::Priority => RenderOutput::Priority,
        OutputPort::Ducked => RenderOutput::Ducked,
        OutputPort::Extension(id) => {
            let NodeKind::Extension { definition } = &n.kind else {
                unreachable!()
            };
            RenderOutput::Extension(
                definition.outputs.iter().position(|p| p.id == id).unwrap() as u8
            )
        }
    }
}
fn compile_input(n: &Node, p: InputPort) -> RenderInput {
    match p {
        InputPort::Main => RenderInput::Main,
        InputPort::Priority => RenderInput::Priority,
        InputPort::Ducked => RenderInput::Ducked,
        InputPort::Extension(id) => {
            let NodeKind::Extension { definition } = &n.kind else {
                unreachable!()
            };
            RenderInput::Extension(definition.inputs.iter().position(|p| p.id == id).unwrap() as u8)
        }
    }
}
fn node_gains(node: &Node, solo: bool) -> Frame {
    if matches!(node.kind, NodeKind::Recording | NodeKind::AutoDucking) {
        return [1.; 2];
    }
    if node.kind.is_processor() {
        if matches!(node.kind, NodeKind::VoiceChanger) && !node.effects_disabled {
            let gain = db_to_gain(node.voice.as_ref().map_or(0.0, |v| v.output_db));
            return [gain; 2];
        }
        return [1.0; 2];
    }
    let gain = if node.mute || (solo && node.kind.is_source() && !node.solo) {
        0.0
    } else {
        db_to_gain(node.gain_db)
    };
    [
        gain * (1.0 - node.pan.max(0.0)),
        gain * (1.0 + node.pan.min(0.0)),
    ]
}
#[cfg(test)]
mod tests {
    #[test]
    fn sources_handle_reordering_missing_audio_and_backward_fanout() {
        use crate::project::Node;
        let mut p = Project {
            nodes: vec![
                Node::new(4, "Out", NodeKind::Output { device: None }, [0.; 2]),
                Node::new(3, "Sum", NodeKind::Processor, [0.; 2]),
                Node::new(1, "A", NodeKind::Soundboard, [0.; 2]),
                Node::new(2, "B", NodeKind::Soundboard, [0.; 2]),
            ],
            connections: vec![],
            ..Project::default()
        };
        p.connect(1, 3).unwrap();
        p.connect(2, 3).unwrap();
        p.connect(3, 4).unwrap();
        p.connect(1, 4).unwrap();
        let mut g = Graph::new(&p, &EffectRegistry::default()).unwrap();
        for inputs in [
            vec![
                (1, [[0.1, -0.2]; BLOCK_SIZE]),
                (2, [[0.3, 0.1]; BLOCK_SIZE]),
            ],
            vec![
                (2, [[0.3, 0.1]; BLOCK_SIZE]),
                (1, [[0.1, -0.2]; BLOCK_SIZE]),
            ],
            vec![],
            vec![(2, [[0.3, 0.1]; BLOCK_SIZE])],
            vec![(1, [[0.1, -0.2]; BLOCK_SIZE])],
        ] {
            g.process(&inputs);
            let a = inputs
                .iter()
                .find(|(id, _)| *id == 1)
                .map_or([0.; 2], |(_, f)| f[0]);
            let b = inputs
                .iter()
                .find(|(id, _)| *id == 2)
                .map_or([0.; 2], |(_, f)| f[0]);
            // A fans out directly before Sum is processed. Preserve that addition order.
            let expected = [a[0] + (a[0] + b[0]), a[1] + (a[1] + b[1])];
            assert_eq!(g.nodes[0].frames, [expected; BLOCK_SIZE]);
        }
    }

    use super::*;
    #[test]
    fn diagnostic_tap_is_exact_isolated_bypassed_and_never_changes_audio() {
        let registry = EffectRegistry::default();
        let mut p = Project {
            nodes: vec![
                Node::new(1, "Source", NodeKind::Soundboard, [0.; 2]),
                Node::new(2, "Rack", NodeKind::Processor, [0.; 2]),
            ],
            connections: vec![],
            ..Project::default()
        };
        p.connect(1, 2).unwrap();
        for gain in [6., -3.] {
            let mut effect = crate::effects::EffectSpec::new(registry.get("gain").unwrap());
            effect.parameters.insert("gain".into(), gain);
            p.nodes[1].effects.push(effect);
        }
        let target = p.nodes[1].effects[1].id;
        let mut measured = Graph::new(&p, &registry).unwrap();
        let mut reference = Graph::new(&p, &registry).unwrap();
        measured.monitor = Some(Box::new(EffectMonitor::new(2, target, 77)));
        let inputs = [(1, [[0.1, -0.2]; BLOCK_SIZE])];
        measured.process(&inputs);
        reference.process(&inputs);
        assert_eq!(measured.nodes[1].frames, reference.nodes[1].frames);
        let tap = measured.monitor.as_ref().unwrap();
        assert!(tap.valid && !tap.bypassed);
        assert_eq!(tap.token, 77);
        assert!((tap.before[0][0] - 0.1 * db_to_gain(6.)).abs() < 1e-6);
        assert!((tap.after[0][1] - (-0.2 * db_to_gain(3.))).abs() < 1e-6);
        p.nodes[1].effects[1].enabled = false;
        measured.update(&p, &registry).unwrap();
        measured.process(&inputs);
        let tap = measured.monitor.as_ref().unwrap();
        assert!(tap.valid && tap.bypassed);
        assert_eq!(tap.before, tap.after);
        measured.monitor.as_mut().unwrap().effect = u64::MAX;
        measured.process(&inputs);
        assert!(!measured.monitor.as_ref().unwrap().valid);
    }
    #[test]
    fn recording_ports_are_isolated_and_old_connections_default_to_passthrough() {
        let mut p = Project {
            nodes: vec![
                Node::new(1, "Input", NodeKind::Soundboard, [0.; 2]),
                Node::new(2, "Recorder", NodeKind::Recording, [0.; 2]),
                Node::new(3, "Live", NodeKind::Processor, [0.; 2]),
                Node::new(4, "Preview", NodeKind::Processor, [0.; 2]),
                Node::new(5, "Send", NodeKind::Processor, [0.; 2]),
            ],
            connections: vec![],
            ..Project::default()
        };
        p.connect(1, 2).unwrap();
        for (to, port) in [
            (3, OutputPort::Passthrough),
            (4, OutputPort::Preview),
            (5, OutputPort::Send),
        ] {
            p.connect_port(2, to, port).unwrap();
        }
        let mut graph = Graph::new(&p, &EffectRegistry::default()).unwrap();
        graph.process_recording(
            &[(1, [[0.2; 2]; BLOCK_SIZE])],
            |_, _| true,
            |_, input, preview, send| {
                assert_eq!(input, &[[0.2; 2]; BLOCK_SIZE]);
                preview.fill([0.3; 2]);
                send.fill([0.4; 2]);
            },
            |_, _, _, _| {},
        );
        assert_eq!(graph.nodes[2].frames, [[0.2; 2]; BLOCK_SIZE]);
        assert_eq!(graph.nodes[3].frames, [[0.3; 2]; BLOCK_SIZE]);
        assert_eq!(graph.nodes[4].frames, [[0.4; 2]; BLOCK_SIZE]);
        assert!(p.connect_port(1, 3, OutputPort::Preview).is_err());
        let legacy: crate::project::Connection =
            serde_json::from_str(r#"{"from":1,"to":2}"#).unwrap();
        assert_eq!(legacy.port, OutputPort::Passthrough);
        p.connect_port(2, 3, OutputPort::Preview).unwrap();
        assert!(p.connect_port(2, 3, OutputPort::Preview).is_err());
    }
    #[test]
    fn recorded_playback_is_not_a_live_loopback_feedback_path() {
        let mut p = Project {
            nodes: vec![
                Node::new(
                    1,
                    "Desktop",
                    NodeKind::Input {
                        device: None,
                        loopback: true,
                    },
                    [0.; 2],
                ),
                Node::new(2, "Recorder", NodeKind::Recording, [0.; 2]),
                Node::new(3, "Speakers", NodeKind::Output { device: None }, [0.; 2]),
            ],
            connections: vec![],
            ..Project::default()
        };
        p.connect(1, 2).unwrap();
        p.connect_port(2, 3, OutputPort::Preview).unwrap();
        assert!(validate_loopback_routes(&p, |_| "same-device".into()).is_ok());
        p.connect(2, 3).unwrap();
        assert!(validate_loopback_routes(&p, |_| "same-device".into()).is_err());
    }
    fn switch_project() -> Project {
        let mut project = Project {
            nodes: vec![
                Node::new(1, "Input", NodeKind::Soundboard, [0.; 2]),
                Node::new(2, "Choose voice", NodeKind::Switch, [0.; 2]),
                Node::new(3, "Voice A", NodeKind::Processor, [0.; 2]),
                Node::new(4, "Voice B", NodeKind::Processor, [0.; 2]),
            ],
            connections: vec![],
            ..Project::default()
        };
        project.connect(1, 2).unwrap();
        project.connect(2, 3).unwrap();
        project.connect(2, 4).unwrap();
        project
    }
    #[test]
    fn switch_routes_exactly_one_destination_and_updates_live() {
        let mut project = switch_project();
        let registry = EffectRegistry::default();
        let mut graph = Graph::new(&project, &registry).unwrap();
        graph.process(&[(1, [[0.2; 2]; BLOCK_SIZE])]);
        assert_eq!(graph.nodes[2].frames, [[0.2; 2]; BLOCK_SIZE]);
        assert_eq!(graph.nodes[3].frames, [[0.; 2]; BLOCK_SIZE]);
        assert!(project.select_switch_output(2, 4));
        graph.update(&project, &registry).unwrap();
        graph.process(&[(1, [[0.3; 2]; BLOCK_SIZE])]);
        assert_eq!(graph.nodes[2].frames, [[0.; 2]; BLOCK_SIZE]);
        assert_eq!(graph.nodes[3].frames, [[0.3; 2]; BLOCK_SIZE]);
        assert!(!project.select_switch_output(2, 1));
        assert_eq!(project.nodes[1].switch_output, Some(4));
        let restored: Project =
            serde_json::from_str(&serde_json::to_string(&project).unwrap()).unwrap();
        assert_eq!(restored.nodes[1].switch_output, Some(4));
    }
    #[test]
    fn switch_deletion_reselects_remaining_route_and_cleans_shortcuts() {
        let mut project = switch_project();
        project.nodes[1].switch_keybinds.insert(3, "Ctrl+F9".into());
        project.nodes[1]
            .switch_keybinds
            .insert(4, "Ctrl+F10".into());
        project.disconnect(2, 3);
        assert_eq!(project.nodes[1].switch_output, Some(4));
        assert!(!project.nodes[1].switch_keybinds.contains_key(&3));
        project.remove_node(4);
        assert_eq!(project.nodes[1].switch_output, None);
        assert!(project.nodes[1].switch_keybinds.is_empty());
        validate(&project).unwrap();
        project.nodes[1].switch_output = Some(999);
        assert!(validate(&project).is_err());
    }
    #[test]
    fn effect_reordering_preserves_the_correct_instance_history() {
        let registry = EffectRegistry::default();
        let mut project = Project {
            nodes: vec![
                Node::new(1, "Source", NodeKind::Soundboard, [0.; 2]),
                Node::new(2, "Effects", NodeKind::Processor, [0.; 2]),
            ],
            connections: vec![crate::project::Connection {
                input: Default::default(),
                port: Default::default(),
                from: 1,
                to: 2,
            }],
            ..Project::default()
        };
        let mut first = crate::effects::EffectSpec::new(registry.get("delay").unwrap());
        first.parameters.insert("time".into(), 10.);
        first.parameters.insert("mix".into(), 1.);
        first.parameters.insert("feedback".into(), 0.);
        let mut second = crate::effects::EffectSpec::new(registry.get("delay").unwrap());
        second.enabled = false;
        project.nodes[1].effects = vec![first, second];
        let mut graph = Graph::new(&project, &registry).unwrap();
        let mut impulse = [[0.; 2]; BLOCK_SIZE];
        impulse[0] = [0.5; 2];
        graph.process(&[(1, impulse)]);
        project.nodes[1].effects.swap(0, 1);
        graph.update(&project, &registry).unwrap();
        for _ in 0..2 {
            graph.process(&[]);
        }
        graph.process(&[]);
        assert_eq!(graph.nodes[1].frames[96], [0.5; 2]);
    }
    #[test]
    fn legacy_effect_ids_are_assigned_and_shortcuts_roundtrip_by_identity() {
        use crate::shortcuts::{EffectAction, ShortcutAction};
        let registry = EffectRegistry::default();
        let mut project = switch_project();
        let mut effect = crate::effects::EffectSpec::new(registry.get("gain").unwrap());
        effect
            .keybinds
            .insert(EffectAction::Toggle, "Ctrl+F8".into());
        let id = effect.id;
        project.nodes[2].effects.push(effect);
        let restored: Project =
            serde_json::from_str(&serde_json::to_string(&project).unwrap()).unwrap();
        assert_eq!(restored.nodes[2].effects[0].id, id);
        assert_eq!(
            restored.nodes[2].effects[0].keybinds[&EffectAction::Toggle],
            "Ctrl+F8"
        );
        let action = ShortcutAction::Effect {
            id: 3,
            effect: id,
            action: EffectAction::Toggle,
        };
        assert!(action.exists(&project));
        project.nodes[2].effects.clear();
        assert!(!action.exists(&project));
        let mut legacy = serde_json::to_value(&restored).unwrap();
        legacy["nodes"][2]["effects"][0]
            .as_object_mut()
            .unwrap()
            .remove("id");
        legacy["nodes"][2]["effects"][0]
            .as_object_mut()
            .unwrap()
            .remove("keybinds");
        let migrated: Project = serde_json::from_value(legacy).unwrap();
        assert!(migrated.nodes[2].effects[0].keybinds.is_empty());
        validate(&migrated).unwrap();
    }
    #[test]
    fn legacy_projects_and_missing_voice_references_roundtrip() {
        let original = Project::default();
        let mut value = serde_json::to_value(&original).unwrap();
        for node in value["nodes"].as_array_mut().unwrap() {
            node.as_object_mut().unwrap().remove("voice");
        }
        let mut restored: Project = serde_json::from_value(value).unwrap();
        assert_eq!(restored.nodes, original.nodes);
        let mut voice = Node::new(42, "Missing voice", NodeKind::VoiceChanger, [0.0; 2]);
        voice.voice.as_mut().unwrap().pack = "missing-pack-v1".into();
        voice.keybinds.insert(
            crate::shortcuts::NodeAction::ToggleDisabled,
            "Ctrl+Alt+F8".into(),
        );
        restored.nodes.push(voice.clone());
        assert!(validate(&restored).is_ok());
        let loaded: Project =
            serde_json::from_str(&serde_json::to_string(&restored).unwrap()).unwrap();
        assert_eq!(loaded.nodes.last().unwrap(), &voice);
    }
    #[test]
    fn voice_fails_silent_without_host_bypasses_and_converts_once() {
        let mut p = Project {
            nodes: vec![
                Node::new(
                    1,
                    "Mic",
                    NodeKind::Input {
                        device: None,
                        loopback: false,
                    },
                    [0.0; 2],
                ),
                Node::new(2, "Voice", NodeKind::VoiceChanger, [0.0; 2]),
                Node::new(3, "Bus A", NodeKind::Output { device: None }, [0.0; 2]),
                Node::new(4, "Bus B", NodeKind::Output { device: None }, [0.0; 2]),
            ],
            connections: vec![
                crate::project::Connection {
                    input: Default::default(),
                    port: Default::default(),
                    from: 1,
                    to: 2,
                },
                crate::project::Connection {
                    input: Default::default(),
                    port: Default::default(),
                    from: 2,
                    to: 3,
                },
                crate::project::Connection {
                    input: Default::default(),
                    port: Default::default(),
                    from: 2,
                    to: 4,
                },
            ],
            ..Default::default()
        };
        let registry = EffectRegistry::default();
        let input = [(1, [[0.25; 2]; BLOCK_SIZE])];
        let mut graph = Graph::new(&p, &registry).unwrap();
        graph.process(&input);
        assert!(graph.nodes[2].frames.iter().flatten().all(|v| *v == 0.0));
        graph.process_with_external(&input, |_, _| false);
        assert!(graph.nodes[2].frames.iter().flatten().all(|v| *v == 0.0));
        let mut count = 0;
        graph.process_with_external(&input, |node, frames| {
            assert_eq!(node.id, 2);
            count += 1;
            frames.fill([0.1; 2]);
            true
        });
        assert_eq!(count, 1);
        assert_eq!(graph.nodes[2].frames, graph.nodes[3].frames);
        assert!(graph.nodes[2].frames[100][0] > 0.09);
        p.nodes[1].effects_disabled = true;
        graph.update(&p, &registry).unwrap();
        graph.process(&input);
        assert!((graph.nodes[2].frames[100][0] - 0.25).abs() < 1e-6);
        let mut other = Node::new(5, "Second voice", NodeKind::VoiceChanger, [0.0; 2]);
        other.effects_disabled = true;
        p.nodes.push(other);
        assert!(validate(&p).is_ok());
        p.nodes[1].effects_disabled = false;
        p.nodes[4].effects_disabled = false;
        assert!(validate(&p).unwrap_err().contains("Only one Voice Changer"));
    }
    #[test]
    fn application_capture_is_routable_persistent_and_uses_source_shortcuts() {
        use crate::project::ApplicationTarget;
        let mut p = fixture();
        p.nodes[0].kind = NodeKind::ApplicationCapture {
            target: Some(ApplicationTarget {
                executable: "C:/Apps/player.exe".into(),
                window_title: "Player".into(),
                match_title: false,
            }),
        };
        let restored: Project = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(restored.nodes[0], p.nodes[0]);
        let registry = EffectRegistry::default();
        let mut graph = Graph::new(&restored, &registry).unwrap();
        graph.process(&[(1, [[0.25; 2]; BLOCK_SIZE])]);
        assert!(graph
            .nodes
            .iter()
            .any(|n| matches!(n.config.kind, NodeKind::Output { .. }) && n.peak > 0.));
        crate::shortcuts::NodeAction::Mute
            .apply(&mut p.nodes[0])
            .unwrap();
        graph.update(&p, &registry).unwrap();
        graph.process(&[(1, [[0.25; 2]; BLOCK_SIZE])]);
        graph.process(&[(1, [[0.25; 2]; BLOCK_SIZE])]);
        assert!(graph
            .nodes
            .iter()
            .filter(|n| matches!(n.config.kind, NodeKind::Output { .. }))
            .all(|n| n.peak == 0.));
        if let NodeKind::ApplicationCapture {
            target: Some(target),
        } = &mut p.nodes[0].kind
        {
            target.executable.clear();
        }
        assert!(Graph::new(&p, &registry).is_err());
    }
    #[test]
    fn vst_nodes_follow_topology_bypass_gain_and_solo_rules() {
        let mut p = Project::default();
        p.nodes.clear();
        p.connections.clear();
        p.nodes
            .push(Node::new(1, "Synth", NodeKind::VstInstrument, [0.0; 2]));
        p.nodes
            .push(Node::new(2, "Plugin FX", NodeKind::VstEffect, [0.0; 2]));
        p.nodes.push(Node::new(
            3,
            "Out",
            NodeKind::Output { device: None },
            [0.0; 2],
        ));
        p.connections = vec![
            crate::project::Connection {
                input: Default::default(),
                port: Default::default(),
                from: 1,
                to: 2,
            },
            crate::project::Connection {
                input: Default::default(),
                port: Default::default(),
                from: 2,
                to: 3,
            },
        ];
        // A VST effect has processor semantics, even with legacy channel fields.
        p.nodes[1].gain_db = -20.0;
        p.nodes[1].mute = true;
        p.nodes[1].solo = true;
        let registry = EffectRegistry::default();
        let mut graph = Graph::new(&p, &registry).unwrap();
        let render = |graph: &mut Graph| {
            let mut order = vec![];
            graph.process_with_plugins(&[], |node, frames| {
                order.push(node.id);
                if node.id == 1 {
                    frames.fill([0.4; 2]);
                } else {
                    for frame in frames {
                        frame[0] *= 0.5;
                        frame[1] *= 0.5;
                    }
                }
            });
            order
        };
        assert_eq!(render(&mut graph), vec![1, 2]);
        assert!((graph.nodes[2].frames[127][0] - 0.2).abs() < 0.0001);
        p.nodes[1].effects_disabled = true;
        graph.update(&p, &registry).unwrap();
        assert_eq!(render(&mut graph), vec![1]);
        assert!((graph.nodes[2].frames[127][0] - 0.4).abs() < 0.0001);
        assert!(p.connect(2, 1).is_err());
        assert!(p.connect(3, 2).is_err());
        p.nodes[0].mute = true;
        graph.update(&p, &registry).unwrap();
        render(&mut graph);
        assert_eq!(graph.nodes[2].frames[127], [0.0; 2]);
    }
    #[test]
    fn listen_feedback_is_rejected_even_with_a_different_primary_output() {
        let mut p = Project::default();
        p.nodes[1].mute = false;
        p.nodes[4].kind = NodeKind::Output {
            device: Some("cable::1".into()),
        };
        p.nodes[4].listen_enabled = true;
        let identity = |id: &Option<String>| id.clone().unwrap_or_else(|| "default::1".into());
        assert!(validate_loopback_routes(&p, identity).is_err());
        p.nodes[4].listen_enabled = false;
        assert!(validate_loopback_routes(&p, identity).is_ok());
        p.nodes[4].listen_enabled = true;
        p.nodes[4].listen_device = Some("headphones::1".into());
        assert!(validate_loopback_routes(&p, identity).is_ok());
        p.nodes[4].listen_device = None;
        p.nodes[4].mute = true;
        assert!(validate_loopback_routes(&p, identity).is_ok());
    }
    #[test]
    fn processor_disable_is_true_passthrough_and_preserves_effect_flags() {
        let mut p = fixture();
        let r = EffectRegistry::default();
        let mut gain = EffectSpec::new(r.get("gain").unwrap());
        gain.parameters.insert("gain".into(), -6.0206);
        p.nodes[2].effects.push(gain);
        p.nodes[2].gain_db = -30.0;
        p.nodes[2].pan = -1.0;
        p.nodes[2].mute = true;
        let original = p.nodes[2].effects.clone();
        let mut g = Graph::new(&p, &r).unwrap();
        g.process(&[(1, [[0.4; 2]; BLOCK_SIZE])]);
        assert!((g.nodes[3].frames[0][0] - 0.2).abs() < 1e-5);
        for (action, expected) in [
            (crate::shortcuts::NodeAction::Disable, 0.4),
            (crate::shortcuts::NodeAction::Enable, 0.2),
            (crate::shortcuts::NodeAction::ToggleDisabled, 0.4),
        ] {
            action.apply(&mut p.nodes[2]).unwrap();
            g.update(&p, &r).unwrap();
            g.process(&[(1, [[0.4; 2]; BLOCK_SIZE])]);
            assert!((g.nodes[3].frames[0][0] - expected).abs() < 1e-5);
            assert_eq!(p.nodes[2].effects, original);
        }
    }
    #[test]
    fn source_shortcuts_change_processed_audio() {
        use crate::shortcuts::NodeAction as A;
        let mut p = fixture();
        let r = EffectRegistry::default();
        let mut g = Graph::new(&p, &r).unwrap();
        for (action, expected) in [
            (A::Mute, [0.0, 0.0]),
            (A::Unmute, [0.4, 0.4]),
            (A::ToggleMute, [0.0, 0.0]),
            (A::ToggleMute, [0.4, 0.4]),
            (A::PanLeft100, [0.4, 0.0]),
            (A::PanLeft50, [0.4, 0.2]),
            (A::PanCenter, [0.4, 0.4]),
            (A::PanRight50, [0.2, 0.4]),
            (A::PanRight100, [0.0, 0.4]),
        ] {
            action.apply(&mut p.nodes[0]).unwrap();
            g.update(&p, &r).unwrap();
            g.process(&[(1, [[0.4; 2]; BLOCK_SIZE])]);
            g.process(&[(1, [[0.4; 2]; BLOCK_SIZE])]);
            assert_eq!(g.nodes[3].frames[0], expected, "{action:?}");
        }
        A::PanCenter.apply(&mut p.nodes[0]).unwrap();
        A::VolumeUp.apply(&mut p.nodes[0]).unwrap();
        g.update(&p, &r).unwrap();
        g.process(&[(1, [[0.4; 2]; BLOCK_SIZE])]);
        g.process(&[(1, [[0.4; 2]; BLOCK_SIZE])]);
        assert!((g.nodes[3].frames[0][0] - 0.4488074).abs() < 1e-5);
        A::VolumeDown.apply(&mut p.nodes[0]).unwrap();
        A::SoloOn.apply(&mut p.nodes[0]).unwrap();
        g.update(&p, &r).unwrap();
        g.process(&[(1, [[0.4; 2]; BLOCK_SIZE]), (2, [[0.2; 2]; BLOCK_SIZE])]);
        g.process(&[(1, [[0.4; 2]; BLOCK_SIZE]), (2, [[0.2; 2]; BLOCK_SIZE])]);
        assert_eq!(g.nodes[3].frames[0], [0.4; 2]);
        A::SoloOff.apply(&mut p.nodes[0]).unwrap();
        g.update(&p, &r).unwrap();
        g.process(&[(1, [[0.4; 2]; BLOCK_SIZE]), (2, [[0.2; 2]; BLOCK_SIZE])]);
        g.process(&[(1, [[0.4; 2]; BLOCK_SIZE]), (2, [[0.2; 2]; BLOCK_SIZE])]);
        assert!((g.nodes[3].frames[0][0] - 0.6).abs() < 1e-6);
    }
    #[test]
    fn legacy_projects_load_with_unbound_node_keys_and_enabled_processors() {
        let p = fixture();
        let mut data = serde_json::to_value(&p).unwrap();
        for node in data["nodes"].as_array_mut().unwrap() {
            let fields = node.as_object_mut().unwrap();
            fields.remove("effects_disabled");
            fields.remove("keybinds");
        }
        let restored: Project = serde_json::from_value(data).unwrap();
        assert!(restored
            .nodes
            .iter()
            .all(|n| !n.effects_disabled && n.keybinds.is_empty()));
        Graph::new(&restored, &EffectRegistry::default()).unwrap();
    }
    #[test]
    fn node_keybinds_roundtrip_and_are_removed_with_node() {
        let mut p = fixture();
        p.nodes[0]
            .keybinds
            .insert(crate::shortcuts::NodeAction::Mute, "Ctrl+Alt+F1".into());
        p.nodes[2].effects_disabled = true;
        p.nodes[2].keybinds.insert(
            crate::shortcuts::NodeAction::ToggleDisabled,
            "Ctrl+Alt+F2".into(),
        );
        let mut restored: Project =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(restored.nodes, p.nodes);
        restored.remove_node(3);
        assert!(!restored.nodes.iter().any(|n| n.id == 3));
        assert!(restored.connections.is_empty());
    }
    #[test]
    fn rejects_loopback_to_captured_device() {
        let mut p = Project::default();
        p.nodes[1].mute = false;
        p.connect(2, 4).unwrap();
        assert!(
            validate_loopback_routes(&p, |id| id.clone().unwrap_or_else(|| "default".into()))
                .is_err()
        );
        p.nodes[3].mute = true;
        assert!(
            validate_loopback_routes(&p, |id| id.clone().unwrap_or_else(|| "default".into()))
                .is_ok()
        );
    }
    #[test]
    fn parameter_changes_preserve_delay_history() {
        let mut p = fixture();
        let r = EffectRegistry::default();
        let mut spec = EffectSpec::new(r.get("delay").unwrap());
        spec.parameters.insert("time".into(), 10.0);
        spec.parameters.insert("mix".into(), 1.0);
        spec.parameters.insert("feedback".into(), 0.0);
        p.nodes[2].effects.push(spec);
        let mut graph = Graph::new(&p, &r).unwrap();
        let mut impulse = [[0.0; 2]; BLOCK_SIZE];
        impulse[0] = [1.0; 2];
        graph.process(&[(1, impulse)]);
        p.nodes[2].effects[0]
            .parameters
            .insert("feedback".into(), 0.1);
        graph.update(&p, &r).unwrap();
        graph.process(&[]);
        graph.process(&[]);
        graph.process(&[]);
        assert_eq!(graph.nodes[3].frames[96], [1.0; 2]);
    }
    #[test]
    fn mute_is_ramped_then_silent() {
        let mut p = fixture();
        let r = EffectRegistry::default();
        let mut graph = Graph::new(&p, &r).unwrap();
        p.nodes[0].mute = true;
        graph.update(&p, &r).unwrap();
        graph.process(&[(1, [[0.5; 2]; BLOCK_SIZE])]);
        assert!(graph.nodes[3].frames[0][0] > 0.4);
        graph.process(&[(1, [[0.5; 2]; BLOCK_SIZE])]);
        assert_eq!(graph.nodes[3].frames[0], [0.0; 2]);
    }
    use crate::{
        effects::EffectSpec,
        project::{Connection, Node},
    };
    fn fixture() -> Project {
        Project {
            version: 1,
            name: "Test".into(),
            nodes: vec![
                Node::new(1, "A", NodeKind::Soundboard, [0.0; 2]),
                Node::new(2, "B", NodeKind::Soundboard, [0.0; 2]),
                Node::new(3, "FX", NodeKind::Processor, [0.0; 2]),
                Node::new(4, "Out", NodeKind::Output { device: None }, [0.0; 2]),
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
                Connection {
                    input: Default::default(),
                    port: Default::default(),
                    from: 3,
                    to: 4,
                },
            ],
            sounds: vec![],
        }
    }
    #[test]
    fn routes_and_sums_sources() {
        let p = fixture();
        let mut g = Graph::new(&p, &EffectRegistry::default()).unwrap();
        g.process(&[(1, [[0.2; 2]; BLOCK_SIZE]), (2, [[0.3; 2]; BLOCK_SIZE])]);
        assert!((g.nodes[3].frames[0][0] - 0.5).abs() < 1e-6);
    }
    #[test]
    fn prevents_cycles_transactionally() {
        let mut p = fixture();
        let id = p.next_id();
        p.nodes
            .push(Node::new(id, "FX2", NodeKind::Processor, [0.0; 2]));
        p.connect(3, id).unwrap();
        let before = p.connections.len();
        assert!(p.connect(id, 3).is_err());
        assert_eq!(p.connections.len(), before);
    }
    #[test]
    fn mute_solo_and_pan_affect_real_signal() {
        let mut p = fixture();
        p.nodes[0].solo = true;
        p.nodes[0].pan = 1.0;
        let mut g = Graph::new(&p, &EffectRegistry::default()).unwrap();
        g.process(&[(1, [[0.2; 2]; BLOCK_SIZE]), (2, [[0.3; 2]; BLOCK_SIZE])]);
        assert_eq!(g.nodes[3].frames[0], [0.0, 0.2]);
    }
    #[test]
    fn clips_output_and_scrubs_nan() {
        let p = fixture();
        let mut g = Graph::new(&p, &EffectRegistry::default()).unwrap();
        g.process(&[(1, [[2.0, f32::NAN]; BLOCK_SIZE])]);
        assert_eq!(g.nodes[3].frames[0], [1.0, 0.0]);
        assert!(g.nodes[3].peak >= 2.0);
    }
    #[test]
    fn bypass_changes_audio() {
        let mut p = fixture();
        let r = EffectRegistry::default();
        let mut spec = EffectSpec::new(r.get("gain").unwrap());
        spec.parameters.insert("gain".into(), -6.0206);
        p.nodes[2].effects.push(spec);
        let mut g = Graph::new(&p, &r).unwrap();
        g.process(&[(1, [[0.4; 2]; BLOCK_SIZE])]);
        assert!((g.nodes[3].frames[0][0] - 0.2).abs() < 1e-5);
        p.nodes[2].effects[0].enabled = false;
        g.update(&p, &r).unwrap();
        g.process(&[(1, [[0.4; 2]; BLOCK_SIZE])]);
        assert_eq!(g.nodes[3].frames[0], [0.4; 2]);
    }
    #[test]
    fn rejects_unknown_effect_and_dangling_edges() {
        let mut p = fixture();
        p.connections[0].to = 99;
        assert!(validate(&p).is_err());
        p = fixture();
        p.nodes[0].effects.push(EffectSpec {
            extension: None,
            extension_descriptor: None,
            extension_keybinds: Default::default(),
            id: 123,
            keybinds: Default::default(),
            kind: "missing".into(),
            enabled: true,
            parameters: Default::default(),
        });
        assert!(Graph::new(&p, &EffectRegistry::default()).is_err());
    }
    #[test]
    fn project_roundtrip() {
        let p = fixture();
        let serialized = serde_json::to_string(&p).unwrap();
        let restored: Project = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored.nodes, p.nodes);
        assert_eq!(restored.connections, p.connections);
    }
    #[test]
    fn removal_cleans_connections() {
        let mut p = fixture();
        p.remove_node(3);
        assert!(p.connections.is_empty());
        assert!(validate(&p).is_ok());
    }
}
