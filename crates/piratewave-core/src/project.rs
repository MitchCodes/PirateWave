use crate::effects::EffectSpec;
use serde::{Deserialize, Serialize};

/// Persist application identity rather than a PID, which changes on relaunch.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApplicationTarget {
    pub executable: String,
    pub window_title: String,
    #[serde(default)]
    pub match_title: bool,
}
impl ApplicationTarget {
    pub fn validate(&self) -> Result<(), String> {
        if self.executable.trim().is_empty()
            || self.executable.len() > 4096
            || self.executable.contains('\0')
            || self.window_title.len() > 4096
            || self.window_title.contains('\0')
            || (self.match_title && self.window_title.trim().is_empty())
        {
            return Err("Invalid application capture target".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum NodeKind {
    Input {
        device: Option<String>,
        loopback: bool,
    },
    ApplicationCapture {
        target: Option<ApplicationTarget>,
    },
    Soundboard,
    VstInstrument,
    VstEffect,
    Processor,
    Switch,
    Recording,
    AutoDucking,
    VoiceChanger,
    Output {
        device: Option<String>,
    },
    Extension {
        definition: Box<crate::extensions::TypeDescriptor>,
    },
}
impl NodeKind {
    pub fn is_source(&self) -> bool {
        matches!(
            self,
            Self::Input { .. }
                | Self::ApplicationCapture { .. }
                | Self::Soundboard
                | Self::VstInstrument
        )
    }
    pub fn is_processor(&self) -> bool {
        matches!(
            self,
            Self::Processor | Self::VstEffect | Self::VoiceChanger | Self::Extension { .. }
        )
    }
    /// Rack bypass and per-effect shortcuts, including slot-only recording racks.
    pub fn has_effect_controls(&self) -> bool {
        self.is_processor() || matches!(self, Self::Recording)
    }
    pub fn is_plugin(&self) -> bool {
        matches!(self, Self::VstInstrument | Self::VstEffect)
    }
    pub fn label(&self) -> &str {
        match self {
            Self::Extension { definition } => &definition.name,
            Self::Input { loopback: true, .. } => "DESKTOP CAPTURE",
            Self::Input { .. } => "AUDIO INPUT",
            Self::ApplicationCapture { .. } => "APPLICATION CAPTURE",
            Self::Soundboard => "SOUNDBOARD",
            Self::Switch => "SWITCH",
            Self::Recording => "RECORDING",
            Self::AutoDucking => "AUTO DUCKING",
            Self::Processor => "PROCESSOR",
            Self::VoiceChanger => "VOICE CHANGER",
            Self::VstInstrument => "VST INSTRUMENT",
            Self::VstEffect => "VST EFFECT",
            Self::Output { .. } => "OUTPUT BUS",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Node {
    pub id: u64,
    pub name: String,
    pub kind: NodeKind,
    pub gain_db: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    /// Bypass a processor's entire chain, preserving individual effect settings.
    #[serde(default)]
    pub effects_disabled: bool,
    /// Mirror an output bus after its gain, mute and effects. None selects the default playback device.
    #[serde(default)]
    pub listen_enabled: bool,
    #[serde(default)]
    pub listen_device: Option<String>,
    #[serde(default)]
    pub keybinds: std::collections::BTreeMap<crate::shortcuts::NodeAction, String>,
    #[serde(default)]
    pub plugin: Option<crate::plugins::NodePlugin>,
    #[serde(default)]
    pub voice: Option<crate::voice::VoiceConfig>,
    #[serde(default)]
    pub switch_output: Option<u64>,
    #[serde(default)]
    pub switch_keybinds: std::collections::BTreeMap<u64, String>,
    #[serde(default)]
    pub recording: crate::recording::RecordingConfig,
    #[serde(default)]
    pub ducking: crate::ducking::DuckingConfig,
    #[serde(
        default,
        skip_serializing_if = "crate::extensions::NodeSettings::is_empty"
    )]
    pub extension: crate::extensions::NodeExtension,
    pub effects: Vec<EffectSpec>,
    pub position: [f32; 2],
}
impl Node {
    pub fn new(id: u64, name: &str, kind: NodeKind, position: [f32; 2]) -> Self {
        let voice = matches!(kind, NodeKind::VoiceChanger).then(crate::voice::VoiceConfig::default);
        let extension = if let NodeKind::Extension { definition } = &kind {
            crate::extensions::NodeExtension::new(crate::extensions::defaults(definition))
        } else {
            Default::default()
        };
        Self {
            id,
            name: name.into(),
            kind,
            gain_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            effects_disabled: false,
            listen_enabled: false,
            listen_device: None,
            keybinds: Default::default(),
            plugin: None,
            voice,
            extension,
            switch_output: None,
            switch_keybinds: Default::default(),
            recording: Default::default(),
            ducking: Default::default(),
            effects: vec![],
            position,
        }
    }
}
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum OutputPort {
    #[default]
    Passthrough,
    Preview,
    Send,
    Priority,
    Ducked,
    Extension(u32),
}
impl OutputPort {
    pub const ALL: [Self; 3] = [Self::Passthrough, Self::Preview, Self::Send];
    pub fn label(self) -> &'static str {
        match self {
            Self::Extension(_) => "Audio",
            Self::Passthrough => "Passthrough",
            Self::Preview => "Preview",
            Self::Send => "Send",
            Self::Priority => "Priority",
            Self::Ducked => "Ducked",
        }
    }
    pub fn y(self) -> f32 {
        match self {
            Self::Extension(_) => 64.,
            Self::Passthrough => 64.,
            Self::Preview => 92.,
            Self::Send => 120.,
            Self::Priority => 92.,
            Self::Ducked => 120.,
        }
    }
}
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum InputPort {
    #[default]
    Main,
    Priority,
    Ducked,
    Extension(u32),
}
impl InputPort {
    pub fn y(self, kind: &NodeKind) -> f32 {
        match self {
            Self::Extension(id) => {
                if let NodeKind::Extension { definition } = kind {
                    64. + definition
                        .inputs
                        .iter()
                        .position(|p| p.id == id)
                        .unwrap_or(0) as f32
                        * 28.
                } else {
                    64.
                }
            }
            Self::Priority => 64.,
            Self::Ducked => 100.,
            Self::Main => {
                if matches!(kind, NodeKind::Recording) {
                    92.
                } else {
                    47.
                }
            }
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Extension(_) => "Audio",
            Self::Main => "Input",
            Self::Priority => "Priority",
            Self::Ducked => "Ducked",
        }
    }
}
impl NodeKind {
    pub fn input_label(&self, port: InputPort) -> &str {
        if let (Self::Extension { definition }, InputPort::Extension(id)) = (self, port) {
            definition
                .inputs
                .iter()
                .find(|p| p.id == id)
                .map_or("Audio", |p| p.name.as_str())
        } else {
            port.label()
        }
    }
    pub fn outputs(&self) -> std::borrow::Cow<'_, [OutputPort]> {
        if let Self::Extension { definition } = self {
            return std::borrow::Cow::Owned(
                definition
                    .outputs
                    .iter()
                    .map(|p| OutputPort::Extension(p.id))
                    .collect(),
            );
        }
        std::borrow::Cow::Borrowed(match self {
            Self::Recording => &[
                OutputPort::Passthrough,
                OutputPort::Preview,
                OutputPort::Send,
            ],
            Self::AutoDucking => &[
                OutputPort::Passthrough,
                OutputPort::Priority,
                OutputPort::Ducked,
            ],
            Self::Output { .. } => &[],
            _ => &[OutputPort::Passthrough],
        })
    }
    pub fn inputs(&self) -> std::borrow::Cow<'_, [InputPort]> {
        if let Self::Extension { definition } = self {
            return std::borrow::Cow::Owned(
                definition
                    .inputs
                    .iter()
                    .map(|p| InputPort::Extension(p.id))
                    .collect(),
            );
        }
        std::borrow::Cow::Borrowed(if self.is_source() {
            &[]
        } else if matches!(self, Self::AutoDucking) {
            &[InputPort::Priority, InputPort::Ducked]
        } else {
            &[InputPort::Main]
        })
    }
    pub fn output_label(&self, port: OutputPort) -> &str {
        if let (Self::Extension { definition }, OutputPort::Extension(id)) = (self, port) {
            return definition
                .outputs
                .iter()
                .find(|p| p.id == id)
                .map_or("Audio", |p| p.name.as_str());
        }
        if matches!(self, Self::AutoDucking) && port == OutputPort::Passthrough {
            "Mixed"
        } else {
            port.label()
        }
    }
    pub fn output_y(&self, port: OutputPort) -> f32 {
        if let (Self::Extension { definition }, OutputPort::Extension(id)) = (self, port) {
            return 64.
                + definition
                    .outputs
                    .iter()
                    .position(|p| p.id == id)
                    .unwrap_or(0) as f32
                    * 28.;
        }
        if matches!(self, Self::Recording | Self::AutoDucking) {
            port.y()
        } else {
            47.
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Connection {
    #[serde(default)]
    pub input: InputPort,
    #[serde(default)]
    pub port: OutputPort,
    pub from: u64,
    pub to: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sound {
    pub id: u64,
    pub name: String,
    pub path: String,
    pub shortcut: String,
    pub volume: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub nodes: Vec<Node>,
    pub connections: Vec<Connection>,
    pub sounds: Vec<Sound>,
}
impl Default for Project {
    fn default() -> Self {
        let mic = Node::new(
            1,
            "Microphone",
            NodeKind::Input {
                device: None,
                loopback: false,
            },
            [35.0, 45.0],
        );
        let mut desktop = Node::new(
            2,
            "Desktop audio",
            NodeKind::Input {
                device: None,
                loopback: true,
            },
            [35.0, 190.0],
        );
        desktop.mute = true;
        let soundboard = Node::new(3, "Soundboard", NodeKind::Soundboard, [35.0, 335.0]);
        let mut monitor = Node::new(
            4,
            "Monitor",
            NodeKind::Output { device: None },
            [590.0, 100.0],
        );
        monitor.gain_db = -12.0;
        // A second bus is unassigned until an actual output device is selected.
        let stream = Node::new(
            5,
            "Stream mix",
            NodeKind::Output {
                device: Some(String::new()),
            },
            [590.0, 290.0],
        );
        Self {
            version: 1,
            name: "My studio".into(),
            nodes: vec![mic, desktop, soundboard, monitor, stream],
            connections: vec![
                Connection {
                    input: Default::default(),
                    port: Default::default(),
                    from: 1,
                    to: 5,
                },
                Connection {
                    input: Default::default(),
                    port: Default::default(),
                    from: 2,
                    to: 5,
                },
                Connection {
                    input: Default::default(),
                    port: Default::default(),
                    from: 3,
                    to: 4,
                },
                Connection {
                    input: Default::default(),
                    port: Default::default(),
                    from: 3,
                    to: 5,
                },
            ],
            sounds: vec![],
        }
    }
}
impl Project {
    pub fn next_id(&self) -> u64 {
        self.nodes
            .iter()
            .map(|n| n.id)
            .chain(self.sounds.iter().map(|s| s.id))
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }
    pub fn remove_node(&mut self, id: u64) {
        self.nodes.retain(|n| n.id != id);
        self.connections.retain(|c| c.from != id && c.to != id);
        self.clean_switches();
    }
    pub fn clean_switches(&mut self) {
        for node in &mut self.nodes {
            if !matches!(node.kind, NodeKind::Switch) {
                continue;
            }
            node.switch_keybinds.retain(|to, _| {
                self.connections
                    .iter()
                    .any(|c| c.from == node.id && c.to == *to)
            });
            if !node.switch_output.is_some_and(|to| {
                self.connections
                    .iter()
                    .any(|c| c.from == node.id && c.to == to)
            }) {
                node.switch_output = self
                    .connections
                    .iter()
                    .find(|c| c.from == node.id)
                    .map(|c| c.to);
            }
        }
    }
    pub fn disconnect(&mut self, from: u64, to: u64) {
        self.connections.retain(|c| c.from != from || c.to != to);
        self.clean_switches();
    }
    pub fn select_switch_output(&mut self, id: u64, output: u64) -> bool {
        if !self
            .connections
            .iter()
            .any(|c| c.from == id && c.to == output)
        {
            return false;
        }
        if let Some(node) = self
            .nodes
            .iter_mut()
            .find(|n| n.id == id && matches!(n.kind, NodeKind::Switch))
        {
            node.switch_output = Some(output);
            true
        } else {
            false
        }
    }
    pub fn connect(&mut self, from: u64, to: u64) -> Result<(), String> {
        self.connect_port(from, to, OutputPort::Passthrough)
    }
    pub fn connect_port(&mut self, from: u64, to: u64, port: OutputPort) -> Result<(), String> {
        self.connect_ports(from, to, port, InputPort::Main)
    }
    pub fn disconnect_ports(&mut self, from: u64, to: u64, port: OutputPort, input: InputPort) {
        self.connections
            .retain(|c| !(c.from == from && c.to == to && c.port == port && c.input == input));
        self.clean_switches();
    }
    pub fn connect_ports(
        &mut self,
        from: u64,
        to: u64,
        port: OutputPort,
        input: InputPort,
    ) -> Result<(), String> {
        if self
            .connections
            .iter()
            .any(|c| c.from == from && c.to == to && c.port == port && c.input == input)
        {
            return Err("These ports are already connected".into());
        }
        self.connections.push(Connection {
            from,
            to,
            port,
            input,
        });
        if let Err(error) = crate::graph::validate(self) {
            self.connections.pop();
            return Err(error);
        }
        self.clean_switches();
        Ok(())
    }
}
