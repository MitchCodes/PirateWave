//! Recording file work is bounded and runs off the render/UI threads.
use crate::assets::{self, Clip};
#[cfg(windows)]
use anyhow::Context;
use piratewave_core::{recording::Capture, Frame, SAMPLE_RATE};
use std::{
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    thread,
};
#[derive(Clone)]
pub struct Audio {
    pub clip: Clip,
    pub waveform: Vec<[f32; 2]>,
    pub asset: String,
}
impl Audio {
    pub fn from_frames(frames: Vec<Frame>, asset: String) -> Self {
        let waveform = waveform(&frames, 512);
        Self {
            clip: Clip {
                frames: Arc::new(frames),
            },
            waveform,
            asset,
        }
    }
}
pub enum Job {
    Capture {
        plan: Capture,
        node: u64,
        slot: u64,
        token: u64,
    },
    Load {
        node: u64,
        slot: u64,
        token: u64,
        asset: String,
    },
    Export {
        effects: Vec<piratewave_core::effects::EffectSpec>,
        bypass: bool,
        audio: Clip,
        start: f32,
        end: f32,
        path: PathBuf,
    },
    Delete(String),
}
pub enum Event {
    Audio {
        node: u64,
        slot: u64,
        token: u64,
        audio: Audio,
        captured: bool,
    },
    Failed {
        node: u64,
        slot: u64,
        token: u64,
        error: String,
    },
    Saved(PathBuf),
    ExportFailed(String),
}
pub struct Service {
    pub tx: mpsc::SyncSender<Job>,
    pub events: mpsc::SyncSender<Event>,
    pub rx: mpsc::Receiver<Event>,
}
impl Default for Service {
    fn default() -> Self {
        let (tx, jobs) = mpsc::sync_channel::<Job>(8);
        let (events, rx) = mpsc::sync_channel(64);
        let jobs = Arc::new(Mutex::new(jobs));
        for worker in 0..3 {
            let replies = events.clone();
            let jobs = jobs.clone();
            thread::Builder::new()
                .name(format!("piratewave-recording-files-{worker}"))
                .spawn(move || loop {
                    let Ok(job) = jobs.lock().unwrap().recv() else {
                        break;
                    };
                    match job {
                        Job::Capture {
                            plan,
                            node,
                            slot,
                            token,
                        } => {
                            let result = (|| -> anyhow::Result<Audio> {
                                let frames = plan.materialize().map_err(anyhow::Error::msg)?;
                                let dir = directory();
                                std::fs::create_dir_all(&dir)?;
                                let path = dir.join(format!("recording-{token}-{node}-{slot}.wav"));
                                write_wav(&path, &frames)?;
                                Ok(Audio::from_frames(frames, path.to_string_lossy().into()))
                            })();
                            emit_audio(&replies, node, slot, token, result, true);
                        }
                        Job::Load {
                            node,
                            slot,
                            token,
                            asset,
                        } => {
                            let result =
                                assets::load_with_limit(Path::new(&asset), 600).map(|clip| Audio {
                                    waveform: waveform(&clip.frames, 512),
                                    clip,
                                    asset,
                                });
                            emit_audio(&replies, node, slot, token, result, false);
                        }
                        Job::Export {
                            effects,
                            bypass,
                            audio,
                            start,
                            end,
                            path,
                        } => {
                            let result = (|| -> anyhow::Result<()> {
                                let frames = cropped(&audio.frames, start, end)?;
                                if bypass || effects.iter().all(|e| !e.enabled) {
                                    export(&path, frames)
                                } else {
                                    let processed = render_effects(frames, &effects)?;
                                    export(&path, &processed)
                                }
                            })();
                            let event = match result {
                                Ok(()) => Event::Saved(path),
                                Err(error) => {
                                    crate::logging::error(format_args!(
                                        "Recording export failed: {error:#}"
                                    ));
                                    Event::ExportFailed(format!("{error:#}"))
                                }
                            };
                            if replies.send(event).is_err() {
                                break;
                            }
                        }
                        Job::Delete(asset) => {
                            if let Err(error) = delete_managed(&asset) {
                                crate::logging::warn(format_args!("Recording cleanup: {error:#}"));
                            }
                        }
                    }
                })
                .expect("Recording file worker");
        }
        Self { tx, events, rx }
    }
}
/// Same fresh history and zero-padded final block as a slot voice; only the
/// selected samples are saved, so preview/send/export have identical duration.
fn render_effects(
    frames: &[Frame],
    specs: &[piratewave_core::effects::EffectSpec],
) -> anyhow::Result<Vec<Frame>> {
    let mut effects = piratewave_core::recording_effects::RecordingEffects::new(
        specs,
        false,
        &piratewave_core::effects::EffectRegistry::default(),
    )
    .map_err(anyhow::Error::msg)?;
    let mut result = Vec::with_capacity(frames.len());
    for input in frames.chunks(piratewave_core::BLOCK_SIZE) {
        let mut block = [[0.; 2]; piratewave_core::BLOCK_SIZE];
        block[..input.len()].copy_from_slice(input);
        effects.process(&mut block);
        anyhow::ensure!(!effects.faulted(), "Recording effect failed during export");
        result.extend_from_slice(&block[..input.len()]);
    }
    Ok(result)
}
fn emit_audio(
    replies: &mpsc::SyncSender<Event>,
    node: u64,
    slot: u64,
    token: u64,
    result: anyhow::Result<Audio>,
    captured: bool,
) {
    let event = match result {
        Ok(audio) => Event::Audio {
            node,
            slot,
            token,
            audio,
            captured,
        },
        Err(error) => {
            crate::logging::error(format_args!("Recording {node}/{slot}: {error:#}"));
            Event::Failed {
                node,
                slot,
                token,
                error: format!("{error:#}"),
            }
        }
    };
    let _ = replies.send(event);
}
pub fn directory() -> PathBuf {
    crate::storage::default_path().with_file_name("recordings")
}
pub fn delete_managed(asset: &str) -> anyhow::Result<()> {
    let path = Path::new(asset);
    if !path.exists() {
        return Ok(());
    }
    let root = directory().canonicalize()?;
    let actual = path.canonicalize()?;
    if actual.parent() == Some(root.as_path())
        && actual
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("recording-"))
    {
        std::fs::remove_file(actual)?;
    }
    Ok(())
}
pub fn waveform(frames: &[Frame], bins: usize) -> Vec<[f32; 2]> {
    if frames.is_empty() {
        return vec![];
    }
    frames
        .chunks(frames.len().div_ceil(bins))
        .map(|block| {
            let mut range = [0_f32; 2];
            for f in block {
                for &v in f {
                    range[0] = range[0].min(v);
                    range[1] = range[1].max(v);
                }
            }
            range
        })
        .collect()
}
pub fn cropped(frames: &[Frame], start: f32, end: f32) -> anyhow::Result<&[Frame]> {
    anyhow::ensure!(
        start.is_finite() && end.is_finite() && start >= 0.,
        "Invalid recording crop"
    );
    let a = (start * SAMPLE_RATE as f32).round() as usize;
    let b = if end <= 0. {
        frames.len()
    } else {
        ((end * SAMPLE_RATE as f32).round() as usize).min(frames.len())
    };
    anyhow::ensure!(a < b && a < frames.len(), "Crop must contain audio");
    Ok(&frames[a..b])
}
pub fn write_wav(path: &Path, frames: &[Frame]) -> anyhow::Result<()> {
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for frame in frames {
        for &sample in frame {
            writer.write_sample(if sample.is_finite() { sample } else { 0. })?;
        }
    }
    writer.finalize()?;
    Ok(())
}
pub fn export(path: &Path, frames: &[Frame]) -> anyhow::Result<()> {
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    anyhow::ensure!(
        extension == "wav" || extension == "mp3",
        "Choose a .wav or .mp3 filename"
    );
    // Encode into a sibling temporary file, then publish only a complete export.
    let token = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temp = path.with_file_name(format!(".piratewave-export-{token}.{extension}"));
    let result = if extension == "wav" {
        write_wav(&temp, frames)
    } else {
        write_mp3(&temp, frames)
    };
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}
#[cfg(not(windows))]
fn write_mp3(_: &Path, _: &[Frame]) -> anyhow::Result<()> {
    anyhow::bail!("Native MP3 export requires Windows Media Foundation")
}
#[cfg(windows)]
fn write_mp3(path: &Path, frames: &[Frame]) -> anyhow::Result<()> {
    use windows::{
        core::{Interface, PCWSTR},
        Win32::{
            Media::MediaFoundation::*,
            System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
        },
    };
    struct Platform;
    impl Drop for Platform {
        fn drop(&mut self) {
            unsafe {
                let _ = MFShutdown();
                CoUninitialize();
            }
        }
    }
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        if let Err(error) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
            CoUninitialize();
            return Err(error.into());
        }
        let _platform = Platform;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let writer = MFCreateSinkWriterFromURL(PCWSTR(wide.as_ptr()), None, None)?;
        let types = MFTranscodeGetAudioOutputAvailableTypes(
            &MFAudioFormat_MP3,
            MFT_ENUM_FLAG_ALL.0 as u32,
            None,
        )
        .context("Find native MP3 encoder formats")?;
        let mut chosen = None;
        for index in 0..types.GetElementCount()? {
            let media = types.GetElement(index)?.cast::<IMFMediaType>()?;
            if media.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(0) == 2
                && media
                    .GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)
                    .unwrap_or(0)
                    == SAMPLE_RATE
            {
                let bytes = media
                    .GetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND)
                    .unwrap_or(0);
                if bytes == 32000 {
                    chosen = Some(media);
                    break;
                }
                if chosen.is_none() {
                    chosen = Some(media);
                }
            }
        }
        let output = chosen
            .ok_or_else(|| anyhow::anyhow!("Windows MP3 encoder has no stereo 48 kHz format"))?;
        let stream = writer
            .AddStream(&output)
            .context("Configure MP3 output stream")?;
        let input = MFCreateMediaType()?;
        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        input.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        input.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
        input.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, SAMPLE_RATE)?;
        input.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        input.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)?;
        input.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, SAMPLE_RATE * 4)?;
        writer
            .SetInputMediaType(stream, &input, None)
            .context("Configure PCM input to MP3 encoder")?;
        writer.BeginWriting()?;
        let mut position = 0_u64;
        for block in frames.chunks(4096) {
            let length = (block.len() * 4) as u32;
            let buffer = MFCreateMemoryBuffer(length)?;
            let mut raw = std::ptr::null_mut();
            buffer.Lock(&mut raw, None, None)?;
            let bytes = std::slice::from_raw_parts_mut(raw, length as usize);
            for (destination, sample) in bytes
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .zip(block.iter().flatten())
            {
                let value = (sample.clamp(-1., 1.) * 32767.).round() as i16;
                destination.copy_from_slice(&value.to_le_bytes());
            }
            buffer.Unlock()?;
            buffer.SetCurrentLength(length)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            let start = position * 10_000_000 / SAMPLE_RATE as u64;
            position += block.len() as u64;
            sample.SetSampleTime(start as i64)?;
            sample
                .SetSampleDuration((position * 10_000_000 / SAMPLE_RATE as u64 - start) as i64)?;
            writer.WriteSample(stream, &sample)?;
        }
        writer.Finalize()?;
    }
    Ok(())
}
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slot_export_jobs_apply_effect_snapshot_and_bypass_to_cropped_wav_and_mp3() {
        use piratewave_core::effects::{EffectRegistry, EffectSpec};
        let registry = EffectRegistry::default();
        let service = Service::default();
        let dir =
            std::env::temp_dir().join(format!("piratewave-slot-effects-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let frames: Vec<_> = (0..48000)
            .map(|i| {
                let v = (std::f32::consts::TAU * 440. * i as f32 / 48000.).sin() * 0.2;
                [v, -v]
            })
            .collect();
        let clip = Clip {
            frames: Arc::new(frames.clone()),
        };
        for (label, bypass, enabled, scale) in [
            ("wet", false, true, 0.5),
            ("rack-bypass", true, true, 1.),
            ("effect-bypass", false, false, 1.),
        ] {
            for extension in if cfg!(windows) {
                vec!["wav", "mp3"]
            } else {
                vec!["wav"]
            } {
                let mut gain = EffectSpec::new(registry.get("gain").unwrap());
                gain.parameters.insert("gain".into(), -6.0206);
                gain.enabled = enabled;
                let path = dir.join(format!("{label}.{extension}"));
                service
                    .tx
                    .send(Job::Export {
                        audio: clip.clone(),
                        start: 0.2,
                        end: 0.7,
                        path: path.clone(),
                        effects: vec![gain],
                        bypass,
                    })
                    .unwrap();
                match service
                    .rx
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .unwrap()
                {
                    Event::Saved(saved) => assert_eq!(saved, path),
                    Event::ExportFailed(e) => panic!("{e}"),
                    _ => panic!("Unexpected worker event"),
                }
                let saved = assets::load_with_limit(&path, 5).unwrap();
                assert!((saved.duration() - 0.5).abs() < 0.08);
                let rms = (saved.frames.iter().map(|f| f[0] * f[0]).sum::<f32>()
                    / saved.frames.len() as f32)
                    .sqrt();
                assert!(
                    (rms - 0.2 / std::f32::consts::SQRT_2 * scale).abs() < 0.012,
                    "{label}.{extension}: {rms}"
                );
                std::fs::remove_file(path).unwrap();
            }
        }
        assert_eq!(*clip.frames, frames);
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn offline_delay_uses_fresh_history_and_preserves_exact_partial_block_crop() {
        use piratewave_core::effects::{EffectRegistry, EffectSpec};
        let registry = EffectRegistry::default();
        let mut delay = EffectSpec::new(registry.get("delay").unwrap());
        delay.parameters.insert("time".into(), 1.);
        delay.parameters.insert("mix".into(), 1.);
        let mut input = vec![[0.; 2]; 131];
        input[1] = [1.; 2];
        let selected = &input[1..130];
        let output = render_effects(selected, &[delay.clone()]).unwrap();
        assert_eq!(output.len(), 129);
        assert_eq!(output[0], [0.; 2]);
        assert!(output.iter().any(|f| f[0] > 0.1));
        assert_eq!(output, render_effects(selected, &[delay]).unwrap());
        assert_eq!(input[1], [1.; 2]);
    }
    #[test]
    fn wav_and_mp3_exports_apply_crop_and_decode_back() {
        let dir =
            std::env::temp_dir().join(format!("piratewave-recording-codec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let frames: Vec<_> = (0..48000)
            .map(|i| {
                let value = (std::f32::consts::TAU * 440. * i as f32 / 48000.).sin() * 0.2;
                [value, -value]
            })
            .collect();
        let selection = cropped(&frames, 0.2, 0.7).unwrap();
        assert_eq!(selection.len(), 24000);
        for extension in if cfg!(windows) {
            vec!["wav", "mp3"]
        } else {
            vec!["wav"]
        } {
            let path = dir.join(format!("cropped-{extension}-\u{97f3}.{extension}"));
            export(&path, selection).unwrap_or_else(|e| panic!("{extension}: {e:#}"));
            let decoded = assets::load_with_limit(&path, 5).unwrap();
            assert!(
                (decoded.duration() - 0.5).abs() < 0.08,
                "{extension}: {}",
                decoded.duration()
            );
            assert!(decoded.frames.iter().any(|f| f[0].abs() > 0.05));
            export(&path, selection).unwrap();
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn waveform_and_crop_reject_empty_invalid_ranges() {
        let frames = vec![[-0.4, 0.8]; 100];
        let bins = waveform(&frames, 10);
        assert_eq!(bins.len(), 10);
        assert!(bins.iter().all(|b| *b == [-0.4, 0.8]));
        assert!(cropped(&frames, f32::NAN, 1.).is_err());
        assert!(cropped(&frames, 10., 11.).is_err());
        assert!(cropped(&frames, 0.001, 0.0001).is_err());
    }
}
