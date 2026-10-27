//! Windows process-loopback capture. No injection or OBS runtime dependency.
//! The worker owns COM and WASAPI; the graph reads a bounded stereo queue.
use crate::audio_io::{self, Counters, QueueReader};
use anyhow::{bail, Result};
use crossbeam_queue::ArrayQueue;
use piratewave_core::{project::ApplicationTarget, SAMPLE_RATE};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// Bounded recovery policy on the capture worker, never on the render thread.
/// Windows can keep returning silent packets after a process audio graph changes.
/// Silence is not an error: an occasional idle probe also covers that failure mode.
#[cfg(any(windows, test))]
struct CaptureWatchdog {
    last_packet: std::time::Instant,
    last_signal: std::time::Instant,
    retry_seconds: u64,
}
#[cfg(any(windows, test))]
impl CaptureWatchdog {
    fn new(now: std::time::Instant) -> Self {
        Self {
            last_packet: now,
            last_signal: now,
            retry_seconds: 3,
        }
    }
    fn observe(&mut self, now: std::time::Instant, packets: bool, signal: bool) -> bool {
        if packets {
            self.last_packet = now;
        }
        if signal {
            self.last_signal = now;
            self.retry_seconds = 3;
        }
        now.duration_since(self.last_packet) >= std::time::Duration::from_secs(self.retry_seconds)
            || now.duration_since(self.last_signal)
                >= std::time::Duration::from_secs(self.retry_seconds.max(30))
    }
    fn restarted(&mut self, now: std::time::Instant) {
        self.last_packet = now;
        self.last_signal = now;
        self.retry_seconds = (self.retry_seconds * 2).min(60);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub pid: u32,
    pub executable: String,
    pub title: String,
}

fn same_executable(a: &str, b: &str) -> bool {
    a.replace('/', "\\")
        .eq_ignore_ascii_case(&b.replace('/', "\\"))
}

fn resolve<'a>(
    target: &ApplicationTarget,
    candidates: &'a [Application],
) -> Option<&'a Application> {
    candidates
        .iter()
        .filter(|app| {
            same_executable(&app.executable, &target.executable)
                && (!target.match_title || app.title.eq_ignore_ascii_case(&target.window_title))
        })
        .min_by_key(|app| {
            (
                !app.title.eq_ignore_ascii_case(&target.window_title),
                app.title.is_empty(),
                app.pid,
            )
        })
}

