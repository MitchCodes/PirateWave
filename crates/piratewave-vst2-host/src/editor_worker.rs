//! Owns VST2 editor calls and Windows messages separately from audio processing.
use crate::window::{editor_key, EditorWindow};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};
use vst::{
    editor::{Editor, Key, KeyCode},
    host::{PluginInstance, PluginStateHandle},
    plugin::Plugin,
};
use windows_sys::Win32::{
    System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
    UI::WindowsAndMessaging::*,
};

enum Command {
    Open,
    Close,
    Capture(mpsc::SyncSender<Result<Option<crate::StateChunk>, String>>),
    #[cfg(debug_assertions)]
    TestKey(u32),
    Shutdown(PluginInstance),
}
pub struct Worker {
    tx: mpsc::Sender<Command>,
    open: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    size: Arc<AtomicU64>,
    has_editor: bool,
}
struct ThreadGuard {
    open: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
}
impl Drop for ThreadGuard {
    fn drop(&mut self) {
        self.open.store(false, Ordering::Release);
        if std::thread::panicking() {
            if let Ok(mut error) = self.error.lock() {
                *error = Some("VST2 editor thread failed; close and reload the plugin".into());
            }
        }
    }
}
impl Worker {
    pub fn start(
        mut editor: Option<Box<dyn Editor + Send>>,
        title: String,
        resize: Arc<Mutex<Option<(i32, i32)>>>,
        state_handle: PluginStateHandle,
        parameter_ids: Vec<u32>,
        values: Arc<Mutex<std::collections::BTreeMap<u32, f64>>>,
        edits: Arc<crossbeam_queue::ArrayQueue<(u32, f64, u64)>>,
        revisions: Arc<crate::ParameterRevisions>,
        program_count: i32,
    ) -> anyhow::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let open = Arc::new(AtomicBool::new(false));
        let open_thread = open.clone();
        let error = Arc::new(Mutex::new(None));
        let error_thread = error.clone();
        let size = Arc::new(AtomicU64::new(0));
        let size_thread = size.clone();
        let has_editor = editor.is_some();
        thread::Builder::new()
            .name("piratewave-vst2-editor".into())
            .spawn(move || {
                let _guard = ThreadGuard {
                    open: open_thread.clone(),
                    error: error_thread.clone(),
                };
                let initialized = unsafe {
                    CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) >= 0
                };
                let mut state_handle = Some(state_handle);
                let mut window: Option<EditorWindow> = None;
                let close =
                    |window: &mut Option<EditorWindow>,
                     editor: &mut Option<Box<dyn Editor + Send>>| {
                        if let Some(w) = window.take() {
                            if let Some(editor) = editor {
                                editor.close();
                            }
                            drop(w);
                        }
                        open_thread.store(false, Ordering::Release);
                    };
                let poll_parameters = |handle: &PluginStateHandle| {
                    for &id in &parameter_ids {
                        let Some(previous) = revisions.observe(id) else {
                            continue;
                        };
                        let value = handle.parameter(id as i32) as f64;
                        if value.is_finite() && (0.0..=1.0).contains(&value) {
                            if let Ok(mut last) = values.lock() {
                                if last.get(&id).is_some_and(|old| *old != value) {
                                    if let Some(revision) = revisions.mark_polled(id, previous) {
                                        last.insert(id, value);
                                        let _ = edits.push((id, value, revision));
                                    }
                                } else if revisions.current(id, previous) {
                                    last.insert(id, value);
                                }
                            }
                        }
                    }
                };
                loop {
                    unsafe {
                        let mut msg: MSG = std::mem::zeroed();
                        for _ in 0..64 {
                            if PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) == 0 {
                                break;
                            }
                            let handled =
                                if let (Some(window), Some(editor)) = (&window, &mut editor) {
                                    if msg.hwnd.cast::<std::ffi::c_void>() == window.handle() {
                                        editor_key(&msg).is_some_and(|key| {
                                            let keycode = KeyCode {
                                                character: char::from_u32(key.character as u32)
                                                    .unwrap_or('\0'),
                                                key: Key::try_from(key.code as isize)
                                                    .unwrap_or(Key::None),
                                                modifier: key.modifiers as u8,
                                            };
                                            if key.down {
                                                editor.key_down(keycode)
                                            } else {
                                                editor.key_up(keycode)
                                            }
                                        })
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                };
                            if !handled {
                                TranslateMessage(&msg);
                                DispatchMessageW(&msg);
                            }
                        }
                    }
                    if window.as_ref().is_some_and(|w| !w.visible()) {
                        if let Some(handle) = &state_handle {
                            poll_parameters(handle);
                        }
                        close(&mut window, &mut editor);
                    }
                    if window.is_some() {
                        if let Some(editor) = &mut editor {
                            editor.idle();
                        }
                        if let Some(handle) = &state_handle {
                            poll_parameters(handle);
                        }
                        let request = resize.lock().ok().and_then(|mut r| r.take());
                        if let (Some((width, height)), Some(w)) = (request, &window) {
                            let _ = w.resize(width, height);
                        }
                        if let Some(w) = &window {
                            let (width, height) = w.client_size();
                            size_thread
                                .store(((width as u64) << 32) | height as u64, Ordering::Release);
                        }
                    }
                    match rx.recv_timeout(Duration::from_millis(10)) {
                        Ok(Command::Open) => {
                            if let Some(w) = &window {
                                w.show();
                                continue;
                            }
                            if let Some(handle) = &state_handle {
                                for &id in &parameter_ids {
                                    let value = handle.parameter(id as i32) as f64;
                                    if let Ok(mut last) = values.lock() {
                                        last.insert(id, value);
                                    }
                                }
                            }
                            let Some(editor) = &mut editor else {
                                open_thread.store(false, Ordering::Release);
                                continue;
                            };
                            let (width, height) = editor.size();
                            let result =
                                EditorWindow::new(&title, width, height, false).and_then(|w| {
                                    if !editor.open(w.handle()) {
                                        editor.close();
                                        anyhow::bail!("Plugin failed to open its editor");
                                    }
                                    w.show();
                                    Ok(w)
                                });
                            match result {
                                Ok(w) => {
                                    let (width, height) = w.client_size();
                                    size_thread.store(
                                        ((width as u64) << 32) | height as u64,
                                        Ordering::Release,
                                    );
                                    window = Some(w);
                                    open_thread.store(true, Ordering::Release);
                                }
                                Err(e) => {
                                    open_thread.store(false, Ordering::Release);
                                    if let Ok(mut error) = error_thread.lock() {
                                        *error = Some(format!("{e:#}"));
                                    }
                                }
                            }
                        }
                        Ok(Command::Close) => {
                            if window.is_some() {
                                if let Some(handle) = &state_handle {
                                    poll_parameters(handle);
                                }
                            }
                            close(&mut window, &mut editor);
                        }
                        Ok(Command::Capture(reply)) => {
                            let handle = state_handle.as_ref().unwrap();
                            let state =
                                handle
                                    .read_state_chunk(true, 64 * 1024 * 1024)
                                    .and_then(|bank| match bank {
                                        Some(data) => {
                                            let current = handle.current_program();
                                            let program = (0..program_count)
                                                .contains(&current)
                                                .then_some(current as u32);
                                            Ok(Some((data, true, program)))
                                        }
                                        None => handle
                                            .read_state_chunk(false, 64 * 1024 * 1024)
                                            .map(|preset| {
                                                preset.map(|data| {
                                                    let current = handle.current_program();
                                                    let program = (0..program_count)
                                                        .contains(&current)
                                                        .then_some(current as u32);
                                                    (data, false, program)
                                                })
                                            }),
                                    });
                            let _ = reply.send(state);
                        }
                        #[cfg(debug_assertions)]
                        Ok(Command::TestKey(key)) => {
                            if let Some(window) = &window {
                                unsafe {
                                    PostMessageW(
                                        window.handle().cast(),
                                        WM_KEYDOWN,
                                        key as usize,
                                        0,
                                    );
                                    PostMessageW(window.handle().cast(), WM_KEYUP, key as usize, 0);
                                }
                            }
                        }
                        Ok(Command::Shutdown(mut instance)) => {
                            close(&mut window, &mut editor);
                            drop(editor);
                            drop(state_handle.take());
                            instance.stop_process();
                            instance.suspend();
                            drop(instance);
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            close(&mut window, &mut editor);
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
                if initialized {
                    unsafe {
                        CoUninitialize();
                    }
                }
            })?;
        Ok(Self {
            tx,
            open,
            error,
            size,
            has_editor,
        })
    }
    pub fn open(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.has_editor, "This plugin has no vendor editor");
        if !self.open.swap(true, Ordering::AcqRel) {
            self.size.store(0, Ordering::Release);
        }
        if let Err(error) = self.tx.send(Command::Open) {
            self.open.store(false, Ordering::Release);
            anyhow::bail!("Editor thread stopped: {error}");
        }
        Ok(())
    }
    pub fn close(&self) {
        let _ = self.tx.send(Command::Close);
    }
    pub fn mark_open_timed_out(&self) {
        self.open.store(false, Ordering::Release);
        let _ = self.tx.send(Command::Close);
    }
    #[cfg(debug_assertions)]
    pub fn simulate_slow_open(&self) {
        self.size.store(0, Ordering::Release);
        self.open.store(true, Ordering::Release);
    }
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }
    pub fn size(&self) -> (i32, i32) {
        let packed = self.size.load(Ordering::Acquire);
        ((packed >> 32) as i32, packed as u32 as i32)
    }
    pub fn take_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|mut e| e.take())
    }
    pub fn shutdown(self, instance: PluginInstance) {
        if let Err(mpsc::SendError(Command::Shutdown(mut instance))) =
            self.tx.send(Command::Shutdown(instance))
        {
            instance.stop_process();
            instance.suspend();
        }
    }
    pub fn request_capture(
        &self,
    ) -> anyhow::Result<mpsc::Receiver<Result<Option<crate::StateChunk>, String>>> {
        let (reply, receiver) = mpsc::sync_channel(1);
        self.tx
            .send(Command::Capture(reply))
            .map_err(|e| anyhow::anyhow!("Editor thread stopped: {e}"))?;
        Ok(receiver)
    }
    #[cfg(debug_assertions)]
    pub fn test_key(&self, key: u32) -> anyhow::Result<()> {
        self.tx
            .send(Command::TestKey(key))
            .map_err(|e| anyhow::anyhow!("Editor thread stopped: {e}"))
    }
}
