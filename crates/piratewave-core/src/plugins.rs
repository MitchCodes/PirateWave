//! Serializable plugin identities and settings. The native host owns plugin code.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginFormat {
    Vst2,
    Vst3,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginRole {
    Instrument,
    Effect,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginParameter {
    pub id: u32,
    pub name: String,
    pub unit: String,
    pub default: f64,
    pub steps: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginDescriptor {
    pub path: String,
    /// VST3 component CID as raw ABI bytes, or the VST2 unique ID in decimal.
    pub class_id: String,
    pub format: PluginFormat,
    pub role: PluginRole,
    pub name: String,
    pub vendor: String,
    pub parameters: Vec<PluginParameter>,
}
impl PluginDescriptor {
    pub fn same_plugin(&self, other: &Self) -> bool {
        self.path == other.path && self.class_id == other.class_id && self.format == other.format
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.path.is_empty()
            || self.path.len() > 32768
            || self.class_id.is_empty()
            || self.parameters.len() > 4096
        {
            return Err("Invalid plugin identity or too many parameters (maximum 4096)".into());
        }
        let mut ids = std::collections::HashSet::new();
        for p in &self.parameters {
            if !ids.insert(p.id) || !p.default.is_finite() || !(0.0..=1.0).contains(&p.default) {
                return Err("Invalid plugin parameter metadata".into());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginState {
    /// VST3 component state or VST2 bank/program chunk.
    #[serde(with = "state_bytes")]
    pub component: Arc<Vec<u8>>,
    /// VST3 controller state, when supplied separately.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "state_bytes_option"
    )]
    pub controller: Option<Arc<Vec<u8>>>,
    /// Whether the VST2 chunk is a bank rather than a single program.
    #[serde(default, skip_serializing_if = "is_false")]
    pub vst2_bank: bool,
    /// Selected VST2 program when a chunk was captured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vst2_program: Option<u32>,
}
fn is_false(value: &bool) -> bool {
    !value
}

mod state_bytes {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use serde::{de::Error, Deserialize, Deserializer, Serializer};
    use std::sync::Arc;
    pub fn serialize<S: Serializer>(
        bytes: &Arc<Vec<u8>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes.as_slice()))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Arc<Vec<u8>>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Encoded {
            Base64(String),
            Legacy(Vec<u8>),
        }
        match Encoded::deserialize(deserializer)? {
            Encoded::Base64(text) => STANDARD
                .decode(text)
                .map(Arc::new)
                .map_err(D::Error::custom),
            Encoded::Legacy(bytes) => Ok(Arc::new(bytes)),
        }
    }
}
mod state_bytes_option {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::sync::Arc;
    pub fn serialize<S: Serializer>(
        bytes: &Option<Arc<Vec<u8>>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match bytes {
            Some(bytes) => super::state_bytes::serialize(bytes, serializer),
            None => serializer.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Arc<Vec<u8>>>, D::Error> {
        #[derive(Deserialize)]
        struct Bytes(#[serde(with = "super::state_bytes")] Arc<Vec<u8>>);
        Ok(Option::<Bytes>::deserialize(deserializer)?.map(|value| value.0))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodePlugin {
    pub descriptor: PluginDescriptor,
    #[serde(default)]
    pub parameters: BTreeMap<u32, f64>,
    /// Optional opaque state captured from the live plugin instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<PluginState>,
    #[serde(default = "default_note")]
    pub note: u8,
    #[serde(default = "default_velocity")]
    pub velocity: f32,
}
fn default_note() -> u8 {
    60
}
fn default_velocity() -> f32 {
    0.8
}
impl NodePlugin {
    pub fn new(descriptor: PluginDescriptor) -> Self {
        Self {
            descriptor,
            parameters: BTreeMap::new(),
            state: None,
            note: default_note(),
            velocity: default_velocity(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        self.descriptor.validate()?;
        if let Some(state) = &self.state {
            const MAX_STATE_BYTES: usize = 64 * 1024 * 1024;
            if state.component.len() > MAX_STATE_BYTES
                || state
                    .controller
                    .as_ref()
                    .is_some_and(|v| v.len() > MAX_STATE_BYTES)
            {
                return Err("Plugin state exceeds 64 MiB per component".into());
            }
            if state.vst2_program.is_some_and(|program| {
                self.descriptor.format != PluginFormat::Vst2 || program > i32::MAX as u32
            }) {
                return Err("Invalid VST2 bank program".into());
            }
        }
        if self.note > 127 || !self.velocity.is_finite() || !(0.0..=1.0).contains(&self.velocity) {
            return Err("Invalid instrument note or velocity".into());
        }
        for (id, value) in &self.parameters {
            if !value.is_finite()
                || !(0.0..=1.0).contains(value)
                || !self.descriptor.parameters.iter().any(|p| p.id == *id)
            {
                return Err("Invalid plugin parameter value".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_and_roundtrips_plugin_configuration() {
        let mut config = NodePlugin::new(PluginDescriptor {
            path: "test.vst3".into(),
            class_id: "1234".into(),
            format: PluginFormat::Vst3,
            role: PluginRole::Instrument,
            name: "Test".into(),
            vendor: "PirateWave".into(),
            parameters: vec![PluginParameter {
                id: 42,
                name: "Gain".into(),
                unit: String::new(),
                default: 0.5,
                steps: 0,
            }],
        });
        config.parameters.insert(42, 0.25);
        config.validate().unwrap();
        let serialized = serde_json::to_string(&config).unwrap();
        assert!(!serialized.contains("\"state\""));
        let decoded: NodePlugin = serde_json::from_str(&serialized).unwrap();
        assert_eq!(decoded, config);
        assert!(decoded.state.is_none());
        config.state = Some(PluginState {
            component: Arc::new(vec![0, 1, 255]),
            controller: Some(Arc::new(vec![2, 3])),
            vst2_bank: false,
            vst2_program: None,
        });
        let decoded: NodePlugin =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(decoded, config);
        let encoded = serde_json::to_value(&config).unwrap();
        assert_eq!(encoded["state"]["component"], "AAH/");
        let mut legacy = encoded;
        legacy["state"]["component"] = serde_json::json!([0, 1, 255]);
        legacy["state"]["controller"] = serde_json::json!([2, 3]);
        assert_eq!(
            serde_json::from_value::<NodePlugin>(legacy).unwrap(),
            config
        );
        config.descriptor.format = PluginFormat::Vst2;
        let state = config.state.as_mut().unwrap();
        state.controller = None;
        state.vst2_bank = true;
        state.vst2_program = Some(2);
        config.validate().unwrap();
        let saved = serde_json::to_value(&config).unwrap();
        assert_eq!(saved["state"]["vst2_program"], 2);
        assert_eq!(
            serde_json::from_value::<NodePlugin>(saved.clone()).unwrap(),
            config
        );
        let mut old = saved;
        old["state"].as_object_mut().unwrap().remove("vst2_program");
        assert_eq!(
            serde_json::from_value::<NodePlugin>(old)
                .unwrap()
                .state
                .unwrap()
                .vst2_program,
            None
        );
        config.state.as_mut().unwrap().vst2_program = Some(u32::MAX);
        assert!(config.validate().is_err());
        config.state.as_mut().unwrap().vst2_program = None;
        config.parameters.insert(42, f64::NAN);
        assert!(config.validate().is_err());
        config.parameters.insert(42, 0.5);
        config.parameters.insert(99, 0.5);
        assert!(config.validate().is_err());
    }
}
