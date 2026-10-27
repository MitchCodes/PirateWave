//! Stable project configuration; model/runtime implementations live outside DSP.
use serde::{Deserialize, Serialize};
pub const LLVC_PACK: &str = "llvc-f8312-v1";
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoiceBackend {
    #[default]
    Auto,
    Cpu,
    DirectMl,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    pub pack: String,
    pub backend: VoiceBackend,
    pub output_db: f32,
}
impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            pack: LLVC_PACK.into(),
            backend: VoiceBackend::Auto,
            output_db: 0.0,
        }
    }
}
impl VoiceConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.pack.is_empty()
            || self.pack.len() > 128
            || !self
                .pack
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || !self.output_db.is_finite()
            || !(-60.0..=12.0).contains(&self.output_db)
        {
            return Err("Invalid voice configuration".into());
        }
        Ok(())
    }
}

/// Control-plane selection for direct, mutually exclusive voice branches.
/// Match the graph's default switch destination (first connected output).
/// Unknown or non-switch routes remain eligible, retaining the one-runtime guard.
pub fn route_selected(project: &crate::project::Project, node: u64) -> bool {
    use crate::project::NodeKind;
    let mut incoming = project
        .connections
        .iter()
        .filter(|edge| edge.to == node)
        .peekable();
    if incoming.peek().is_none() {
        return true;
    }
    incoming.any(|edge| {
        let Some(source) = project.nodes.iter().find(|source| source.id == edge.from) else {
            return true;
        };
        if !matches!(source.kind, NodeKind::Switch) {
            return true;
        }
        source.switch_output.or_else(|| {
            project
                .connections
                .iter()
                .find(|edge| edge.from == source.id)
                .map(|edge| edge.to)
        }) == Some(node)
    })
}
