//! VST3 control and editor thread, with DSP state exclusively on the engine thread.
use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Mutex,
};
use std::thread;
use std::time::Duration;

enum Command {
    Open,
    Close,
    SyncParameters,
    Capture(mpsc::SyncSender<Result<Option<piratewave_core::plugins::PluginState>, String>>),
    Restore(
        piratewave_core::plugins::PluginState,
        mpsc::SyncSender<Result<(), String>>,
    ),
    Shutdown,
    #[cfg(debug_assertions)]
    SimulatePanic,
    #[cfg(debug_assertions)]
    TestKey(u32),
    #[cfg(debug_assertions)]
    TestResize(i32, i32),
}
type StateReply = mpsc::Receiver<Result<Option<piratewave_core::plugins::PluginState>, String>>;
pub(in crate::plugins) struct Threaded {
    audio: Option<AudioState>,
    descriptor: PluginDescriptor,
    tx: mpsc::Sender<Command>,
    editor_open: Arc<AtomicBool>,
    parameters_queued: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
    editor_size: Arc<AtomicU64>,
    editor_resizable: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    edits: Arc<crossbeam_queue::ArrayQueue<(u32, f64, u64)>>,
    pending_capture: Option<StateReply>,
    capture_started: Option<Instant>,
    editor_used: bool,
    editor_started: Option<Instant>,
    last_values: std::collections::HashMap<u32, f64>,
}
impl Threaded {
    pub(in crate::plugins) fn load(path: &Path, class_id: &str) -> Result<Self> {
        let path = path.to_owned();
        let class_id = class_id.to_owned();
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let editor_open = Arc::new(AtomicBool::new(false));
        let open_thread = editor_open.clone();
        let parameters_queued = Arc::new(AtomicBool::new(false));
        let parameters_queued_thread = parameters_queued.clone();
        let alive = Arc::new(AtomicBool::new(true));
        let alive_thread = alive.clone();
        let editor_size = Arc::new(AtomicU64::new(0));
        let size_thread = editor_size.clone();
        let editor_resizable = Arc::new(AtomicBool::new(false));
        let resizable_thread = editor_resizable.clone();
        let error = Arc::new(Mutex::new(None));
        let error_thread = error.clone();
        let edits = Arc::new(crossbeam_queue::ArrayQueue::new(4096));
        let edits_thread = edits.clone();
        thread::Builder::new()
            .name("piratewave-vst3-control".into())
            .spawn(move || {
                let _thread = PluginThread::default();
                let mut plugin = match Vst3::load(&path, &class_id) {
                    Ok(plugin) => plugin,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("{e:#}")));
                        return;
                    }
                };
                plugin
                    .handler
                    .state
                    .initialize_parameters(plugin.descriptor.parameters.iter().map(|p| p.id));
                let audio = plugin.take_audio_state();
                let descriptor = plugin.descriptor.clone();
                if ready_tx.send(Ok((audio, descriptor))).is_err() {
                    return;
                }
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut was_open = false;
                    loop {
                        pump_editor_messages(&mut plugin);
                        plugin.editor_idle();
                        let (width, height) = plugin.editor_size();
                        let resizable = plugin
                            .editor
                            .as_ref()
                            .is_some_and(|(view, _, _)| unsafe { view.canResize() == kResultOk });
                        resizable_thread.store(resizable, Ordering::Release);
                        size_thread
                            .store(((width as u64) << 32) | height as u64, Ordering::Release);
                        if was_open && !plugin.editor_open() {
                            open_thread.store(false, Ordering::Release);
                            was_open = false;
                        }
                        for edit in plugin.take_versioned_editor_edits() {
                            if edits_thread.push(edit).is_err() {
                                if let Ok(mut error) = error_thread.lock() {
                                    *error = Some("VST3 editor parameter queue is full".into());
                                }
                                break;
                            }
                        }
                        match rx.recv_timeout(Duration::from_millis(10)) {
                            Ok(Command::Open) => {
                                if let Err(e) = plugin.open_editor() {
                                    if let Ok(mut error) = error_thread.lock() {
                                        *error = Some(format!("{e:#}"));
                                    }
                                }
                                was_open = plugin.editor_open();
                                open_thread.store(was_open, Ordering::Release);
                            }
                            Ok(Command::Close) => {
                                plugin.close_editor();
                                was_open = false;
                                open_thread.store(false, Ordering::Release);
                            }
                            Ok(Command::SyncParameters) => {
                                parameters_queued_thread.store(false, Ordering::Release);
                                for (id, value, sequence) in
                                    plugin.handler.state.pending_host_values()
                                {
                                    let applied = plugin.handler.state.current(id, sequence);
                                    if applied {
                                        plugin.set_parameter(id, value);
                                    }
                                    plugin.handler.state.finish_host(
                                        id,
                                        sequence,
                                        applied.then_some(value),
                                    );
                                }
                            }
                            Ok(Command::Capture(reply)) => {
                                let _ = reply
                                    .send(plugin.capture_state().map_err(|e| format!("{e:#}")));
                            }
                            Ok(Command::Restore(state, reply)) => {
                                let _ = reply.send(
                                    plugin.restore_state(&state).map_err(|e| format!("{e:#}")),
                                );
                            }
                            #[cfg(debug_assertions)]
                            Ok(Command::SimulatePanic) => {
                                panic!("Intentional VST3 control-thread failure for diagnostics")
                            }
                            #[cfg(debug_assertions)]
                            Ok(Command::TestKey(key)) => {
                                if let Some((_, _, window)) = &plugin.editor {
                                    unsafe {
                                        use windows_sys::Win32::UI::WindowsAndMessaging::{
                                            PostMessageW, WM_KEYDOWN, WM_KEYUP,
                                        };
                                        PostMessageW(
                                            window.handle().cast(),
                                            WM_KEYDOWN,
                                            key as usize,
                                            0,
                                        );
                                        PostMessageW(
                                            window.handle().cast(),
                                            WM_KEYUP,
                                            key as usize,
                                            0,
                                        );
                                    }
                                }
                            }
                            #[cfg(debug_assertions)]
                            Ok(Command::TestResize(width, height)) => {
                                if let Some((_, _, window)) = &plugin.editor {
                                    let _ = window.resize(width, height);
                                }
                            }
                            Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                                break
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                        }
                    }
                }));
                if result.is_err() {
                    alive_thread.store(false, Ordering::Release);
                    open_thread.store(false, Ordering::Release);
                    if let Ok(mut error) = error_thread.lock() {
                        *error = Some("VST3 control thread failed; plugin will be reloaded".into());
                    }
                    // Keep the component and library alive until the engine releases its
                    // processor pointer, even if UI handling panicked.
                    while let Ok(command) = rx.recv() {
                        match command {
                            Command::Shutdown => break,
                            Command::Capture(reply) => {
                                let _ = reply.send(Err("VST3 control thread failed".into()));
                            }
                            Command::Restore(_, reply) => {
                                let _ = reply.send(Err("VST3 control thread failed".into()));
                            }
                            _ => {}
                        }
                    }
                }
            })?;
        let (audio, descriptor) = ready_rx
            .recv_timeout(Duration::from_secs(30))
            .map_err(|e| anyhow::anyhow!("VST3 control thread did not start: {e}"))?
            .map_err(anyhow::Error::msg)?;
        Ok(Self {
            audio: Some(audio),
            last_values: descriptor
                .parameters
                .iter()
                .map(|parameter| (parameter.id, parameter.default))
                .collect(),
            descriptor,
            tx,
            editor_open,
            parameters_queued,
            alive,
            editor_size,
            editor_resizable,
            error,
            edits,
            pending_capture: None,
            capture_started: None,
            editor_used: false,
            editor_started: None,
        })
    }
    #[cfg(debug_assertions)]
    pub(in crate::plugins) fn simulate_failure_for_diagnostics(&self) -> Result<()> {
        self.tx
            .send(Command::SimulatePanic)
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
    #[cfg(debug_assertions)]
    pub(in crate::plugins) fn send_key_for_diagnostics(&self, key: u32) -> Result<()> {
        self.tx
            .send(Command::TestKey(key))
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
    #[cfg(debug_assertions)]
    pub(in crate::plugins) fn resize_for_diagnostics(&self, width: i32, height: i32) -> Result<()> {
        self.tx
            .send(Command::TestResize(width, height))
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
    fn request_capture(&mut self) -> Result<()> {
        anyhow::ensure!(
            self.alive.load(Ordering::Acquire),
            "VST3 control thread failed"
        );
        if self.pending_capture.is_none() {
            let (tx, rx) = mpsc::sync_channel(1);
            self.tx
                .send(Command::Capture(tx))
                .map_err(|e| anyhow::anyhow!("VST3 control thread stopped: {e}"))?;
            self.pending_capture = Some(rx);
            self.capture_started = Some(Instant::now());
        }
        Ok(())
    }
}

fn pump_editor_messages(plugin: &mut Vst3) {
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
        };
        let mut message: MSG = std::mem::zeroed();
        for _ in 0..64 {
            if PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) == 0 {
                break;
            }
            if !plugin.handle_editor_key(&message) {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}
impl Hosted for Threaded {
    #[cfg(debug_assertions)]
    fn test_editor_timeout(&mut self) -> Result<()> {
        self.editor_size.store(0, Ordering::Release);
        self.editor_open.store(true, Ordering::Release);
        self.editor_started = Instant::now().checked_sub(Duration::from_secs(6));
        self.editor_idle();
        Ok(())
    }
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }
    fn open_editor(&mut self) -> Result<()> {
        anyhow::ensure!(
            self.alive.load(Ordering::Acquire),
            "VST3 control thread failed"
        );
        if !self.editor_used {
            while self.audio.as_ref().unwrap().handler.edits.pop().is_some() {}
            while self.edits.pop().is_some() {}
        }
        self.editor_used = true;
        if !self.editor_open.swap(true, Ordering::AcqRel) {
            self.editor_size.store(0, Ordering::Release);
        }
        if let Err(error) = self.tx.send(Command::Open) {
            self.editor_open.store(false, Ordering::Release);
            anyhow::bail!("VST3 control thread stopped: {error}");
        }
        self.editor_started = Some(Instant::now());
        Ok(())
    }
    fn close_editor(&mut self) {
        self.editor_started = None;
        let _ = self.tx.send(Command::Close);
    }
    fn editor_idle(&mut self) {
        if self.editor_size().0 > 0 || !self.editor_open() {
            self.editor_started = None;
        } else if self
            .editor_started
            .is_some_and(|start| start.elapsed() >= Duration::from_secs(5))
        {
            self.editor_started = None;
            self.editor_open.store(false, Ordering::Release);
            let _ = self.tx.send(Command::Close);
            if let Ok(mut error) = self.error.lock() {
                *error = Some("Plugin editor did not open within five seconds".into());
            }
        }
    }
    fn editor_open(&self) -> bool {
        self.editor_open.load(Ordering::Acquire)
    }
    fn editor_size(&self) -> (i32, i32) {
        let value = self.editor_size.load(Ordering::Acquire);
        ((value >> 32) as i32, value as u32 as i32)
    }
    fn editor_resizable(&self) -> bool {
        self.editor_resizable.load(Ordering::Acquire)
    }
    fn take_editor_error(&mut self) -> Option<String> {
        self.error.lock().ok().and_then(|mut error| error.take())
    }
    fn invalidate_capture(&mut self) {
        self.pending_capture = None;
        self.capture_started = None;
    }
    fn take_editor_edits(&mut self) -> Vec<(u32, f64)> {
        let mut edits = Vec::new();
        while let Some((id, value, sequence)) = self.audio.as_ref().unwrap().handler.edits.pop() {
            if self.editor_used && self.audio.as_ref().unwrap().handler.current(id, sequence) {
                if self.last_values.insert(id, value) != Some(value) {
                    self.audio.as_mut().unwrap().set_parameter(id, value);
                    edits.push((id, value));
                }
            }
        }
        while let Some((id, value, sequence)) = self.edits.pop() {
            if self.editor_used && self.audio.as_ref().unwrap().handler.current(id, sequence) {
                if self.last_values.insert(id, value) != Some(value) {
                    self.audio.as_mut().unwrap().set_parameter(id, value);
                    edits.push((id, value));
                }
            }
        }
        if !edits.is_empty() {
            self.pending_capture = None;
            self.capture_started = None;
        }
        edits
    }
    fn set_parameter(&mut self, id: u32, value: f64) {
        self.pending_capture = None;
        self.capture_started = None;
        self.last_values.insert(id, value);
        self.audio.as_mut().unwrap().set_parameter(id, value);
        self.audio.as_ref().unwrap().handler.record_host(id, value);
        if !self.parameters_queued.swap(true, Ordering::AcqRel)
            && self.tx.send(Command::SyncParameters).is_err()
        {
            if let Ok(mut error) = self.error.lock() {
                *error = Some("VST3 control thread stopped".into());
            }
        }
    }
    fn note(&mut self, note: u8, velocity: f32, on: bool) {
        self.audio.as_mut().unwrap().note(note, velocity, on);
    }
    fn process(&mut self, frames: &mut [Frame]) -> Result<()> {
        anyhow::ensure!(
            self.alive.load(Ordering::Acquire),
            "VST3 control thread failed"
        );
        self.audio.as_mut().unwrap().process(frames)
    }
    fn flush_parameter_changes(&mut self) -> Result<()> {
        self.audio.as_mut().unwrap().flush_parameter_changes()
    }
    fn capture_state(&mut self) -> Result<Option<piratewave_core::plugins::PluginState>> {
        self.audio.as_mut().unwrap().flush_parameter_changes()?;
        self.request_capture()?;
        match self.pending_capture.as_ref().unwrap().try_recv() {
            Ok(result) => {
                self.pending_capture = None;
                self.capture_started = None;
                result.map_err(anyhow::Error::msg)
            }
            Err(mpsc::TryRecvError::Empty)
                if self
                    .capture_started
                    .is_some_and(|start| start.elapsed() >= Duration::from_secs(5)) =>
            {
                self.pending_capture = None;
                self.capture_started = None;
                anyhow::bail!("VST3 state capture timed out")
            }
            Err(mpsc::TryRecvError::Empty) => anyhow::bail!("VST3 state capture is pending"),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending_capture = None;
                self.capture_started = None;
                anyhow::bail!("VST3 state capture failed because the control thread stopped")
            }
        }
    }
    fn capture_state_for_shutdown(
        &mut self,
    ) -> Result<Option<piratewave_core::plugins::PluginState>> {
        self.audio.as_mut().unwrap().flush_parameter_changes()?;
        self.pending_capture = None;
        self.capture_started = None;
        self.request_capture()?;
        let receiver = self.pending_capture.take().unwrap();
        self.capture_started = None;
        receiver
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| anyhow::anyhow!("VST3 state capture did not finish before shutdown: {e}"))?
            .map_err(anyhow::Error::msg)
    }
    fn restore_state(&mut self, state: &piratewave_core::plugins::PluginState) -> Result<()> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.tx.send(Command::Restore(state.clone(), tx))?;
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|e| anyhow::anyhow!("VST3 state restore timed out: {e}"))?
            .map_err(anyhow::Error::msg)
    }
}
impl Drop for Threaded {
    fn drop(&mut self) {
        // All process calls have stopped before the control thread deactivates the component.
        drop(self.audio.take());
        let _ = self.tx.send(Command::Shutdown);
    }
}
