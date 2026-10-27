//! Immutable extension catalog and preallocated graph instances. The desktop
//! owns discovery/loading; core never reads packages or loads native code.
use crate::{
    effects::{AudioEffect, EffectFactory, ParameterDescriptor},
    project::{Node, NodeKind},
    Frame, BLOCK_SIZE, SAMPLE_RATE,
};
pub use piratewave_extension_sdk::{Action, Manifest, Parameter, TypeDescriptor, TypeKind};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub schema: u32,
    pub bytes: Vec<u8>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            schema: 1,
            bytes: vec![],
        }
    }
}
impl State {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema == 0 || self.bytes.len() > piratewave_extension_sdk::MAX_STATE {
            Err("Invalid extension state".into())
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeSettings {
    pub state: State,
    pub parameters: BTreeMap<String, f32>,
    pub keybinds: BTreeMap<String, String>,
}
/// Keep unused extension configuration out of built-in node storage/clones.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeExtension(Option<Box<NodeSettings>>);
impl NodeExtension {
    pub fn new(settings: NodeSettings) -> Self {
        Self(Some(Box::new(settings)))
    }
}
static EMPTY_SETTINGS: NodeSettings = NodeSettings {
    state: State {
        schema: 1,
        bytes: Vec::new(),
    },
    parameters: BTreeMap::new(),
    keybinds: BTreeMap::new(),
};
impl std::ops::Deref for NodeExtension {
    type Target = NodeSettings;
    fn deref(&self) -> &NodeSettings {
        self.0.as_deref().unwrap_or(&EMPTY_SETTINGS)
    }
}
impl std::ops::DerefMut for NodeExtension {
    fn deref_mut(&mut self) -> &mut NodeSettings {
        self.0.get_or_insert_with(Box::default)
    }
}
impl NodeSettings {
    pub fn is_empty(&self) -> bool {
        self.state == State::default() && self.parameters.is_empty() && self.keybinds.is_empty()
    }
}
pub trait Instance: Send {
    fn set(&mut self, index: u32, value: f32) -> bool;
    fn process(
        &mut self,
        inputs: &[[Frame; BLOCK_SIZE]],
        outputs: &mut [[Frame; BLOCK_SIZE]],
    ) -> bool;
    fn reset(&mut self) -> bool;
}
pub trait Provider: Send + Sync {
    fn migrate(&self, _old: &State) -> Result<State, String> {
        Err("No state migration provided".into())
    }
    fn create(&self, rate: f32, state: &State) -> Result<Box<dyn Instance>, String>;
}
pub struct Entry {
    pub descriptor: TypeDescriptor,
    pub provider: Arc<dyn Provider>,
    pub factory: EffectFactory,
}
static CATALOG: OnceLock<Vec<Entry>> = OnceLock::new();
pub fn catalog() -> &'static [Entry] {
    CATALOG.get().map_or(&[], Vec::as_slice)
}
pub fn get(id: &str) -> Option<&'static Entry> {
    catalog().iter().find(|e| e.descriptor.id == id)
}
fn pin(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}
/// Install once before constructing any registries. Metadata is bounded and
/// pinned for the application lifetime; updating/removing packages needs restart.
pub fn install(types: Vec<(TypeDescriptor, Arc<dyn Provider>)>) -> Result<(), String> {
    if CATALOG.get().is_some() {
        return Err("Extension catalog already initialized".into());
    }
    if types.len() > 128 {
        return Err("Too many extension types".into());
    }
    let mut ids = std::collections::HashSet::new();
    for (t, _) in &types {
        t.validate()?;
        if !ids.insert(&t.id) {
            return Err("Duplicate extension type".into());
        }
    }
    let entries = types
        .into_iter()
        .map(|(descriptor, provider)| {
            let parameters: Vec<_> = descriptor
                .parameters
                .iter()
                .map(|p| ParameterDescriptor {
                    id: pin(p.id.clone()),
                    label: pin(p.name.clone()),
                    min: p.min,
                    max: p.max,
                    default: p.default,
                    unit: pin(p.unit.clone()),
                })
                .collect();
            let factory = EffectFactory {
                id: pin(descriptor.id.clone()),
                name: pin(descriptor.name.clone()),
                description: pin(descriptor.description.clone()),
                parameters: Box::leak(parameters.into_boxed_slice()),
                create: |_| Box::new(Unavailable),
            };
            Entry {
                descriptor,
                provider,
                factory,
            }
        })
        .collect();
    CATALOG
        .set(entries)
        .map_err(|_| "Extension catalog already initialized".into())
}
pub fn defaults(d: &TypeDescriptor) -> NodeSettings {
    NodeSettings {
        state: State {
            schema: d.state_schema,
            bytes: d.default_state.clone(),
        },
        parameters: d
            .parameters
            .iter()
            .map(|p| (p.id.clone(), p.default))
            .collect(),
        keybinds: Default::default(),
    }
}
pub fn validate_values(
    d: &TypeDescriptor,
    values: &BTreeMap<String, f32>,
    state: &State,
) -> Result<(), String> {
    state.validate()?;
    if state.schema != d.state_schema {
        return Err("Incompatible extension state schema".into());
    }
    for (id, v) in values {
        if !d.parameters.iter().any(|p| p.id == *id && p.valid(*v)) {
            return Err(format!("Invalid extension parameter {id}"));
        }
    }
    Ok(())
}
pub fn validate_node(n: &Node) -> Result<(), String> {
    if let NodeKind::Extension { definition } = &n.kind {
        definition.validate()?;
        if definition.kind != TypeKind::Node {
            return Err("Not an extension node type".into());
        }
        n.extension.state.validate()?;
        // Validate against the saved schema even when its DLL is absent. A
        // changed installed schema is unavailable, never an implicit migration.
        validate_values(definition, &n.extension.parameters, &n.extension.state)?;
        if n.extension
            .keybinds
            .iter()
            .any(|(id, key)| key.len() > 256 || !definition.actions.iter().any(|a| &a.id == id))
        {
            return Err("Invalid extension action binding".into());
        }
        if !n.effects.is_empty() {
            return Err("Route a processor before or after an extension node".into());
        }
    }
    Ok(())
}
pub struct Unavailable;
impl AudioEffect for Unavailable {
    fn set_parameter(&mut self, _: &str, _: f32) -> Result<(), String> {
        Ok(())
    }
    fn process(&mut self, _: &mut [Frame]) {}
    fn reset(&mut self) {}
    fn faulted(&self) -> bool {
        true
    }
}
pub struct Effect {
    inner: Box<dyn Instance>,
    descriptor: &'static TypeDescriptor,
    failed: bool,
    configuration_failed: bool,
    input: Box<[Frame; BLOCK_SIZE]>,
    output: Box<[Frame; BLOCK_SIZE]>,
}
impl Effect {
    pub fn new(e: &'static Entry, rate: f32, state: &State) -> Result<Self, String> {
        Ok(Self {
            inner: e.provider.create(rate, state)?,
            descriptor: &e.descriptor,
            failed: false,
            configuration_failed: false,
            input: Box::new([[0.; 2]; BLOCK_SIZE]),
            output: Box::new([[0.; 2]; BLOCK_SIZE]),
        })
    }
}
impl AudioEffect for Effect {
    fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), String> {
        let (index, p) = self
            .descriptor
            .parameters
            .iter()
            .enumerate()
            .find(|(_, p)| p.id == id)
            .ok_or("Missing extension parameter")?;
        if !p.valid(value) {
            return Err("Invalid extension parameter".into());
        }
        if !self.inner.set(index as u32, value) {
            self.failed = true;
            self.configuration_failed = true;
        }
        Ok(())
    }
    fn process(&mut self, frames: &mut [Frame]) {
        if self.failed {
            return;
        }
        for block in frames.chunks_mut(BLOCK_SIZE) {
            self.input.fill([0.; 2]);
            self.input[..block.len()].copy_from_slice(block);
            self.output.fill([0.; 2]);
            if !self.inner.process(
                std::slice::from_ref(self.input.as_ref()),
                std::slice::from_mut(self.output.as_mut()),
            ) || self.output.iter().flatten().any(|v| !v.is_finite())
            {
                self.failed = true;
                return;
            }
            block.copy_from_slice(&self.output[..block.len()]);
        }
    }
    fn reset(&mut self) {
        self.failed = self.configuration_failed || !self.inner.reset();
    }
    fn faulted(&self) -> bool {
        self.failed
    }
}
pub struct Runtime {
    pub inputs: Vec<[Frame; BLOCK_SIZE]>,
    pub outputs: Vec<[Frame; BLOCK_SIZE]>,
    pub output_ids: Vec<u32>,
    bypass: Vec<(usize, usize)>,
    inner: Option<Box<dyn Instance>>,
    pub fault: bool,
    configuration_failed: bool,
}
impl Runtime {
    pub fn new(n: &Node) -> Self {
        let NodeKind::Extension { definition: d } = &n.kind else {
            unreachable!()
        };
        let inner = get(&d.id).filter(|e| e.descriptor == **d).and_then(|e| {
            e.provider
                .create(SAMPLE_RATE as f32, &n.extension.state)
                .ok()
        });
        let mut result = Self {
            inputs: vec![[[0.; 2]; BLOCK_SIZE]; d.inputs.len()],
            outputs: vec![[[0.; 2]; BLOCK_SIZE]; d.outputs.len()],
            output_ids: d.outputs.iter().map(|p| p.id).collect(),
            bypass: d
                .bypass
                .iter()
                .map(|b| {
                    (
                        d.inputs.iter().position(|p| p.id == b.input).unwrap(),
                        d.outputs.iter().position(|p| p.id == b.output).unwrap(),
                    )
                })
                .collect(),
            fault: inner.is_none(),
            configuration_failed: false,
            inner,
        };
        if result.configure(n).is_err() {
            result.fault = true;
            result.inner = None;
        }
        result
    }
    pub fn configure(&mut self, n: &Node) -> Result<(), String> {
        let NodeKind::Extension { definition: d } = &n.kind else {
            unreachable!()
        };
        self.configuration_failed = false;
        if let Some(inner) = &mut self.inner {
            for (i, p) in d.parameters.iter().enumerate() {
                if !inner.set(
                    i as u32,
                    *n.extension.parameters.get(&p.id).unwrap_or(&p.default),
                ) {
                    self.configuration_failed = true;
                    self.fault = true;
                    return Err("Extension rejected parameter".into());
                }
            }
        }
        Ok(())
    }
    pub fn reset(&mut self) {
        if let Some(i) = &mut self.inner {
            self.fault = self.configuration_failed || !i.reset();
        }
    }
    pub fn process(&mut self, bypass: bool) {
        for b in &mut self.outputs {
            b.fill([0.; 2]);
        }
        if bypass {
            for &(i, o) in &self.bypass {
                self.outputs[o] = self.inputs[i];
            }
            return;
        }
        if self.fault {
            return;
        }
        if !self
            .inner
            .as_mut()
            .is_some_and(|i| i.process(&self.inputs, &mut self.outputs))
            || self
                .outputs
                .iter()
                .flatten()
                .flatten()
                .any(|v| !v.is_finite())
        {
            self.fault = true;
            for b in &mut self.outputs {
                b.fill([0.; 2]);
            }
        }
    }
}