pub struct ApplicationStream {
    pub resampler: QueueReader,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for ApplicationStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

#[cfg(windows)]
pub use platform::applications;
#[cfg(not(windows))]
pub fn applications() -> Result<Vec<Application>> {
    Ok(Vec::new())
}

pub fn open(
    target: &ApplicationTarget,
    counters: Arc<Counters>,
    errors: Arc<ArrayQueue<String>>,
) -> Result<ApplicationStream> {
    if target.executable.trim().is_empty() {
        bail!("Choose an application to capture first");
    }
    #[cfg(not(windows))]
    {
        let _ = (target, counters, errors);
        bail!("Application Capture requires Windows 10 build 20348 or later");
    }
    #[cfg(windows)]
    {
        let queue = Arc::new(ArrayQueue::new(audio_io::QUEUE_CAPACITY));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let (target, queue, stop) = (target.clone(), queue.clone(), stop.clone());
            std::thread::Builder::new()
                .name("piratewave-application-capture".into())
                .spawn(move || platform::run(target, queue, counters, errors, stop))?
        };
        Ok(ApplicationStream {
            resampler: QueueReader::new(queue, SAMPLE_RATE, SAMPLE_RATE, audio_io::TARGET_FRAMES),
            stop,
            worker: Some(worker),
        })
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use anyhow::Context;
    use piratewave_core::Frame;
    use std::{
        ffi::c_void,
        ptr,
        sync::{atomic::AtomicU32, mpsc},
        time::{Duration, Instant},
    };
    use windows::{
        core::{IUnknown, IUnknown_Vtbl, Interface, GUID, HRESULT, PCWSTR, PROPVARIANT},
        Win32::{
            Foundation::{CloseHandle, HANDLE, WAIT_FAILED},
            Media::Audio::*,
            System::{
                Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
                Threading::{CreateEventW, WaitForSingleObject},
            },
        },
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle as SysCloseHandle, BOOL, HWND, LPARAM},
        System::{
            Diagnostics::ToolHelp::*,
            Threading::{
                GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
            IsWindowVisible,
        },
    };

    fn executable(pid: u32) -> Option<String> {
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let mut path = vec![0u16; 32768];
            let mut size = path.len() as u32;
            let ok = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                path.as_mut_ptr(),
                &mut size,
            );
            SysCloseHandle(process);
            (ok != 0).then(|| String::from_utf16_lossy(&path[..size as usize]))
        }
    }
    unsafe extern "system" fn window(hwnd: HWND, context: LPARAM) -> BOOL {
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let length = GetWindowTextLengthW(hwnd);
        if length <= 0 {
            return 1;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == GetCurrentProcessId() {
            return 1;
        }
        let Some(executable) = executable(pid) else {
            return 1;
        };
        let mut title = vec![0u16; length as usize + 1];
        let count = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
        if count > 0 {
            let apps = &mut *(context as *mut Vec<Application>);
            apps.push(Application {
                pid,
                executable,
                title: String::from_utf16_lossy(&title[..count as usize]),
            });
        }
        1
    }
    pub fn applications() -> Result<Vec<Application>> {
        let mut apps = Vec::<Application>::new();
        unsafe {
            if EnumWindows(Some(window), &mut apps as *mut _ as isize) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        apps.sort_by(|a, b| {
            a.executable
                .to_lowercase()
                .cmp(&b.executable.to_lowercase())
                .then(a.title.cmp(&b.title))
                .then(a.pid.cmp(&b.pid))
        });
        apps.dedup();
        Ok(apps)
    }
    fn candidates(target: &ApplicationTarget) -> Result<Vec<Application>> {
        let mut apps = applications()?;
        if target.match_title {
            return Ok(apps);
        }
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error().into());
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut more = Process32FirstW(snapshot, &mut entry);
            while more != 0 {
                let pid = entry.th32ProcessID;
                if pid != GetCurrentProcessId() && !apps.iter().any(|a| a.pid == pid) {
                    if let Some(executable) = executable(pid) {
                        if same_executable(&executable, &target.executable) {
                            apps.push(Application {
                                pid,
                                executable,
                                title: String::new(),
                            });
                        }
                    }
                }
                more = Process32NextW(snapshot, &mut entry);
            }
            SysCloseHandle(snapshot);
        }
        Ok(apps)
    }

    // An owned COM result can cross only MTA worker threads. Dropping a cancelled
    // activation result releases its COM reference, including late callbacks.
    struct Activated(usize);
    unsafe impl Send for Activated {}
    impl Drop for Activated {
        fn drop(&mut self) {
            if self.0 != 0 {
                unsafe {
                    drop(IUnknown::from_raw(self.0 as *mut c_void));
                }
            }
        }
    }
    impl Activated {
        unsafe fn into_client(mut self) -> windows::core::Result<IAudioClient> {
            let unknown = IUnknown::from_raw(self.0 as *mut c_void);
            self.0 = 0;
            unknown.cast()
        }
    }
    #[repr(C)]
    struct Blob {
        size: u32,
        data: *mut u8,
    }
    #[repr(C)]
    struct ActivationVariant {
        vt: u16,
        reserved: [u16; 3],
        blob: Blob,
    }
    #[repr(C)]
    struct Completion {
        vtable: *const IActivateAudioInterfaceCompletionHandler_Vtbl,
        refs: AtomicU32,
        sender: mpsc::SyncSender<std::result::Result<Activated, HRESULT>>,
        parameters: AUDIOCLIENT_ACTIVATION_PARAMS,
        variant: ActivationVariant,
    }
    const S_OK: HRESULT = HRESULT(0);
    const E_POINTER: HRESULT = HRESULT(0x80004003u32 as i32);
    const E_NOINTERFACE: HRESULT = HRESULT(0x80004002u32 as i32);
    const AGILE: GUID = GUID::from_u128(0x94ea2b94_e9cc_49e0_c0ff_ee64ca8f5b90);
    unsafe extern "system" fn query(
        this: *mut c_void,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> HRESULT {
        if iid.is_null() || out.is_null() {
            return E_POINTER;
        }
        *out = ptr::null_mut();
        if *iid == IUnknown::IID
            || *iid == IActivateAudioInterfaceCompletionHandler::IID
            || *iid == AGILE
        {
            *out = this;
            add_ref(this);
            S_OK
        } else {
            E_NOINTERFACE
        }
    }
    unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
        (*(this as *mut Completion))
            .refs
            .fetch_add(1, Ordering::Relaxed)
            + 1
    }
    unsafe extern "system" fn release(this: *mut c_void) -> u32 {
        let count = (*(this as *mut Completion))
            .refs
            .fetch_sub(1, Ordering::Release)
            - 1;
        if count == 0 {
            std::sync::atomic::fence(Ordering::Acquire);
            drop(Box::from_raw(this as *mut Completion));
        }
        count
    }
    unsafe extern "system" fn completed(this: *mut c_void, operation: *mut c_void) -> HRESULT {
        if operation.is_null() {
            return E_POINTER;
        }
        // Borrow the callback argument; Windows owns this reference.
        let operation =
            std::mem::ManuallyDrop::new(IActivateAudioInterfaceAsyncOperation::from_raw(operation));
        let mut status = S_OK;
        let mut unknown = None;
        let result = operation
            .GetActivateResult(&mut status, &mut unknown)
            .and_then(|_| status.ok())
            .and_then(|_| unknown.ok_or_else(|| windows::core::Error::from(E_POINTER)))
            .map(|object| Activated(object.into_raw() as usize))
            .map_err(|error| error.code());
        let _ = (*(this as *mut Completion)).sender.try_send(result);
        S_OK
    }
    static COMPLETION_VTABLE: IActivateAudioInterfaceCompletionHandler_Vtbl =
        IActivateAudioInterfaceCompletionHandler_Vtbl {
            base__: IUnknown_Vtbl {
                QueryInterface: query,
                AddRef: add_ref,
                Release: release,
            },
            ActivateCompleted: completed,
        };
    unsafe fn activate(pid: u32, stop: &AtomicBool) -> Result<Option<IAudioClient>> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut completion = Box::new(Completion {
            vtable: &COMPLETION_VTABLE,
            refs: AtomicU32::new(1),
            sender,
            parameters: AUDIOCLIENT_ACTIVATION_PARAMS {
                ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
                Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                    ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                        TargetProcessId: pid,
                        ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                    },
                },
            },
            variant: ActivationVariant {
                vt: 65,
                reserved: [0; 3],
                blob: Blob {
                    size: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                    data: ptr::null_mut(),
                },
            },
        });
        completion.variant.blob.data = &mut completion.parameters as *mut _ as *mut u8;
        let variant = &completion.variant as *const _ as *const PROPVARIANT;
        let handler = IActivateAudioInterfaceCompletionHandler::from_raw(
            Box::into_raw(completion) as *mut c_void
        );
        let _operation = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(variant),
            &handler,
        )
        .context("Process-loopback activation failed (requires Windows 10 build 20348 or later)")?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if stop.load(Ordering::Acquire) {
                return Ok(None);
            }
            match receiver.recv_timeout(Duration::from_millis(25)) {
                Ok(Ok(value)) => return Ok(Some(value.into_client()?)),
                Ok(Err(code)) => {
                    return Err(windows::core::Error::from(code))
                        .context("Process-loopback GetActivateResult")
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    bail!("Application capture activation callback disconnected")
                }
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                    bail!("Application capture activation timed out")
                }
                Err(_) => {}
            }
        }
    }
    struct Event(HANDLE);
    impl Drop for Event {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    struct Capture {
        client: IAudioClient,
        capture: IAudioCaptureClient,
        event: Event,
    }
    impl Drop for Capture {
        fn drop(&mut self) {
            unsafe {
                let _ = self.client.Stop();
            }
        }
    }
    impl Capture {
        unsafe fn start(pid: u32, stop: &AtomicBool) -> Result<Option<Self>> {
            let Some(client) = activate(pid, stop)? else {
                return Ok(None);
            };
            // Process-loopback does not implement GetMixFormat. Request a fixed,
            // shared-mode float format, converted by the Windows audio engine.
            let format = WAVEFORMATEX {
                wFormatTag: 3,
                nChannels: 2,
                nSamplesPerSec: SAMPLE_RATE,
                nAvgBytesPerSec: SAMPLE_RATE * 8,
                nBlockAlign: 8,
                wBitsPerSample: 32,
                cbSize: 0,
            };
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK
                        | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
                    200_000,
                    0,
                    &format,
                    None,
                )
                .context("Process-loopback IAudioClient::Initialize")?;
            let event = Event(CreateEventW(None, false, false, PCWSTR::null())?);
            client.SetEventHandle(event.0)?;
            let capture = client.GetService::<IAudioCaptureClient>()?;
            client.Start()?;
            Ok(Some(Self {
                client,
                capture,
                event,
            }))
        }
        unsafe fn read(&self, subscribers: &std::sync::Mutex<HubState>) -> Result<(bool, bool)> {
            if WaitForSingleObject(self.event.0, 25) == WAIT_FAILED {
                return Err(std::io::Error::last_os_error().into());
            }
            let subscribers = subscribers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            #[cfg(test)]
            if subscribers.force_failure.swap(false, Ordering::Relaxed) {
                bail!("Injected private capture failure");
            }
            let (mut packets, mut signal) = (false, false);
            while self.capture.GetNextPacketSize()? > 0 {
                packets = true;
                let (mut data, mut frames, mut flags) = (ptr::null_mut(), 0, 0);
                self.capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null();
                for i in 0..frames as usize {
                    let frame = if silent {
                        [0.0; 2]
                    } else {
                        let left = ptr::read_unaligned(data.cast::<f32>().add(i * 2));
                        let right = ptr::read_unaligned(data.cast::<f32>().add(i * 2 + 1));
                        [
                            if left.is_finite() { left } else { 0.0 },
                            if right.is_finite() { right } else { 0.0 },
                        ]
                    };
                    signal = signal || frame != [0.0; 2];
                    for subscriber in &subscribers.subscribers {
                        if subscriber.queue.push(frame).is_err() {
                            subscriber
                                .counters
                                .overflows
                                .fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                for subscriber in &subscribers.subscribers {
                    subscriber
                        .counters
                        .captured
                        .fetch_add(frames as u64, Ordering::Relaxed);
                }
                self.capture.ReleaseBuffer(frames)?;
            }
            Ok((packets, signal))
        }
    }
    struct Subscriber {
        id: u64,
        queue: Arc<ArrayQueue<Frame>>,
        counters: Arc<Counters>,
        errors: Arc<ArrayQueue<String>>,
    }
    #[derive(Default)]
    struct HubState {
        #[cfg(test)]
        force_failure: AtomicBool,
        subscribers: Vec<Subscriber>,
        message: Option<String>,
    }
    struct Hub {
        state: Arc<std::sync::Mutex<HubState>>,
        stop: Arc<AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
    }
    impl Drop for Hub {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                worker.thread().unpark();
                let _ = worker.join();
            }
        }
    }
    fn hubs() -> &'static std::sync::Mutex<std::collections::HashMap<u32, Hub>> {
        static HUBS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<u32, Hub>>> =
            std::sync::OnceLock::new();
        HUBS.get_or_init(Default::default)
    }
    #[cfg(test)]
    pub(super) fn inject_capture_failure(pid: u32) {
        let hubs = hubs().lock().unwrap();
        let hub = hubs.get(&pid).expect("Native fixture capture is active");
        hub.state
            .lock()
            .unwrap()
            .force_failure
            .store(true, Ordering::Relaxed);
    }
    struct Subscription {
        pid: u32,
        id: u64,
    }
    impl Subscription {
        fn is_alive(&self) -> bool {
            hubs()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&self.pid)
                .is_some_and(|hub| {
                    hub.worker
                        .as_ref()
                        .is_some_and(|worker| !worker.is_finished())
                })
        }
    }
    impl Drop for Subscription {
        fn drop(&mut self) {
            let mut hubs = hubs()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(hub) = hubs.get_mut(&self.pid) {
                let empty = {
                    let mut state = hub
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    state
                        .subscribers
                        .retain(|subscriber| subscriber.id != self.id);
                    state.subscribers.is_empty()
                };
                // Serialize the final Stop/release with creating a new client for
                // this PID. Initializing two process-loopback clients for one PID
                // concurrently fails with E_UNEXPECTED on tested Windows builds.
                if empty {
                    hubs.remove(&self.pid);
                }
            }
        }
    }
    fn report(state: &std::sync::Mutex<HubState>, message: String) {
        let mut state = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.message.as_ref() != Some(&message) {
            for subscriber in &state.subscribers {
                let _ = subscriber.errors.push(message.clone());
            }
            state.message = Some(message);
        }
    }
    fn subscribe(
        pid: u32,
        queue: Arc<ArrayQueue<Frame>>,
        counters: Arc<Counters>,
        errors: Arc<ArrayQueue<String>>,
    ) -> Result<Subscription> {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let mut hubs = hubs()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let std::collections::hash_map::Entry::Vacant(entry) = hubs.entry(pid) {
            let state = Arc::new(std::sync::Mutex::new(HubState::default()));
            let stop = Arc::new(AtomicBool::new(false));
            let worker = {
                let (state, stop) = (state.clone(), stop.clone());
                std::thread::Builder::new()
                    .name(format!("piratewave-process-loopback-{pid}"))
                    .spawn(move || capture_worker(pid, state, stop))?
            };
            entry.insert(Hub {
                state,
                stop,
                worker: Some(worker),
            });
        }
        let hub = hubs.get_mut(&pid).expect("Capture hub inserted");
        if hub
            .worker
            .as_ref()
            .is_none_or(|worker| worker.is_finished())
        {
            if let Some(worker) = hub.worker.take() {
                let _ = worker.join();
            }
            let (state, stop) = (hub.state.clone(), hub.stop.clone());
            hub.worker = Some(
                std::thread::Builder::new()
                    .name(format!("piratewave-process-loopback-{pid}"))
                    .spawn(move || capture_worker(pid, state, stop))?,
            );
        }
        let mut state = hub
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(message) = &state.message {
            let _ = errors.push(message.clone());
        }
        state.subscribers.push(Subscriber {
            id,
            queue,
            counters,
            errors,
        });
        Ok(Subscription { pid, id })
    }
    fn capture_worker(pid: u32, state: Arc<std::sync::Mutex<HubState>>, stop: Arc<AtomicBool>) {
        unsafe {
            if let Err(error) = CoInitializeEx(None, COINIT_MULTITHREADED).ok() {
                report(
                    &state,
                    format!("Application Capture COM initialization: {error}"),
                );
                return;
            }
            let mut capture: Option<Capture> = None;
            let mut watchdog = CaptureWatchdog::new(Instant::now());
            while !stop.load(Ordering::Acquire) {
                if capture.is_none() {
                    match Capture::start(pid, &stop) {
                        Ok(Some(value)) => {
                            capture = Some(value);
                            report(&state, format!("Application Capture connected (PID {pid})"));
                        }
                        Ok(None) => break,
                        Err(error) => {
                            report(
                                &state,
                                format!("Cannot capture application PID {pid}: {error:#}"),
                            );
                            std::thread::park_timeout(Duration::from_secs(1));
                            continue;
                        }
                    }
                }
                if let Some(value) = &capture {
                    let reason = match value.read(&state) {
                        Ok((packets, signal))
                            if watchdog.observe(Instant::now(), packets, signal) =>
                        {
                            Some("idle/stalled stream health probe".to_owned())
                        }
                        Ok(_) => None,
                        Err(error) => Some(format!("{error:#}")),
                    };
                    if let Some(reason) = reason {
                        // Release the old COM client/event before initializing its replacement.
                        capture = None;
                        watchdog.restarted(Instant::now());
                        crate::logging::info(format_args!(
                            "Application Capture reconnecting (PID {pid}): {reason}"
                        ));
                        report(
                            &state,
                            format!("Application Capture reconnecting (PID {pid}): {reason}"),
                        );
                        std::thread::park_timeout(Duration::from_millis(250));
                    }
                }
            }
            drop(capture);
            CoUninitialize();
        }
    }
    pub(super) fn run(
        target: ApplicationTarget,
        queue: Arc<ArrayQueue<Frame>>,
        counters: Arc<Counters>,
        errors: Arc<ArrayQueue<String>>,
        stop: Arc<AtomicBool>,
    ) {
        let mut active: Option<Subscription> = None;
        let mut last_report = String::new();
        let mut next_scan = Instant::now();
        while !stop.load(Ordering::Acquire) {
            let mut issue = None;
            if Instant::now() >= next_scan {
                next_scan = Instant::now() + Duration::from_secs(1);
                let selected =
                    candidates(&target).map(|apps| resolve(&target, &apps).map(|app| app.pid));
                match selected {
                    Ok(Some(pid))
                        if active
                            .as_ref()
                            .is_some_and(|capture| capture.pid == pid && capture.is_alive()) => {}
                    Ok(Some(pid)) => {
                        active = None;
                        while queue.pop().is_some() {}
                        match subscribe(pid, queue.clone(), counters.clone(), errors.clone()) {
                            Ok(stream) => {
                                active = Some(stream);
                                last_report.clear();
                            }
                            Err(error) => {
                                issue =
                                    Some(format!("Cannot capture {}: {error:#}", target.executable))
                            }
                        }
                    }
                    Ok(None) => {
                        active = None;
                        while queue.pop().is_some() {}
                        issue = Some(format!("Waiting for application: {}", target.executable));
                    }
                    Err(error) => {
                        active = None;
                        issue = Some(format!("Application discovery: {error:#}"));
                    }
                }
            }
            if let Some(issue) = issue {
                if issue != last_report {
                    crate::logging::warn(format_args!("Application capture: {issue}"));
                    let _ = errors.push(issue.clone());
                    last_report = issue;
                }
            }
            std::thread::park_timeout(Duration::from_millis(25));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn watchdog_recovers_stalls_and_silent_packets_without_resetting_active_audio() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let mut watchdog = CaptureWatchdog::new(start);
        for second in 0..120 {
            assert!(!watchdog.observe(start + Duration::from_secs(second), true, true));
        }
        assert!(!watchdog.observe(start + Duration::from_secs(121), false, false));
        assert!(watchdog.observe(start + Duration::from_secs(122), false, false));
        watchdog.restarted(start + Duration::from_secs(122));
        assert!(!watchdog.observe(start + Duration::from_secs(127), false, false));
        assert!(watchdog.observe(start + Duration::from_secs(128), false, false));
        let mut watchdog = CaptureWatchdog::new(start);
        for second in 0..30 {
            assert!(!watchdog.observe(start + Duration::from_secs(second), true, false));
        }
        assert!(watchdog.observe(start + Duration::from_secs(30), true, false));
        for _ in 0..100 {
            watchdog.restarted(start);
        }
        assert_eq!(watchdog.retry_seconds, 60);
        assert!(!watchdog.observe(start + Duration::from_secs(59), true, false));
        assert!(watchdog.observe(start + Duration::from_secs(60), true, false));
        assert!(!watchdog.observe(start + Duration::from_secs(61), true, true));
        assert_eq!(watchdog.retry_seconds, 3);
    }
    fn target(strict: bool) -> ApplicationTarget {
        ApplicationTarget {
            executable: "C:\\Apps\\Player.exe".into(),
            window_title: "Live".into(),
            match_title: strict,
        }
    }
    #[test]
    fn executable_matching_uses_full_path_and_windows_case() {
        assert!(same_executable(
            "c:/apps/PLAYER.exe",
            "C:\\Apps\\Player.exe"
        ));
        assert!(!same_executable(
            "C:\\Other\\Player.exe",
            "C:\\Apps\\Player.exe"
        ));
    }
    #[test]
    fn title_matching_is_optional_and_resolution_deterministic() {
        let apps = vec![
            Application {
                pid: 80,
                executable: target(false).executable,
                title: "Other".into(),
            },
            Application {
                pid: 100,
                executable: target(false).executable,
                title: "LIVE".into(),
            },
        ];
        // A title remains a useful hint even when strict matching is disabled:
        // choosing the second instance should not attach to the oldest PID.
        assert_eq!(resolve(&target(false), &apps).unwrap().pid, 100);
        assert_eq!(resolve(&target(true), &apps).unwrap().pid, 100);
        assert!(resolve(&target(true), &apps[..1]).is_none());
        // A changed title still falls back to the same executable by default.
        assert_eq!(resolve(&target(false), &apps[..1]).unwrap().pid, 80);
        let mut changed = target(false);
        changed.window_title = "Previous title".into();
        assert_eq!(resolve(&changed, &apps).unwrap().pid, 80);
    }
    #[test]
    fn process_restart_resolves_new_pid() {
        let mut apps = vec![Application {
            pid: 12,
            executable: target(false).executable,
            title: String::new(),
        }];
        assert_eq!(resolve(&target(false), &apps).unwrap().pid, 12);
        apps[0].pid = 900;
        assert_eq!(resolve(&target(false), &apps).unwrap().pid, 900);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "Opt-in Windows audio test; uses installed virtual cable and owned tone fixture"]
    fn concurrent_capture_of_same_process() -> Result<()> {
        use std::{
            os::windows::process::CommandExt,
            process::Command,
            time::{Duration, Instant},
        };
        let device = audio_io::devices()?
            .outputs
            .into_iter()
            .find(|device| device.name.contains("CABLE Input"))
            .ok_or_else(|| anyhow::anyhow!("Virtual cable required"))?;
        let fixture = std::env::current_dir()?.join("target/debug/piratewave-desktop.exe");
        let title = format!("PirateWave concurrent capture {}", std::process::id());
        let mut child = Command::new(&fixture)
            .args(["--application-capture-tone", &title, "440", &device.id])
            .creation_flags(0x08000000)
            .spawn()?;
        struct Fixture<'a>(&'a mut std::process::Child, String);
        impl Drop for Fixture<'_> {
            fn drop(&mut self) {
                unsafe {
                    use windows_sys::Win32::UI::WindowsAndMessaging::*;
                    let title: Vec<_> = self.1.encode_utf16().chain(Some(0)).collect();
                    let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
                    if !hwnd.is_null() {
                        PostMessageW(hwnd, WM_CLOSE, 0, 0);
                    }
                }
                let deadline = Instant::now() + Duration::from_secs(2);
                while Instant::now() < deadline {
                    if matches!(self.0.try_wait(), Ok(Some(_))) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let fixture_pid = child.id();
        let _fixture = Fixture(&mut child, title.clone());
        let target = ApplicationTarget {
            // Resolve redirected build directories to the path reported by Windows.
            executable: std::fs::canonicalize(&fixture)?
                .to_string_lossy()
                .trim_start_matches(r"\\?\")
                .to_owned(),
            window_title: title,
            match_title: true,
        };
        let mut streams = Vec::new();
        let mut reports = Vec::new();
        for _ in 0..2 {
            let report = Arc::new(ArrayQueue::new(32));
            streams.push(open(
                &target,
                Arc::new(Counters::default()),
                report.clone(),
            )?);
            reports.push(report);
        }
        let mut counts = [0usize; 2];
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(6) {
            for (index, stream) in streams.iter_mut().enumerate() {
                for _ in 0..1024 {
                    if let Some(frame) = stream.resampler.next_frame() {
                        if frame[0].abs() > 0.01 {
                            counts[index] += 1;
                        }
                    } else {
                        break;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let messages: Vec<Vec<_>> = reports
            .iter()
            .map(|queue| std::iter::from_fn(|| queue.pop()).collect())
            .collect();
        println!("Concurrent captured samples {counts:?}, messages {messages:?}");
        anyhow::ensure!(
            counts.iter().all(|count| *count > 48000),
            "Concurrent capture failed: {counts:?} {messages:?}"
        );
        streams.remove(0);
        let start = Instant::now();
        platform::inject_capture_failure(fixture_pid);
        let mut count = 0;
        while start.elapsed() < Duration::from_secs(2) {
            for _ in 0..1024 {
                if streams[0]
                    .resampler
                    .next_frame()
                    .is_some_and(|frame| frame[0].abs() > 0.01)
                {
                    count += 1;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        anyhow::ensure!(
            count > 12000,
            "Dropping one concurrent reader stopped the other"
        );
        anyhow::ensure!(
            std::iter::from_fn(|| reports[1].pop())
                .any(|message| message.contains("Injected private capture failure")),
            "Recovery was not exercised"
        );
        // A subscriber can join an already initialized client. Its queue is
        // independent, so an unread/full queue must not steal another's audio.
        let late_messages = Arc::new(ArrayQueue::new(16));
        let mut late = open(
            &target,
            Arc::new(Counters::default()),
            late_messages.clone(),
        )?;
        let start = Instant::now();
        let mut late_count = 0;
        while start.elapsed() < Duration::from_secs(2) {
            for _ in 0..1024 {
                if late
                    .resampler
                    .next_frame()
                    .is_some_and(|frame| frame[0].abs() > 0.01)
                {
                    late_count += 1;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        anyhow::ensure!(late_count > 24000, "Late subscriber did not capture");
        drop(late);
        drop(streams);
        // Last-subscriber teardown must completely release WASAPI before a
        // fresh hub for the same PID initializes its replacement client.
        let mut reopened = open(&target, Arc::new(Counters::default()), late_messages)?;
        let start = Instant::now();
        let mut reopened_count = 0;
        while start.elapsed() < Duration::from_secs(2) {
            for _ in 0..1024 {
                if reopened
                    .resampler
                    .next_frame()
                    .is_some_and(|frame| frame[0].abs() > 0.01)
                {
                    reopened_count += 1;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        anyhow::ensure!(
            reopened_count > 24000,
            "Capture did not reopen after final subscriber drop"
        );
        Ok(())
    }
}
