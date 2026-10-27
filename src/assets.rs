use anyhow::{anyhow, Context, Result};
use piratewave_core::{Frame, SAMPLE_RATE};
use std::{fs::File, path::Path, sync::Arc};
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error, formats::FormatOptions,
    io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

#[derive(Clone)]
pub struct Clip {
    pub frames: Arc<Vec<Frame>>,
}
impl Clip {
    pub fn duration(&self) -> f32 {
        self.frames.len() as f32 / SAMPLE_RATE as f32
    }
}
/// Decode on a loader thread, never inside an audio callback. Assets are bounded to
/// two minutes and stereo-mapped to prevent unbounded memory use from large files.
pub fn load(path: &Path) -> Result<Clip> {
    load_with_limit(path, 120)
}
pub fn load_with_limit(path: &Path, seconds: usize) -> Result<Clip> {
    let file = File::open(path).with_context(|| format!("Opening {}", path.display()))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }
    let probe = symphonia::default::get_probe().format(
        &hint,
        stream,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probe.format;
    let track = format
        .default_track()
        .ok_or_else(|| anyhow!("No audio track in this file"))?;
    let id = track.id;
    let rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| anyhow!("Unknown sample rate"))?;
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let mut frames = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = decoder.decode(&packet)?;
        let channels = decoded.spec().channels.count();
        if channels == 0 {
            return Err(anyhow!("Audio has no channels"));
        }
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        buffer.copy_interleaved_ref(decoded);
        for f in buffer.samples().chunks_exact(channels) {
            let left = f[0];
            let right = if channels > 1 { f[1] } else { left };
            if !left.is_finite() || !right.is_finite() {
                return Err(anyhow!("File contains non-finite samples"));
            }
            frames.push([left, right]);
        }
        if frames.len() > rate as usize * seconds {
            return Err(anyhow!("Audio file exceeds the {seconds}-second limit"));
        }
    }
    if frames.is_empty() {
        return Err(anyhow!("Audio file is empty"));
    }
    if rate == SAMPLE_RATE {
        return Ok(Clip {
            frames: Arc::new(frames),
        });
    }
    let output_len = (frames.len() as u64 * SAMPLE_RATE as u64 / rate as u64) as usize;
    let mut converted = Vec::with_capacity(output_len);
    for i in 0..output_len {
        let position = i as f64 * rate as f64 / SAMPLE_RATE as f64;
        let a = position as usize;
        let b = (a + 1).min(frames.len() - 1);
        let fraction = (position - a as f64) as f32;
        converted.push([
            frames[a][0] + (frames[b][0] - frames[a][0]) * fraction,
            frames[a][1] + (frames[b][1] - frames[a][1]) * fraction,
        ]);
    }
    Ok(Clip {
        frames: Arc::new(converted),
    })
}
pub fn demo() -> Clip {
    let count = SAMPLE_RATE as usize / 3;
    let frames = (0..count)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let envelope = (1.0 - i as f32 / count as f32).powi(2);
            let sample = (t * 660.0 * std::f32::consts::TAU).sin() * envelope * 0.08;
            [sample; 2]
        })
        .collect();
    Clip {
        frames: Arc::new(frames),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_mono_wav_and_resamples() {
        let path =
            std::env::temp_dir().join(format!("piratewave-decode-{}.wav", std::process::id()));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..4410 {
            writer
                .write_sample(((i as f32 * 0.12).sin() * 8192.0) as i16)
                .unwrap();
        }
        writer.finalize().unwrap();
        let clip = load(&path).unwrap();
        assert_eq!(clip.frames.len(), 4800);
        assert!(clip
            .frames
            .iter()
            .all(|f| f[0] == f[1] && f[0].abs() <= 0.251));
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn rejects_invalid_file() {
        let path = std::env::temp_dir().join(format!(
            "piratewave-invalid-audio-{}.wav",
            std::process::id()
        ));
        std::fs::write(&path, b"not audio").unwrap();
        assert!(load(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
