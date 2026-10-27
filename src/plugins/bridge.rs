//! x86 VST2 process adapter. The job object also cleans up helpers if a scanner/host crashes.
use super::*;
use piratewave_vst2_host::protocol::*;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Child, Command, Stdio},
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
    process: Process,
    descriptor: PluginDescriptor,
    parameters: BTreeMap<u32, f64>,
    notes: Vec<(u8, f32, bool)>,
    request: Vec<u8>,
    response: Vec<u8>,
}
impl Bridge {
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
                        stream.write_all(b"GO1\0")?;
                        break stream;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) => return Err(error.into()),
            }
        };
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
            process,
            descriptor,
            parameters: BTreeMap::new(),
            notes: Vec::with_capacity(256),
            request: Vec::with_capacity(65536),
            response,
        })
    }
}
impl Hosted for Bridge {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }
    fn set_parameter(&mut self, id: u32, value: f64) {
        self.parameters.insert(id, value);
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
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(100) {
            if self.process.child.try_wait().ok().flatten().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