/// Stable FNV-1a identifier for action dispatch; persisted bindings retain full
/// strings. Descriptor validation rejects collisions within each action set.
pub fn action_key(id: &str) -> u64 {
    id.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}
pub fn descriptor(n: &Node, effect: Option<u64>) -> Option<&TypeDescriptor> {
    match effect {
        None => {
            if let NodeKind::Extension { definition } = &n.kind {
                Some(definition)
            } else {
                None
            }
        }
        Some(id) => n
            .effects
            .iter()
            .find(|e| e.id == id)?
            .extension_descriptor
            .as_ref(),
    }
}
pub fn action_descriptor(
    p: &crate::project::Project,
    id: u64,
    effect: Option<u64>,
    key: u64,
) -> Option<&Action> {
    let n = p.nodes.iter().find(|n| n.id == id)?;
    let d = descriptor(n, effect)?;
    let active = get(&d.id)?;
    if active.descriptor != *d {
        return None;
    }
    d.actions.iter().find(|a| action_key(&a.id) == key)
}
pub fn apply_action(
    p: &mut crate::project::Project,
    id: u64,
    effect: Option<u64>,
    key: u64,
) -> Result<(), String> {
    let a = action_descriptor(p, id, effect, key)
        .ok_or("Extension action unavailable")?
        .clone();
    let n = p
        .nodes
        .iter_mut()
        .find(|n| n.id == id)
        .ok_or("Missing node")?;
    let d = descriptor(n, effect).ok_or("Missing extension descriptor")?;
    let param = d
        .parameters
        .iter()
        .find(|p| p.id == a.parameter)
        .ok_or("Missing action parameter")?
        .clone();
    let values = if let Some(id) = effect {
        &mut n
            .effects
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or("Missing effect")?
            .parameters
    } else {
        &mut n.extension.parameters
    };
    let old = *values.get(&a.parameter).unwrap_or(&param.default);
    values.insert(
        a.parameter,
        if a.toggle && old == a.value {
            param.default
        } else {
            a.value
        },
    );
    Ok(())
}
pub fn action_rows(n: &Node) -> Vec<(crate::shortcuts::ShortcutAction, String)> {
    let mut rows = vec![];
    for (effect, d) in std::iter::once((None, descriptor(n, None))).chain(
        n.effects
            .iter()
            .map(|e| (Some(e.id), e.extension_descriptor.as_ref())),
    ) {
        if let Some(d) = d {
            for a in &d.actions {
                rows.push((
                    crate::shortcuts::ShortcutAction::Extension {
                        id: n.id,
                        effect,
                        action: action_key(&a.id),
                    },
                    format!("{}: {}", d.name, a.name),
                ));
            }
        }
    }
    rows
}
pub fn action_binding(n: &Node, effect: Option<u64>, key: u64) -> Option<&str> {
    let d = descriptor(n, effect)?;
    let id = &d.actions.iter().find(|a| action_key(&a.id) == key)?.id;
    if let Some(target) = effect {
        n.effects
            .iter()
            .find(|e| e.id == target)?
            .extension_keybinds
            .get(id)
            .map(String::as_str)
    } else {
        n.extension.keybinds.get(id).map(String::as_str)
    }
}

