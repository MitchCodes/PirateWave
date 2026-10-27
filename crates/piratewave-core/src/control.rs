//! Controller catalog and portable, name-based targets. No OS or audio-thread I/O.
use crate::{
    effects::EffectRegistry,
    project::{Node, NodeKind, Project},
    recording::SlotAction,
    shortcuts::{AppAction, EffectAction, NodeAction, ShortcutAction},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Selector {
    pub category: String,
    pub node_kind: String,
    pub node: String,
    pub target: String,
    pub action: String,
    #[serde(default)]
    pub plugin: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub selector: Selector,
    pub label: String,
    pub state: String,
    /// Current boolean state of this control, independent of its display label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    pub adjustment: bool,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub step: f64,
    pub unit: String,
    pub choices: Vec<String>,
    pub discrete: bool,
}
pub fn kind(kind: &NodeKind) -> &str {
    match kind {
        NodeKind::Extension { definition } => &definition.id,
        NodeKind::Input {
            loopback: false, ..
        } => "input",
        NodeKind::Input { loopback: true, .. } => "desktop",
        NodeKind::ApplicationCapture { .. } => "application_capture",
        NodeKind::Soundboard => "soundboard",
        NodeKind::Output { .. } => "output",
        NodeKind::Processor => "processor",
        NodeKind::VstEffect => "vst_effect",
        NodeKind::VstInstrument => "vst_instrument",
        NodeKind::VoiceChanger => "voice_changer",
        NodeKind::Switch => "switch",
        NodeKind::Recording => "recording",
        NodeKind::AutoDucking => "auto_ducking",
    }
}
fn code<T: Serialize>(v: T) -> String {
    serde_json::to_value(v)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}
fn base(
    category: &str,
    n: Option<&Node>,
    target: &str,
    action: String,
    label: &str,
    state: String,
) -> Entry {
    Entry {
        selector: Selector {
            category: category.into(),
            node_kind: n.map_or("", |n| kind(&n.kind)).into(),
            node: n.map_or("", |n| &n.name).into(),
            target: target.into(),
            action,
            plugin: String::new(),
        },
        label: label.into(),
        state,
        active: None,
        adjustment: false,
        value: 0.,
        min: 0.,
        max: 1.,
        default: 0.,
        step: 1.,
        unit: String::new(),
        choices: vec![],
        discrete: false,
    }
}
pub fn catalog(p: &Project, registry: &EffectRegistry, running: bool) -> Vec<Entry> {
    let mut entries = vec![];
    for a in AppAction::ALL {
        let mut entry = base(
            "application",
            None,
            "",
            code(a),
            a.label(),
            if running { "Running" } else { "Stopped" }.into(),
        );
        if !matches!(a, AppAction::StopSounds) {
            entry.active = Some(running);
        }
        entries.push(entry);
    }
    for s in &p.sounds {
        entries.push(base(
            "soundboard",
            None,
            &s.name,
            "play".into(),
            &s.name,
            "Ready".into(),
        ));
    }
    for n in &p.nodes {
        for (effect, d) in std::iter::once((None, crate::extensions::descriptor(n, None))).chain(
            n.effects
                .iter()
                .map(|e| (Some(e), e.extension_descriptor.as_ref())),
        ) {
            if let Some(d) = d {
                let available = crate::extensions::get(&d.id).is_some_and(|e| e.descriptor == *d);
                for a in &d.actions {
                    let mut e = base(
                        "extension_action",
                        Some(n),
                        effect.map_or("", |e| e.kind.as_str()),
                        a.id.clone(),
                        &a.name,
                        if available { "Ready" } else { "Unavailable" }.into(),
                    );
                    if available {
                        if let Some(param) = d.parameters.iter().find(|p| p.id == a.parameter) {
                            let values = effect.map_or(&n.extension.parameters, |e| &e.parameters);
                            let value = *values.get(&param.id).unwrap_or(&param.default);
                            if a.toggle {
                                e.active = Some(value == a.value);
                            }
                            e.state = if value == a.value {
                                "Active".into()
                            } else if !param.choices.is_empty() {
                                param
                                    .choices
                                    .get(value as usize)
                                    .cloned()
                                    .unwrap_or_default()
                            } else {
                                format!("{} {}", value, param.unit)
                            };
                        }
                    }
                    e.selector.plugin = d.id.clone();
                    entries.push(e);
                }
                if effect.is_none() {
                    for param in &d.parameters {
                        let mut e = base(
                            "extension_parameter",
                            Some(n),
                            &d.id,
                            param.id.clone(),
                            &param.name,
                            if available { "Ready" } else { "Unavailable" }.into(),
                        );
                        e.selector.plugin = d.id.clone();
                        e.adjustment = true;
                        e.value = *n
                            .extension
                            .parameters
                            .get(&param.id)
                            .unwrap_or(&param.default) as f64;
                        e.min = param.min as f64;
                        e.max = param.max as f64;
                        e.default = param.default as f64;
                        e.step = if param.step > 0. {
                            param.step as f64
                        } else {
                            (param.max - param.min) as f64 * 0.01
                        };
                        e.discrete = param.step > 0.;
                        e.unit = param.unit.clone();
                        e.choices = param.choices.clone();
                        entries.push(e);
                    }
                }
            }
        }
        for &a in NodeAction::for_kind(&n.kind) {
            let state = if matches!(
                a,
                NodeAction::Mute | NodeAction::Unmute | NodeAction::ToggleMute
            ) {
                if n.mute {
                    "Muted"
                } else {
                    "Unmuted"
                }
            } else if matches!(
                a,
                NodeAction::ListenOn | NodeAction::ListenOff | NodeAction::ToggleListening
            ) {
                if n.listen_enabled {
                    "Listening"
                } else {
                    "Not listening"
                }
            } else if matches!(
                a,
                NodeAction::SoloOn | NodeAction::SoloOff | NodeAction::ToggleSolo
            ) {
                if n.solo {
                    "Solo on"
                } else {
                    "Solo off"
                }
            } else if matches!(n.kind, NodeKind::Recording)
                && matches!(
                    a,
                    NodeAction::Disable | NodeAction::Enable | NodeAction::ToggleDisabled
                )
            {
                if n.effects_disabled {
                    "Bypassed"
                } else {
                    "Enabled"
                }
            } else if matches!(n.kind, NodeKind::Recording) {
                if n.recording.enabled {
                    "Armed"
                } else {
                    "Stopped"
                }
            } else if n.kind.has_effect_controls() || matches!(n.kind, NodeKind::AutoDucking) {
                if n.effects_disabled {
                    "Bypassed"
                } else {
                    "Enabled"
                }
            } else if n.mute {
                "Muted"
            } else if n.solo {
                "Solo"
            } else {
                "Ready"
            };
            let mut entry = base("node", Some(n), "", code(a), a.label(), state.into());
            entry.active = match a {
                NodeAction::Mute | NodeAction::Unmute | NodeAction::ToggleMute => Some(n.mute),
                NodeAction::SoloOn | NodeAction::SoloOff | NodeAction::ToggleSolo => Some(n.solo),
                NodeAction::ListenOn | NodeAction::ListenOff | NodeAction::ToggleListening => {
                    Some(n.listen_enabled)
                }
                NodeAction::Disable
                | NodeAction::Enable
                | NodeAction::ToggleDisabled
                | NodeAction::EnableDucking
                | NodeAction::DisableDucking
                | NodeAction::ToggleDucking => Some(!n.effects_disabled),
                NodeAction::StartRecording | NodeAction::StopRecording => Some(n.recording.enabled),
                _ => None,
            };
            entries.push(entry);
        }
        if n.kind.is_source() || matches!(n.kind, NodeKind::Output { .. }) {
            let mut e = base(
                "mixer",
                Some(n),
                "",
                "volume".into(),
                "Volume",
                if n.mute { "Muted" } else { "" }.into(),
            );
            e.adjustment = true;
            e.active = Some(n.mute);
            e.value = n.gain_db as f64;
            e.min = -60.;
            e.max = 12.;
            e.step = 1.;
            e.unit = "dB".into();
            entries.push(e);
            if n.kind.is_source() {
                let mut e = base("mixer", Some(n), "", "pan".into(), "Pan", String::new());
                e.adjustment = true;
                e.value = n.pan as f64;
                e.min = -1.;
                e.max = 1.;
                e.step = 0.05;
                entries.push(e);
            }
        }
        if matches!(n.kind, NodeKind::Recording) {
            for s in &n.recording.slots {
                for a in SlotAction::ALL {
                    entries.push(base(
                        "recording_slot",
                        Some(n),
                        &s.name,
                        code(a),
                        a.label(),
                        if s.asset.is_some() {
                            "Captured"
                        } else {
                            "Empty"
                        }
                        .into(),
                    ));
                }
            }
        }
        if matches!(n.kind, NodeKind::AutoDucking) {
            for parameter in crate::ducking::PARAMETERS {
                let mut e = base(
                    "ducking_parameter",
                    Some(n),
                    "",
                    parameter.id.into(),
                    parameter.label,
                    String::new(),
                );
                e.adjustment = true;
                e.value = n.ducking.value(parameter.id) as f64;
                e.min = parameter.min as f64;
                e.max = parameter.max as f64;
                e.default = parameter.default as f64;
                e.step = parameter.step as f64;
                e.unit = parameter.unit.into();
                e.choices = parameter.choices.iter().map(|s| (*s).into()).collect();
                e.discrete = !parameter.choices.is_empty();
                entries.push(e);
            }
        }
        if matches!(n.kind, NodeKind::Switch) {
            let mut destinations = std::collections::HashSet::new();
            for c in p
                .connections
                .iter()
                .filter(|c| c.from == n.id && destinations.insert(c.to))
            {
                if let Some(dest) = p.nodes.iter().find(|d| d.id == c.to) {
                    let mut e = base(
                        "switch",
                        Some(n),
                        &dest.name,
                        "activate".into(),
                        &dest.name,
                        if n.switch_output == Some(dest.id) {
                            "Active"
                        } else {
                            "Inactive"
                        }
                        .into(),
                    );
                    e.selector.plugin = kind(&dest.kind).into();
                    e.active = Some(n.switch_output == Some(dest.id));
                    if !entries.iter().any(|old: &Entry| old.selector == e.selector) {
                        entries.push(e);
                    }
                }
            }
        }
        for effect in &n.effects {
            if n.kind.has_effect_controls() {
                for a in EffectAction::ALL {
                    let mut entry = base(
                        "effect",
                        Some(n),
                        &effect.kind,
                        code(a),
                        a.label(),
                        if effect.enabled {
                            "Enabled"
                        } else {
                            "Disabled"
                        }
                        .into(),
                    );
                    entry.active = Some(effect.enabled);
                    entries.push(entry);
                }
            }
            if let Some(factory) = registry.get(&effect.kind) {
                for param in factory.parameters {
                    let mut e = base(
                        "effect_parameter",
                        Some(n),
                        &effect.kind,
                        param.id.into(),
                        param.label,
                        String::new(),
                    );
                    e.adjustment = true;
                    e.value = *effect.parameters.get(param.id).unwrap_or(&param.default) as f64;
                    e.min = param.min as f64;
                    e.max = param.max as f64;
                    e.default = param.default as f64;
                    e.step = (e.max - e.min) * 0.01;
                    e.unit = param.unit.into();
                    let choices = crate::rack_processors::choices(&effect.kind, param.id)
                        .or_else(|| crate::voice_effects::choices(&effect.kind, param.id))
                        .or_else(|| {
                            (effect.kind == "automatic_voice_correction")
                                .then(|| crate::voice_correction::choices(param.id))
                                .flatten()
                        });
                    if let Some(choices) = choices {
                        e.choices = choices.iter().map(|s| s.to_string()).collect();
                        e.step = 1.;
                        e.discrete = true;
                    }
                    if let Some(d) = &effect.extension_descriptor {
                        if let Some(p) = d.parameters.iter().find(|p| p.id == param.id) {
                            e.step = if p.step > 0. {
                                p.step as f64
                            } else {
                                (p.max - p.min) as f64 * 0.01
                            };
                            e.discrete = p.step > 0.;
                            e.choices = p.choices.clone();
                        }
                    }
                    entries.push(e);
                }
            }
        }
        if let Some(plugin) = &n.plugin {
            for param in &plugin.descriptor.parameters {
                let mut e = base(
                    "vst_parameter",
                    Some(n),
                    &plugin.descriptor.name,
                    param.id.to_string(),
                    &param.name,
                    String::new(),
                );
                e.selector.plugin = format!(
                    "{:?}:{}",
                    plugin.descriptor.format, plugin.descriptor.class_id
                );
                e.adjustment = true;
                e.value = *plugin.parameters.get(&param.id).unwrap_or(&param.default);
                e.default = param.default;
                e.discrete = param.steps > 0;
                e.step = if param.steps > 0 {
                    1. / param.steps as f64
                } else {
                    0.01
                };
                e.unit = "normalized".into();
                entries.push(e);
            }
        }
    }
    entries
}
fn unique<T>(items: impl Iterator<Item = T>) -> Result<T, String> {
    let mut items = items;
    let first = items.next().ok_or("Missing target")?;
    if items.next().is_some() {
        Err("Ambiguous target".into())
    } else {
        Ok(first)
    }
}
pub fn entry<'a>(entries: &'a [Entry], selector: &Selector) -> Result<&'a Entry, String> {
    unique(entries.iter().filter(|e| &e.selector == selector))
}
pub fn node<'a>(p: &'a Project, s: &Selector) -> Result<&'a Node, String> {
    unique(
        p.nodes
            .iter()
            .filter(|n| n.name == s.node && kind(&n.kind) == s.node_kind),
    )
}
fn decode<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(s.into()))
        .map_err(|_| "Unsupported action".into())
}
pub fn resolve(p: &Project, s: &Selector) -> Result<ShortcutAction, String> {
    if s.category == "application" {
        return Ok(ShortcutAction::App(decode(&s.action)?));
    }
    if s.category == "soundboard" {
        return Ok(ShortcutAction::Sound(
            unique(p.sounds.iter().filter(|sound| sound.name == s.target))?.id,
        ));
    }
    let n = node(p, s)?;
    let action = match s.category.as_str() {
        "extension_action" => {
            let effect = if s.target.is_empty() {
                None
            } else {
                Some(unique(n.effects.iter().filter(|e| e.kind == s.target))?.id)
            };
            let d = crate::extensions::descriptor(n, effect).ok_or("Missing extension")?;
            if d.id != s.plugin {
                return Err("Extension identity changed".into());
            }
            ShortcutAction::Extension {
                id: n.id,
                effect,
                action: crate::extensions::action_key(&s.action),
            }
        }
        "node" => ShortcutAction::Node {
            id: n.id,
            action: decode(&s.action)?,
        },
        "effect" => ShortcutAction::Effect {
            id: n.id,
            effect: unique(n.effects.iter().filter(|e| e.kind == s.target))?.id,
            action: decode(&s.action)?,
        },
        "recording_slot" => ShortcutAction::RecordingSlot {
            id: n.id,
            slot: unique(
                n.recording
                    .slots
                    .iter()
                    .filter(|slot| slot.name == s.target),
            )?
            .id,
            action: decode(&s.action)?,
        },
        "switch" => {
            let dest = unique(
                p.nodes
                    .iter()
                    .filter(|d| d.name == s.target && kind(&d.kind) == s.plugin),
            )?;
            ShortcutAction::SwitchOutput {
                id: n.id,
                output: dest.id,
            }
        }
        _ => return Err("Unsupported action".into()),
    };
    if action.exists(p) {
        Ok(action)
    } else {
        Err("Missing target".into())
    }
}
pub fn adjust(
    p: &mut Project,
    registry: &EffectRegistry,
    s: &Selector,
    ticks: i32,
    sensitivity: f64,
    reset: bool,
) -> Result<(), String> {
    if !sensitivity.is_finite()
        || !(0.01..=100.).contains(&sensitivity)
        || ticks.unsigned_abs() > 10000
    {
        return Err("Invalid adjustment".into());
    }
    let entries = catalog(p, registry, false);
    let e = entry(&entries, s)?;
    if !e.adjustment {
        return Err("Not an adjustment".into());
    }
    let id = node(p, s)?.id;
    let mut value = if reset {
        e.default
    } else {
        e.value + ticks as f64 * e.step * sensitivity
    };
    if e.discrete {
        value = e.min + ((value - e.min) / e.step).round() * e.step;
    }
    value = value.clamp(e.min, e.max);
    let n = p.nodes.iter_mut().find(|n| n.id == id).unwrap();
    match s.category.as_str() {
        "mixer" => match s.action.as_str() {
            "volume" => {
                if reset {
                    n.mute = !n.mute;
                } else {
                    n.gain_db = value as f32;
                }
            }
            "pan" => n.pan = value as f32,
            _ => return Err("Unsupported adjustment".into()),
        },
        "extension_parameter" => {
            let NodeKind::Extension { definition: d } = &n.kind else {
                return Err("Missing extension".into());
            };
            if d.id != s.plugin
                || !crate::extensions::get(&d.id).is_some_and(|e| e.descriptor == **d)
            {
                return Err("Extension unavailable or incompatible".into());
            }
            n.extension
                .parameters
                .insert(s.action.clone(), value as f32);
        }
        "ducking_parameter" => n.ducking.set(&s.action, value as f32)?,
        "effect_parameter" => {
            let e = unique(n.effects.iter_mut().filter(|e| e.kind == s.target))?;
            if let Some(d) = &e.extension_descriptor {
                if !crate::extensions::get(&d.id).is_some_and(|e| e.descriptor == *d) {
                    return Err("Extension unavailable or incompatible".into());
                }
            }
            e.parameters.insert(s.action.clone(), value as f32);
        }
        "vst_parameter" => {
            n.plugin
                .as_mut()
                .ok_or("Missing plugin")?
                .parameters
                .insert(s.action.parse().map_err(|_| "Invalid parameter")?, value);
        }
        _ => return Err("Unsupported adjustment".into()),
    }
    Ok(())
}

/// Reset a parameter's value. Legacy controller `adjust(reset=true)` also
/// implements volume-knob press (mute); keep that behavior for v1 clients.
pub fn reset_parameter(
    p: &mut Project,
    registry: &EffectRegistry,
    s: &Selector,
) -> Result<(), String> {
    if s.category == "mixer" && s.action == "volume" {
        let entries = catalog(p, registry, false);
        let e = entry(&entries, s)?;
        let id = node(p, s)?.id;
        p.nodes.iter_mut().find(|n| n.id == id).unwrap().gain_db =
            e.default.clamp(e.min, e.max) as f32;
        Ok(())
    } else {
        adjust(p, registry, s, 0, 1., true)
    }
}

#[cfg(test)]
mod tests;
