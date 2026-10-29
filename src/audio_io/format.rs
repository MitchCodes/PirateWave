//! Shared-mode negotiation. Capability discovery is never called by a callback.
use super::*;
pub use piratewave_core::audio_format::{AudioFormat, SampleEncoding};

pub fn encoding(format: SampleFormat) -> Option<SampleEncoding> {
    Some(match format {
        SampleFormat::F32 => SampleEncoding::F32,
        SampleFormat::F64 => SampleEncoding::F64,
        SampleFormat::I8 => SampleEncoding::I8,
        SampleFormat::I16 => SampleEncoding::I16,
        SampleFormat::I32 => SampleEncoding::I32,
        SampleFormat::I64 => SampleEncoding::I64,
        SampleFormat::U8 => SampleEncoding::U8,
        SampleFormat::U16 => SampleEncoding::U16,
        SampleFormat::U32 => SampleEncoding::U32,
        SampleFormat::U64 => SampleEncoding::U64,
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatChoice {
    pub rate: u32,
    pub encoding: SampleEncoding,
    pub channels: u16,
}
impl FormatChoice {
    fn from_config(c: &cpal::SupportedStreamConfig) -> Option<Self> {
        Some(Self {
            rate: c.sample_rate().0,
            encoding: encoding(c.sample_format())?,
            channels: c.channels(),
        })
    }
}
#[derive(Clone, Debug)]
pub struct Capabilities {
    pub default: FormatChoice,
    pub choices: Vec<FormatChoice>,
}
fn configs(
    device: &cpal::Device,
    output: bool,
) -> Result<(
    cpal::SupportedStreamConfig,
    Vec<cpal::SupportedStreamConfigRange>,
)> {
    let default = if output {
        device.default_output_config()?
    } else {
        device.default_input_config()?
    };
    let ranges = if output {
        device.supported_output_configs()?.collect()
    } else {
        device.supported_input_configs()?.collect()
    };
    Ok((default, ranges))
}
pub fn capabilities(id: &Option<String>, output: bool) -> Result<Capabilities> {
    let device = find_device(id, output)?;
    let (default, ranges) = configs(&device, output)?;
    let default = FormatChoice::from_config(&default)
        .ok_or_else(|| anyhow!("Unsupported default sample encoding"))?;
    let mut choices = vec![default.clone()];
    for range in ranges {
        if range.channels() != default.channels {
            continue;
        }
        let Some(encoding) = encoding(range.sample_format()) else {
            continue;
        };
        for rate in [
            8000,
            16000,
            22050,
            24000,
            32000,
            44100,
            48000,
            88200,
            96000,
            176400,
            192000,
            default.rate,
        ] {
            if (range.min_sample_rate().0..=range.max_sample_rate().0).contains(&rate) {
                let choice = FormatChoice {
                    rate,
                    encoding,
                    channels: range.channels(),
                };
                if !choices.contains(&choice) {
                    choices.push(choice);
                }
            }
        }
    }
    Ok(Capabilities { default, choices })
}

pub fn negotiate(
    device: &cpal::Device,
    output: bool,
    preference: &AudioFormat,
) -> Result<(cpal::SupportedStreamConfig, bool)> {
    let default = if output {
        device.default_output_config()?
    } else {
        device.default_input_config()?
    };
    if preference == &AudioFormat::default() {
        return Ok((default, false));
    }
    let rate = preference.sample_rate.unwrap_or(default.sample_rate().0);
    let wanted = preference
        .sample_format
        .or_else(|| encoding(default.sample_format()));
    if rate == default.sample_rate().0 && wanted == encoding(default.sample_format()) {
        return Ok((default, false));
    }
    let ranges = if output {
        device
            .supported_output_configs()
            .map(|r| r.collect::<Vec<_>>())
    } else {
        device
            .supported_input_configs()
            .map(|r| r.collect::<Vec<_>>())
    };
    Ok(select(default, ranges.unwrap_or_default(), preference))
}

fn select(
    default: cpal::SupportedStreamConfig,
    mut ranges: Vec<cpal::SupportedStreamConfigRange>,
    preference: &AudioFormat,
) -> (cpal::SupportedStreamConfig, bool) {
    let rate = preference.sample_rate.unwrap_or(default.sample_rate().0);
    let wanted = preference
        .sample_format
        .or_else(|| encoding(default.sample_format()));
    if rate == default.sample_rate().0 && wanted == encoding(default.sample_format()) {
        return (default, false);
    }
    ranges.sort_by_key(|r| encoding(r.sample_format()) != wanted);
    for range in ranges {
        if range.channels() == default.channels()
            && encoding(range.sample_format()).is_some()
            && (preference.sample_format.is_none() || encoding(range.sample_format()) == wanted)
            && (range.min_sample_rate().0..=range.max_sample_rate().0).contains(&rate)
        {
            return (range.with_sample_rate(cpal::SampleRate(rate)), false);
        }
    }
    (default, true)
}

pub fn description(config: &cpal::SupportedStreamConfig, fallback: bool) -> String {
    let label = encoding(config.sample_format()).map_or("Unknown", SampleEncoding::label);
    format!(
        "{} Hz · {} · {} channels{}; engine 48000 Hz{}",
        config.sample_rate().0,
        label,
        config.channels(),
        if fallback {
            " · requested format unavailable; using device default"
        } else {
            ""
        },
        if config.sample_rate().0 == SAMPLE_RATE {
            " · clock synchronization only"
        } else {
            " · rate conversion"
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn range(rate: u32, format: SampleFormat) -> cpal::SupportedStreamConfigRange {
        cpal::SupportedStreamConfigRange::new(
            2,
            cpal::SampleRate(rate),
            cpal::SampleRate(rate),
            cpal::SupportedBufferSize::Unknown,
            format,
        )
    }
    #[test]
    #[ignore = "opens silent shared-mode output streams; never changes Windows settings"]
    fn native_shared_output_formats_and_owner_shutdown() {
        let caps = capabilities(&None, true).unwrap();
        let mut choices = caps.choices.clone();
        choices.truncate(8);
        for choice in choices {
            let counters = Arc::new(Counters::default());
            let errors = Arc::new(ArrayQueue::new(4));
            let preference = AudioFormat {
                sample_rate: Some(choice.rate),
                sample_format: Some(choice.encoding),
            };
            let mut stream =
                open_output_with_format(1, &None, &preference, counters.clone(), errors.clone())
                    .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(200));
            assert!(counters.rendered.load(Ordering::Relaxed) > 0);
            assert!(stream.health.failure().is_none());
            assert_eq!(stream.requested_format, preference);
            eprintln!("{}", stream.format_status);
            drop(stream);
        }
    }
    #[test]
    fn negotiation_auto_exact_and_unavailable() {
        let default = range(48000, SampleFormat::F32).with_sample_rate(cpal::SampleRate(48000));
        let supported = vec![
            range(44100, SampleFormat::I16),
            range(96000, SampleFormat::F32),
        ];
        let (active, fallback) =
            select(default.clone(), supported.clone(), &AudioFormat::default());
        assert!(!fallback);
        assert_eq!(active.sample_rate().0, 48000);
        let (active, fallback) = select(
            default.clone(),
            supported.clone(),
            &AudioFormat {
                sample_rate: Some(44100),
                sample_format: None,
            },
        );
        assert!(!fallback);
        assert_eq!(active.sample_format(), SampleFormat::I16);
        let preference = AudioFormat {
            sample_rate: Some(96000),
            sample_format: Some(SampleEncoding::I16),
        };
        let (active, fallback) = select(default, supported, &preference);
        assert!(fallback);
        assert_eq!(active.sample_rate().0, 48000);
        assert_eq!(preference.sample_rate, Some(96000));
    }
}
