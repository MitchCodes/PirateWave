//! Persistent workflow definitions. Runtime state and media stay outside workspaces.
use crate::{
    effects::EffectRegistry,
    project::{NodeKind, OutputPort, Project},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Create a stable workspace-local identity for a workflow object.
pub fn new_id() -> u64 {
    crate::effects::new_effect_id()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Workflows {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub obs_scene_links: Vec<ObsSceneLink>,
    pub scenes: Vec<Scene>,
    pub sessions: Vec<SessionConfig>,
    pub rules: Vec<Rule>,
    pub automation_keybinds: BTreeMap<AutomationAction, String>,
}
impl Workflows {
    pub fn is_empty(&self) -> bool {
        self.obs_scene_links.is_empty()
            && self.scenes.is_empty()
            && self.sessions.is_empty()
            && self.rules.is_empty()
            && self.automation_keybinds.values().all(|s| s.is_empty())
    }
}
/// Exact OBS program-scene name mapped to a stable audio-scene ID.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObsSceneLink {
    pub obs_scene: String,
    pub audio_scene: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Hash, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationAction {
    Arm,
    Disarm,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scene {
    pub id: u64,
    pub name: String,
    pub duration_ms: u32,
    pub settings: Vec<SceneSetting>,
    #[serde(default)]
    pub effect_kinds: Vec<(u64, u64, String)>,
    #[serde(default)]
    pub keybind: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneSetting {
    pub node: u64,
    pub included: bool,
    pub value: Control,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    DuckingParameter {
        parameter: String,
        value: f32,
    },
    Gain {
        value: f32,
    },
    Pan {
        value: f32,
    },
    Mute {
        value: bool,
    },
    Solo {
        value: bool,
    },
    Listening {
        value: bool,
    },
    Bypass {
        value: bool,
    },
    Switch {
        value: Option<u64>,
    },
    EffectEnabled {
        effect: u64,
        value: bool,
    },
    Parameter {
        effect: u64,
        parameter: String,
        value: f32,
    },
    Ducking {
        value: crate::ducking::DuckingConfig,
    },
}
impl Control {
    pub fn label(&self) -> String {
        match self {
            Self::DuckingParameter { parameter, .. } => crate::ducking::PARAMETERS
                .iter()
                .find(|p| p.id == parameter)
                .map_or_else(|| parameter.clone(), |p| p.label.into()),
            Self::Gain { .. } => "Volume".into(),
            Self::Pan { .. } => "Pan".into(),
            Self::Mute { .. } => "Mute".into(),
            Self::Solo { .. } => "Solo".into(),
            Self::Listening { .. } => "Listening".into(),
            Self::Bypass { .. } => "Rack bypass".into(),
            Self::Switch { .. } => "Switch destination".into(),
            Self::EffectEnabled { effect, .. } => format!("Effect {effect} enabled"),
            Self::Parameter { parameter, .. } => parameter.clone(),
            Self::Ducking { .. } => "Auto Ducking settings".into(),
        }
    }
    pub fn same_target(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::DuckingParameter { parameter: a, .. },
                Self::DuckingParameter { parameter: b, .. },
            ) => a == b,
            (
                Self::Parameter {
                    effect: a,
                    parameter: p,
                    ..
                },
                Self::Parameter {
                    effect: b,
                    parameter: q,
                    ..
                },
            ) => a == b && p == q,
            (Self::EffectEnabled { effect: a, .. }, Self::EffectEnabled { effect: b, .. }) => {
                a == b
            }
            _ => std::mem::discriminant(self) == std::mem::discriminant(other),
        }
    }
}
impl Scene {
    /// Merge a control-plane acknowledgment without overwriting newer manual edits.
    pub fn merge_controls(
        base: &Project,
        current: &Project,
        recalled: &Project,
        registry: &EffectRegistry,
    ) -> Result<Project, String> {
        let mut changes = Self::capture(recalled, registry, String::new());
        for setting in &mut changes.settings {
            setting.included = base
                .nodes
                .iter()
                .find(|n| n.id == setting.node)
                .zip(current.nodes.iter().find(|n| n.id == setting.node))
                .is_some_and(|(a, b)| !crate::scene_runtime::control_changed(&setting.value, a, b));
        }
        changes.apply(current, registry)
    }

    pub fn capture(project: &Project, registry: &EffectRegistry, name: String) -> Self {
        let mut settings = Vec::new();
        for n in &project.nodes {
            let mut values = Vec::new();
            if n.kind.is_source() || matches!(n.kind, NodeKind::Output { .. } | NodeKind::Switch) {
                values.extend([
                    Control::Gain { value: n.gain_db },
                    Control::Pan { value: n.pan },
                    Control::Mute { value: n.mute },
                ]);
            }
            if n.kind.is_source() {
                values.push(Control::Solo { value: n.solo });
            }
            if matches!(n.kind, NodeKind::Output { .. }) {
                values.push(Control::Listening {
                    value: n.listen_enabled,
                });
            }
            if n.kind.has_effect_controls() || matches!(n.kind, NodeKind::AutoDucking) {
                values.push(Control::Bypass {
                    value: n.effects_disabled,
                });
            }
            if matches!(n.kind, NodeKind::Switch) {
                values.push(Control::Switch {
                    value: n.switch_output,
                });
            }
            if matches!(n.kind, NodeKind::AutoDucking) {
                values.extend(crate::ducking::PARAMETERS.iter().map(|p| {
                    Control::DuckingParameter {
                        parameter: p.id.into(),
                        value: n.ducking.value(p.id),
                    }
                }));
            }
            for e in &n.effects {
                if e.extension.is_some() {
                    continue;
                }
                if let Some(factory) = registry.get(&e.kind) {
                    values.push(Control::EffectEnabled {
                        effect: e.id,
                        value: e.enabled,
                    });
                    for p in factory.parameters {
                        values.push(Control::Parameter {
                            effect: e.id,
                            parameter: p.id.into(),
                            value: e.parameters.get(p.id).copied().unwrap_or(p.default),
                        });
                    }
                }
            }
            settings.extend(values.into_iter().map(|value| SceneSetting {
                node: n.id,
                included: true,
                value,
            }));
        }
        Self {
            id: crate::effects::new_effect_id(),
            effect_kinds: project
                .nodes
                .iter()
                .flat_map(|n| {
                    n.effects
                        .iter()
                        .filter(|e| e.extension.is_none())
                        .map(|e| (n.id, e.id, e.kind.clone()))
                })
                .collect(),
            name,
            duration_ms: 500,
            settings,
            keybind: String::new(),
        }
    }
    pub fn apply(&self, project: &Project, registry: &EffectRegistry) -> Result<Project, String> {
        if self.duration_ms > 10000 {
            return Err("Scene duration must be at most 10 seconds".into());
        }
        let mut next = project.clone();
        for s in self.settings.iter().filter(|s| s.included) {
            let n = next
                .nodes
                .iter_mut()
                .find(|n| n.id == s.node)
                .ok_or_else(|| format!("Scene target {} is missing", s.node))?;
            if let Control::Parameter { effect, .. } | Control::EffectEnabled { effect, .. } =
                &s.value
            {
                if let Some((_, _, kind)) = self
                    .effect_kinds
                    .iter()
                    .find(|(node, id, _)| *node == s.node && id == effect)
                {
                    if !n
                        .effects
                        .iter()
                        .any(|e| e.id == *effect && e.kind == *kind && e.extension.is_none())
                    {
                        return Err(format!("Scene effect changed on {}", n.name));
                    }
                }
            }
            match &s.value {
                Control::DuckingParameter { parameter, value }
                    if matches!(n.kind, NodeKind::AutoDucking) =>
                {
                    n.ducking.set(parameter, *value)?
                }
                Control::Gain { value } => n.gain_db = *value,
                Control::Pan { value } => n.pan = *value,
                Control::Mute { value } => n.mute = *value,
                Control::Solo { value } if n.kind.is_source() => n.solo = *value,
                Control::Listening { value } if matches!(n.kind, NodeKind::Output { .. }) => {
                    n.listen_enabled = *value
                }
                Control::Bypass { value }
                    if n.kind.has_effect_controls() || matches!(n.kind, NodeKind::AutoDucking) =>
                {
                    n.effects_disabled = *value
                }
                Control::Switch { value } if matches!(n.kind, NodeKind::Switch) => {
                    n.switch_output = *value
                }
                Control::Ducking { value } if matches!(n.kind, NodeKind::AutoDucking) => {
                    n.ducking = value.clone()
                }
                Control::EffectEnabled { effect, value } => {
                    let e = n
                        .effects
                        .iter_mut()
                        .find(|e| e.id == *effect && e.extension.is_none())
                        .ok_or("Scene effect is missing or incompatible")?;
                    e.enabled = *value;
                }
                Control::Parameter {
                    effect,
                    parameter,
                    value,
                } => {
                    let e = n
                        .effects
                        .iter_mut()
                        .find(|e| e.id == *effect && e.extension.is_none())
                        .ok_or("Scene effect is missing or incompatible")?;
                    let factory = registry
                        .get(&e.kind)
                        .ok_or("Scene processor is unavailable")?;
                    let p = factory
                        .parameters
                        .iter()
                        .find(|p| p.id == parameter)
                        .ok_or("Scene parameter is unavailable")?;
                    if !value.is_finite() || !(p.min..=p.max).contains(value) {
                        return Err("Scene parameter is out of range".into());
                    }
                    e.parameters.insert(parameter.clone(), *value);
                }
                _ => {
                    return Err(format!(
                        "{} is incompatible with {}",
                        s.value.label(),
                        n.name
                    ))
                }
            }
        }
        crate::graph::validate_with_registry(&next, registry)?;
        Ok(next)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureStage {
    Raw,
    Processed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CapturePoint {
    pub node: u64,
    pub port: OutputPort,
    pub stage: CaptureStage,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Track {
    pub id: u64,
    pub name: String,
    pub point: CapturePoint,
    pub armed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionConfig {
    pub id: u64,
    pub name: String,
    pub history_seconds: f32,
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub take_ids: Vec<String>,
    #[serde(default)]
    pub keybinds: BTreeMap<SessionAction, String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionAction {
    Start,
    Stop,
    Capture,
    Marker,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rule {
    pub id: u64,
    pub name: String,
    pub enabled: bool,
    pub trigger: Trigger,
    pub match_all: bool,
    pub conditions: Vec<Condition>,
    pub actions: Vec<Step>,
    pub cooldown_ms: u32,
    #[serde(default)]
    pub keybind: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Trigger {
    Manual,
    Timer {
        interval_ms: u64,
        repeating: bool,
    },
    Level {
        node: u64,
        above: bool,
        threshold_db: f32,
        hysteresis_db: f32,
        hold_ms: u32,
    },
    Silence {
        node: u64,
        threshold_db: f32,
        hold_ms: u32,
    },
    Audio {
        running: bool,
    },
    Scene {
        id: u64,
    },
    Device {
        node: u64,
        available: bool,
    },
    Completion {
        operation: Operation,
        node: Option<u64>,
        success: bool,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Sound,
    SlotPlayback,
    SlotCapture,
    Session,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Condition {
    Activity {
        node: u64,
        active: bool,
    },
    NodeEnabled {
        node: u64,
        enabled: bool,
    },
    Recorder {
        node: u64,
        active: bool,
    },
    Audio {
        running: bool,
    },
    Scene {
        id: u64,
    },
    NodeMute {
        node: u64,
        value: bool,
    },
    Effect {
        node: u64,
        effect: u64,
        enabled: bool,
    },
    Level {
        node: u64,
        above: bool,
        db: f32,
    },
    Device {
        node: u64,
        available: bool,
    },
    Recording {
        session: u64,
        active: bool,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Step {
    Action {
        action: crate::shortcuts::ShortcutAction,
    },
    Set {
        node: u64,
        control: Control,
    },
    Scene {
        id: u64,
    },
    Session {
        id: u64,
        action: SessionAction,
    },
    Wait {
        milliseconds: u32,
    },
    Notify {
        text: String,
    },
}
pub fn validate(project: &Project) -> Result<(), String> {
    let mut obs_names = std::collections::HashSet::new();
    if project.workflows.obs_scene_links.len() > 128
        || project.workflows.obs_scene_links.iter().any(|link| {
            link.obs_scene.trim().is_empty()
                || link.obs_scene.len() > 256
                || link.audio_scene == 0
                || !obs_names.insert(&link.obs_scene)
        })
    {
        return Err(
            "OBS scene links need unique OBS names and valid audio-scene IDs (up to 128 links)"
                .into(),
        );
    }

    use std::collections::HashSet;
    fn named<'a>(items: impl Iterator<Item = (u64, &'a str)>, limit: usize) -> Result<(), String> {
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for (id, name) in items {
            if id == 0
                || name.trim().is_empty()
                || name.len() > 256
                || !ids.insert(id)
                || !names.insert(name.trim().to_lowercase())
                || ids.len() > limit
            {
                return Err("Workflow names and IDs must be unique and within limits".into());
            }
        }
        Ok(())
    }
    named(
        project
            .workflows
            .scenes
            .iter()
            .map(|s| (s.id, s.name.as_str())),
        128,
    )?;
    named(
        project
            .workflows
            .sessions
            .iter()
            .map(|s| (s.id, s.name.as_str())),
        32,
    )?;
    named(
        project
            .workflows
            .rules
            .iter()
            .map(|s| (s.id, s.name.as_str())),
        128,
    )?;
    for s in &project.workflows.scenes {
        if s.duration_ms > 10000 || s.settings.len() > 8192 || s.keybind.len() > 256 {
            return Err("Invalid scene size, duration or shortcut".into());
        }
        for (i, setting) in s.settings.iter().enumerate() {
            if s.settings[..i]
                .iter()
                .any(|old| old.node == setting.node && old.value.same_target(&setting.value))
            {
                return Err("Duplicate scene control".into());
            }
            match &setting.value {
                Control::Gain { value }
                    if !value.is_finite() || !(-60.0..=24.0).contains(value) =>
                {
                    return Err("Invalid scene gain".into())
                }
                Control::Pan { value } if !value.is_finite() || !(-1.0..=1.0).contains(value) => {
                    return Err("Invalid scene pan".into())
                }
                Control::DuckingParameter { parameter, value }
                | Control::Parameter {
                    parameter, value, ..
                } if parameter.len() > 128 || !value.is_finite() => {
                    return Err("Invalid scene parameter".into())
                }
                Control::Ducking { value } => value.validate()?,
                _ => {}
            }
        }
    }
    for s in &project.workflows.sessions {
        named(s.tracks.iter().map(|t| (t.id, t.name.as_str())), 32)?;
        if !s.history_seconds.is_finite()
            || !(0.1..=600.).contains(&s.history_seconds)
            || s.keybinds.values().any(|v| v.len() > 256)
        {
            return Err("Invalid session history or shortcut".into());
        }
        if s.take_ids.len() > 10000
            || s.take_ids.iter().any(|id| {
                !id.starts_with("take-")
                    || id.len() > 64
                    || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            return Err("Invalid session media references".into());
        }
    }
    for r in &project.workflows.rules {
        if r.actions.len() > 32
            || r.conditions.len() > 32
            || r.keybind.len() > 256
            || r.cooldown_ms > 3600000
        {
            return Err("Automation rule exceeds limits".into());
        }
        match r.trigger {
            Trigger::Timer { interval_ms, .. } if !(50..=86400000).contains(&interval_ms) => {
                return Err("Timer must be between 50 ms and one day".into())
            }
            Trigger::Level {
                threshold_db,
                hysteresis_db,
                hold_ms,
                ..
            } if !threshold_db.is_finite()
                || !(-100.0..=0.).contains(&threshold_db)
                || !hysteresis_db.is_finite()
                || !(0.0..=40.).contains(&hysteresis_db)
                || hold_ms > 600000 =>
            {
                return Err("Invalid automation level settings".into())
            }
            Trigger::Silence {
                threshold_db,
                hold_ms,
                ..
            } if !threshold_db.is_finite()
                || !(-100.0..=0.).contains(&threshold_db)
                || hold_ms > 600000 =>
            {
                return Err("Invalid silence settings".into())
            }
            _ => {}
        }
        if r.actions.iter().any(|s| {
            matches!(
                s,
                Step::Action {
                    action: crate::shortcuts::ShortcutAction::RecordingSlot {
                        action: crate::recording::SlotAction::Delete,
                        ..
                    }
                }
            )
        }) {
            return Err("Automation cannot delete recordings".into());
        }
        if r.actions.iter().any(|s| {
            matches!(s,Step::Wait{milliseconds} if *milliseconds>86400000)
                || matches!(s,Step::Notify{text} if text.len()>1024)
        }) {
            return Err("Automation delay or message exceeds limits".into());
        }
    }
    Ok(())
}
impl CapturePoint {
    pub fn validate(self, project: &Project) -> Result<(), String> {
        let n = project
            .nodes
            .iter()
            .find(|n| n.id == self.node)
            .ok_or("Recording track node is missing")?;
        if self.stage == CaptureStage::Raw {
            if !n.kind.is_source() || self.port != OutputPort::Passthrough {
                return Err("Raw capture requires an audio source".into());
            }
        } else if !(n.kind.outputs().contains(&self.port)
            || matches!(n.kind, NodeKind::Output { .. }) && self.port == OutputPort::Passthrough)
        {
            return Err("Recording output port is missing".into());
        }
        Ok(())
    }
}

impl Rule {
    pub fn validate_targets(
        &self,
        project: &Project,
        registry: &EffectRegistry,
    ) -> Result<(), String> {
        fn node(project: &Project, id: u64) -> Result<(), String> {
            if project.nodes.iter().any(|n| n.id == id) {
                Ok(())
            } else {
                Err(format!("Node {id} is missing"))
            }
        }
        fn scene(project: &Project, id: u64) -> Result<(), String> {
            if project.workflows.scenes.iter().any(|s| s.id == id) {
                Ok(())
            } else {
                Err(format!("Scene {id} is missing"))
            }
        }
        fn session(project: &Project, id: u64) -> Result<(), String> {
            if project.workflows.sessions.iter().any(|s| s.id == id) {
                Ok(())
            } else {
                Err(format!("Session {id} is missing"))
            }
        }
        match self.trigger {
            Trigger::Level { node: id, .. }
            | Trigger::Silence { node: id, .. }
            | Trigger::Device { node: id, .. } => node(project, id)?,
            Trigger::Scene { id } => scene(project, id)?,
            Trigger::Completion {
                operation,
                node: Some(id),
                ..
            } => match operation {
                Operation::Session => session(project, id)?,
                Operation::Sound => {
                    if !project.sounds.iter().any(|s| s.id == id) {
                        return Err(format!("Sound {id} is missing"));
                    }
                }
                _ => node(project, id)?,
            },
            _ => {}
        }
        for condition in &self.conditions {
            match condition {
                Condition::Scene { id } => scene(project, *id)?,
                Condition::Recording { session: id, .. } => session(project, *id)?,
                Condition::Activity { node: id, .. }
                | Condition::NodeEnabled { node: id, .. }
                | Condition::Recorder { node: id, .. }
                | Condition::NodeMute { node: id, .. }
                | Condition::Level { node: id, .. }
                | Condition::Device { node: id, .. } => node(project, *id)?,
                Condition::Effect {
                    node: id, effect, ..
                } => {
                    node(project, *id)?;
                    if !project
                        .nodes
                        .iter()
                        .find(|n| n.id == *id)
                        .unwrap()
                        .effects
                        .iter()
                        .any(|e| e.id == *effect)
                    {
                        return Err(format!("Effect {effect} is missing"));
                    }
                }
                _ => {}
            }
        }
        for step in &self.actions {
            match step {
                Step::Action { action } => {
                    if !action.exists(project) {
                        return Err("Action target is missing".into());
                    }
                    if matches!(
                        action,
                        crate::shortcuts::ShortcutAction::RecordingSlot {
                            action: crate::recording::SlotAction::Delete,
                            ..
                        }
                    ) {
                        return Err("Automation cannot delete recordings".into());
                    }
                }
                Step::Scene { id } => scene(project, *id)?,
                Step::Session { id, .. } => session(project, *id)?,
                Step::Set { node: id, control } => {
                    let test = Scene {
                        id: 0,
                        name: String::new(),
                        duration_ms: 0,
                        effect_kinds: vec![],
                        settings: vec![SceneSetting {
                            node: *id,
                            included: true,
                            value: control.clone(),
                        }],
                        keybind: String::new(),
                    };
                    test.apply(project, registry)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scene_recalls_only_eligible_controls_and_rejects_removed_effects() {
        let mut p = Project::default();
        let r = EffectRegistry::default();
        p.nodes[0].gain_db = -9.;
        p.nodes[0]
            .effects
            .push(crate::effects::EffectSpec::new(r.get("high_pass").unwrap()));
        let s = Scene::capture(&p, &r, "Interview".into());
        let device = if let NodeKind::Input { device, .. } = &mut p.nodes[0].kind {
            *device = Some("different".into());
            device.clone()
        } else {
            unreachable!()
        };
        p.nodes[0].gain_db = 3.;
        let restored = s.apply(&p, &r).unwrap();
        assert_eq!(restored.nodes[0].gain_db, -9.);
        assert!(matches!(&restored.nodes[0].kind,NodeKind::Input{device:d,..} if *d==device));
        p.nodes[0].effects.clear();
        assert!(s.apply(&p, &r).is_err());
        assert_eq!(p.nodes[0].gain_db, 3.);
    }
    #[test]
    fn legacy_workspaces_have_no_workflows() {
        let p = Project::default();
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("workflows"));
        let restored: Project = serde_json::from_str(&json).unwrap();
        assert!(restored.workflows.is_empty());
    }
    #[test]
    fn duplicate_names_and_raw_bus_tracks_are_rejected() {
        let mut p = Project::default();
        let scene = Scene::capture(&p, &EffectRegistry::default(), "Show".into());
        p.workflows.scenes = vec![
            scene.clone(),
            Scene {
                id: scene.id + 1,
                name: "show".into(),
                ..scene
            },
        ];
        assert!(validate(&p).is_err());
        let bus = p
            .nodes
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Output { .. }))
            .unwrap();
        assert!(CapturePoint {
            node: bus.id,
            port: OutputPort::Passthrough,
            stage: CaptureStage::Raw
        }
        .validate(&p)
        .is_err());
    }
}
