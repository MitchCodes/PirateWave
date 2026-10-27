//! Platform-independent shortcut actions and persistent binding data.
use crate::project::{Node, NodeKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppAction {
    StopSounds,
    StartAudio,
    StopAudio,
    ToggleAudio,
}
impl AppAction {
    pub const ALL: [Self; 4] = [
        Self::StopSounds,
        Self::StartAudio,
        Self::StopAudio,
        Self::ToggleAudio,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::StopSounds => "Stop all soundboard sounds",
            Self::StartAudio => "Start audio",
            Self::StopAudio => "Stop audio",
            Self::ToggleAudio => "Toggle audio",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeAction {
    Mute,
    Unmute,
    ToggleMute,
    SoloOn,
    SoloOff,
    ToggleSolo,
    VolumeUp,
    VolumeDown,
    ResetVolume,
    ListenOn,
    ListenOff,
    ToggleListening,
    PanLeft100,
    PanLeft50,
    PanCenter,
    PanRight50,
    PanRight100,
    Disable,
    Enable,
    ToggleDisabled,
    PlayNote,
    ReleaseNote,
    AllNotesOff,
    StartRecording,
    StopRecording,
    StopPlayingRecording,
    EnableDucking,
    DisableDucking,
    ToggleDucking,
}
impl NodeAction {
    pub const SOURCE: [Self; 14] = [
        Self::Mute,
        Self::Unmute,
        Self::ToggleMute,
        Self::SoloOn,
        Self::SoloOff,
        Self::ToggleSolo,
        Self::VolumeUp,
        Self::VolumeDown,
        Self::ResetVolume,
        Self::PanLeft100,
        Self::PanLeft50,
        Self::PanCenter,
        Self::PanRight50,
        Self::PanRight100,
    ];
    pub const INSTRUMENT: [Self; 17] = [
        Self::Mute,
        Self::Unmute,
        Self::ToggleMute,
        Self::SoloOn,
        Self::SoloOff,
        Self::ToggleSolo,
        Self::VolumeUp,
        Self::VolumeDown,
        Self::ResetVolume,
        Self::PanLeft100,
        Self::PanLeft50,
        Self::PanCenter,
        Self::PanRight50,
        Self::PanRight100,
        Self::PlayNote,
        Self::ReleaseNote,
        Self::AllNotesOff,
    ];
    pub const OUTPUT: [Self; 8] = [
        Self::Mute,
        Self::Unmute,
        Self::VolumeUp,
        Self::VolumeDown,
        Self::ResetVolume,
        Self::ListenOn,
        Self::ListenOff,
        Self::ToggleListening,
    ];
    pub const RECORDING: [Self; 6] = [
        Self::StartRecording,
        Self::StopRecording,
        Self::StopPlayingRecording,
        Self::Disable,
        Self::Enable,
        Self::ToggleDisabled,
    ];
    pub const DUCKING: [Self; 3] = [
        Self::EnableDucking,
        Self::DisableDucking,
        Self::ToggleDucking,
    ];
    pub const PROCESSOR: [Self; 3] = [Self::Disable, Self::Enable, Self::ToggleDisabled];
    pub fn for_kind(kind: &NodeKind) -> &'static [Self] {
        if matches!(kind, NodeKind::AutoDucking) {
            &Self::DUCKING
        } else if matches!(kind, NodeKind::Recording) {
            &Self::RECORDING
        } else if matches!(kind, NodeKind::Switch) {
            &[]
        } else if matches!(kind, NodeKind::VstInstrument) {
            &Self::INSTRUMENT
        } else if kind.is_source() {
            &Self::SOURCE
        } else if kind.is_processor() {
            &Self::PROCESSOR
        } else {
            &Self::OUTPUT
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Mute => "Mute",
            Self::Unmute => "Unmute",
            Self::ToggleMute => "Toggle mute",
            Self::SoloOn => "Solo on",
            Self::SoloOff => "Solo off",
            Self::ToggleSolo => "Toggle solo",
            Self::VolumeUp => "Volume up 1 dB",
            Self::VolumeDown => "Volume down 1 dB",
            Self::ResetVolume => "Reset volume to 0 dB",
            Self::ListenOn => "Turn on listening",
            Self::ListenOff => "Turn off listening",
            Self::ToggleListening => "Toggle listening",
            Self::PanLeft100 => "Set 100% left",
            Self::PanLeft50 => "Set 50% left",
            Self::PanCenter => "Set balanced (0%)",
            Self::PanRight50 => "Set 50% right",
            Self::PanRight100 => "Set 100% right",
            Self::Disable => "Disable (passthrough)",
            Self::Enable => "Enable effects",
            Self::ToggleDisabled => "Toggle effects",
            Self::PlayNote => "Play audition note",
            Self::ReleaseNote => "Release audition note",
            Self::AllNotesOff => "Stop all notes",
            Self::StartRecording => "Start recording",
            Self::StopRecording => "Stop recording",
            Self::StopPlayingRecording => "Stop playing recording",
            Self::EnableDucking => "Enable ducking",
            Self::DisableDucking => "Disable ducking",
            Self::ToggleDucking => "Toggle ducking",
        }
    }
    /// Returns whether the node changed. Unsupported actions never mutate it.
    pub fn apply(self, node: &mut Node) -> Result<bool, String> {
        if !Self::for_kind(&node.kind).contains(&self) {
            return Err(format!("{} does not support {}", node.name, self.label()));
        }
        let before = (
            node.mute,
            node.solo,
            node.gain_db,
            node.pan,
            node.effects_disabled,
            node.listen_enabled,
            node.recording.enabled,
        );
        match self {
            Self::EnableDucking => node.effects_disabled = false,
            Self::DisableDucking => node.effects_disabled = true,
            Self::ToggleDucking => node.effects_disabled = !node.effects_disabled,
            Self::StartRecording => node.recording.enabled = true,
            Self::StopRecording => node.recording.enabled = false,
            Self::PlayNote | Self::ReleaseNote | Self::AllNotesOff | Self::StopPlayingRecording => {
            }
            Self::Mute => node.mute = true,
            Self::Unmute => node.mute = false,
            Self::ToggleMute => node.mute = !node.mute,
            Self::SoloOn => node.solo = true,
            Self::SoloOff => node.solo = false,
            Self::ToggleSolo => node.solo = !node.solo,
            Self::VolumeUp => {
                if node.gain_db < 12.0 {
                    node.gain_db = (node.gain_db + 1.0).min(12.0);
                }
            }
            Self::VolumeDown => node.gain_db = (node.gain_db - 1.0).max(-60.0),
            Self::ResetVolume => node.gain_db = 0.0,
            Self::ListenOn => node.listen_enabled = true,
            Self::ListenOff => node.listen_enabled = false,
            Self::ToggleListening => node.listen_enabled = !node.listen_enabled,
            Self::PanLeft100 => node.pan = -1.0,
            Self::PanLeft50 => node.pan = -0.5,
            Self::PanCenter => node.pan = 0.0,
            Self::PanRight50 => node.pan = 0.5,
            Self::PanRight100 => node.pan = 1.0,
            Self::Disable => node.effects_disabled = true,
            Self::Enable => node.effects_disabled = false,
            Self::ToggleDisabled => node.effects_disabled = !node.effects_disabled,
        }
        Ok(before
            != (
                node.mute,
                node.solo,
                node.gain_db,
                node.pan,
                node.effects_disabled,
                node.listen_enabled,
                node.recording.enabled,
            ))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectAction {
    Enable,
    Disable,
    Toggle,
}
impl EffectAction {
    pub const ALL: [Self; 3] = [Self::Enable, Self::Disable, Self::Toggle];
    pub fn label(self) -> &'static str {
        match self {
            Self::Enable => "Enable effect",
            Self::Disable => "Disable effect",
            Self::Toggle => "Toggle effect",
        }
    }
    pub fn apply(self, effect: &mut crate::effects::EffectSpec) {
        effect.enabled = match self {
            Self::Enable => true,
            Self::Disable => false,
            Self::Toggle => !effect.enabled,
        };
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShortcutAction {
    Extension {
        id: u64,
        effect: Option<u64>,
        action: u64,
    },
    App(AppAction),
    Node {
        id: u64,
        action: NodeAction,
    },
    Sound(u64),
    RecordingSlot {
        id: u64,
        slot: u64,
        action: crate::recording::SlotAction,
    },
    SwitchOutput {
        id: u64,
        output: u64,
    },
    Effect {
        id: u64,
        effect: u64,
        action: EffectAction,
    },
}
impl ShortcutAction {
    pub fn exists(self, project: &crate::project::Project) -> bool {
        match self {
            Self::Extension { id, effect, action } => {
                crate::extensions::action_descriptor(project, id, effect, action).is_some()
            }
            Self::RecordingSlot { id, slot, .. } => project.nodes.iter().any(|n| {
                n.id == id
                    && matches!(n.kind, NodeKind::Recording)
                    && n.recording.slots.iter().any(|s| s.id == slot)
            }),
            Self::App(_) => true,
            Self::Sound(id) => project.sounds.iter().any(|s| s.id == id),
            Self::Node { id, action } => project
                .nodes
                .iter()
                .any(|n| n.id == id && NodeAction::for_kind(&n.kind).contains(&action)),
            Self::SwitchOutput { id, output } => {
                project
                    .nodes
                    .iter()
                    .any(|n| n.id == id && matches!(n.kind, NodeKind::Switch))
                    && project
                        .connections
                        .iter()
                        .any(|c| c.from == id && c.to == output)
            }
            Self::Effect { id, effect, .. } => project.nodes.iter().any(|n| {
                n.id == id
                    && n.kind.has_effect_controls()
                    && n.effects.iter().any(|e| e.id == effect)
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub loupedeck_enabled: bool,
    pub version: u32,
    pub keybinds: BTreeMap<AppAction, String>,
    pub startup: StartupSettings,
    pub ui: UiPreferences,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UtilityFilter {
    #[default]
    All,
    Processors,
    Switches,
    Recorders,
    AutoDucking,
}
impl UtilityFilter {
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::Processors,
        Self::Switches,
        Self::Recorders,
        Self::AutoDucking,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Processors => "Processors",
            Self::Switches => "Switches",
            Self::Recorders => "Recorders",
            Self::AutoDucking => "Auto Ducking",
        }
    }
    pub fn includes(self, kind: &NodeKind) -> bool {
        !kind.is_source()
            && !matches!(kind, NodeKind::Output { .. })
            && match self {
                Self::All => true,
                Self::Processors => kind.is_processor(),
                Self::Switches => matches!(kind, NodeKind::Switch),
                Self::Recorders => matches!(kind, NodeKind::Recording),
                Self::AutoDucking => matches!(kind, NodeKind::AutoDucking),
            }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Dark,
    Light,
    Bumblebee,
    MintGreen,
    Halloween,
    Christmas,
    Ocean,
    Rose,
    Lavender,
    Ember,
    Midnight,
}
impl Theme {
    pub const ALL: [Self; 11] = [
        Self::Dark,
        Self::Light,
        Self::Bumblebee,
        Self::MintGreen,
        Self::Halloween,
        Self::Christmas,
        Self::Ocean,
        Self::Rose,
        Self::Lavender,
        Self::Ember,
        Self::Midnight,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::Bumblebee => "Bumblebee",
            Self::MintGreen => "Mint Green",
            Self::Halloween => "Halloween",
            Self::Christmas => "Christmas",
            Self::Ocean => "Ocean",
            Self::Rose => "Rose",
            Self::Lavender => "Lavender",
            Self::Ember => "Ember",
            Self::Midnight => "Midnight",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPreferences {
    pub theme: Theme,
    pub utilities_expanded: bool,
    pub utilities_height: f32,
    pub utilities_filter: UtilityFilter,
    /// Presentation-only node order, keyed by managed workspace path.
    pub mixer_orders: BTreeMap<String, Vec<u64>>,
}
impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            utilities_expanded: false,
            utilities_height: 240.,
            utilities_filter: UtilityFilter::All,
            mixer_orders: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StartupSettings {
    pub start_with_windows: bool,
    pub start_minimized: bool,
    pub start_audio_automatically: bool,
}
impl Default for AppSettings {
    fn default() -> Self {
        Self {
            loupedeck_enabled: false,
            version: 1,
            startup: StartupSettings::default(),
            ui: UiPreferences::default(),
            keybinds: AppAction::ALL
                .into_iter()
                .map(|action| {
                    (
                        action,
                        if action == AppAction::StopSounds {
                            "Ctrl+Alt+Space".into()
                        } else {
                            String::new()
                        },
                    )
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> Node {
        Node::new(1, "Input", NodeKind::Soundboard, [0.0; 2])
    }
    #[test]
    fn plugin_nodes_offer_appropriate_unbound_actions() {
        let mut instrument = Node::new(1, "Synth", NodeKind::VstInstrument, [0.0; 2]);
        let mut effect = Node::new(2, "VST FX", NodeKind::VstEffect, [0.0; 2]);
        assert!(NodeAction::SOURCE
            .iter()
            .all(|a| NodeAction::for_kind(&instrument.kind).contains(a)));
        for a in [
            NodeAction::PlayNote,
            NodeAction::ReleaseNote,
            NodeAction::AllNotesOff,
        ] {
            assert!(NodeAction::for_kind(&instrument.kind).contains(&a));
            assert!(a.apply(&mut effect).is_err());
        }
        NodeAction::Mute.apply(&mut instrument).unwrap();
        assert!(instrument.mute);
        NodeAction::Disable.apply(&mut effect).unwrap();
        assert!(effect.effects_disabled);
        NodeAction::ToggleDisabled.apply(&mut effect).unwrap();
        assert!(!effect.effects_disabled);
        assert!(instrument.keybinds.is_empty() && effect.keybinds.is_empty());
    }
    #[test]
    fn output_actions_reset_and_listen_state_are_persistent() {
        let mut n = Node::new(1, "Out", NodeKind::Output { device: None }, [0.0; 2]);
        assert!(n.keybinds.is_empty());
        assert!(!n.listen_enabled);
        assert!(n.listen_device.is_none());
        n.listen_device = Some("headphones::1".into());
        for (action, expected) in [
            (NodeAction::ListenOn, true),
            (NodeAction::ListenOn, true),
            (NodeAction::ListenOff, false),
            (NodeAction::ToggleListening, true),
            (NodeAction::ToggleListening, false),
        ] {
            action.apply(&mut n).unwrap();
            assert_eq!(n.listen_enabled, expected);
        }
        NodeAction::Mute.apply(&mut n).unwrap();
        assert!(n.mute);
        NodeAction::Unmute.apply(&mut n).unwrap();
        assert!(!n.mute);
        NodeAction::VolumeDown.apply(&mut n).unwrap();
        assert_eq!(n.gain_db, -1.0);
        NodeAction::VolumeUp.apply(&mut n).unwrap();
        assert_eq!(n.gain_db, 0.0);
        n.gain_db = -24.0;
        NodeAction::ResetVolume.apply(&mut n).unwrap();
        assert_eq!(n.gain_db, 0.0);
        n.keybinds
            .insert(NodeAction::ToggleListening, "Ctrl+Alt+L".into());
        let decoded: Node = serde_json::from_str(&serde_json::to_string(&n).unwrap()).unwrap();
        assert_eq!(decoded, n);
        let mut old = serde_json::to_value(&n).unwrap();
        old.as_object_mut().unwrap().remove("listen_enabled");
        old.as_object_mut().unwrap().remove("listen_device");
        let legacy: Node = serde_json::from_value(old).unwrap();
        assert!(!legacy.listen_enabled);
        assert!(legacy.listen_device.is_none());
        let mut input = source();
        input.gain_db = 12.0;
        NodeAction::ResetVolume.apply(&mut input).unwrap();
        assert_eq!(input.gain_db, 0.0);
    }
    #[test]
    fn all_source_actions_work_for_every_source_kind() {
        for kind in [
            NodeKind::Input {
                device: None,
                loopback: false,
            },
            NodeKind::Input {
                device: None,
                loopback: true,
            },
            NodeKind::Soundboard,
            NodeKind::ApplicationCapture { target: None },
        ] {
            let mut n = Node::new(1, "Source", kind, [0.0; 2]);
            for (action, expected) in [
                (NodeAction::Mute, true),
                (NodeAction::Unmute, false),
                (NodeAction::ToggleMute, true),
                (NodeAction::ToggleMute, false),
            ] {
                action.apply(&mut n).unwrap();
                assert_eq!(n.mute, expected);
            }
            for (action, expected) in [
                (NodeAction::SoloOn, true),
                (NodeAction::SoloOff, false),
                (NodeAction::ToggleSolo, true),
                (NodeAction::ToggleSolo, false),
            ] {
                action.apply(&mut n).unwrap();
                assert_eq!(n.solo, expected);
            }
            NodeAction::VolumeUp.apply(&mut n).unwrap();
            assert_eq!(n.gain_db, 1.0);
            NodeAction::VolumeDown.apply(&mut n).unwrap();
            assert_eq!(n.gain_db, 0.0);
            for (action, expected) in [
                (NodeAction::PanLeft100, -1.0),
                (NodeAction::PanLeft50, -0.5),
                (NodeAction::PanCenter, 0.0),
                (NodeAction::PanRight50, 0.5),
                (NodeAction::PanRight100, 1.0),
            ] {
                action.apply(&mut n).unwrap();
                assert_eq!(n.pan, expected);
            }
        }
    }
    #[test]
    fn volume_actions_respect_fader_limits() {
        let mut n = source();
        n.gain_db = 12.0;
        assert!(!NodeAction::VolumeUp.apply(&mut n).unwrap());
        n.gain_db = -60.0;
        assert!(!NodeAction::VolumeDown.apply(&mut n).unwrap());
    }
    #[test]
    fn processor_actions_and_type_safety() {
        let mut n = Node::new(2, "FX", NodeKind::Processor, [0.0; 2]);
        NodeAction::Disable.apply(&mut n).unwrap();
        assert!(n.effects_disabled);
        NodeAction::Enable.apply(&mut n).unwrap();
        assert!(!n.effects_disabled);
        NodeAction::ToggleDisabled.apply(&mut n).unwrap();
        assert!(n.effects_disabled);
        assert!(NodeAction::Mute.apply(&mut n).is_err());
        assert!(NodeAction::Disable.apply(&mut source()).is_err());
        let mut out = Node::new(3, "Out", NodeKind::Output { device: None }, [0.0; 2]);
        assert!(NodeAction::Mute.apply(&mut out).unwrap());
        assert!(NodeAction::SoloOn.apply(&mut out).is_err());
    }
    #[test]
    fn global_defaults_and_roundtrip() {
        let s = AppSettings::default();
        assert_eq!(s.keybinds[&AppAction::StopSounds], "Ctrl+Alt+Space");
        assert!(AppAction::ALL[1..].iter().all(|a| s.keybinds[a].is_empty()));
        let decoded: AppSettings =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(decoded, s);
    }
}
