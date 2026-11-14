#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
use anyhow::{Context, Result};
use piratewave_vst2_host::{protocol::*, Hosted, Vst2};
use std::{
    io::{Read, Write},
    net::TcpStream,
    path::Path,
    time::Duration,
};
mod characterize;
fn pump() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let mut message: MSG = std::mem::zeroed();
        for _ in 0..64 {
            if PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) == 0 {
                break;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}
fn main() -> Result<()> {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Com::CoInitializeEx(
            std::ptr::null(),
            windows_sys::Win32::System::Com::COINIT_APARTMENTTHREADED as u32,
        );
    }
    let args: Vec<_> = std::env::args().collect();
    if characterize::run(&args)? {
        return Ok(());
    }
    if args.get(1).is_some_and(|s| s == "--inspect") {
        let path = Path::new(args.get(2).context("Missing plugin path")?);
        let output = args.get(3).context("Missing report path")?;
        let report = match Vst2::load(path) {
            Ok(plugin) => serde_json::json!({"plugins":[plugin.descriptor()],"error":null}),
            Err(error) => serde_json::json!({"plugins":[],"error":format!("{error:#}")}),
        };
        std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
        return Ok(());
    }
    anyhow::ensure!(
        args.len() == 4,
        "Usage: piratewave-vst-bridge <port> <token> <plugin.dll>"
    );
    let port: u16 = args[1].parse()?;
    anyhow::ensure!(args[2].len() == 64, "Invalid bridge token");
    let mut stream = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(args[2].as_bytes())?;
    let mut ack = [0u8; 4];
    stream.read_exact(&mut ack)?;
    anyhow::ensure!(&ack == b"GO3\0", "Bridge handshake failed");
    // State may be tens of MiB. Transfer it on a separate connection so that
    // neither the helper's audio loop nor the host's audio worker copies or
    // waits for the bulk payload.
    let mut state_stream = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))?;
    state_stream.set_nodelay(true)?;
    state_stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    state_stream.write_all(args[2].as_bytes())?;
    state_stream.read_exact(&mut ack)?;
    anyhow::ensure!(&ack == b"GO3\0", "Bridge state handshake failed");
    let (state_tx, state_rx) =
        std::sync::mpsc::sync_channel::<Option<piratewave_vst2_host::StateChunk>>(1);
    std::thread::Builder::new()
        .name("vst2-bridge-state-transfer".into())
        .spawn(move || {
            while let Ok(state) = state_rx.recv() {
                let mut bytes = vec![0];
                if let Some((data, bank, program)) = state {
                    bytes[0] = if bank { 2 } else { 1 };
                    bytes.extend_from_slice(&program.unwrap_or(u32::MAX).to_le_bytes());
                    bytes.extend_from_slice(&data);
                }
                if write_packet(&mut state_stream, STATE, &bytes).is_err() {
                    break;
                }
            }
        })?;
    let mut plugin = match Vst2::load(Path::new(&args[3])) {
        Ok(plugin) => plugin,
        Err(error) => {
            write_packet(&mut stream, ERROR, format!("{error:#}").as_bytes())?;
            return Ok(());
        }
    };
    write_packet(
        &mut stream,
        READY,
        &serde_json::to_vec(plugin.descriptor())?,
    )?;
    // The parent may be paused while audio is bypassed. Detect parent death via EOF/job lifetime.
    stream.set_read_timeout(None)?;
    let mut packet = Vec::with_capacity(65536);
    let mut response = Vec::with_capacity(FRAME_BYTES);
    let mut frames = [[0.0; 2]; piratewave_core::BLOCK_SIZE];
    while let Ok(kind) = read_packet(&mut stream, &mut packet) {
        if kind == QUIT {
            break;
        }
        #[cfg(all(windows, debug_assertions))]
        if kind == TEST_EDITOR_TIMEOUT {
            match plugin.test_editor_timeout() {
                Ok(()) => write_packet(&mut stream, STATE, &[])?,
                Err(error) => write_packet(&mut stream, ERROR, format!("{error:#}").as_bytes())?,
            }
            continue;
        }
        #[cfg(all(windows, debug_assertions))]
        if kind == TEST_EDITOR_KEY {
            let result = if packet.len() == 4 {
                plugin.test_editor_key(u32::from_le_bytes(packet[..4].try_into().unwrap()))
            } else {
                Err(anyhow::anyhow!("Invalid test key"))
            };
            match result {
                Ok(()) => write_packet(&mut stream, STATE, &[])?,
                Err(error) => write_packet(&mut stream, ERROR, format!("{error:#}").as_bytes())?,
            }
            continue;
        }
        if kind == SET_PARAMETERS {
            let result = (|| -> Result<()> {
                anyhow::ensure!(packet.len() >= 2, "Truncated parameter update");
                let count = u16::from_le_bytes(packet[..2].try_into().unwrap()) as usize;
                anyhow::ensure!(
                    count <= 4096 && packet.len() == 2 + count * 12,
                    "Invalid parameter update"
                );
                for item in packet[2..].as_chunks::<12>().0 {
                    let id = u32::from_le_bytes(item[..4].try_into().unwrap());
                    let value = f64::from_le_bytes(item[4..].try_into().unwrap());
                    anyhow::ensure!(
                        value.is_finite()
                            && (0.0..=1.0).contains(&value)
                            && plugin.descriptor().parameters.iter().any(|p| p.id == id),
                        "Invalid plugin parameter"
                    );
                }
                for item in packet[2..].as_chunks::<12>().0 {
                    let id = u32::from_le_bytes(item[..4].try_into().unwrap());
                    let value = f64::from_le_bytes(item[4..].try_into().unwrap());
                    plugin.set_parameter(id, value);
                }
                Ok(())
            })();
            match result {
                Ok(()) => write_packet(&mut stream, STATE, &[])?,
                Err(error) => write_packet(&mut stream, ERROR, format!("{error:#}").as_bytes())?,
            }
            continue;
        }
        #[cfg(windows)]
        if matches!(kind, OPEN_EDITOR | CLOSE_EDITOR | EDITOR_IDLE) {
            pump();
            let edits = plugin.take_editor_edits();
            let result = match kind {
                OPEN_EDITOR => plugin.open_editor(),
                CLOSE_EDITOR => {
                    plugin.close_editor();
                    Ok(())
                }
                _ => {
                    plugin.editor_idle();
                    if let Some(error) = plugin.take_editor_error() {
                        Err(anyhow::anyhow!(error))
                    } else {
                        Ok(())
                    }
                }
            };
            match result {
                Ok(()) => {
                    let mut data = vec![u8::from(plugin.editor_open())];
                    let (width, height) = plugin.editor_size();
                    data.extend_from_slice(&width.to_le_bytes());
                    data.extend_from_slice(&height.to_le_bytes());
                    for (id, value) in edits {
                        data.extend_from_slice(&id.to_le_bytes());
                        data.extend_from_slice(&value.to_le_bytes());
                    }
                    write_packet(&mut stream, EDITOR_STATUS, &data)?;
                }
                Err(error) => write_packet(&mut stream, ERROR, format!("{error:#}").as_bytes())?,
            }
            continue;
        }
        if kind == CAPTURE_STATE || kind == CAPTURE_STATE_FINAL {
            let result = if kind == CAPTURE_STATE_FINAL {
                plugin.capture_state_for_shutdown()
            } else {
                plugin.capture_state()
            };
            match result {
                Ok(state) => match state_tx.try_send(state) {
                    Ok(()) => write_packet(&mut stream, STATE, &[3])?,
                    Err(_) => write_packet(&mut stream, ERROR, b"VST2 state transfer unavailable")?,
                },
                Err(error) => write_packet(&mut stream, ERROR, format!("{error:#}").as_bytes())?,
            }
            continue;
        }
        #[cfg(windows)]
        if kind == INVALIDATE_STATE_CAPTURE {
            plugin.invalidate_capture();
            write_packet(&mut stream, STATE, &[])?;
            continue;
        }
        if kind == RESTORE_STATE {
            let result = if packet.len() >= 5 && packet[0] <= 1 {
                let program = u32::from_le_bytes(packet[1..5].try_into().unwrap());
                plugin.restore_state(
                    &packet[5..],
                    packet[0] == 1,
                    (program != u32::MAX).then_some(program),
                )
            } else {
                Err(anyhow::anyhow!("Invalid VST2 state kind"))
            };
            match result {
                Ok(()) => write_packet(&mut stream, STATE, &[])?,
                Err(error) => write_packet(&mut stream, ERROR, format!("{error:#}").as_bytes())?,
            }
            continue;
        }
        if kind != PROCESS {
            write_packet(&mut stream, ERROR, b"Unexpected bridge message")?;
            break;
        }
        pump();
        if let Err(error) = process_request(&packet, &mut plugin, &mut frames) {
            write_packet(&mut stream, ERROR, format!("{error:#}").as_bytes())?;
            break;
        }
        response.clear();
        append_frames(&mut response, &frames);
        write_packet(&mut stream, AUDIO, &response)?;
    }
    drop(plugin);
    Ok(())
}