pub fn set_binding(
    n: &mut Node,
    effect: Option<u64>,
    key: u64,
    text: String,
) -> Result<(), String> {
    let id = descriptor(n, effect)
        .and_then(|d| d.actions.iter().find(|a| action_key(&a.id) == key))
        .ok_or("Missing extension action")?
        .id
        .clone();
    let bindings = if let Some(target) = effect {
        &mut n
            .effects
            .iter_mut()
            .find(|e| e.id == target)
            .ok_or("Missing effect")?
            .extension_keybinds
    } else {
        &mut n.extension.keybinds
    };
    if text.trim().is_empty() {
        bindings.remove(&id);
    } else {
        bindings.insert(id, text.trim().into());
    }
    Ok(())
}

pub fn effect_spec(d: &TypeDescriptor) -> crate::effects::EffectSpec {
    crate::effects::EffectSpec {
        id: crate::effects::new_effect_id(),
        keybinds: Default::default(),
        kind: d.id.clone(),
        enabled: true,
        parameters: d
            .parameters
            .iter()
            .map(|p| (p.id.clone(), p.default))
            .collect(),
        extension: Some(State {
            schema: d.state_schema,
            bytes: d.default_state.clone(),
        }),
        extension_descriptor: Some(d.clone()),
        extension_keybinds: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        effects::{EffectRegistry, EffectSpec},
        graph::Graph,
        project::{InputPort, OutputPort, Project},
    };
    fn definition() -> TypeDescriptor {
        serde_json::from_str::<Manifest>(include_str!(
            "../../piratewave-example-extension/manifest.json"
        ))
        .unwrap()
        .types
        .remove(1)
    }
    fn node() -> Node {
        Node::new(
            2,
            "Extension",
            NodeKind::Extension {
                definition: Box::new(definition()),
            },
            [-100., 200.],
        )
    }
    #[test]
    fn snapshots_ports_and_missing_plugins_roundtrip_without_loading_code() {
        let n = node();
        let encoded = serde_json::to_vec(&n).unwrap();
        let restored: Node = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(n, restored);
        let mut p = Project {
            nodes: vec![
                Node::new(1, "Source", NodeKind::Soundboard, [0.; 2]),
                n,
                Node::new(3, "Out", NodeKind::Output { device: None }, [0.; 2]),
            ],
            connections: vec![],
            ..Project::default()
        };
        p.connect_ports(1, 2, OutputPort::Passthrough, InputPort::Extension(101))
            .unwrap();
        p.connect_ports(1, 2, OutputPort::Passthrough, InputPort::Extension(202))
            .unwrap();
        p.connect_ports(2, 3, OutputPort::Extension(303), InputPort::Main)
            .unwrap();
        assert!(p
            .connect_ports(2, 3, OutputPort::Extension(999), InputPort::Main)
            .is_err());
        assert!(p
            .connect_ports(1, 2, OutputPort::Passthrough, InputPort::Extension(101))
            .is_err());
        let mut g = Graph::new(&p, &EffectRegistry::default()).unwrap();
        g.process(&[(1, [[0.25; 2]; BLOCK_SIZE])]);
        assert_eq!(g.nodes[2].frames, [[0.; 2]; BLOCK_SIZE]);
        p.nodes[1].effects_disabled = true;
        g.update(&p, &EffectRegistry::default()).unwrap();
        g.process(&[(1, [[0.25; 2]; BLOCK_SIZE])]);
        assert_eq!(g.nodes[2].frames, [[0.25; 2]; BLOCK_SIZE]);
        p.connections
            .retain(|c| c.input != InputPort::Extension(202));
        assert_eq!(p.connections.len(), 2);
        assert_eq!(
            crate::recording::inputs(&p, 3)[0].port,
            OutputPort::Extension(303)
        );
    }
    struct Broken {
        calls: usize,
    }
    impl Instance for Broken {
        fn set(&mut self, _: u32, _: f32) -> bool {
            true
        }
        fn reset(&mut self) -> bool {
            self.calls = 0;
            true
        }
        fn process(&mut self, _: &[[Frame; BLOCK_SIZE]], o: &mut [[Frame; BLOCK_SIZE]]) -> bool {
            self.calls += 1;
            o[0][0] = [f32::NAN; 2];
            true
        }
    }
    #[test]
    fn failed_nodes_silence_all_outputs_and_bypass_uses_only_declared_routes() {
        let n = node();
        let mut rt = Runtime::new(&n);
        rt.inner = Some(Box::new(Broken { calls: 0 }));
        rt.fault = false;
        rt.inputs[0].fill([0.25; 2]);
        rt.inputs[1].fill([-0.125; 2]);
        rt.process(false);
        assert!(rt.fault);
        assert!(rt.outputs.iter().all(|b| *b == [[0.; 2]; BLOCK_SIZE]));
        rt.process(true);
        assert_eq!(rt.outputs[0], rt.inputs[0]);
        assert_eq!(rt.outputs[1], rt.inputs[0]);
        assert_eq!(rt.outputs[2], rt.inputs[1]);
        rt.reset();
        assert!(!rt.fault);
        rt.process(false);
        assert!(rt.fault);
    }
    #[test]
    fn settings_validation_preserves_unknown_package_but_rejects_corrupt_data() {
        let mut n = node();
        assert!(validate_node(&n).is_ok());
        n.extension.parameters.insert("level".into(), f32::NAN);
        assert!(validate_node(&n).is_err());
        n.extension.parameters.clear();
        n.extension.keybinds.insert("deleted".into(), "F1".into());
        assert!(validate_node(&n).is_err());
        assert_ne!(action_key("one"), action_key("two"));
    }
    #[test]
    fn unavailable_processor_is_visible_persistent_and_dry() {
        let d = serde_json::from_str::<Manifest>(include_str!(
            "../../piratewave-example-extension/manifest.json"
        ))
        .unwrap()
        .types
        .remove(0);
        let spec = EffectSpec {
            id: 1,
            keybinds: Default::default(),
            kind: d.id.clone(),
            enabled: true,
            parameters: d
                .parameters
                .iter()
                .map(|p| (p.id.clone(), p.default))
                .collect(),
            extension: Some(State {
                schema: d.state_schema,
                bytes: vec![],
            }),
            extension_descriptor: Some(d),
            extension_keybinds: Default::default(),
        };
        let restored: EffectSpec =
            serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
        assert_eq!(spec, restored);
        let mut e = EffectRegistry::default().create(&spec, 48000.).unwrap();
        assert!(e.faulted());
        let mut f = [[0.3, -0.2]; BLOCK_SIZE];
        e.process(&mut f);
        assert_eq!(f, [[0.3, -0.2]; BLOCK_SIZE]);
    }
    #[test]
    fn changed_port_schema_recompiles_without_resolving_new_ids_in_old_graph() {
        let mut p = Project {
            nodes: vec![Node::new(1, "A", NodeKind::Soundboard, [0.; 2]), node()],
            connections: vec![],
            ..Project::default()
        };
        p.connect_ports(1, 2, OutputPort::Passthrough, InputPort::Extension(101))
            .unwrap();
        let r = EffectRegistry::default();
        let mut graph = Graph::new(&p, &r).unwrap();
        if let NodeKind::Extension { definition } = &mut p.nodes[1].kind {
            definition.inputs[0].id = 111;
            for map in &mut definition.bypass {
                if map.input == 101 {
                    map.input = 111;
                }
            }
        }
        p.connections[0].input = InputPort::Extension(111);
        graph.update(&p, &r).unwrap();
        assert_eq!(graph.nodes[1].config.kind, p.nodes[1].kind);
    }
    #[test]
    fn disabled_or_bypassed_missing_effect_allows_explicit_dry_export() {
        let d = serde_json::from_str::<Manifest>(include_str!(
            "../../piratewave-example-extension/manifest.json"
        ))
        .unwrap()
        .types
        .remove(0);
        let mut spec = effect_spec(&d);
        spec.enabled = false;
        let r = EffectRegistry::default();
        let mut fx =
            crate::recording_effects::RecordingEffects::new(std::slice::from_ref(&spec), false, &r)
                .unwrap();
        assert!(!fx.faulted());
        let mut frames = [[0.2; 2]; BLOCK_SIZE];
        fx.process(&mut frames);
        assert_eq!(frames, [[0.2; 2]; BLOCK_SIZE]);
        spec.enabled = true;
        let fx = crate::recording_effects::RecordingEffects::new(&[spec], true, &r).unwrap();
        assert!(!fx.faulted());
    }
    struct Reject;
    impl Instance for Reject {
        fn set(&mut self, _: u32, _: f32) -> bool {
            false
        }
        fn reset(&mut self) -> bool {
            true
        }
        fn process(&mut self, _: &[[Frame; BLOCK_SIZE]], _: &mut [[Frame; BLOCK_SIZE]]) -> bool {
            panic!("Rejected configuration must not render")
        }
    }
    #[test]
    fn rejected_configuration_latches_local_fault_without_stopping_graph() {
        let n = node();
        let mut rt = Runtime::new(&n);
        rt.inner = Some(Box::new(Reject));
        rt.fault = false;
        assert!(rt.configure(&n).is_err());
        rt.reset();
        assert!(rt.fault);
        rt.process(false);
        assert!(rt.outputs.iter().all(|o| *o == [[0.; 2]; BLOCK_SIZE]));
        let d = serde_json::from_str::<Manifest>(include_str!(
            "../../piratewave-example-extension/manifest.json"
        ))
        .unwrap()
        .types
        .remove(0);
        let mut fx = Effect {
            inner: Box::new(Reject),
            descriptor: Box::leak(Box::new(d)),
            failed: false,
            configuration_failed: false,
            input: Box::new([[0.; 2]; BLOCK_SIZE]),
            output: Box::new([[0.; 2]; BLOCK_SIZE]),
        };
        fx.set_parameter("width", 1.).unwrap();
        fx.reset();
        assert!(fx.faulted());
        let mut frames = [[0.25; 2]; BLOCK_SIZE];
        fx.process(&mut frames);
        assert_eq!(frames, [[0.25; 2]; BLOCK_SIZE]);
    }
}
