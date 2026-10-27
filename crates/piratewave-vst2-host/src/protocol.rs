//! Versioned, bounded, little-endian bridge messages. Audio never uses stdout.
use anyhow::{Context, Result};
use piratewave_core::{Frame, BLOCK_SIZE};
use std::io::{Read, Write};
pub const READY: u8 = 1;
pub const PROCESS: u8 = 2;
pub const AUDIO: u8 = 3;
pub const ERROR: u8 = 4;
pub const QUIT: u8 = 5;
pub const MAX_PACKET: usize = 1024 * 1024;
pub const FRAME_BYTES: usize = BLOCK_SIZE * 8;
pub fn write_packet(stream: &mut impl Write, kind: u8, bytes: &[u8]) -> Result<()> {
    anyhow::ensure!(bytes.len() <= MAX_PACKET, "Bridge packet too large");
    let mut header = [0u8; 9];
    header[..4].copy_from_slice(b"PVB1");
    header[4] = kind;
    header[5..].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
    stream.write_all(&header)?;
    stream.write_all(bytes)?;
    stream.flush()?;
    Ok(())
}
pub fn read_packet(stream: &mut impl Read, buffer: &mut Vec<u8>) -> Result<u8> {
    let mut header = [0u8; 9];
    stream
        .read_exact(&mut header)
        .context("Bridge disconnected or timed out")?;
    anyhow::ensure!(
        &header[..4] == b"PVB1",
        "Bridge protocol version/signature mismatch"
    );
    let size = u32::from_le_bytes(header[5..].try_into().unwrap()) as usize;
    anyhow::ensure!(size <= MAX_PACKET, "Bridge packet exceeds limit");
    buffer.resize(size, 0);
    stream.read_exact(buffer)?;
    Ok(header[4])
}
pub fn append_frames(bytes: &mut Vec<u8>, frames: &[Frame]) {
    for frame in frames {
        for sample in frame {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
    }
}
pub fn decode_frames(bytes: &[u8], frames: &mut [Frame]) -> Result<()> {
    anyhow::ensure!(
        bytes.len() == frames.len() * 8,
        "Wrong bridge audio block size"
    );
    for (frame, bytes) in frames.iter_mut().zip(bytes.as_chunks::<8>().0.iter()) {
        for (sample, bytes) in frame.iter_mut().zip(bytes.as_chunks::<4>().0.iter()) {
            *sample = f32::from_le_bytes(*bytes);
        }
    }
    Ok(())
}
pub fn process_request(
    bytes: &[u8],
    plugin: &mut impl crate::Hosted,
    frames: &mut [Frame; BLOCK_SIZE],
) -> Result<()> {
    anyhow::ensure!(bytes.len() >= 4, "Truncated bridge request");
    let parameters = u16::from_le_bytes(bytes[..2].try_into().unwrap()) as usize;
    let notes = u16::from_le_bytes(bytes[2..4].try_into().unwrap()) as usize;
    anyhow::ensure!(
        parameters <= 4096
            && notes <= 256
            && bytes.len() == 4 + parameters * 12 + notes * 6 + FRAME_BYTES,
        "Invalid bridge request counts/size"
    );
    let mut cursor = 4;
    for _ in 0..parameters {
        let id = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
        let value = f64::from_le_bytes(bytes[cursor + 4..cursor + 12].try_into().unwrap());
        anyhow::ensure!(
            value.is_finite() && (0.0..=1.0).contains(&value),
            "Invalid normalized parameter"
        );
        anyhow::ensure!(
            plugin.descriptor().parameters.iter().any(|p| p.id == id),
            "Unknown parameter ID"
        );
        plugin.set_parameter(id, value);
        cursor += 12;
    }
    for _ in 0..notes {
        let note = bytes[cursor];
        let on = bytes[cursor + 1];
        let velocity = f32::from_le_bytes(bytes[cursor + 2..cursor + 6].try_into().unwrap());
        anyhow::ensure!(
            note <= 127 && on <= 1 && velocity.is_finite() && (0.0..=1.0).contains(&velocity),
            "Invalid note event"
        );
        plugin.note(note, velocity, on != 0);
        cursor += 6;
    }
    decode_frames(&bytes[cursor..], frames)?;
    plugin.process(frames)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packets_and_float_audio_roundtrip_and_reject_oversized_messages() {
        let frames = [[0.25, -0.125]; BLOCK_SIZE];
        let mut audio = vec![];
        append_frames(&mut audio, &frames);
        let mut wire = vec![];
        write_packet(&mut wire, AUDIO, &audio).unwrap();
        let mut decoded = vec![];
        assert_eq!(read_packet(&mut &wire[..], &mut decoded).unwrap(), AUDIO);
        let mut output = [[0.0; 2]; BLOCK_SIZE];
        decode_frames(&decoded, &mut output).unwrap();
        assert_eq!(frames, output);
        let mut malicious = b"PVB1\x03".to_vec();
        malicious.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(read_packet(&mut &malicious[..], &mut decoded).is_err());
        assert!(decode_frames(&[0; 3], &mut output).is_err());
    }
}
