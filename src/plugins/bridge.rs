//! x86 VST2 process adapter. The job object also cleans up helpers if a scanner/host crashes.
use super::*;
use piratewave_vst2_host::protocol::*;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::JobObjects::*,
};
struct Job(HANDLE);
impl Job {
    fn attach(child: &Child) -> Result<Self> {
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            anyhow::ensure!(
                !handle.is_null(),
                "Cannot create VST helper job: {}",
                std::io::Error::last_os_error()
            );
            let job = Self(handle);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            anyhow::ensure!(
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&info) as u32
                ) != 0,
                "Cannot configure VST helper job: {}",
                std::io::Error::last_os_error()
            );
            anyhow::ensure!(
                AssignProcessToJobObject(handle, child.as_raw_handle() as HANDLE) != 0,
                "Cannot attach VST helper job: {}",
                std::io::Error::last_os_error()
            );
            Ok(job)
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Process {
    child: Child,
    _job: Job,
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
pub struct Bridge {
    stream: TcpStream,
    state_stream: TcpStream,
    state_rx: mpsc::Receiver<Result<Option<piratewave_core::plugins::PluginState>, String>>,
    state_in_flight: bool,
    state_invalidated: bool,
    state_transfer_started: Option<Instant>,
    process: Process,
    descriptor: PluginDescriptor,
    parameters: BTreeMap<u32, f64>,
    notes: Vec<(u8, f32, bool)>,
    request: Vec<u8>,
    response: Vec<u8>,
    editor_open: bool,
    editor_needs_drain: bool,
    editor_size: (i32, i32),
    editor_edits: Vec<(u32, f64)>,
    editor_error: Option<String>,
}
impl Bridge {
    fn start_state_transfer(&mut self, command: u8) -> Result<()> {
        write_packet(&mut self.stream, command, &[])?;
        let kind = read_packet(&mut self.stream, &mut self.response)?;
        if kind == ERROR {
            anyhow::bail!(
                "VST2 state capture failed: {}",
                String::from_utf8_lossy(&self.response)
            );
        }
        anyhow::ensure!(
            kind == STATE && self.response == [3],
            "Invalid VST2 state reply"
        );
        self.state_in_flight = true;
        self.state_invalidated = false;
        self.state_transfer_started = Some(Instant::now());
        Ok(())
    }
    pub(super) fn terminate_helper_for_diagnostics(&mut self) -> Result<()> {
        self.process.child.kill()?;
        self.process.child.wait()?;
        Ok(())
    }
    pub(super) fn state_transfer_in_flight_for_diagnostics(&self) -> bool {
        self.state_in_flight
    }
    pub fn load(path: &Path) -> Result<Self> {
        let helper = std::env::current_exe()?
            .parent()
            .context("Application directory unavailable")?
            .join("piratewave-vst-bridge32.exe");
        anyhow::ensure!(helper.is_file(),"This is a 32-bit VST2 plugin. Missing {}. Keep piratewave-vst-bridge32.exe beside PirateWave.exe, or run build.ps1 to build the helper",helper.display());
        anyhow::ensure!(
            piratewave_vst2_host::pe::architecture(&helper)?
                == piratewave_vst2_host::pe::Architecture::X86,
            "VST helper is not 32-bit; rebuild with build.ps1"
        );
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let mut entropy = [0u8; 32];
        getrandom::fill(&mut entropy).map_err(|e| anyhow::anyhow!("VST helper token: {e}"))?;
        let token: String = entropy.iter().map(|b| format!("{b:02x}")).collect();
        let mut child = Command::new(&helper)
            .arg(listener.local_addr()?.port().to_string())
            .arg(&token)
            .arg(path)
            .creation_flags(0x08000000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Starting 32-bit VST helper")?;
        let job = match Job::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let mut process = Process { child, _job: job };
        let start = Instant::now();
        let mut stream = loop {
            if let Some(status) = process.child.try_wait()? {
                anyhow::bail!("32-bit VST helper exited before connecting: {status}");
            }
            anyhow::ensure!(
                start.elapsed() < Duration::from_secs(10),
                "32-bit VST helper connection timed out"
            );
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false)?;
                    stream.set_nodelay(true)?;
                    stream.set_read_timeout(Some(Duration::from_millis(250)))?;
                    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
                    let mut supplied = [0u8; 64];
                    if stream.read_exact(&mut supplied).is_ok() && supplied == token.as_bytes() {
                        stream.write_all(b"GO3\0")?;
                        break stream;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) => return Err(error.into()),
            }
        };
        let state_stream = loop {
            if let Some(status) = process.child.try_wait()? {
                anyhow::bail!("32-bit VST helper exited before state connection: {status}");
            }
            anyhow::ensure!(
                start.elapsed() < Duration::from_secs(10),
                "32-bit VST helper state connection timed out"
            );
            match listener.accept() {
                Ok((mut candidate, _)) => {
                    candidate.set_nonblocking(false)?;
                    candidate.set_nodelay(true)?;
                    candidate.set_read_timeout(Some(Duration::from_millis(250)))?;
                    let mut supplied = [0u8; 64];
                    if candidate.read_exact(&mut supplied).is_ok() && supplied == token.as_bytes() {
                        candidate.write_all(b"GO3\0")?;
                        candidate.set_read_timeout(None)?;
                        break candidate;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) => return Err(error.into()),
            }
        };
        let mut state_reader = state_stream.try_clone()?;
        let (state_tx, state_rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("piratewave-vst2-state-transfer".into())
            .spawn(move || loop {
                let mut bytes = Vec::new();
                let result = read_packet(&mut state_reader, &mut bytes)
                    .and_then(|kind| {
                        anyhow::ensure!(
                            kind == STATE && !bytes.is_empty(),
                            "Invalid VST2 state transfer"
                        );
                        match bytes[0] {
                            0 if bytes.len() == 1 => Ok(None),
                            kind @ 1..=2 if bytes.len() >= 5 => {
                                let program = u32::from_le_bytes(bytes[1..5].try_into().unwrap());
                                anyhow::ensure!(
                                    program == u32::MAX || program <= i32::MAX as u32,
                                    "Invalid VST2 program state"
                                );
                                bytes.drain(..5);
                                Ok(Some(piratewave_core::plugins::PluginState {
                                    component: bytes.into(),
                                    controller: None,
                                    vst2_bank: kind == 2,
                                    vst2_program: (program != u32::MAX).then_some(program),
                                }))
                            }
                            _ => anyhow::bail!("Invalid VST2 state transfer"),
                        }
                    })
                    .map_err(|error| format!("{error:#}"));
                let failed = result.is_err();
                if state_tx.send(result).is_err() || failed {
                    break;
                }
            })?;
        stream.set_read_timeout(Some(Duration::from_secs(25)))?;
        let mut response = Vec::with_capacity(65536);
        let kind=read_packet(&mut stream,&mut response).context("32-bit VST initialization failed; the helper exited, timed out, or the plugin needs activation")?;
        anyhow::ensure!(
            kind != ERROR,
            "32-bit VST load failed: {}",
            String::from_utf8_lossy(&response)
        );
        anyhow::ensure!(kind == READY, "Unexpected 32-bit helper response");
        let descriptor: PluginDescriptor = serde_json::from_slice(&response)?;
        descriptor.validate().map_err(anyhow::Error::msg)?;
        stream.set_read_timeout(Some(Duration::from_millis(500)))?;
        Ok(Self {
            stream,
            state_stream,
            state_rx,
            state_in_flight: false,
            state_invalidated: false,
            state_transfer_started: None,
            process,
            descriptor,
            parameters: BTreeMap::new(),
            notes: Vec::with_capacity(256),
            request: Vec::with_capacity(65536),
            response,
            editor_open: false,
            editor_needs_drain: false,
            editor_size: (0, 0),
            editor_edits: Vec::new(),
            editor_error: None,
        })
    }
    fn editor_command(&mut self, command: u8) -> Result<()> {
        write_packet(&mut self.stream, command, &[])?;
        let kind = read_packet(&mut self.stream, &mut self.response)?;
        if kind == ERROR {
            anyhow::bail!("VST2 editor: {}", String::from_utf8_lossy(&self.response));
        }
        anyhow::ensure!(
            kind == EDITOR_STATUS
                && self.response.len() >= 9
                && self.response[0] <= 1
                && (self.response.len() - 9).is_multiple_of(12)
                && self.response.len() <= 9 + 4096 * 12,
            "Invalid VST2 editor response"
        );
        let was_open = self.editor_open;
        self.editor_open = self.response[0] == 1;
        self.editor_needs_drain = was_open && !self.editor_open;
        self.editor_size = (
            i32::from_le_bytes(self.response[1..5].try_into().unwrap()),
            i32::from_le_bytes(self.response[5..9].try_into().unwrap()),
        );
        anyhow::ensure!(
            (0..=8192).contains(&self.editor_size.0) && (0..=8192).contains(&self.editor_size.1),
            "Invalid VST2 editor size"
        );
        for item in self.response[9..].as_chunks::<12>().0 {
            let id = u32::from_le_bytes(item[..4].try_into().unwrap());
            let value = f64::from_le_bytes(item[4..].try_into().unwrap());
            anyhow::ensure!(
                value.is_finite()
                    && (0.0..=1.0).contains(&value)
                    && self.descriptor.parameters.iter().any(|p| p.id == id),
                "Invalid VST2 editor parameter"
            );
            self.editor_edits.push((id, value));
        }
        Ok(())
    }
}
impl Hosted for Bridge {
    #[cfg(debug_assertions)]
    fn test_editor_timeout(&mut self) -> Result<()> {
        write_packet(&mut self.stream, TEST_EDITOR_TIMEOUT, &[])?;
        let kind = read_packet(&mut self.stream, &mut self.response)?;
        anyhow::ensure!(
            kind == STATE && self.response.is_empty(),
            "VST2 timeout test failed: {}",
            String::from_utf8_lossy(&self.response)
        );
        Ok(())
    }
    #[cfg(debug_assertions)]
    fn test_editor_key(&mut self, key: u32) -> Result<()> {
        write_packet(&mut self.stream, TEST_EDITOR_KEY, &key.to_le_bytes())?;
        let kind = read_packet(&mut self.stream, &mut self.response)?;
        anyhow::ensure!(
            kind == STATE && self.response.is_empty(),
            "VST2 test key failed: {}",
            String::from_utf8_lossy(&self.response)
        );
        Ok(())
    }
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }
    fn open_editor(&mut self) -> Result<()> {
        self.editor_command(OPEN_EDITOR)
    }
    fn close_editor(&mut self) {
        if self.editor_open || self.editor_needs_drain {
            let _ = self.editor_command(CLOSE_EDITOR);
        }
    }
    fn editor_idle(&mut self) {
        if self.editor_open || self.editor_needs_drain {
            if let Err(error) = self
                .flush_parameter_changes()
                .and_then(|()| self.editor_command(EDITOR_IDLE))
            {
                self.editor_open = false;
                self.editor_error = Some(format!("{error:#}"));
                crate::logging::error(format_args!("VST2 editor idle failed: {error:#}"));
            }
        }
    }
    fn editor_open(&self) -> bool {
        self.editor_open
    }
    fn editor_size(&self) -> (i32, i32) {
        self.editor_size
    }
    fn take_editor_error(&mut self) -> Option<String> {
        self.editor_error.take()
    }
    fn invalidate_capture(&mut self) {
        if self.state_in_flight {
            self.state_invalidated = true;
        }
        let result = (|| -> Result<()> {
            write_packet(&mut self.stream, INVALIDATE_STATE_CAPTURE, &[])?;
            let kind = read_packet(&mut self.stream, &mut self.response)?;
            anyhow::ensure!(
                kind == STATE && self.response.is_empty(),
                "Invalid state invalidation reply"
            );
            Ok(())
        })();
        if let Err(error) = result {
            self.editor_error = Some(format!("State invalidation failed: {error:#}"));
        }
    }
    fn take_editor_edits(&mut self) -> Vec<(u32, f64)> {
        if self.state_in_flight && !self.editor_edits.is_empty() {
            self.state_invalidated = true;
        }
        std::mem::take(&mut self.editor_edits)
    }
    fn capture_state(&mut self) -> Result<Option<piratewave_core::plugins::PluginState>> {
        self.flush_parameter_changes()?;
        if self.state_in_flight {
            match self.state_rx.try_recv() {
                Ok(result) => {
                    self.state_in_flight = false;
                    self.state_transfer_started = None;
                    if !self.state_invalidated {
                        return result.map_err(anyhow::Error::msg);
                    }
                }
                Err(mpsc::TryRecvError::Empty)
                    if self
                        .state_transfer_started
                        .is_some_and(|start| start.elapsed() >= Duration::from_secs(5)) =>
                {
                    anyhow::bail!("VST2 state transfer timed out")
                }
                Err(mpsc::TryRecvError::Empty) => anyhow::bail!("VST2 state capture is pending"),
                Err(mpsc::TryRecvError::Disconnected) => {
                    anyhow::bail!("VST2 state transfer stopped")
                }
            }
        }
        self.start_state_transfer(CAPTURE_STATE)?;
        anyhow::bail!("VST2 state capture is pending")
    }
    fn capture_state_for_shutdown(
        &mut self,
    ) -> Result<Option<piratewave_core::plugins::PluginState>> {
        self.flush_parameter_changes()?;
        if self.state_in_flight {
            self.state_rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|e| anyhow::anyhow!("Previous VST2 state transfer did not finish: {e}"))?
                .map_err(anyhow::Error::msg)?;
            self.state_in_flight = false;
            self.state_transfer_started = None;
        }
        self.stream.set_read_timeout(Some(Duration::from_secs(6)))?;
        let result = self.start_state_transfer(CAPTURE_STATE_FINAL);
        self.stream
            .set_read_timeout(Some(Duration::from_millis(500)))?;
        result?;
        let state = self
            .state_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| anyhow::anyhow!("VST2 state transfer did not finish: {e}"))?
            .map_err(anyhow::Error::msg);
        self.state_in_flight = false;
        self.state_transfer_started = None;
        state
    }
    fn restore_state(&mut self, state: &piratewave_core::plugins::PluginState) -> Result<()> {
        let mut bytes = Vec::with_capacity(state.component.len() + 5);
        bytes.push(u8::from(state.vst2_bank));
        bytes.extend_from_slice(&state.vst2_program.unwrap_or(u32::MAX).to_le_bytes());
        bytes.extend_from_slice(&state.component);
        write_packet(&mut self.stream, RESTORE_STATE, &bytes)?;
        let kind = read_packet(&mut self.stream, &mut self.response)?;
        if kind == ERROR {
            anyhow::bail!(
                "VST2 state restore failed: {}",
                String::from_utf8_lossy(&self.response)
            );
        }
        anyhow::ensure!(
            kind == STATE && self.response.is_empty(),
            "Invalid VST2 state reply"
        );
        Ok(())
    }
    fn set_parameter(&mut self, id: u32, value: f64) {
        if self.state_in_flight {
            self.state_invalidated = true;
        }
        self.editor_edits.retain(|(edited_id, _)| *edited_id != id);
        self.parameters.insert(id, value);
    }
    fn flush_parameter_changes(&mut self) -> Result<()> {
        if self.parameters.is_empty() {
            return Ok(());
        }
        self.request.clear();
        self.request
            .extend_from_slice(&(self.parameters.len() as u16).to_le_bytes());
        for (id, value) in &self.parameters {
            self.request.extend_from_slice(&id.to_le_bytes());
            self.request.extend_from_slice(&value.to_le_bytes());
        }
        write_packet(&mut self.stream, SET_PARAMETERS, &self.request)?;
        let kind = read_packet(&mut self.stream, &mut self.response)?;
        if kind == ERROR {
            anyhow::bail!(
                "VST2 parameter update failed: {}",
                String::from_utf8_lossy(&self.response)
            );
        }
        anyhow::ensure!(
            kind == STATE && self.response.is_empty(),
            "Invalid VST2 parameter update reply"
        );
        self.parameters.clear();
        Ok(())
    }
    fn note(&mut self, note: u8, velocity: f32, on: bool) {
        if self.notes.len() < 256 {
            self.notes.push((note, velocity, on));
        }
    }
    fn process(&mut self, frames: &mut [Frame]) -> Result<()> {
        anyhow::ensure!(frames.len() == BLOCK_SIZE, "Unexpected bridge block size");
        self.request.clear();
        self.request
            .extend_from_slice(&(self.parameters.len() as u16).to_le_bytes());
        self.request
            .extend_from_slice(&(self.notes.len() as u16).to_le_bytes());
        for (id, value) in &self.parameters {
            self.request.extend_from_slice(&id.to_le_bytes());
            self.request.extend_from_slice(&value.to_le_bytes());
        }
        self.parameters.clear();
        for (note, velocity, on) in self.notes.drain(..) {
            self.request.extend_from_slice(&[note, u8::from(on)]);
            self.request.extend_from_slice(&velocity.to_le_bytes());
        }
        append_frames(&mut self.request, frames);
        write_packet(&mut self.stream, PROCESS, &self.request)
            .context("32-bit VST helper stopped accepting audio")?;
        let kind = read_packet(&mut self.stream, &mut self.response).context(
            "32-bit VST helper stopped responding (500 ms deadline); plugin processing was stopped",
        )?;
        anyhow::ensure!(
            kind != ERROR,
            "32-bit VST processing failed: {}",
            String::from_utf8_lossy(&self.response)
        );
        anyhow::ensure!(kind == AUDIO, "Unexpected 32-bit VST audio reply");
        decode_frames(&self.response, frames)
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = write_packet(&mut self.stream, QUIT, &[]);
        let _ = self.stream.shutdown(Shutdown::Both);
        let _ = self.state_stream.shutdown(Shutdown::Both);
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(100) {
            if self.process.child.try_wait().ok().flatten().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
