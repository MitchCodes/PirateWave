//! Device preferences, independent of the graph's fixed floating-point format.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioFormat {
    /// None follows the endpoint's shared-mode default.
    pub sample_rate: Option<u32>,
    pub sample_format: Option<SampleEncoding>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleEncoding {
    F32,
    F64,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
}
impl SampleEncoding {
    pub fn label(self) -> &'static str {
        match self {
            Self::F32 => "32-bit float",
            Self::F64 => "64-bit float",
            Self::I8 => "8-bit signed PCM",
            Self::I16 => "16-bit PCM",
            Self::I32 => "32-bit PCM",
            Self::I64 => "64-bit PCM",
            Self::U8 => "8-bit unsigned PCM",
            Self::U16 => "16-bit unsigned PCM",
            Self::U32 => "32-bit unsigned PCM",
            Self::U64 => "64-bit unsigned PCM",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_roundtrip() {
        assert_eq!(
            serde_json::from_str::<AudioFormat>("{}").unwrap(),
            AudioFormat::default()
        );
        let p = AudioFormat {
            sample_rate: Some(96000),
            sample_format: Some(SampleEncoding::I16),
        };
        assert_eq!(
            serde_json::from_str::<AudioFormat>(&serde_json::to_string(&p).unwrap()).unwrap(),
            p
        );
    }
}
