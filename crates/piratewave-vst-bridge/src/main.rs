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
    anyhow::ensure!(&ack == b"GO1\0", "Bridge handshake failed");
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
