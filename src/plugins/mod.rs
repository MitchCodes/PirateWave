//! Native plugin adapters and live graph instances. No plugin runs on a device callback.
#[cfg(windows)]
mod bridge;
pub mod diagnostics;
mod vst2;
mod vst3_host;
mod vst3_support;
use anyhow::{Context, Result};
use piratewave_core::{
    plugins::{NodePlugin, PluginDescriptor, PluginFormat, PluginParameter, PluginRole},
    project::{Node, Project},
    Frame, BLOCK_SIZE, SAMPLE_RATE,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// A plugin's host thread is an STA and services its Windows timer/message callbacks.
pub struct PluginThread {
    #[cfg(windows)]
    initialized: bool,
}
impl Default for PluginThread {
    fn default() -> Self {
        Self {
            #[cfg(windows)]
            initialized: unsafe {
                windows::Win32::System::Com::CoInitializeEx(
                    None,
                    windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
                )
                .is_ok()
            },
        }
    }
}
impl Drop for PluginThread {
    fn drop(&mut self) {
        #[cfg(windows)]
        if self.initialized {
            unsafe {
                windows::Win32::System::Com::CoUninitialize();
            }
        }
    }
}
pub fn pump_messages() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
        };
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

pub trait Hosted {
    #[cfg(all(windows, debug_assertions))]
    fn test_editor_key(&mut self, _key: u32) -> Result<()> {
        anyhow::bail!("Editor key testing is unavailable")
    }
    #[cfg(all(windows, debug_assertions))]
    fn test_editor_timeout(&mut self) -> Result<()> {
        anyhow::bail!("Editor timeout testing is unavailable")
    }
    fn descriptor(&self) -> &PluginDescriptor;
    fn open_editor(&mut self) -> Result<()> {
        anyhow::bail!("This plugin has no supported vendor editor")
    }
    fn close_editor(&mut self) {}
    fn editor_idle(&mut self) {}
    fn take_editor_error(&mut self) -> Option<String> {
        None
    }
    fn invalidate_capture(&mut self) {}
    fn editor_open(&self) -> bool {
        false
    }
    fn editor_size(&self) -> (i32, i32) {
        (0, 0)
    }
    fn editor_resizable(&self) -> bool {
        false
    }
    fn take_editor_edits(&mut self) -> Vec<(u32, f64)> {
        Vec::new()
    }
    fn flush_parameter_changes(&mut self) -> Result<()> {
        Ok(())
    }
    fn capture_state(&mut self) -> Result<Option<piratewave_core::plugins::PluginState>> {
        Ok(None)
    }
    fn capture_state_for_shutdown(
        &mut self,
    ) -> Result<Option<piratewave_core::plugins::PluginState>> {
        self.capture_state()
    }
    fn restore_state(&mut self, _state: &piratewave_core::plugins::PluginState) -> Result<()> {
        anyhow::bail!("Plugin state restoration is unsupported")
    }
    fn set_parameter(&mut self, id: u32, value: f64);
    fn process(&mut self, frames: &mut [Frame]) -> Result<()>;
    fn note(&mut self, note: u8, velocity: f32, on: bool);
}
fn instantiate(descriptor: &PluginDescriptor) -> Result<Box<dyn Hosted>> {
    let plugin: Box<dyn Hosted> = match descriptor.format {
        PluginFormat::Vst2 => load_vst2(Path::new(&descriptor.path))?,
        PluginFormat::Vst3 => {
            #[cfg(windows)]
            {
                Box::new(vst3_host::Threaded::load(
                    Path::new(&descriptor.path),
                    &descriptor.class_id,
                )?)
            }
            #[cfg(not(windows))]
            {
                Box::new(vst3_host::Vst3::load(
                    Path::new(&descriptor.path),
                    &descriptor.class_id,
                )?)
            }
        }
    };
    anyhow::ensure!(
        plugin.descriptor().class_id == descriptor.class_id
            && plugin.descriptor().role == descriptor.role,
        "Plugin identity/type changed; remove and rescan it"
    );
    Ok(plugin)
}
fn load_vst2(path: &Path) -> Result<Box<dyn Hosted>> {
    use piratewave_vst2_host::pe::{architecture, Architecture};
    match architecture(path)? {
        Architecture::X86 if usize::BITS == 64 => {
            #[cfg(windows)]
            {
                Ok(Box::new(bridge::Bridge::load(path)?))
            }
            #[cfg(not(windows))]
            {
                anyhow::bail!("32-bit VST2 bridging is supported on Windows only")
            }
        }
        Architecture::X64 if usize::BITS == 64 => Ok(Box::new(vst2::Vst2::load(path)?)),
        Architecture::X86 if usize::BITS == 32 => Ok(Box::new(vst2::Vst2::load(path)?)),
        architecture => anyhow::bail!(
            "Unsupported VST2 architecture {architecture:?} for a {}-bit host",
            usize::BITS
        ),
    }
}
pub fn scan(path: &Path) -> Result<Vec<PluginDescriptor>> {
    let path = path
        .canonicalize()
        .with_context(|| format!("Plugin does not exist: {}", path.display()))?;
    if path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("vst3"))
    {
        vst3_host::scan(&path)
    } else if path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("dll"))
        && path.is_file()
    {
        Ok(vec![load_vst2(&path)?.descriptor().clone()])
    } else {
        anyhow::bail!("Choose a 64-bit .vst3 file/bundle or a 32-/64-bit VST2 .dll")
    }
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub plugins: Vec<PluginDescriptor>,
}
impl Library {
    pub fn load() -> Self {
        std::fs::read_to_string(library_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> Result<()> {
        crate::storage::save_json(&library_path(), self)
    }
    pub fn import(&mut self, plugins: Vec<PluginDescriptor>) -> Result<()> {
        for plugin in &plugins {
            plugin.validate().map_err(anyhow::Error::msg)?;
        }
        for plugin in plugins {
            if let Some(existing) = self.plugins.iter_mut().find(|p| p.same_plugin(&plugin)) {
                *existing = plugin;
            } else {
                self.plugins.push(plugin);
            }
        }
        self.plugins.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(())
    }
}
fn library_path() -> PathBuf {
    crate::storage::default_path().with_file_name("plugins.json")
}
#[derive(Serialize, Deserialize)]
pub struct ScanReport {
    pub plugins: Vec<PluginDescriptor>,
    pub error: Option<String>,
}
pub fn scan_child(path: &Path, report: &Path) -> Result<()> {
    let _thread = PluginThread::default();
    let result = match scan(path) {
        Ok(plugins) => ScanReport {
            plugins,
            error: None,
        },
        Err(e) => ScanReport {
            plugins: vec![],
            error: Some(format!("{e:#}")),
        },
    };
    std::fs::write(report, serde_json::to_vec(&result)?)?;
    Ok(())
}
/// Scanner is a short-lived child, so broken libraries cannot take down the UI.
pub fn scan_isolated(path: PathBuf) -> Result<Vec<PluginDescriptor>> {
    use std::{
        process::Command,
        time::{Duration, Instant},
    };
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let report = std::env::temp_dir().join(format!(
        "piratewave-scan-{}-{nonce}.json",
        std::process::id()
    ));
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.arg("--scan-plugin").arg(path).arg(&report);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd.spawn()?;
    let start = Instant::now();
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    std::fs::read(&report)
                        .context("Scanner returned no report")
                        .and_then(|bytes| Ok(serde_json::from_slice::<ScanReport>(&bytes)?))
                        .and_then(|r| {
                            if let Some(error) = r.error {
                                anyhow::bail!("{error}");
                            }
                            Ok(r.plugins)
                        })
                } else {
                    Err(anyhow::anyhow!("Plugin scanner exited with {status}; the plugin may be incompatible or need activation"))
                }
            }
            Err(e) => break Err(e.into()),
            Ok(None) if start.elapsed() > Duration::from_secs(30) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(anyhow::anyhow!("Plugin scan timed out after 30 seconds"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
        }
    };
    let _ = std::fs::remove_file(report);
    outcome
}
struct Entry {
    config: NodePlugin,
    plugin: Option<Box<dyn Hosted>>,
    notes: [bool; 128],
    disabled: bool,
    retry_at: Instant,
    failures: u32,
}
pub type PluginEdits = HashMap<u64, (PluginDescriptor, HashMap<u32, (f64, f64)>)>;
#[derive(Default)]
pub struct Rack {
    entries: HashMap<u64, Entry>,
    pending_edits: HashMap<u64, HashMap<u32, (f64, f64)>>,
    state_tracking: bool,
    tracked_nodes: std::collections::HashSet<u64>,
    observed_editors: HashMap<u64, bool>,
    snapshot_cache: HashMap<
        u64,
        (
            PluginDescriptor,
            Option<piratewave_core::plugins::PluginState>,
        ),
    >,
    pub warnings: Vec<String>,
}
impl Rack {
    pub fn set_parameters(&mut self, node_id: u64, values: &[(u32, f64)]) {
        self.snapshot_cache.remove(&node_id);
        if let Some(entry) = self.entries.get_mut(&node_id) {
            for &(id, value) in values {
                if !value.is_finite()
                    || !(0.0..=1.0).contains(&value)
                    || !entry
                        .config
                        .descriptor
                        .parameters
                        .iter()
                        .any(|p| p.id == id)
                {
                    continue;
                }
                if let Some(pending) = self.pending_edits.get_mut(&node_id) {
                    pending.remove(&id);
                }
                entry.config.parameters.insert(id, value);
                if let Some(plugin) = &mut entry.plugin {
                    plugin.set_parameter(id, value);
                }
            }
        }
    }
    pub fn open_editor(&mut self, id: u64) -> Result<()> {
        let entry = self
            .entries
            .get_mut(&id)
            .context("Plugin node is not loaded")?;
        let result = entry
            .plugin
            .as_mut()
            .context("Plugin is unavailable")?
            .open_editor();
        if result.is_ok() {
            self.state_tracking = true;
            self.tracked_nodes.insert(id);
            self.snapshot_cache.remove(&id);
        }
        result
    }
    pub fn close_editors(&mut self) {
        self.snapshot_cache.clear();
        for entry in self.entries.values_mut() {
            if let Some(plugin) = &mut entry.plugin {
                plugin.close_editor();
                plugin.invalidate_capture();
            }
        }
    }
    pub fn close_editor(&mut self, id: u64) {
        if let Some(plugin) = self
            .entries
            .get_mut(&id)
            .and_then(|entry| entry.plugin.as_mut())
        {
            plugin.close_editor();
        }
    }
    pub fn editor_idle(&mut self) {
        for (node_id, entry) in &mut self.entries {
            if let Some(plugin) = &mut entry.plugin {
                let mut edits = plugin.take_editor_edits();
                plugin.editor_idle();
                let is_open = plugin.editor_open();
                let previous = self
                    .observed_editors
                    .insert(*node_id, is_open)
                    .unwrap_or(false);
                if previous != is_open {
                    self.snapshot_cache.remove(node_id);
                    plugin.invalidate_capture();
                }
                if let Some(error) = plugin.take_editor_error() {
                    self.warnings
                        .push(format!("Plugin {node_id} editor: {error}"));
                }
                edits.extend(plugin.take_editor_edits());
                if entry.disabled {
                    if let Err(error) = plugin.flush_parameter_changes() {
                        self.warnings.push(format!(
                            "Plugin {node_id} parameter flush failed: {error:#}"
                        ));
                    }
                }
                for (id, value) in edits {
                    if let Some(parameter) = entry
                        .config
                        .descriptor
                        .parameters
                        .iter()
                        .find(|p| p.id == id)
                    {
                        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                            continue;
                        }
                        let base = *entry
                            .config
                            .parameters
                            .get(&id)
                            .unwrap_or(&parameter.default);
                        let pending = self.pending_edits.entry(*node_id).or_default();
                        if pending.get(&id).map_or(base, |edit| edit.1) == value {
                            continue;
                        }
                        self.snapshot_cache.remove(node_id);
                        pending
                            .entry(id)
                            .and_modify(|edit| edit.1 = value)
                            .or_insert((base, value));
                    }
                }
            }
        }
    }
    pub fn pending_editor_edits(&self) -> PluginEdits {
        self.pending_edits
            .iter()
            .filter_map(|(id, edits)| {
                self.entries
                    .get(id)
                    .map(|entry| (*id, (entry.config.descriptor.clone(), edits.clone())))
            })
            .collect()
    }
    pub fn adopt_editor_edits(&mut self, edits: PluginEdits) {
        self.pending_edits = edits
            .into_iter()
            .filter_map(|(id, (descriptor, values))| {
                self.entries
                    .get(&id)
                    .filter(|entry| entry.config.descriptor.same_plugin(&descriptor))
                    .map(|_| (id, values))
            })
            .collect();
    }
    pub fn open_editor_nodes(&self) -> Vec<u64> {
        self.entries
            .iter()
            .filter_map(|(id, entry)| {
                entry
                    .plugin
                    .as_ref()
                    .filter(|p| p.editor_open())
                    .map(|_| *id)
            })
            .collect()
    }
    pub fn sync(&mut self, project: &Project) {
        self.entries.retain(|id, _| {
            project
                .nodes
                .iter()
                .any(|n| n.id == *id && n.kind.is_plugin() && n.plugin.is_some())
        });
        self.pending_edits
            .retain(|id, _| self.entries.contains_key(id));
        self.tracked_nodes
            .retain(|id| self.entries.contains_key(id));
        self.observed_editors
            .retain(|id, _| self.entries.contains_key(id));
        self.snapshot_cache
            .retain(|id, _| self.entries.contains_key(id));
        for node in project.nodes.iter().filter(|n| n.kind.is_plugin()) {
            let Some(config) = &node.plugin else {
                continue;
            };
            if !self
                .entries
                .get(&node.id)
                .is_some_and(|e| e.config.descriptor.same_plugin(&config.descriptor))
            {
                self.pending_edits.remove(&node.id);
                self.tracked_nodes.remove(&node.id);
                self.observed_editors.remove(&node.id);
                self.snapshot_cache.remove(&node.id);
                let plugin = match instantiate(&config.descriptor) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        self.warnings.push(format!("{}: {e:#}", node.name));
                        None
                    }
                };
                self.entries.insert(
                    node.id,
                    Entry {
                        config: config.clone(),
                        plugin,
                        notes: [false; 128],
                        disabled: node.effects_disabled,
                        retry_at: Instant::now() + Duration::from_secs(1),
                        failures: 0,
                    },
                );
                if let Some(entry) = self.entries.get_mut(&node.id) {
                    if let Some(plugin) = &mut entry.plugin {
                        let restored = if let Some(state) = &config.state {
                            match plugin.restore_state(state) {
                                Ok(()) => true,
                                Err(error) => {
                                    self.warnings.push(format!(
                                        "{}: saved plugin state could not be restored: {error:#}",
                                        node.name
                                    ));
                                    false
                                }
                            }
                        } else {
                            false
                        };
                        for p in &config.descriptor.parameters {
                            if restored && !config.parameters.contains_key(&p.id) {
                                continue;
                            }
                            plugin.set_parameter(
                                p.id,
                                *config.parameters.get(&p.id).unwrap_or(&p.default),
                            );
                        }
                    }
                }
            }
            let entry = self.entries.get_mut(&node.id).unwrap();
            if let Some(edits) = self.pending_edits.get_mut(&node.id) {
                edits.retain(|id, (base, latest)| {
                    let default = config
                        .descriptor
                        .parameters
                        .iter()
                        .find(|p| p.id == *id)
                        .map(|p| p.default);
                    default.is_some_and(|default| {
                        let incoming = *config.parameters.get(id).unwrap_or(&default);
                        if incoming == *latest {
                            return false;
                        }
                        if incoming != *base {
                            *base = incoming;
                        }
                        true
                    })
                });
            }
            if let Some(plugin) = &mut entry.plugin {
                for p in &config.descriptor.parameters {
                    let value = *config.parameters.get(&p.id).unwrap_or(&p.default);
                    let old = *entry.config.parameters.get(&p.id).unwrap_or(&p.default);
                    let pending = self
                        .pending_edits
                        .get(&node.id)
                        .is_some_and(|edits| edits.contains_key(&p.id));
                    if value != old && !pending {
                        plugin.set_parameter(p.id, value);
                    }
                }
                if entry.disabled != node.effects_disabled {
                    for (note, &active) in entry.notes.iter().enumerate() {
                        if active {
                            plugin.note(note as u8, 0.0, false);
                        }
                    }
                    entry.notes.fill(false);
                }
            }
            entry.disabled = node.effects_disabled;
            entry.config = config.clone();
        }
    }
    pub fn recover(&mut self) {
        self.recover_with(instantiate);
    }
    fn recover_with(&mut self, mut load: impl FnMut(&PluginDescriptor) -> Result<Box<dyn Hosted>>) {
        for (id, entry) in &mut self.entries {
            if entry.plugin.is_some() || entry.disabled || Instant::now() < entry.retry_at {
                continue;
            }
            match load(&entry.config.descriptor) {
                Ok(mut plugin) => {
                    let restored = if let Some(state) = &entry.config.state {
                        match plugin.restore_state(state) {
                            Ok(()) => true,
                            Err(error) => {
                                self.warnings.push(format!(
                                    "Plugin {id}: saved state could not be restored: {error:#}"
                                ));
                                false
                            }
                        }
                    } else {
                        false
                    };
                    for p in &entry.config.descriptor.parameters {
                        if restored && !entry.config.parameters.contains_key(&p.id) {
                            continue;
                        }
                        plugin.set_parameter(
                            p.id,
                            *entry.config.parameters.get(&p.id).unwrap_or(&p.default),
                        );
                    }
                    entry.notes.fill(false);
                    entry.plugin = Some(plugin);
                    entry.failures = 0;
                    crate::logging::info(format_args!("Plugin {id} recovered"));
                    self.warnings
                        .push(format!("Plugin {id} recovered automatically"));
                }
                Err(error) => {
                    crate::logging::error(format_args!("Plugin {id} load failed: {error:#}"));
                    entry.failures = entry.failures.saturating_add(1);
                    let seconds = (1_u64 << entry.failures.min(5)).min(30);
                    entry.retry_at = Instant::now() + Duration::from_secs(seconds);
                    self.warnings.push(format!(
                        "Plugin {id} unavailable: {error:#}; retrying in {seconds}s"
                    ));
                }
            }
        }
    }
    pub fn note(&mut self, id: u64, note: u8, velocity: f32, on: bool) {
        if note > 127 {
            return;
        }
        if let Some(entry) = self.entries.get_mut(&id) {
            if entry.config.descriptor.role != PluginRole::Instrument {
                return;
            }
            if let Some(plugin) = &mut entry.plugin {
                // Retriggering a shortcut releases the old voice first.
                if on && entry.notes[note as usize] {
                    plugin.note(note, 0.0, false);
                }
                plugin.note(note, velocity, on);
                entry.notes[note as usize] = on;
            }
        }
    }
    pub fn all_notes_off(&mut self, id: Option<u64>) {
        for (key, entry) in &mut self.entries {
            if id.is_none_or(|id| id == *key) {
                if let Some(plugin) = &mut entry.plugin {
                    for note in 0..128 {
                        plugin.note(note as u8, 0.0, false);
                    }
                }
                entry.notes.fill(false);
            }
        }
    }
    /// Capture state from live instances on their owning thread. Failures are reported
    /// separately so callers can retain each node's last successful snapshot.
    pub fn capture_states(
        &mut self,
        final_capture: bool,
    ) -> (
        HashMap<u64, (PluginDescriptor, piratewave_core::plugins::PluginState)>,
        Vec<String>,
    ) {
        let mut states = HashMap::new();
        let mut errors = Vec::new();
        for (id, entry) in &mut self.entries {
            if !final_capture {
                if let Some((descriptor, state)) = self.snapshot_cache.get(id) {
                    if let Some(state) = state {
                        states.insert(*id, (descriptor.clone(), state.clone()));
                    }
                    continue;
                }
            }
            if let Some(plugin) = &mut entry.plugin {
                let result = if final_capture {
                    plugin.capture_state_for_shutdown()
                } else {
                    plugin.capture_state()
                };
                match result {
                    Ok(Some(state)) => {
                        if !final_capture {
                            self.snapshot_cache.insert(
                                *id,
                                (entry.config.descriptor.clone(), Some(state.clone())),
                            );
                        }
                        states.insert(*id, (entry.config.descriptor.clone(), state));
                    }
                    Ok(None) => {
                        if !final_capture {
                            self.snapshot_cache
                                .insert(*id, (entry.config.descriptor.clone(), None));
                        }
                    }
                    Err(error) => {
                        errors.push(format!("Plugin {id}: state capture failed: {error:#}"))
                    }
                }
            } else if self.tracked_nodes.contains(id) {
                errors.push(format!(
                    "Plugin {id}: state capture unavailable because the plugin is not loaded"
                ));
            }
        }
        if errors.is_empty() || final_capture {
            self.snapshot_cache.clear();
        }
        (states, errors)
    }
    pub fn tracking_state(&self) -> bool {
        self.state_tracking
    }
    pub fn process(&mut self, node: &Node, frames: &mut [Frame]) {
        let Some(entry) = self.entries.get_mut(&node.id) else {
            return;
        };
        let Some(plugin) = &mut entry.plugin else {
            return;
        };
        let mut dry = [[0.0; 2]; BLOCK_SIZE];
        dry.copy_from_slice(frames);
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| plugin.process(frames)))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("Plugin processing panicked")))
                .and_then(|()| {
                    anyhow::ensure!(
                        frames.iter().flatten().all(|v| v.is_finite()),
                        "Plugin returned invalid audio"
                    );
                    Ok(())
                });
        if let Err(error) = result {
            frames.copy_from_slice(&dry);
            self.warnings
                .push(format!("{} stopped processing: {error:#}", node.name));
            crate::logging::error(format_args!("Plugin {} failed: {error:#}", node.name));
            entry.plugin = None;
            entry.notes.fill(false);
            entry.retry_at = Instant::now() + Duration::from_secs(1);
        }
    }
    pub fn statuses(&self) -> HashMap<u64, String> {
        self.entries
            .iter()
            .map(|(id, e)| {
                (
                    *id,
                    if e.plugin.is_some() {
                        "Loaded".into()
                    } else {
                        "Unavailable; retrying automatically".into()
                    },
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};
    struct Fake {
        descriptor: PluginDescriptor,
        broken: bool,
    }
    impl Hosted for Fake {
        fn descriptor(&self) -> &PluginDescriptor {
            &self.descriptor
        }
        fn set_parameter(&mut self, _: u32, _: f64) {}
        fn process(&mut self, frames: &mut [Frame]) -> Result<()> {
            if self.broken {
                frames[0] = [f32::NAN; 2];
            } else {
                frames.fill([0.2; 2]);
            }
            Ok(())
        }
        fn note(&mut self, _: u8, _: f32, _: bool) {}
    }
    #[test]
    fn faulty_plugin_passes_dry_retries_with_backoff_and_recovers() {
        let descriptor = PluginDescriptor {
            path: "test.vst3".into(),
            class_id: "fake".into(),
            format: PluginFormat::Vst3,
            role: PluginRole::Effect,
            name: "Test".into(),
            vendor: "PirateWave".into(),
            parameters: vec![],
        };
        let mut rack = Rack::default();
        rack.entries.insert(
            1,
            Entry {
                config: NodePlugin::new(descriptor.clone()),
                plugin: Some(Box::new(Fake {
                    descriptor: descriptor.clone(),
                    broken: true,
                })),
                notes: [true; 128],
                disabled: false,
                retry_at: Instant::now(),
                failures: 0,
            },
        );
        let node = Node::new(
            1,
            "Plugin",
            piratewave_core::project::NodeKind::VstEffect,
            [0.; 2],
        );
        let mut frames = [[0.5; 2]; BLOCK_SIZE];
        rack.process(&node, &mut frames);
        assert_eq!(frames, [[0.5; 2]; BLOCK_SIZE]);
        assert!(rack.entries[&1].plugin.is_none());
        assert!(rack.entries[&1].notes.iter().all(|n| !*n));
        rack.recover_with(|_| panic!("Must respect cooldown"));
        rack.entries.get_mut(&1).unwrap().retry_at = Instant::now();
        rack.recover_with(|_| Err(anyhow::anyhow!("still unavailable")));
        assert_eq!(rack.entries[&1].failures, 1);
        assert!(rack.entries[&1].retry_at > Instant::now());
        rack.entries.get_mut(&1).unwrap().retry_at = Instant::now();
        rack.recover_with(|d| {
            Ok(Box::new(Fake {
                descriptor: d.clone(),
                broken: false,
            }))
        });
        rack.process(&node, &mut frames);
        assert_eq!(frames, [[0.2; 2]; BLOCK_SIZE]);
        assert!(rack.entries[&1].plugin.is_some());
    }
    struct EditingFake {
        descriptor: PluginDescriptor,
        edits: Rc<RefCell<Vec<(u32, f64)>>>,
        applied: Rc<RefCell<Vec<(u32, f64)>>>,
    }
    struct SnapshotFake {
        descriptor: PluginDescriptor,
        calls: Rc<RefCell<usize>>,
        initially_pending: bool,
    }
    impl Hosted for SnapshotFake {
        fn descriptor(&self) -> &PluginDescriptor {
            &self.descriptor
        }
        fn set_parameter(&mut self, _: u32, _: f64) {}
        fn process(&mut self, _: &mut [Frame]) -> Result<()> {
            Ok(())
        }
        fn note(&mut self, _: u8, _: f32, _: bool) {}
        fn capture_state(&mut self) -> Result<Option<piratewave_core::plugins::PluginState>> {
            let mut calls = self.calls.borrow_mut();
            *calls += 1;
            if self.initially_pending && *calls == 1 {
                anyhow::bail!("state capture is pending");
            }
            Ok(Some(piratewave_core::plugins::PluginState {
                component: vec![*calls as u8].into(),
                controller: None,
                vst2_bank: false,
                vst2_program: None,
            }))
        }
    }
    #[test]
    fn snapshot_batch_keeps_completed_plugins_while_another_is_pending() {
        let descriptor = PluginDescriptor {
            path: "fake.vst3".into(),
            class_id: "fake".into(),
            format: PluginFormat::Vst3,
            role: PluginRole::Effect,
            name: "Fake".into(),
            vendor: "Test".into(),
            parameters: vec![],
        };
        let a = Rc::new(RefCell::new(0));
        let b = Rc::new(RefCell::new(0));
        let mut rack = Rack::default();
        for (id, calls, pending) in [(1, a.clone(), false), (2, b.clone(), true)] {
            rack.entries.insert(
                id,
                Entry {
                    config: NodePlugin::new(descriptor.clone()),
                    plugin: Some(Box::new(SnapshotFake {
                        descriptor: descriptor.clone(),
                        calls,
                        initially_pending: pending,
                    })),
                    notes: [false; 128],
                    disabled: false,
                    retry_at: Instant::now(),
                    failures: 0,
                },
            );
        }
        let (states, errors) = rack.capture_states(false);
        assert_eq!(states.len(), 1);
        assert_eq!(errors.len(), 1);
        let (states, errors) = rack.capture_states(false);
        assert_eq!(states.len(), 2);
        assert!(errors.is_empty());
        assert_eq!((*a.borrow(), *b.borrow()), (1, 2));
        rack.capture_states(false);
        assert_eq!((*a.borrow(), *b.borrow()), (2, 3));
    }
    impl Hosted for EditingFake {
        fn descriptor(&self) -> &PluginDescriptor {
            &self.descriptor
        }
        fn set_parameter(&mut self, id: u32, value: f64) {
            self.applied.borrow_mut().push((id, value));
        }
        fn take_editor_edits(&mut self) -> Vec<(u32, f64)> {
            std::mem::take(&mut *self.edits.borrow_mut())
        }
        fn process(&mut self, _: &mut [Frame]) -> Result<()> {
            Ok(())
        }
        fn note(&mut self, _: u8, _: f32, _: bool) {}
    }
    #[test]
    fn pending_editor_edits_survive_stale_project_updates_and_yield_to_slider() {
        let descriptor = PluginDescriptor {
            path: "fake.vst3".into(),
            class_id: "fake".into(),
            format: PluginFormat::Vst3,
            role: PluginRole::Effect,
            name: "Fake".into(),
            vendor: "Test".into(),
            parameters: vec![PluginParameter {
                id: 5,
                name: "Gain".into(),
                unit: String::new(),
                default: 0.5,
                steps: 0,
            }],
        };
        let edits = Rc::new(RefCell::new(vec![(5, 0.6)]));
        let applied = Rc::new(RefCell::new(Vec::new()));
        let mut rack = Rack::default();
        rack.entries.insert(
            7,
            Entry {
                config: NodePlugin::new(descriptor.clone()),
                plugin: Some(Box::new(EditingFake {
                    descriptor: descriptor.clone(),
                    edits: edits.clone(),
                    applied: applied.clone(),
                })),
                notes: [false; 128],
                disabled: false,
                retry_at: Instant::now(),
                failures: 0,
            },
        );
        let mut node = Node::new(
            7,
            "Fake",
            piratewave_core::project::NodeKind::VstEffect,
            [0.; 2],
        );
        node.plugin = Some(NodePlugin::new(descriptor));
        let mut project = Project {
            nodes: vec![node],
            ..Project::default()
        };
        rack.editor_idle();
        assert_eq!(rack.pending_edits[&7][&5], (0.5, 0.6));
        edits.borrow_mut().push((5, 0.7));
        rack.editor_idle();
        project.nodes[0]
            .plugin
            .as_mut()
            .unwrap()
            .parameters
            .insert(5, 0.6);
        rack.sync(&project);
        assert_eq!(rack.pending_edits[&7][&5], (0.6, 0.7));
        assert!(
            applied.borrow().is_empty(),
            "stale update must not overwrite the editor"
        );
        rack.set_parameters(7, &[(5, 0.2)]);
        assert!(!rack.pending_edits[&7].contains_key(&5));
        assert_eq!(&*applied.borrow(), &[(5, 0.2)]);
    }
}
