//! Optional Beta tools and persistent jobs. No work is performed on audio threads.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

static HEAVY: AtomicBool = AtomicBool::new(false);
/// Fixed models retain latest/previous checkpoints plus atomic-write staging.
pub const JOB_PEAK_BYTES: u64 = 6 * 1024_u64.pow(3);
#[cfg(windows)]
struct Exclusive(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Exclusive {
    fn acquire() -> Result<Self> {
        Self::named("PirateWaveVoiceTrainingHeavyJob")
    }
    fn named(identifier: &str) -> Result<Self> {
        unsafe {
            let name: Vec<u16> = format!("Local\\{identifier}\0").encode_utf16().collect();
            let handle = windows_sys::Win32::System::Threading::CreateMutexW(
                std::ptr::null(),
                1,
                name.as_ptr(),
            );
            ensure!(!handle.is_null(), "Cannot create training job mutex");
            if windows_sys::Win32::Foundation::GetLastError() == 183 {
                windows_sys::Win32::Foundation::CloseHandle(handle);
                anyhow::bail!(
                    "Another PirateWave instance is using this training operation ({identifier})"
                );
            }
            Ok(Self(handle))
        }
    }
}
#[cfg(windows)]
impl Drop for Exclusive {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.0);
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
pub const NATIVE_WORKER_SHA256: &str =
    "5e2d7e362963a9e518f8c8e4b12ded48eaf6c10274bcffa9f712362ff07c95f2";
pub fn native_worker() -> Result<PathBuf> {
    let path = std::env::current_exe()?
        .parent()
        .context("Application folder unavailable")?
        .join("PirateWaveVoiceWorker.exe");
    ensure!(digest(&path)?==NATIVE_WORKER_SHA256,"Matching native training voice worker is missing or changed. Install the full portable PirateWave build.");
    Ok(path)
}
pub fn native_runtime() -> Result<PathBuf> {
    let worker = native_worker()?;
    let source = crate::voice::pack::load_cpu_runtime()?;
    let target = crate::storage::default_path()
        .with_file_name("voice-runtimes")
        .join(format!("cpu-{}", &NATIVE_WORKER_SHA256[..12]));
    fs::create_dir_all(&target)?;
    let manifest = crate::voice::pack::manifest();
    for &name in crate::voice::pack::CPU_RUNTIME_FILES {
        let dest = target.join(name);
        if !dest.exists() {
            fs::hard_link(source.join(name), &dest)
                .or_else(|_| fs::copy(source.join(name), &dest).map(|_| ()))?;
        }
        ensure!(
            digest(&dest)? == manifest.files[name].sha256,
            "Native runtime changed: {name}"
        );
    }
    let dest = target.join("PatchVoiceWorker.exe");
    if !dest.exists() {
        fs::copy(&worker, &dest)?;
    }
    ensure!(
        digest(&dest)? == NATIVE_WORKER_SHA256,
        "Native voice worker cache changed"
    );
    let extra = worker.parent().unwrap().join("msvcp140_1.dll");
    if extra.exists() {
        let spec = &manifest.files["msvcp140_1.dll"];
        ensure!(
            digest(&extra)? == spec.sha256,
            "Native CRT checksum mismatch"
        );
        let dest = target.join("msvcp140_1.dll");
        if !dest.exists() {
            fs::copy(extra, &dest)?;
        }
        ensure!(
            digest(&dest)? == spec.sha256,
            "Cached CRT checksum mismatch"
        );
    }
    Ok(target)
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Part {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
    pub url: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub id: String,
    pub backend: String,
    pub bytes: u64,
    pub installed_bytes: u64,
    pub sha256: String,
    pub manifest_sha256: String,
    pub parts: Vec<Part>,
}
#[derive(Deserialize)]
pub struct Catalog {
    pub schema: u32,
    pub bundles: Vec<Bundle>,
}
pub fn catalog() -> Catalog {
    let catalog: Catalog =
        serde_json::from_str(include_str!("catalog.json")).expect("Training catalog");
    assert_eq!(catalog.schema, 1);
    catalog
}
pub fn bundle(backend: &str) -> Result<Bundle> {
    catalog()
        .bundles
        .into_iter()
        .find(|b| b.backend == backend)
        .context("No matching training bundle in this build")
}
pub fn installed(workspace: &Path, backend: &str) -> Result<PathBuf> {
    let spec = bundle(backend)?;
    let path = workspace
        .join(if backend == "shared" {
            "shared"
        } else {
            "tools"
        })
        .join(&spec.id);
    ensure!(
        path.join("runtime-manifest.json").is_file(),
        "Install the {backend} bundle first"
    );
    Ok(path)
}
pub fn digest(path: &Path) -> Result<String> {
    let mut hash = Sha256::new();
    let mut f = fs::File::open(path)?;
    let mut bytes = [0; 65536];
    loop {
        let n = f.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn nonce() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}
pub fn save<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    crate::storage::save_json(path, value)
}
fn space(path: &Path, additional: u64) -> Result<()> {
    let mut parent = path;
    while !parent.exists() {
        parent = parent.parent().context("Workspace drive unavailable")?;
    }
    ensure!(
        super::free_space(parent).context("Cannot determine available disk space")?
            >= super::required_space(additional).context("Storage estimate overflow")?,
        "Insufficient space including 2 GiB / 10% headroom. Choose another workspace drive."
    );
    Ok(())
}
fn hidden(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
}
pub struct Task {
    pub result: mpsc::Receiver<Result<String, String>>,
    control: Arc<AtomicU8>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Task {
    pub fn spawn(action: impl FnOnce(Arc<AtomicU8>) -> Result<String> + Send + 'static) -> Self {
        let (tx, result) = mpsc::sync_channel(1);
        let control = Arc::new(AtomicU8::new(0));
        let stop = control.clone();
        let handle = Some(thread::spawn(move || {
            let _ = tx.send(action(stop).map_err(|e| format!("{e:#}")));
        }));
        Self {
            result,
            control,
            handle,
        }
    }
    pub fn pause(&self) {
        self.control.store(1, Ordering::Release);
    }
    pub fn cancel(&self) {
        self.control.store(2, Ordering::Release);
    }
    pub fn shutdown(&mut self) {
        self.pause();
        let deadline = Instant::now() + Duration::from_millis(5500);
        while self.handle.as_ref().is_some_and(|h| !h.is_finished()) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        if self.handle.as_ref().is_some_and(|h| h.is_finished()) {
            let _ = self.handle.take().unwrap().join();
        }
    }
}
impl Drop for Task {
    fn drop(&mut self) {
        self.pause();
        if self.handle.as_ref().is_some_and(|h| h.is_finished()) {
            let _ = self.handle.take().unwrap().join();
        }
    }
}
#[derive(Deserialize)]
struct Manifest {
    schema: u32,
    files: std::collections::BTreeMap<String, FileSpec>,
}
#[derive(Deserialize)]
struct FileSpec {
    bytes: u64,
    sha256: String,
}
pub fn verify(path: &Path, spec: &Bundle) -> Result<()> {
    ensure!(
        digest(&path.join("runtime-manifest.json"))? == spec.manifest_sha256,
        "Tools manifest changed; reinstall bundle"
    );
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(path.join("runtime-manifest.json"))?)?;
    ensure!(manifest.schema == 1, "Unsupported tools manifest");
    let expected: std::collections::HashSet<_> = manifest
        .files
        .keys()
        .cloned()
        .chain(std::iter::once("runtime-manifest.json".into()))
        .collect();
    let actual: std::collections::HashSet<_> = walk_files(path)?
        .into_iter()
        .map(|p| {
            p.strip_prefix(path)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    ensure!(
        actual == expected,
        "Unexpected installed files; reinstall tools before executing them"
    );
    for (relative, file) in manifest.files {
        safe_relative(&relative)?;
        let p = path.join(relative);
        ensure!(
            fs::metadata(&p)?.len() == file.bytes && digest(&p)? == file.sha256,
            "Installed tools changed; reinstall bundle"
        );
    }
    Ok(())
}
fn safe_relative(relative: &str) -> Result<()> {
    for component in relative.split('/') {
        let base = component
            .split('.')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        ensure!(
            !["CON", "PRN", "AUX", "NUL"].contains(&base.as_str())
                && !(base.len() == 4
                    && (base.starts_with("COM") || base.starts_with("LPT"))
                    && base.as_bytes()[3].is_ascii_digit()),
            "Reserved Windows filename"
        );
    }
    ensure!(
        !relative.is_empty()
            && !relative.contains(['\\', ':'])
            && relative
                .split('/')
                .all(|p| !p.is_empty() && p != "." && p != ".." && !p.ends_with(['.', ' ']))
            && !Path::new(relative).is_absolute(),
        "Unsafe archive path"
    );
    Ok(())
}
pub fn install(workspace: PathBuf, backend: String, offline: Option<PathBuf>) -> Task {
    Task::spawn(move |cancel| {
        let spec = bundle(&backend)?;
        #[cfg(windows)]
        let _installation_lock = Exclusive::named(&format!("PirateWaveInstall-{}", spec.id))?;
        let parent = workspace.join(if backend == "shared" {
            "shared"
        } else {
            "tools"
        });
        let target = parent.join(&spec.id);
        let repair = target.exists();
        if repair && verify(&target, &spec).is_ok() {
            return Ok(format!("{backend} tools already installed"));
        }
        ensure!(
            !repair || !HEAVY.load(Ordering::Acquire),
            "Pause training before repairing tools"
        );
        #[cfg(windows)]
        let _repair_lock = if repair {
            Some(Exclusive::acquire()?)
        } else {
            None
        };
        space(
            &workspace,
            spec.bytes
                .saturating_mul(2)
                .saturating_add(spec.installed_bytes),
        )?;
        let downloads = workspace.join("downloads").join(&spec.id);
        fs::create_dir_all(&downloads)?;
        for part in &spec.parts {
            safe_relative(&part.name)?;
            let destination = downloads.join(&part.name);
            ensure!(
                cancel.load(Ordering::Acquire) == 0,
                "Installation cancelled"
            );
            if destination.exists()
                && fs::metadata(&destination)?.len() == part.bytes
                && digest(&destination)? == part.sha256
            {
                continue;
            }
            if let Some(folder) = &offline {
                fs::copy(folder.join(&part.name), &destination)
                    .context("Offline bundle part missing")?;
            } else {
                // A complete but corrupt file cannot resume past EOF. Preserve
                // the bad cache entry and download a fresh verified copy.
                if destination.exists() && fs::metadata(&destination)?.len() >= part.bytes {
                    fs::rename(
                        &destination,
                        downloads.join(format!("{}.bad-{}", part.name, nonce())),
                    )?;
                }
                ensure!(
                    part.url.starts_with(
                        "https://github.com/MitchCodes/PatchAudio-Releases/releases/download/"
                    ),
                    "Untrusted download origin"
                );
                let mut cmd = Command::new("curl.exe");
                hidden(&mut cmd);
                cmd.args([
                    "--fail",
                    "--location",
                    "--proto",
                    "=https",
                    "--proto-redir",
                    "=https",
                    "--connect-timeout",
                    "20",
                    "--retry",
                    "2",
                    "--continue-at",
                    "-",
                    "--output",
                ])
                .arg(&destination)
                .arg(&part.url)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
                let mut downloader = cmd
                    .spawn()
                    .context("Windows curl download could not start")?;
                #[cfg(windows)]
                let _download_job = match crate::voice::job::Job::attach(&downloader) {
                    Ok(job) => job,
                    Err(error) => {
                        let _ = downloader.kill();
                        let _ = downloader.wait();
                        return Err(error);
                    }
                };
                let status = loop {
                    if cancel.load(Ordering::Acquire) != 0 {
                        let _ = downloader.kill();
                        let _ = downloader.wait();
                        anyhow::bail!("Download stopped; partial data is retained for resuming");
                    }
                    if let Some(status) = downloader.try_wait()? {
                        break status;
                    }
                    thread::sleep(Duration::from_millis(100));
                };
                ensure!(status.success(),"Download failed. Partial download is retained; retry or import the offline bundle.");
            }
            ensure!(
                fs::metadata(&destination)?.len() == part.bytes
                    && digest(&destination)? == part.sha256,
                "Bundle part checksum failed"
            );
        }
        let archive = downloads.join("bundle.zip");
        let mut assembled = fs::File::create(&archive)?;
        for part in &spec.parts {
            std::io::copy(
                &mut fs::File::open(downloads.join(&part.name))?,
                &mut assembled,
            )?;
        }
        assembled.sync_all()?;
        drop(assembled);
        ensure!(
            digest(&archive)? == spec.sha256,
            "Combined archive checksum failed"
        );
        fs::create_dir_all(&parent)?;
        let staging = parent.join(format!("staging-{}", nonce()));
        fs::create_dir(&staging)?;
        let mut zip = zip::ZipArchive::new(fs::File::open(&archive)?)?;
        let mut names = std::collections::HashSet::new();
        let mut total = 0_u64;
        for i in 0..zip.len() {
            ensure!(
                cancel.load(Ordering::Acquire) == 0,
                "Installation cancelled; staged files retained for inspection"
            );
            let mut entry = zip.by_index(i)?;
            let name = entry.name().to_owned();
            safe_relative(&name)?;
            ensure!(
                !entry.is_dir()
                    && entry.unix_mode().is_none_or(|m| m & 0o170000 != 0o120000)
                    && names.insert(name.to_lowercase()),
                "Archive links or duplicate files are forbidden"
            );
            total = total
                .checked_add(entry.size())
                .context("Archive size overflow")?;
            ensure!(
                total <= spec.installed_bytes,
                "Archive exceeds catalog size"
            );
            let file = staging.join(name);
            fs::create_dir_all(file.parent().unwrap())?;
            std::io::copy(&mut entry, &mut fs::File::create(file)?)?;
        }
        ensure!(
            total == spec.installed_bytes,
            "Installed size differs from catalog"
        );
        verify(&staging, &spec)?;
        let previous = publish_staged(&staging, &target)?;
        Ok(match previous {
            Some(path) => format!(
                "{backend} tools repaired. Previous files retained at {}",
                path.display()
            ),
            None => format!("{backend} bundle installed"),
        })
    })
}
fn publish_staged(staging: &Path, target: &Path) -> Result<Option<PathBuf>> {
    ensure!(staging.is_dir(), "Verified installation staging is missing");
    let previous = if target.exists() {
        let path = target.with_file_name(format!(
            "retired-{}-{}",
            target.file_name().unwrap().to_string_lossy(),
            nonce()
        ));
        fs::rename(target, &path)?;
        Some(path)
    } else {
        None
    };
    if let Err(error) = fs::rename(staging, target) {
        if let Some(path) = &previous {
            fs::rename(path, target).with_context(|| {
                format!(
                    "Could not restore previous tools; preserved at {}",
                    path.display()
                )
            })?;
        }
        return Err(error.into());
    }
    Ok(previous)
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Recording {
    pub id: String,
    pub path: String,
    pub name: String,
    pub role: String,
    pub excluded: bool,
    pub seconds: f32,
    pub clipping: f32,
    pub silence: f32,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Job {
    pub schema: u32,
    pub id: String,
    pub name: String,
    pub backend: String,
    pub runtime: PathBuf,
    pub shared: PathBuf,
    pub native_worker: PathBuf,
    pub status: String,
    pub message: String,
    pub recordings: Vec<Recording>,
    pub recipe: String,
}
impl Job {
    pub fn load(folder: &Path) -> Result<Self> {
        Ok(serde_json::from_slice(&fs::read(folder.join("job.json"))?)?)
    }
    pub fn save(&self, folder: &Path) -> Result<()> {
        save(&folder.join("job.json"), self)
    }
}
pub fn jobs(workspace: &Path) -> Vec<(PathBuf, Job)> {
    fs::read_dir(workspace.join("jobs"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| Job::load(&e.path()).ok().map(|j| (e.path(), j)))
        .collect()
}
pub fn create(workspace: &Path, name: String, backend: String) -> Result<PathBuf> {
    ensure!(
        !name.trim().is_empty() && name.len() <= 100,
        "Choose a voice name (up to 100 bytes)"
    );
    let id = format!("voice-{}", nonce());
    let folder = workspace.join("jobs").join(&id);
    fs::create_dir_all(folder.join("recordings"))?;
    let runtime = workspace.join("tools").join(bundle(&backend)?.id);
    let shared = workspace.join("shared").join(bundle("shared")?.id);
    let native_worker = native_worker()?;
    Job {
        schema: 1,
        id,
        name,
        backend,
        runtime,
        shared,
        native_worker,
        status: "draft".into(),
        message: String::new(),
        recordings: vec![],
        recipe: "llvc-beta-no-f0-250-1000-v1".into(),
    }
    .save(&folder)?;
    Ok(folder)
}
pub fn import(folder: PathBuf, paths: Vec<PathBuf>, role: String) -> Task {
    Task::spawn(move |_| {
        let mut job = Job::load(&folder)?;
        ensure!(
            job.status == "draft",
            "Recordings are locked after training starts. Create a new job to change the dataset."
        );
        ensure!(
            ["train", "validation", "test"].contains(&role.as_str()),
            "Unknown recording role"
        );
        for path in paths {
            space(&folder, 512 * 1024 * 1024)?;
            let clip = crate::assets::load_with_limit(&path, 1800)?;
            space(
                &folder,
                (clip.frames.len() as u64)
                    .saturating_mul(8)
                    .saturating_add(4096),
            )?;
            let id = nonce();
            let relative = format!("recordings/{id}.wav");
            let spec = hound::WavSpec {
                channels: 2,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            };
            let mut out = hound::WavWriter::create(folder.join(&relative), spec)?;
            let mut clipped = 0usize;
            let mut quiet = 0usize;
            for frame in clip.frames.iter() {
                let peak = frame[0].abs().max(frame[1].abs());
                clipped += usize::from(peak >= 0.999);
                quiet += usize::from(peak < 0.003);
                out.write_sample(frame[0])?;
                out.write_sample(frame[1])?;
            }
            out.finalize()?;
            job.recordings.push(Recording {
                id,
                path: relative,
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
                role: role.clone(),
                excluded: false,
                seconds: clip.duration(),
                clipping: clipped as f32 / clip.frames.len() as f32,
                silence: quiet as f32 / clip.frames.len() as f32,
            });
            job.save(&folder)?;
        }
        Ok("Recordings copied and analyzed. Originals were preserved.".into())
    })
}
pub fn record(folder: PathBuf, device: Option<String>, role: String) -> Task {
    Task::spawn(move |stop| {
        let mut job = Job::load(&folder)?;
        ensure!(
            job.status == "draft",
            "Recordings are locked once training starts"
        );
        ensure!(
            ["train", "validation", "test"].contains(&role.as_str()),
            "Unknown recording role"
        );
        space(&folder, 48000 * 300 * 8)?;
        let _audio_clock = crate::audio_scheduling::AudioThread::register();
        let counters = Arc::new(crate::audio_io::Counters::default());
        let errors = Arc::new(crossbeam_queue::ArrayQueue::new(4));
        let mut input =
            crate::audio_io::open_input(&device, false, counters.clone(), errors.clone())?;
        let id = nonce();
        let relative = format!("recordings/{id}.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut out = hound::WavWriter::create(folder.join(&relative), spec)?;
        let mut count = 0usize;
        let mut clipped = 0usize;
        let mut quiet = 0usize;
        let mut deadline: Option<Instant> = None;
        while stop.load(Ordering::Acquire) == 0 && count < 48000 * 300 {
            if let Some(error) = errors.pop() {
                anyhow::bail!("Recording device failed: {error}");
            }
            if let Some(frame) = input.resampler.next_frame() {
                deadline.get_or_insert_with(Instant::now);
                let peak = frame[0].abs().max(frame[1].abs());
                clipped += usize::from(peak >= 0.999);
                quiet += usize::from(peak < 0.003);
                out.write_sample(frame[0])?;
                out.write_sample(frame[1])?;
                count += 1;
                // QueueReader's clock compensation assumes real-time consumption.
                // Draining whole capture packets here repeatedly starves/reprimes it.
                if count.is_multiple_of(piratewave_core::BLOCK_SIZE) {
                    let next = deadline.as_mut().unwrap();
                    *next += Duration::from_secs_f64(piratewave_core::BLOCK_SIZE as f64 / 48000.);
                    if let Some(wait) = next.checked_duration_since(Instant::now()) {
                        thread::sleep(wait);
                    }
                }
            } else {
                thread::sleep(Duration::from_millis(1));
            }
        }
        out.finalize()?;
        ensure!(count >= 48000, "Record at least one second of speech");
        let interruptions =
            input.resampler.underruns() + counters.overflows.load(Ordering::Relaxed);
        job.recordings.push(Recording {
            id,
            path: relative,
            name: "Raw microphone take".into(),
            role,
            excluded: interruptions > 0,
            seconds: count as f32 / 48000.,
            clipping: clipped as f32 / count as f32,
            silence: quiet as f32 / count as f32,
        });
        job.save(&folder)?;
        Ok(format!(
            "Raw microphone recorded. Capture interruptions: {interruptions}. {}",
            if interruptions > 0 {
                "This take is excluded by default; audition it or record again."
            } else {
                "Ready for review."
            }
        ))
    })
}
pub fn audition(path: PathBuf, device: Option<String>) -> Task {
    Task::spawn(move |stop| {
        ensure!(
            device.as_ref().is_some_and(|d| !d.is_empty()),
            "Choose a named monitoring device before previewing audio"
        );
        let clip = crate::assets::load_with_limit(&path, 1800)?;
        let errors = Arc::new(crossbeam_queue::ArrayQueue::new(4));
        let output = crate::audio_io::open_output(
            0,
            &device,
            Arc::new(crate::audio_io::Counters::default()),
            errors.clone(),
        )?;
        let mut at = 0;
        while stop.load(Ordering::Acquire) == 0 {
            if let Some(error) = errors.pop() {
                anyhow::bail!("{error}");
            }
            while at < clip.frames.len() && output.queue.len() < crate::audio_io::TARGET_FRAMES {
                if output.queue.push(clip.frames[at]).is_ok() {
                    at += 1
                } else {
                    break;
                }
            }
            if at == clip.frames.len() && output.queue.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        Ok("Monitor preview finished".into())
    })
}
pub fn cleanup(folder: PathBuf, checkpoints: bool) -> Task {
    Task::spawn(move |_| {
        ensure!(
            !HEAVY.load(Ordering::Acquire),
            "Stop training before cleanup"
        );
        #[cfg(windows)]
        let _exclusive = Exclusive::acquire()?;
        let mut job = Job::load(&folder)?;
        ensure!(
            ["draft", "paused", "failed", "cancelled", "complete"].contains(&job.status.as_str()),
            "Cleanup is unavailable for an active or interrupted job; pause/resume it first"
        );
        let mut reclaimed = 0u64;
        for entry in fs::read_dir(&folder)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let selected = if checkpoints {
                name == "teacher.checkpoint.pt"
                    || name == "teacher.checkpoint.pt.previous"
                    || name == "llvc.checkpoint.pt"
                    || name == "llvc.checkpoint.pt.previous"
                    || name == "discriminator-warmup.checkpoint.pt"
                    || name == "discriminator-warmup.checkpoint.pt.previous"
            } else {
                name.starts_with("pair-") && name.ends_with(".wav")
            };
            if selected && entry.file_type()?.is_file() {
                reclaimed += entry.metadata()?.len();
                fs::remove_file(entry.path())?;
            }
        }
        if checkpoints && !["draft", "complete", "cancelled"].contains(&job.status.as_str()) {
            job.status = "cancelled".into();
            job.message = "Checkpoints explicitly removed; this job is no longer resumable.".into();
            job.save(&folder)?;
        }
        Ok(format!(
            "Reclaimed {}. Original recordings and installed voices were preserved.",
            super::bytes(reclaimed)
        ))
    })
}
pub fn start(folder: PathBuf) -> Task {
    Task::spawn(move |control| {
        ensure!(
            !super::AUDIO_RUNNING.load(Ordering::Acquire),
            "Stop audio before starting heavy training work"
        );
        ensure!(
            HEAVY
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "Another training job is already running"
        );
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                HEAVY.store(false, Ordering::Release);
            }
        }
        let _guard = Guard;
        #[cfg(windows)]
        let _exclusive = Exclusive::acquire()?;
        let mut job = Job::load(&folder)?;
        let generation = super::AUDIO_STARTS.load(Ordering::Acquire);
        if job.backend == "cuda" {
            super::hardware::cuda_devices()?;
        }
        ensure!(
            [
                "draft",
                "paused",
                "failed",
                "interrupted",
                "preparing",
                "training_teacher",
                "generating_pairs",
                "training_llvc",
                "exporting",
                "validating"
            ]
            .contains(&job.status.as_str()),
            "This job cannot be started"
        );
        for role in ["train", "validation", "test"] {
            ensure!(
                job.recordings.iter().any(|r| r.role == role && !r.excluded),
                "Add separate training, validation and test recordings"
            );
        }
        ensure!(job.recordings.iter().filter(|r|!r.excluded && r.role=="train").map(|r|r.seconds).sum::<f32>()<=480.,"This Beta supports up to eight minutes of included training speech. Use shorter takes or exclude clips; original recordings are preserved.");
        ensure!(
            job.recordings
                .iter()
                .filter(|r| !r.excluded)
                .map(|r| r.seconds)
                .sum::<f32>()
                <= 1800.,
            "This Beta supports up to thirty minutes of included recordings across all roles"
        );
        space(&folder, JOB_PEAK_BYTES)?;
        verify(&job.runtime, &bundle(&job.backend)?)?;
        verify(&job.shared, &bundle("shared")?)?;
        let inference_runtime = native_runtime()?;
        job.native_worker = inference_runtime.join("PatchVoiceWorker.exe");
        ensure!(
            !super::AUDIO_RUNNING.load(Ordering::Acquire)
                && super::AUDIO_STARTS.load(Ordering::Acquire) == generation
                && control.load(Ordering::Acquire) == 0,
            "Audio started or a pause was requested; training was not launched"
        );
        if folder.join("pause.request").exists() {
            fs::remove_file(folder.join("pause.request"))?;
        }
        job.status = "preparing".into();
        job.message = "Beta: current voices may sound robotic. Local processing only.".into();
        job.save(&folder)?;
        let mut command = Command::new(job.runtime.join("python.exe"));
        hidden(&mut command);
        let paths = std::env::join_paths(std::iter::once(inference_runtime).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        ))?;
        command
            .arg("-B")
            .arg(job.runtime.join("worker/desktop_worker.py"))
            .arg("--job")
            .arg(folder.join("job.json"))
            .env_remove("PYTHONHOME")
            .env_remove("PYTHONPATH")
            .env_remove("VIRTUAL_ENV")
            .env_remove("CONDA_PREFIX")
            .env("PYTHONNOUSERSITE", "1")
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("PATH", paths)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        #[cfg(windows)]
        let _job = match crate::voice::job::Job::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let error_path = folder.join("worker.stderr.txt");
        thread::spawn(move || {
            let mut f = stderr;
            let mut tail = Vec::new();
            let mut buffer = [0; 4096];
            while let Ok(n) = f.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                tail.extend_from_slice(&buffer[..n]);
                if tail.len() > 32768 {
                    tail.drain(..tail.len() - 32768);
                }
            }
            let _ = fs::write(error_path, tail);
        });
        let (tx, rx) = mpsc::sync_channel(16);
        thread::spawn(move || {
            let mut f = std::io::BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match f.by_ref().take(8192).read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    _ => {
                        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&line) {
                            let _ = tx.try_send(value);
                        }
                    }
                }
            }
        });
        let mut deadline = None;
        let mut requested = 0u8;
        let mut last_space = Instant::now();
        let mut first_error: Option<String> = None;
        let mut live_stage = "preparing".to_string();
        let mut last_report = Instant::now();
        loop {
            while let Ok(mut value) = rx.try_recv() {
                if first_error.is_none() {
                    first_error = value
                        .get("error")
                        .and_then(|v| v.as_str())
                        .map(|s| s.chars().take(2000).collect());
                }
                if value.get("chunk").is_some() {
                    value["stage"] = "validating".into();
                }
                if let Some(stage) = value.get("stage").and_then(|s| s.as_str()) {
                    let stage = match stage {
                        "teacher" => "training_teacher",
                        "llvc" => "training_llvc",
                        "export_and_native_validation_pending" => "exporting",
                        other => other,
                    }
                    .to_string();
                    value["stage"] = stage.clone().into();
                    live_stage = stage.clone();
                    if [
                        "preparing",
                        "training_teacher",
                        "generating_pairs",
                        "training_llvc",
                        "exporting",
                        "validating",
                    ]
                    .contains(&stage.as_str())
                        && job.status != stage
                    {
                        job.status = stage;
                        job.save(&folder)?;
                    }
                }
                let _ = save(&folder.join("live.json"), &value);
            }
            if last_report.elapsed() > Duration::from_secs(2)
                && !["exporting", "validating", "complete", "paused", "failed"]
                    .contains(&live_stage.as_str())
            {
                if let Some(report) = fs::read(folder.join("report.json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                {
                    let stage = match report["stage"].as_str().unwrap_or("") {
                        "teacher_training" if report.get("teacher").is_some() => {
                            Some("generating_pairs")
                        }
                        "teacher_feature_preparation" => Some("preparing"),
                        "llvc_content_extractor_loading" => Some("training_llvc"),
                        _ => None,
                    };
                    if let Some(stage) = stage {
                        if live_stage != stage {
                            live_stage = stage.into();
                            job.status = stage.into();
                            job.save(&folder)?;
                            let _ = save(
                                &folder.join("live.json"),
                                &serde_json::json!({"stage":stage}),
                            );
                        }
                    }
                }
                last_report = Instant::now();
            }
            let stop = control.load(Ordering::Acquire);
            if deadline.is_none()
                && (stop != 0
                    || super::AUDIO_RUNNING.load(Ordering::Acquire)
                    || super::AUDIO_STARTS.load(Ordering::Acquire) != generation)
            {
                requested = stop.max(1);
                fs::write(folder.join("pause.request"), b"pause")?;
                deadline = Some(Instant::now() + Duration::from_secs(5));
            }
            if last_space.elapsed() > Duration::from_secs(10) {
                if space(&folder, 128 * 1024 * 1024).is_err() && deadline.is_none() {
                    requested = 1;
                    fs::write(folder.join("pause.request"), b"storage")?;
                    deadline = Some(Instant::now() + Duration::from_secs(5));
                }
                last_space = Instant::now();
            }
            if let Some(status) = child.try_wait()? {
                while let Ok(value) = rx.recv_timeout(Duration::from_millis(50)) {
                    if first_error.is_none() {
                        first_error = value
                            .get("error")
                            .and_then(|v| v.as_str())
                            .map(|s| s.chars().take(2000).collect());
                    }
                }
                let progress: serde_json::Value = fs::read(folder.join("progress.json"))
                    .ok()
                    .and_then(|v| serde_json::from_slice(&v).ok())
                    .unwrap_or_default();
                job.status = if requested == 2 {
                    "cancelled"
                } else if requested != 0 || progress["stage"] == "paused" {
                    "paused"
                } else if status.success() && progress["stage"] == "complete" {
                    "complete"
                } else {
                    "failed"
                }
                .into();
                job.message = if job.status == "complete" {
                    "Beta voice trained; review the samples and install the CPU voice.".into()
                } else if requested != 0 || job.status == "paused" {
                    "Checkpoint pause requested; explicit resume is required.".into()
                } else {
                    first_error.map(|error|format!("Training failed: {error}. Previous checkpoints were preserved.")).unwrap_or_else(||"Worker failed. Previous checkpoints were preserved; see worker.stderr.txt and report.json.".into())
                };
                job.save(&folder)?;
                break;
            }
            if deadline.is_some_and(|d| Instant::now() > d) {
                let _ = child.kill();
                let _ = child.wait();
                job.status = if requested == 2 {
                    "cancelled"
                } else {
                    "paused"
                }
                .into();
                job.message="Worker stopped after the five-second pause deadline. Resume uses the previous valid checkpoint.".into();
                job.save(&folder)?;
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        Ok(job.message)
    })
}

/// Reproducible portable-workflow check using permissioned bundled speech.
pub fn self_test(workspace: PathBuf, offline: PathBuf, backend: String) -> Result<()> {
    for kind in [backend.as_str(), "shared"] {
        let task = install(workspace.clone(), kind.into(), Some(offline.clone()));
        task.result.recv()?.map_err(anyhow::Error::msg)?;
    }
    let name = format!("Release validation {backend}");
    let folder = if let Some((path, _)) = jobs(&workspace)
        .into_iter()
        .find(|(_, j)| j.name == name && j.backend == backend && j.status != "cancelled")
    {
        path
    } else {
        create(&workspace, name, backend.clone())?
    };
    let job = Job::load(&folder)?;
    let mut files: Vec<PathBuf> = walk_files(&job.shared.join("LibriSpeech/dev-clean/1462"))?
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "flac"))
        .collect();
    files.sort();
    ensure!(
        files.len() >= 3,
        "Licensed validation recordings unavailable"
    );
    if job.recordings.is_empty() {
        for (file, role) in files
            .into_iter()
            .take(3)
            .zip(["train", "validation", "test"])
        {
            let task = import(folder.clone(), vec![file], role.into());
            task.result.recv()?.map_err(anyhow::Error::msg)?;
        }
    }
    if job.status != "complete" {
        let task = start(folder.clone());
        // Diagnostic CLI only: exercise the actual application's bounded pause
        // control before a separate invocation resumes the identical job.
        if let Ok(value) = std::env::var("PIRATEWAVE_TRAINING_TEST_PAUSE_AT") {
            let step: u64 = value.parse()?;
            ensure!(
                (1..250).contains(&step),
                "Choose a teacher pause update between 1 and 249"
            );
            loop {
                match task.result.try_recv() {
                    Ok(result) => {
                        result.map_err(anyhow::Error::msg)?;
                        anyhow::bail!("Training finished before the requested pause test");
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        anyhow::bail!("Pause-test worker disconnected")
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
                let live: serde_json::Value = fs::read(folder.join("live.json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or_default();
                if live["stage"] == "training_teacher"
                    && live["step"].as_u64().is_some_and(|v| v >= step)
                {
                    let began = Instant::now();
                    task.pause();
                    task.result.recv()?.map_err(anyhow::Error::msg)?;
                    let paused = Job::load(&folder)?;
                    let seconds = began.elapsed().as_secs_f64();
                    let bytes = fs::metadata(folder.join("teacher.checkpoint.pt"))?.len();
                    ensure!(
                        paused.status == "paused" && seconds < 6. && bytes > 0,
                        "Bounded checkpoint pause failed"
                    );
                    save(
                        &workspace.join(format!("pause-{backend}.json")),
                        &serde_json::json!({"backend":backend,"seconds":seconds,"checkpoint_bytes":bytes,"status":paused.status,"message":paused.message,"pass":true}),
                    )?;
                    println!("Portable {backend} pause passed; explicitly resume without PIRATEWAVE_TRAINING_TEST_PAUSE_AT.");
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
        task.result.recv()?.map_err(anyhow::Error::msg)?;
    }
    let job = Job::load(&folder)?;
    ensure!(job.status == "complete", "{}", job.message);
    let id = crate::voice::local::install(&folder)?;
    crate::voice::local::resolve(&id)?;
    let live = crate::voice::local::check_pipeline(&id, &folder.join("review/3-original.wav"))?;
    save(
        &workspace.join(format!("release-{backend}.json")),
        &serde_json::json!({"schema":1,"backend":backend,"job":folder,"voice":id,
        "import_training_export_install_pass":true,"live_pipeline":live,"clean_windows_qualified":false,"listening_quality":"Current Beta quality accepted by user; not improved"}),
    )?;
    println!("Portable {backend} workflow passed: {}", folder.display());
    Ok(())
}
fn walk_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = vec![];
    while let Some(p) = pending.pop() {
        let metadata = fs::symlink_metadata(&p)?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "Links are not allowed in installed tools"
        );
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                metadata.file_attributes() & 0x400 == 0,
                "Reparse points are not allowed in installed tools"
            );
        }
        for entry in fs::read_dir(p)? {
            let e = entry?;
            let meta = e.file_type()?;
            ensure!(
                !meta.is_symlink(),
                "Links are not allowed in installed tools"
            );
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                ensure!(
                    e.metadata()?.file_attributes() & 0x400 == 0,
                    "Reparse points are not allowed in installed tools"
                );
            }
            if meta.is_dir() {
                pending.push(e.path())
            } else if meta.is_file() {
                files.push(e.path())
            }
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_repair_retains_previous_files_and_rejects_missing_staging() {
        let root = std::env::temp_dir().join(format!("piratewave-repair-{}", nonce()));
        let old = root.join("tools");
        let new = root.join("staging");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir(&new).unwrap();
        fs::write(old.join("runtime"), b"previous bytes").unwrap();
        fs::write(new.join("runtime"), b"verified bytes").unwrap();
        assert!(publish_staged(&root.join("missing"), &old).is_err());
        assert_eq!(fs::read(old.join("runtime")).unwrap(), b"previous bytes");
        let previous = publish_staged(&new, &old).unwrap().unwrap();
        assert_eq!(fs::read(old.join("runtime")).unwrap(), b"verified bytes");
        assert_eq!(
            fs::read(previous.join("runtime")).unwrap(),
            b"previous bytes"
        );
        for directory in [old, previous] {
            fs::remove_file(directory.join("runtime")).unwrap();
            fs::remove_dir(directory).unwrap();
        }
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn tool_verification_rejects_injected_python_and_changed_files() {
        let root = std::env::temp_dir().join(format!("piratewave-runtime-test-{}", nonce()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("worker.py"), b"trusted").unwrap();
        let manifest = serde_json::json!({"schema":1,"files":{"worker.py":{"bytes":7,"sha256":digest(&root.join("worker.py")).unwrap()}}});
        fs::write(
            root.join("runtime-manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let spec = Bundle {
            id: "test".into(),
            backend: "cpu".into(),
            bytes: 0,
            installed_bytes: 0,
            sha256: String::new(),
            manifest_sha256: digest(&root.join("runtime-manifest.json")).unwrap(),
            parts: vec![],
        };
        verify(&root, &spec).unwrap();
        fs::write(root.join("sitecustomize.py"), b"injected").unwrap();
        assert!(verify(&root, &spec).is_err());
        fs::remove_file(root.join("sitecustomize.py")).unwrap();
        fs::write(root.join("worker.py"), b"changed").unwrap();
        assert!(verify(&root, &spec).is_err());
        fs::remove_file(root.join("worker.py")).unwrap();
        fs::remove_file(root.join("runtime-manifest.json")).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn archive_paths_are_restricted_on_windows() {
        for invalid in [
            "../x",
            "C:/x",
            "a\\b",
            "a/../b",
            "/root",
            "a/",
            "x.",
            "a/NUL.txt",
            "CON",
            "LPT1.txt",
        ] {
            assert!(safe_relative(invalid).is_err(), "{invalid}");
        }
        assert!(safe_relative("sources/rvc/configs/v1/32k.json").is_ok());
    }
}
