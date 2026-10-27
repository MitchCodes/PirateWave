//! Serializable plugin identities and settings. The native host owns plugin code.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
pub struct NodePlugin {
    pub descriptor: PluginDescriptor,
    #[serde(default)]
    pub parameters: BTreeMap<u32, f64>,
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
            note: default_note(),
            velocity: default_velocity(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        self.descriptor.validate()?;
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
        let decoded: NodePlugin =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(decoded, config);
        config.parameters.insert(42, f64::NAN);
        assert!(config.validate().is_err());
        config.parameters.insert(42, 0.5);
        config.parameters.insert(99, 0.5);
        assert!(config.validate().is_err());
    }
}
