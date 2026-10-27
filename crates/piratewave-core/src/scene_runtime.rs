//! Prepared scene transitions. Allocation and target resolution happen in prepare;
//! block advancement only mutates existing parameters and preallocated DSP state.
use crate::{
    effects::EffectRegistry,
    graph::RuntimeNode,
    project::{Node, Project},
    workflows::{Control, Scene},
    BLOCK_SIZE, SAMPLE_RATE,
};

pub(crate) struct SwitchFade {
    pub(crate) active: bool,
    routes: Vec<(u64, f32, f32)>,
    previous: f32,
    current: f32,
}
impl SwitchFade {
    pub(crate) fn weights(&self, id: u64) -> (f32, f32) {
        self.routes
            .iter()
            .find(|(dest, _, _)| *dest == id)
            .map_or((0., 0.), |(_, from, to)| {
                (
                    from + (to - from) * self.previous,
                    from + (to - from) * self.current,
                )
            })
    }
}
pub struct Transition {
    pub id: u64,
    pub elapsed: u64,
    pub total: u64,
    pub modified: bool,
    pub finished: bool,
    pub target: Project,
    steps: Vec<Step>,
    masks: Vec<Mask>,
}
struct Step {
    node: usize,
    value: Control,
    from: f32,
    effect: Option<usize>,
    smooth: bool,
    applied: bool,
    owned: bool,
}
struct Mask {
    node: usize,
    from: f32,
    to: f32,
    owned: bool,
}
fn scalar(c: &Control, n: &Node) -> f32 {
    match c {
        Control::Gain { .. } => n.gain_db,
        Control::Pan { .. } => n.pan,
        Control::Parameter {
            effect, parameter, ..
        } => n
            .effects
            .iter()
            .find(|e| e.id == *effect)
            .and_then(|e| e.parameters.get(parameter))
            .copied()
            .unwrap_or(0.),
        _ => 0.,
    }
}
fn smooth(c: &Control, n: &Node) -> bool {
    match c {
        Control::Gain { .. } | Control::Pan { .. } => true,
        Control::Parameter {
            effect, parameter, ..
        } => n.effects.iter().find(|e| e.id == *effect).is_some_and(|e| {
            crate::effects::scene_transition(&e.kind, parameter)
                == crate::effects::SceneTransition::Interpolate
        }),
        _ => false,
    }
}
fn guarded(control: &Control, smooth: bool) -> bool {
    matches!(
        control,
        Control::Bypass { .. }
            | Control::EffectEnabled { .. }
            | Control::Ducking { .. }
            | Control::DuckingParameter { .. }
    ) || matches!(control, Control::Parameter { .. }) && !smooth
}
fn mask(n: &Node, solo: bool) -> f32 {
    if n.mute || solo && n.kind.is_source() && !n.solo {
        0.
    } else {
        1.
    }
}
impl Transition {
    pub fn prepare(
        scene: &Scene,
        project: &Project,
        nodes: &mut [RuntimeNode],
        registry: &EffectRegistry,
    ) -> Result<Self, String> {
        // Apply against actual current controls when replacing an unfinished scene.
        let mut current = project.clone();
        for n in &mut current.nodes {
            if let Some(live) = nodes.iter().find(|v| v.config.id == n.id) {
                *n = live.config.clone();
            }
        }
        let target = scene.apply(&current, registry)?;
        let before_solo = current.nodes.iter().any(|n| n.kind.is_source() && n.solo);
        let after_solo = target.nodes.iter().any(|n| n.kind.is_source() && n.solo);
        let masks = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| Mask {
                node: i,
                from: n.scene_mask.unwrap_or_else(|| mask(&n.config, before_solo)),
                to: mask(&target.nodes[i], after_solo),
                owned: true,
            })
            .collect();
        let mut steps = Vec::new();
        for setting in scene.settings.iter().filter(|s| s.included) {
            let index = nodes
                .iter()
                .position(|n| n.config.id == setting.node)
                .ok_or("Scene node is missing")?;
            let n = &mut nodes[index];
            if let Control::Switch { value } = setting.value {
                let fallback = project
                    .connections
                    .iter()
                    .find(|c| c.from == n.config.id)
                    .map(|c| c.to);
                let from = n.config.switch_output.or(fallback);
                let to = value.or(fallback);
                let mut destinations = Vec::new();
                for c in project.connections.iter().filter(|c| c.from == n.config.id) {
                    if !destinations.contains(&c.to) {
                        destinations.push(c.to);
                    }
                }
                let routes = destinations
                    .into_iter()
                    .map(|id| {
                        let start = n
                            .scene_switch
                            .as_ref()
                            .map_or(if from == Some(id) { 1. } else { 0. }, |fade| {
                                fade.weights(id).1
                            });
                        (id, start, if to == Some(id) { 1. } else { 0. })
                    })
                    .collect();
                n.scene_switch = Some(SwitchFade {
                    active: true,
                    routes,
                    previous: 0.,
                    current: 0.,
                });
            }

            let effect = match &setting.value {
                Control::Parameter { effect, .. } | Control::EffectEnabled { effect, .. } => Some(
                    n.config
                        .effects
                        .iter()
                        .position(|e| e.id == *effect)
                        .ok_or("Scene effect is missing")?,
                ),
                _ => None,
            };
            let from = if let Control::Parameter { parameter, .. } = &setting.value {
                let e = &n.config.effects[effect.unwrap()];
                e.parameters
                    .get(parameter)
                    .copied()
                    .or_else(|| {
                        registry.get(&e.kind).and_then(|f| {
                            f.parameters
                                .iter()
                                .find(|p| p.id == parameter)
                                .map(|p| p.default)
                        })
                    })
                    .unwrap_or(0.)
            } else {
                scalar(&setting.value, &n.config)
            };
            let equal = match &setting.value {
                Control::Gain { value }
                | Control::Pan { value }
                | Control::Parameter { value, .. } => from == *value,
                _ => read_control(&setting.value, &n.config) == setting.value,
            };
            if equal && !matches!(setting.value, Control::Switch { .. }) {
                continue;
            }
            if let Control::Parameter { parameter, .. } = &setting.value {
                n.config.effects[effect.unwrap()]
                    .parameters
                    .entry(parameter.clone())
                    .or_insert(from);
            }
            if matches!(setting.value, Control::EffectEnabled { value: true, .. })
                && !n.config.effects[effect.unwrap()].enabled
            {
                if let Some(dsp) = n.effects.get_mut(effect.unwrap()) {
                    dsp.reset();
                }
            }
            if matches!(setting.value, Control::Bypass { value: false })
                && n.config.effects_disabled
            {
                for e in &mut n.effects {
                    e.reset();
                }
            }
            if matches!(
                setting.value,
                Control::Ducking { .. } | Control::DuckingParameter { .. }
            ) {
                if let Some(d) = &mut n.ducking {
                    d.prepare_scene();
                }
            }
            steps.push(Step {
                node: index,
                value: setting.value.clone(),
                from,
                effect,
                smooth: smooth(&setting.value, &n.config),
                applied: false,
                owned: true,
            });
        }
        Ok(Self {
            id: scene.id,
            elapsed: 0,
            total: (scene.duration_ms as u64 * SAMPLE_RATE as u64 / 1000).max(
                if steps.iter().any(|s| guarded(&s.value, s.smooth)) {
                    (BLOCK_SIZE * 4) as u64
                } else {
                    BLOCK_SIZE as u64
                },
            ),
            modified: false,
            finished: false,
            target,
            steps,
            masks,
        })
    }
    pub fn values(&self, nodes: &[RuntimeNode]) -> Vec<crate::workflows::SceneSetting> {
        self.steps
            .iter()
            .map(|step| crate::workflows::SceneSetting {
                node: nodes[step.node].config.id,
                included: true,
                value: read_control(&step.value, &nodes[step.node].config),
            })
            .collect()
    }
    pub fn progress(&self) -> f32 {
        if self.finished {
            1.
        } else {
            self.elapsed as f32 / self.total as f32
        }
    }
    pub fn cancel_manual(&mut self, previous: &Project, next: &Project, nodes: &mut [RuntimeNode]) {
        for s in &mut self.steps {
            let id = nodes[s.node].config.id;
            let old = previous.nodes.iter().find(|n| n.id == id);
            let new = next.nodes.iter().find(|n| n.id == id);
            if old
                .zip(new)
                .is_none_or(|(a, b)| control_changed(&s.value, a, b))
            {
                s.owned = false;
                self.modified = true;
                if matches!(s.value, Control::Switch { .. }) {
                    nodes[s.node].scene_switch = None;
                }
            }
        }
        let solo_changed = previous.nodes.iter().any(|a| {
            next.nodes
                .iter()
                .find(|b| b.id == a.id)
                .is_none_or(|b| a.solo != b.solo)
        });
        for m in &mut self.masks {
            let id = nodes[m.node].config.id;
            if solo_changed
                || previous
                    .nodes
                    .iter()
                    .find(|n| n.id == id)
                    .zip(next.nodes.iter().find(|n| n.id == id))
                    .is_none_or(|(a, b)| a.mute != b.mute)
            {
                m.owned = false;
                nodes[m.node].scene_mask = None;
                self.modified = true;
            }
        }
    }
    pub fn effective_project(
        &self,
        project: &Project,
        nodes: &[RuntimeNode],
        registry: &EffectRegistry,
    ) -> Result<Project, String> {
        let settings = self
            .steps
            .iter()
            .filter(|s| s.owned)
            .map(|s| crate::workflows::SceneSetting {
                node: nodes[s.node].config.id,
                included: true,
                value: read_control(&s.value, &nodes[s.node].config),
            })
            .collect();
        Scene {
            id: self.id,
            effect_kinds: Vec::new(),
            name: String::new(),
            duration_ms: 0,
            settings,
            keybind: String::new(),
        }
        .apply(project, registry)
    }
    pub fn tick(&mut self, nodes: &mut [RuntimeNode]) {
        if self.finished {
            // Keep the final block's previous-to-target ramp, then retire it.
            for node in nodes {
                if let Some(fade) = &mut node.scene_switch {
                    fade.active = false;
                }
            }
            return;
        }
        self.elapsed = (self.elapsed + BLOCK_SIZE as u64).min(self.total);
        let t = self.progress();
        for n in nodes.iter_mut() {
            n.scene_guard = 1.;
        }
        for m in &self.masks {
            if m.owned {
                nodes[m.node].scene_mask = Some(m.from + (m.to - m.from) * t);
            }
        }
        for s in &mut self.steps {
            if !s.owned {
                continue;
            }
            let n = &mut nodes[s.node];
            let guard = guarded(&s.value, s.smooth);
            if guard && self.elapsed < (BLOCK_SIZE * 4) as u64 {
                let x = self.elapsed as f32 / (BLOCK_SIZE * 4) as f32;
                n.scene_guard = n.scene_guard.min((2. * x - 1.).abs());
            }
            let amount = if s.smooth { t } else { 1. };
            if !s.smooth && s.applied {
                continue;
            }
            if guard && self.elapsed <= (BLOCK_SIZE * 2) as u64 {
                continue;
            }
            match &s.value {
                Control::DuckingParameter { parameter, value } => {
                    let _ = n.config.ducking.set(parameter, *value);
                    if let Some(d) = &mut n.ducking {
                        d.configure_scene(&n.config.ducking);
                    }
                }
                Control::Gain { value } => n.config.gain_db = s.from + (*value - s.from) * amount,
                Control::Pan { value } => n.config.pan = s.from + (*value - s.from) * amount,
                Control::Mute { value } => n.config.mute = *value,
                Control::Solo { value } => n.config.solo = *value,
                Control::Listening { value } => n.config.listen_enabled = *value,
                Control::Bypass { value } => n.config.effects_disabled = *value,
                Control::Switch { value } => {
                    if let Some(fade) = &mut n.scene_switch {
                        fade.previous = fade.current;
                        fade.current = t;
                    }
                    if t >= 1. {
                        n.config.switch_output = *value;
                    }
                }
                Control::EffectEnabled { value, .. } => {
                    n.config.effects[s.effect.unwrap()].enabled = *value
                }
                Control::Parameter {
                    parameter, value, ..
                } => {
                    let v = s.from + (*value - s.from) * amount;
                    let i = s.effect.unwrap();
                    if let Some(current) = n.config.effects[i].parameters.get_mut(parameter) {
                        *current = v;
                    }
                    if let Some(dsp) = n.effects.get_mut(i) {
                        let _ = dsp.set_parameter(parameter, v);
                    }
                }
                Control::Ducking { value } => {
                    n.config.ducking = value.clone();
                    if let Some(d) = &mut n.ducking {
                        d.configure_scene(value);
                    }
                }
            }
            // Switch weights need advancement for the entire transition.
            s.applied = !matches!(s.value, Control::Switch { .. });
        }
        if t >= 1. {
            self.finished = true;
            for n in nodes {
                n.scene_mask = None;
                n.scene_guard = 1.;
            }
        }
    }
}
pub(crate) fn control_changed(c: &Control, a: &Node, b: &Node) -> bool {
    match c {
        Control::DuckingParameter { parameter, .. } => {
            a.ducking.value(parameter) != b.ducking.value(parameter)
        }
        Control::Gain { .. } => a.gain_db != b.gain_db,
        Control::Pan { .. } => a.pan != b.pan,
        Control::Mute { .. } => a.mute != b.mute,
        Control::Solo { .. } => a.solo != b.solo,
        Control::Listening { .. } => a.listen_enabled != b.listen_enabled,
        Control::Bypass { .. } => a.effects_disabled != b.effects_disabled,
        Control::Switch { .. } => a.switch_output != b.switch_output,
        Control::Ducking { .. } => a.ducking != b.ducking,
        Control::Parameter {
            effect, parameter, ..
        } => {
            a.effects
                .iter()
                .find(|e| e.id == *effect)
                .and_then(|e| e.parameters.get(parameter))
                != b.effects
                    .iter()
                    .find(|e| e.id == *effect)
                    .and_then(|e| e.parameters.get(parameter))
        }
        Control::EffectEnabled { effect, .. } => {
            a.effects
                .iter()
                .find(|e| e.id == *effect)
                .map(|e| e.enabled)
                != b.effects
                    .iter()
                    .find(|e| e.id == *effect)
                    .map(|e| e.enabled)
        }
    }
}

fn read_control(c: &Control, n: &Node) -> Control {
    match c {
        Control::DuckingParameter { parameter, .. } => Control::DuckingParameter {
            parameter: parameter.clone(),
            value: n.ducking.value(parameter),
        },
        Control::Gain { .. } => Control::Gain { value: n.gain_db },
        Control::Pan { .. } => Control::Pan { value: n.pan },
        Control::Mute { .. } => Control::Mute { value: n.mute },
        Control::Solo { .. } => Control::Solo { value: n.solo },
        Control::Listening { .. } => Control::Listening {
            value: n.listen_enabled,
        },
        Control::Bypass { .. } => Control::Bypass {
            value: n.effects_disabled,
        },
        Control::Switch { .. } => Control::Switch {
            value: n.switch_output,
        },
        Control::Ducking { .. } => Control::Ducking {
            value: n.ducking.clone(),
        },
        Control::Parameter {
            effect, parameter, ..
        } => Control::Parameter {
            effect: *effect,
            parameter: parameter.clone(),
            value: scalar(c, n),
        },
        Control::EffectEnabled { effect, .. } => Control::EffectEnabled {
            effect: *effect,
            value: n
                .effects
                .iter()
                .find(|e| e.id == *effect)
                .is_some_and(|e| e.enabled),
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{graph::Graph, workflows::SceneSetting};
    #[test]
    fn gain_recall_interpolates_and_manual_gain_preserves_pan_transition() {
        let r = EffectRegistry::default();
        let mut p = Project::default();
        let id = p.nodes[0].id;
        let scene = Scene {
            id: 9,
            effect_kinds: Vec::new(),
            name: "Talk".into(),
            duration_ms: 100,
            settings: vec![
                SceneSetting {
                    node: id,
                    included: true,
                    value: Control::Gain { value: -20. },
                },
                SceneSetting {
                    node: id,
                    included: true,
                    value: Control::Pan { value: 1. },
                },
            ],
            keybind: String::new(),
        };
        let mut graph = Graph::new(&p, &r).unwrap();
        p = graph.recall_scene(&scene, &p, &r).unwrap();
        for _ in 0..10 {
            graph.process(&[(id, [[0.1; 2]; BLOCK_SIZE])]);
        }
        let n = &graph.nodes[0];
        assert!(n.config.gain_db < 0. && n.config.gain_db > -20.);
        assert!(n.config.pan > 0. && n.config.pan < 1.);
        p.nodes[0].gain_db = -3.;
        graph.update(&p, &r).unwrap();
        for _ in 0..50 {
            graph.process(&[(id, [[0.1; 2]; BLOCK_SIZE])]);
        }
        assert_eq!(graph.nodes[0].config.gain_db, -3.);
        assert_eq!(graph.nodes[0].config.pan, 1.);
        assert!(graph.scene.as_ref().unwrap().modified);
    }
    #[test]
    fn new_recall_begins_at_current_interpolated_gain() {
        let r = EffectRegistry::default();
        let p = Project::default();
        let id = p.nodes[0].id;
        let mut scene = Scene {
            id: 1,
            effect_kinds: Vec::new(),
            name: "A".into(),
            duration_ms: 1000,
            settings: vec![SceneSetting {
                node: id,
                included: true,
                value: Control::Gain { value: -40. },
            }],
            keybind: String::new(),
        };
        let mut g = Graph::new(&p, &r).unwrap();
        let target = g.recall_scene(&scene, &p, &r).unwrap();
        for _ in 0..100 {
            g.process(&[]);
        }
        let current = g.nodes[0].config.gain_db;
        scene.settings[0].value = Control::Gain { value: 0. };
        g.recall_scene(&scene, &target, &r).unwrap();
        g.process(&[]);
        assert!((g.nodes[0].config.gain_db - current).abs() < 0.2);
    }
}
#[cfg(test)]
mod switch_tests {
    use super::*;
    use crate::{
        graph::Graph,
        project::{Connection, InputPort, Node, NodeKind, OutputPort},
        workflows::SceneSetting,
    };
    #[test]
    fn interrupted_switch_crossfades_from_the_actual_three_destination_mix() {
        let r = EffectRegistry::default();
        let mut p = Project {
            nodes: vec![
                Node::new(1, "Input", NodeKind::Soundboard, [0.; 2]),
                Node::new(2, "Switch", NodeKind::Switch, [0.; 2]),
                Node::new(3, "A", NodeKind::Processor, [0.; 2]),
                Node::new(4, "B", NodeKind::Processor, [0.; 2]),
                Node::new(5, "C", NodeKind::Processor, [0.; 2]),
            ],
            connections: [(1, 2), (2, 3), (2, 4), (2, 5)]
                .into_iter()
                .map(|(from, to)| Connection {
                    from,
                    to,
                    port: OutputPort::Passthrough,
                    input: InputPort::Main,
                })
                .collect(),
            ..Project::default()
        };
        p.nodes[1].switch_output = Some(3);
        let mut scene = Scene {
            id: 1,
            name: "Switch".into(),
            duration_ms: 100,
            effect_kinds: vec![],
            settings: vec![SceneSetting {
                node: 2,
                included: true,
                value: Control::Switch { value: Some(4) },
            }],
            keybind: String::new(),
        };
        let mut g = Graph::new(&p, &r).unwrap();
        let target = g.recall_scene(&scene, &p, &r).unwrap();
        for _ in 0..15 {
            g.process(&[(1, [[0.25; 2]; BLOCK_SIZE])]);
        }
        let last: [f32; 3] = std::array::from_fn(|i| g.nodes[i + 2].frames[BLOCK_SIZE - 1][0]);
        scene.settings[0].value = Control::Switch { value: Some(5) };
        g.recall_scene(&scene, &target, &r).unwrap();
        g.process(&[(1, [[0.25; 2]; BLOCK_SIZE])]);
        for (i, value) in last.iter().enumerate() {
            assert!((g.nodes[i + 2].frames[0][0] - value).abs() < 0.0001);
        }
        for frame in 0..BLOCK_SIZE {
            let sum: f32 = g.nodes[2..5].iter().map(|n| n.frames[frame][0]).sum();
            assert!((sum - 0.25).abs() < 0.00001);
        }
        for _ in 0..40 {
            g.process(&[(1, [[0.25; 2]; BLOCK_SIZE])]);
        }
        assert_eq!(g.nodes[2].frames[0], [0.; 2]);
        assert_eq!(g.nodes[3].frames[0], [0.; 2]);
        assert_eq!(g.nodes[4].frames[0], [0.25; 2]);
    }
}
#[cfg(test)]
mod retention_tests {
    use super::*;
    use crate::{
        effects::EffectSpec,
        graph::Graph,
        project::{Node, NodeKind},
    };
    #[test]
    fn identical_scene_is_sample_identical_and_retains_a_pending_delay_tail() {
        let registry = EffectRegistry::default();
        let mut source = Node::new(1, "Source", NodeKind::Soundboard, [0.; 2]);
        let mut delay = EffectSpec::new(registry.get("delay").unwrap());
        delay.parameters.insert("time".into(), 100.);
        delay.parameters.insert("mix".into(), 1.);
        source.effects.push(delay);
        let project = Project {
            nodes: vec![source],
            connections: vec![],
            ..Project::default()
        };
        let mut baseline = Graph::new(&project, &registry).unwrap();
        let mut recalled = Graph::new(&project, &registry).unwrap();
        let mut impulse = [[0.; 2]; BLOCK_SIZE];
        impulse[0] = [0.25; 2];
        baseline.process(&[(1, impulse)]);
        recalled.process(&[(1, impulse)]);
        let scene = Scene::capture(&project, &registry, "Unchanged".into());
        recalled.recall_scene(&scene, &project, &registry).unwrap();
        let mut peak = 0f32;
        for _ in 0..80 {
            baseline.process(&[]);
            recalled.process(&[]);
            assert_eq!(baseline.nodes[0].frames, recalled.nodes[0].frames);
            peak = peak.max(recalled.nodes[0].peak);
        }
        assert!(peak > 0.1);
    }
}
#[cfg(test)]
mod ducking_scene_tests {
    use super::*;
    use crate::{
        effects::EffectRegistry,
        graph::Graph,
        project::{Node, NodeKind},
    };
    #[test]
    fn ducking_parameters_can_be_excluded_individually_and_zero_duration_applies_guarded_changes() {
        let registry = EffectRegistry::default();
        let mut p = Project {
            nodes: vec![Node::new(1, "Ducking", NodeKind::AutoDucking, [0.; 2])],
            connections: vec![],
            ..Project::default()
        };
        let mut scene = Scene::capture(&p, &registry, "Speech".into());
        scene.duration_ms = 0;
        assert_eq!(
            scene
                .settings
                .iter()
                .filter(|s| matches!(s.value, Control::DuckingParameter { .. }))
                .count(),
            crate::ducking::PARAMETERS.len()
        );
        for s in &mut scene.settings {
            if let Control::DuckingParameter { parameter, value } = &mut s.value {
                match parameter.as_str() {
                    "detection" => *value = 1.,
                    "lookahead_enabled" => *value = 1.,
                    "lookahead" => *value = 50.,
                    "threshold" => {
                        *value = -10.;
                        s.included = false;
                    }
                    _ => {}
                }
            }
        }
        let mut graph = Graph::new(&p, &registry).unwrap();
        p = graph.recall_scene(&scene, &p, &registry).unwrap();
        for _ in 0..8 {
            graph.process(&[]);
        }
        assert!(graph.scene.as_ref().unwrap().finished);
        assert_eq!(
            graph.nodes[0].config.ducking.detection,
            crate::ducking::Detection::Speech
        );
        assert!(graph.nodes[0].config.ducking.lookahead_enabled);
        assert_eq!(graph.nodes[0].config.ducking.lookahead, 50.);
        assert_eq!(graph.nodes[0].config.ducking.threshold, -35.);
        assert_eq!(p.nodes[0].ducking.threshold, -35.);
    }
}
