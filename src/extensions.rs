//! Native extension discovery/installation. Never called by graph rendering.
use anyhow::{Context, Result};
use piratewave_core::{
    extensions::{self, Instance, Provider, State},
    Frame, BLOCK_SIZE,
};
use piratewave_extension_sdk::{Api, Manifest, ABI_VERSION, MAX_PORTS};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::c_void,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

pub fn root() -> PathBuf {
    crate::storage::settings_path().with_file_name("extensions")
}
#[derive(Default, Serialize, Deserialize)]
struct Selection {
    #[serde(default)]
    versions: BTreeMap<String, String>,
    #[serde(default)]
    disabled: BTreeSet<String>,
}
fn selection(root: &Path) -> Result<Selection> {
    let p = root.join("selection.json");
    if p.exists() {
        Ok(serde_json::from_slice(&std::fs::read(p)?)?)
    } else {
        Ok(Selection::default())
    }
}
fn save_selection(root: &Path, s: &Selection) -> Result<()> {
    std::fs::create_dir_all(root)?;
    crate::storage::save_json(&root.join("selection.json"), s)
}
#[derive(Clone)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub version: String,
    pub enabled: bool,
    pub active: bool,
    pub status: String,
}
static ROWS: OnceLock<Mutex<Vec<Row>>> = OnceLock::new();
pub fn rows() -> Vec<Row> {
    ROWS.get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}
fn set_row(row: Row) {
    let mut rows = ROWS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    rows.retain(|r| r.id != row.id);
    rows.push(row);
}
pub fn enable(id: &str, enabled: bool) -> Result<()> {
    let root = root();
    let mut s = selection(&root)?;
    if enabled {
        s.disabled.remove(id);
    } else {
        s.disabled.insert(id.into());
    }
    save_selection(&root, &s)?;
    if let Some(r) = ROWS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter_mut()
        .find(|r| r.id == id)
    {
        r.enabled = enabled;
        r.status = "Restart PirateWave to apply this change".into();
    }
    Ok(())
}
fn manifest(path: &Path) -> Result<Manifest> {
    let file = path.join("manifest.json");
    anyhow::ensure!(
        std::fs::metadata(&file)?.len() <= 262144,
        "Manifest too large"
    );
    let m = piratewave_extension_sdk::parse_manifest(&std::fs::read(file)?)
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        piratewave_extension_sdk::valid_id(&m.version)
            && !m.version.contains('/')
            && !m.version.starts_with('.'),
        "Invalid package version"
    );
    anyhow::ensure!(
        m.id != "." && m.id != ".." && !m.id.starts_with('.'),
        "Invalid package ID"
    );
    let base = path.canonicalize()?;
    let dll = path.join(&m.library).canonicalize()?;
    anyhow::ensure!(
        dll.parent() == Some(base.as_path()),
        "Library escapes package"
    );
    anyhow::ensure!(
        std::fs::metadata(&dll)?.len() <= 64 * 1024 * 1024,
        "Extension DLL too large"
    );
    anyhow::ensure!(
        piratewave_vst2_host::pe::architecture(&dll)?
            == piratewave_vst2_host::pe::Architecture::X64,
        "Extensions require Windows x64"
    );
    Ok(m)
}
struct Native {
    _library: libloading::Library,
    api: Api,
}
fn load(path: &Path, m: &Manifest) -> Result<Arc<Native>> {
    // Absolute DLL path and LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR / DEFAULT_DIRS
    // prevent resolving dependencies through the process working directory.
    let dll = path.join(&m.library).canonicalize()?;
    #[cfg(windows)]
    let library: libloading::Library = unsafe {
        libloading::os::windows::Library::load_with_flags(&dll, 0x00000100 | 0x00001000)?
    }
    .into();
    #[cfg(not(windows))]
    let library = unsafe { libloading::Library::new(&dll)? };
    unsafe {
        let entry: libloading::Symbol<unsafe extern "C" fn() -> *const Api> =
            library.get(b"piratewave_extension_api\0")?;
        let api = entry();
        anyhow::ensure!(!api.is_null(), "Missing ABI table");
        // Read only the fixed prefix until version/size are accepted.
        let header = api.cast::<u32>();
        anyhow::ensure!(
            *header == ABI_VERSION && *header.add(1) == std::mem::size_of::<Api>() as u32,
            "Incompatible extension ABI"
        );
        let api = std::ptr::read(api);
        let mut len = 0;
        let bytes = (api.describe)(&mut len);
        anyhow::ensure!(!bytes.is_null() && len <= 262144, "Invalid DLL manifest");
        let actual = piratewave_extension_sdk::parse_manifest(std::slice::from_raw_parts(
            bytes,
            len as usize,
        ))
        .map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            &actual == m,
            "DLL identity/schema differs from package manifest"
        );
        Ok(Arc::new(Native {
            _library: library,
            api,
        }))
    }
}
struct Factory {
    native: Arc<Native>,
    index: u32,
    schema: u32,
}
struct Hosted {
    native: Arc<Native>,
    handle: *mut c_void,
}
// Each instance is exclusively owned by one graph/export worker. There is no
// concurrent invocation; the ABI requires instances to be movable between threads.
unsafe impl Send for Hosted {}
impl Drop for Hosted {
    fn drop(&mut self) {
        unsafe {
            (self.native.api.destroy)(self.handle);
        }
    }
}
impl Provider for Factory {
    fn migrate(&self, old: &State) -> Result<State, String> {
        old.validate()?;
        let mut bytes = vec![0u8; piratewave_extension_sdk::MAX_STATE];
        let mut len = 0;
        if unsafe {
            (self.native.api.migrate)(
                self.index,
                old.schema,
                old.bytes.as_ptr(),
                old.bytes.len() as u32,
                bytes.as_mut_ptr(),
                &mut len,
            )
        } != 0
            || len as usize > bytes.len()
        {
            return Err("Extension does not support this state migration".into());
        }
        bytes.truncate(len as usize);
        Ok(State {
            schema: self.schema,
            bytes,
        })
    }

    fn create(&self, rate: f32, state: &State) -> Result<Box<dyn Instance>, String> {
        state.validate()?;
        let handle = unsafe {
            (self.native.api.create)(
                self.index,
                rate,
                BLOCK_SIZE as u32,
                state.schema,
                state.bytes.as_ptr(),
                state.bytes.len() as u32,
            )
        };
        if handle.is_null() {
            return Err("Extension initialization failed".into());
        }
        Ok(Box::new(Hosted {
            native: self.native.clone(),
            handle,
        }))
    }
}
impl Instance for Hosted {
    fn set(&mut self, index: u32, value: f32) -> bool {
        unsafe { (self.native.api.set_parameter)(self.handle, index, value) == 0 }
    }
    fn reset(&mut self) -> bool {
        unsafe { (self.native.api.reset)(self.handle) == 0 }
    }
    fn process(
        &mut self,
        inputs: &[[Frame; BLOCK_SIZE]],
        outputs: &mut [[Frame; BLOCK_SIZE]],
    ) -> bool {
        let mut ins = [std::ptr::null(); MAX_PORTS];
        let mut outs = [std::ptr::null_mut(); MAX_PORTS];
        for (p, b) in ins.iter_mut().zip(inputs) {
            *p = b.as_ptr().cast();
        }
        for (p, b) in outs.iter_mut().zip(outputs.iter_mut()) {
            *p = b.as_mut_ptr().cast();
        }
        unsafe {
            (self.native.api.process)(
                self.handle,
                ins.as_ptr(),
                inputs.len() as u32,
                outs.as_ptr(),
                outputs.len() as u32,
                BLOCK_SIZE as u32,
            ) == 0
        }
    }
}
pub fn scan_child(path: &Path, report: &Path) -> Result<()> {
    let m = manifest(path)?;
    let native = load(path, &m)?;
    for (index, t) in m.types.iter().enumerate() {
        let factory = Factory {
            native: native.clone(),
            index: index as u32,
            schema: t.state_schema,
        };
        let mut i = factory
            .create(
                48000.,
                &State {
                    schema: t.state_schema,
                    bytes: t.default_state.clone(),
                },
            )
            .map_err(anyhow::Error::msg)?;
        for (n, p) in t.parameters.iter().enumerate() {
            anyhow::ensure!(i.set(n as u32, p.default), "Default parameter rejected");
        }
        let inputs = vec![[[0.; 2]; BLOCK_SIZE]; t.inputs.len()];
        let mut outputs = vec![[[0.; 2]; BLOCK_SIZE]; t.outputs.len()];
        anyhow::ensure!(
            i.process(&inputs, &mut outputs)
                && outputs.iter().flatten().flatten().all(|v| v.is_finite())
                && i.reset(),
            "Extension smoke callback failed"
        );
    }
    std::fs::write(report, serde_json::to_vec(&m)?)?;
    Ok(())
}
fn scan(path: &Path) -> Result<Manifest> {
    use std::process::{Command, Stdio};
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let report = std::env::temp_dir().join(format!(
        "piratewave-extension-{}-{stamp}.json",
        std::process::id()
    ));
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.arg("--scan-extension")
        .arg(path)
        .arg(&report)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd.spawn()?;
    let start = Instant::now();
    let result = loop {
        match child.try_wait() {
            Ok(Some(s)) => {
                break if s.success() {
                    std::fs::read(&report)
                        .context("Scanner produced no report")
                        .and_then(|b| {
                            piratewave_extension_sdk::parse_manifest(&b).map_err(anyhow::Error::msg)
                        })
                } else {
                    Err(anyhow::anyhow!("Extension scanner failed: {s}"))
                }
            }
            Ok(None) if start.elapsed() < Duration::from_secs(10) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            other => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(anyhow::anyhow!("Extension scan timeout/error: {other:?}"));
            }
        }
    };
    let _ = std::fs::remove_file(report);
    result
}
pub fn initialize() -> Result<()> {
    let root = root();
    let selected = selection(&root)?;
    let mut types = vec![];
    for (id, version) in &selected.versions {
        if !piratewave_extension_sdk::valid_id(id)
            || id.contains('/')
            || !piratewave_extension_sdk::valid_id(version)
            || version.contains('/')
            || id.starts_with('.')
            || version.starts_with('.')
        {
            crate::logging::error(format_args!("Invalid installed extension selector"));
            continue;
        }
        let path = root.join(id).join(version);
        let enabled = !selected.disabled.contains(id);
        let result = (|| -> Result<Manifest> {
            let m = manifest(&path)?;
            anyhow::ensure!(
                &m.id == id && &m.version == version,
                "Installed identity changed"
            );
            if enabled {
                let scanned = scan(&path)?;
                anyhow::ensure!(scanned == m, "Package changed during scan");
                let native = load(&path, &m)?;
                for (index, t) in m.types.iter().enumerate() {
                    types.push((
                        t.clone(),
                        Arc::new(Factory {
                            native: native.clone(),
                            index: index as u32,
                            schema: t.state_schema,
                        }) as Arc<dyn Provider>,
                    ));
                }
            }
            Ok(m)
        })();
        let (name, status, active) = match result {
            Ok(m) => (
                m.name,
                if enabled { "Ready" } else { "Disabled" }.into(),
                enabled,
            ),
            Err(e) => {
                crate::logging::error(format_args!("Extension {id} unavailable: {e:#}"));
                (id.clone(), format!("Unavailable: {e:#}"), false)
            }
        };
        set_row(Row {
            id: id.clone(),
            name,
            version: version.clone(),
            enabled,
            active,
            status,
        });
    }
    extensions::install(types).map_err(anyhow::Error::msg)
}
/// Explicit installation of a trusted .piratewaveext ZIP. Never overwrite loaded
/// versions; a new version is selected only after complete isolated validation.
pub fn install(package: &Path) -> Result<()> {
    install_at(package, &root())
}
fn install_at(package: &Path, root: &Path) -> Result<()> {
    let file = std::fs::File::open(package)?;
    anyhow::ensure!(
        file.metadata()?.len() <= 64 * 1024 * 1024,
        "Package exceeds 64 MiB"
    );
    let mut zip = zip::ZipArchive::new(file)?;
    anyhow::ensure!(zip.len() <= 128, "Too many package files");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    std::fs::create_dir_all(root)?;
    let stage = root.join(format!(".install-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&stage)?;
    let outcome = (|| -> Result<()> {
        let mut names = BTreeSet::new();
        let mut size = 0u64;
        for index in 0..zip.len() {
            let mut f = zip.by_index(index)?;
            let path = f
                .enclosed_name()
                .ok_or_else(|| anyhow::anyhow!("Unsafe package path"))?
                .to_owned();
            anyhow::ensure!(
                !f.name().contains(['\\', ':'])
                    && !path
                        .components()
                        .any(|c| c.as_os_str().to_string_lossy().ends_with(['.', ' ']))
                    && names.insert(path.to_string_lossy().to_lowercase()),
                "Unsafe/duplicate package path"
            );
            anyhow::ensure!(
                f.unix_mode().is_none_or(|m| m & 0o170000 != 0o120000),
                "Package symlinks are forbidden"
            );
            size = size
                .checked_add(f.size())
                .ok_or_else(|| anyhow::anyhow!("Invalid package size"))?;
            anyhow::ensure!(size <= 128 * 1024 * 1024, "Expanded package too large");
            let target = stage.join(path);
            if f.is_dir() {
                std::fs::create_dir_all(target)?;
            } else {
                std::fs::create_dir_all(target.parent().unwrap())?;
                let mut out = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(target)?;
                std::io::copy(&mut f, &mut out)?;
            }
        }
        let m = scan(&stage)?;
        let target = root.join(&m.id).join(&m.version);
        anyhow::ensure!(
            !target.exists(),
            "That version is installed; publish a new version instead of replacing a loaded DLL"
        );
        std::fs::create_dir_all(target.parent().unwrap())?;
        std::fs::rename(&stage, &target)?;
        let mut s = selection(root)?;
        s.versions.insert(m.id.clone(), m.version.clone());
        s.disabled.remove(&m.id);
        save_selection(root, &s)?;
        set_row(Row {
            id: m.id,
            name: m.name,
            version: m.version,
            enabled: true,
            active: false,
            status: "Installed; restart PirateWave to activate".into(),
        });
        Ok(())
    })();
    // Stage is always a uniquely created child of root; successful installs moved it.
    if stage.exists() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    outcome
}

pub fn fixture_catalog(path: &Path) -> Result<Manifest> {
    let m = scan(path)?;
    let native = load(path, &m)?;
    set_row(Row {
        id: m.id.clone(),
        name: m.name.clone(),
        version: m.version.clone(),
        enabled: true,
        active: true,
        status: "SDK fixture (not installed)".into(),
    });
    extensions::install(
        m.types
            .iter()
            .enumerate()
            .map(|(i, d)| {
                (
                    d.clone(),
                    Arc::new(Factory {
                        native: native.clone(),
                        index: i as u32,
                        schema: d.state_schema,
                    }) as Arc<dyn Provider>,
                )
            })
            .collect(),
    )
    .map_err(anyhow::Error::msg)?;
    Ok(m)
}
/// Opt-in native fixture. Does not install packages or open audio devices.
pub fn self_test(path: &Path) -> Result<()> {
    use piratewave_core::{
        effects::{EffectRegistry, EffectSpec},
        graph::Graph,
        project::{InputPort, Node, NodeKind, OutputPort, Project},
        recording_effects::RecordingEffects,
    };
    let m = fixture_catalog(path)?;
    let registry = EffectRegistry::default();
    let effect = m
        .types
        .iter()
        .find(|t| t.kind == piratewave_extension_sdk::TypeKind::Processor)
        .context("No fixture processor")?;
    let node = m
        .types
        .iter()
        .find(|t| t.kind == piratewave_extension_sdk::TypeKind::Node)
        .context("No fixture node")?;
    anyhow::ensure!(
        node.inputs.len() == 2 && node.outputs.len() == 3,
        "Expected SDK Multi Mix fixture"
    );
    let mut spec = EffectSpec::new(
        registry
            .get(&effect.id)
            .context("Processor not registered")?,
    );
    spec.parameters.insert("width".into(), 0.);
    let mut width = registry.create(&spec, 48000.).map_err(anyhow::Error::msg)?;
    width.reset();
    let mut stereo = [[0.3, -0.1]; BLOCK_SIZE];
    width.process(&mut stereo);
    anyhow::ensure!(
        stereo
            .iter()
            .all(|f| (f[0] - 0.1).abs() < 1e-6 && (f[1] - 0.1).abs() < 1e-6),
        "Native rack processor did not perform stereo-to-mono DSP"
    );

    let mut p = Project {
        version: 1,
        name: "Native extension validation".into(),
        nodes: vec![
            Node::new(1, "A", NodeKind::Soundboard, [0.; 2]),
            Node::new(2, "B", NodeKind::Soundboard, [0.; 2]),
            Node::new(
                3,
                "Multi",
                NodeKind::Extension {
                    definition: Box::new(node.clone()),
                },
                [300.; 2],
            ),
        ],
        connections: vec![],
        sounds: vec![],
    };
    for i in 0..3 {
        p.nodes.push(Node::new(
            4 + i,
            "Output",
            NodeKind::Output { device: None },
            [600., i as f32 * 150.],
        ));
        p.connect_ports(
            3,
            4 + i,
            OutputPort::Extension(node.outputs[i as usize].id),
            InputPort::Main,
        )
        .map_err(anyhow::Error::msg)?;
    }
    p.connect_ports(
        1,
        3,
        OutputPort::Passthrough,
        InputPort::Extension(node.inputs[0].id),
    )
    .map_err(anyhow::Error::msg)?;
    p.connect_ports(
        2,
        3,
        OutputPort::Passthrough,
        InputPort::Extension(node.inputs[1].id),
    )
    .map_err(anyhow::Error::msg)?;
    let inputs = [
        (1, [[0.2, 0.1]; BLOCK_SIZE]),
        (2, [[0.1, -0.1]; BLOCK_SIZE]),
    ];
    let mut g = Graph::new(&p, &registry).map_err(anyhow::Error::msg)?;
    g.process(&inputs);
    for (id, expected) in [(4, [0.3, 0.]), (5, [0.2, 0.1]), (6, [0.1, -0.1])] {
        let block = &g.nodes.iter().find(|n| n.config.id == id).unwrap().frames;
        anyhow::ensure!(
            block
                .iter()
                .all(|f| (f[0] - expected[0]).abs() < 1e-6 && (f[1] - expected[1]).abs() < 1e-6),
            "Wrong independent port output"
        );
    }
    let mut taps = vec![];
    g.process_recording(
        &inputs,
        |_, _| false,
        |_, _, _, _| {},
        |id, port, _, _| {
            if id == 3 {
                taps.push(port);
            }
        },
    );
    for port in &node.outputs {
        anyhow::ensure!(
            taps.contains(&OutputPort::Extension(port.id)),
            "Missing distinct recording history"
        );
    }
    let before = serde_json::to_vec(&p)?;
    let restored: Project = serde_json::from_slice(&before)?;
    Graph::new(&restored, &registry).map_err(anyhow::Error::msg)?;
    // A saved snapshot with a different schema must remain openable but silent.
    let mut missing = p.clone();
    if let NodeKind::Extension { definition } = &mut missing.nodes[2].kind {
        definition.id = "org.piratewave.missing/node".into();
    }
    let mut absent = Graph::new(&missing, &registry).map_err(anyhow::Error::msg)?;
    absent.process(&inputs);
    anyhow::ensure!(
        absent
            .nodes
            .iter()
            .find(|n| n.config.id == 4)
            .unwrap()
            .frames
            == [[0.; 2]; BLOCK_SIZE],
        "Unavailable node did not silence"
    );
    let mut unavailable = spec.clone();
    unavailable.kind = "org.piratewave.missing/effect".into();
    unavailable.extension_descriptor.as_mut().unwrap().id = unavailable.kind.clone();
    let mut dry = [[0.2, -0.1]; BLOCK_SIZE];
    registry
        .create(&unavailable, 48000.)
        .map_err(anyhow::Error::msg)?
        .process(&mut dry);
    anyhow::ensure!(
        dry == [[0.2, -0.1]; BLOCK_SIZE],
        "Unavailable effect did not pass through"
    );
    // Independent live/export instances and shared descriptor-based controls.
    p.nodes[0].effects.push(spec.clone());
    let catalog = piratewave_core::control::catalog(&p, &registry, false);
    for category in [
        "extension_parameter",
        "extension_action",
        "effect_parameter",
    ] {
        anyhow::ensure!(
            catalog.iter().any(|e| e.selector.category == category),
            "Missing controller entry {category}"
        );
    }
    let knob = catalog
        .iter()
        .find(|e| e.selector.category == "extension_parameter")
        .unwrap()
        .selector
        .clone();
    piratewave_core::control::adjust(&mut p, &registry, &knob, -25, 1., false)
        .map_err(anyhow::Error::msg)?;
    piratewave_core::control::adjust(&mut p, &registry, &knob, 0, 1., true)
        .map_err(anyhow::Error::msg)?;
    let mut other_project = p.clone();
    for n in &mut other_project.nodes {
        n.id += 1000;
    }
    for c in &mut other_project.connections {
        c.from += 1000;
        c.to += 1000;
    }
    piratewave_core::control::adjust(&mut other_project, &registry, &knob, -1, 1., false)
        .map_err(anyhow::Error::msg)?;
    let mut duplicate = other_project
        .nodes
        .iter()
        .find(|n| matches!(n.kind, NodeKind::Extension { .. }))
        .unwrap()
        .clone();
    duplicate.id += 1000;
    other_project.nodes.push(duplicate);
    anyhow::ensure!(
        piratewave_core::control::adjust(&mut other_project, &registry, &knob, 1, 1., false)
            .unwrap_err()
            .contains("Ambiguous"),
        "Duplicate extension names were not rejected"
    );
    other_project
        .nodes
        .retain(|n| !matches!(n.kind, NodeKind::Extension { .. }));
    anyhow::ensure!(
        piratewave_core::control::adjust(&mut other_project, &registry, &knob, 1, 1., false)
            .unwrap_err()
            .contains("Missing"),
        "Deleted extension was still controlled"
    );
    let action = catalog
        .iter()
        .find(|e| e.selector.category == "extension_action" && e.selector.target.is_empty())
        .unwrap()
        .selector
        .clone();
    let action = piratewave_core::control::resolve(&p, &action).map_err(anyhow::Error::msg)?;
    if let piratewave_core::shortcuts::ShortcutAction::Extension { id, effect, action } = action {
        extensions::apply_action(&mut p, id, effect, action).map_err(anyhow::Error::msg)?;
    } else {
        anyhow::bail!("Wrong extension dispatch");
    }
    let mut a = RecordingEffects::new(std::slice::from_ref(&spec), false, &registry)
        .map_err(anyhow::Error::msg)?;
    let mut b = RecordingEffects::new(std::slice::from_ref(&spec), false, &registry)
        .map_err(anyhow::Error::msg)?;
    let mut first = [[0.3, -0.1]; BLOCK_SIZE];
    let mut second = first;
    a.process(&mut first);
    b.process(&mut second);
    anyhow::ensure!(first == second, "Independent export instances differ");
    let entry = extensions::get(&node.id).unwrap();
    let migrated = entry
        .provider
        .migrate(&State {
            schema: 1,
            bytes: vec![],
        })
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        migrated.schema == 2 && migrated.bytes.is_empty(),
        "Explicit SDK state migration failed"
    );
    #[cfg(windows)]
    crate::hotkeys::self_test().map_err(anyhow::Error::msg)?;
    // Exercise real installation in an isolated, disposable settings root.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temp = std::env::temp_dir().join(format!(
        "piratewave-extension-install-test-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir(&temp)?;
    let installation = (|| -> Result<()> {
        let package = temp.join("example.piratewaveext");
        let mut archive = zip::ZipWriter::new(std::fs::File::create(&package)?);
        use std::io::Write;
        for name in ["manifest.json", m.library.as_str()] {
            archive.start_file(
                name,
                zip::write::FileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )?;
            archive.write_all(&std::fs::read(path.join(name))?)?;
        }
        archive.finish()?;
        let root = temp.join("settings");
        install_at(&package, &root)?;
        let selected = selection(&root)?;
        anyhow::ensure!(
            selected.versions.get(&m.id) == Some(&m.version),
            "Install did not select version"
        );
        anyhow::ensure!(
            install_at(&package, &root).is_err(),
            "Installed version was overwritten"
        );
        anyhow::ensure!(
            std::fs::read(root.join(&m.id).join(&m.version).join(&m.library))?
                == std::fs::read(path.join(&m.library))?,
            "Installed DLL changed"
        );
        Ok(())
    })();
    std::fs::remove_dir_all(&temp)?;
    installation?;
    let start = Instant::now();
    for _ in 0..10000 {
        g.process(&inputs);
    }
    println!("PASS: isolated DLL scan/ABI identity; native processor/node DSP; independent multiport routing and recording taps; legacy/project round trip; unavailable fallback; action/knob/reset catalogs; independent live/export instances. Native graph: {:.3}% realtime CPU budget.",start.elapsed().as_secs_f64()/(10000.*BLOCK_SIZE as f64/48000.)*100.);
    Ok(())
}

/// Explicit, offline migration: the original project is never overwritten.
pub fn migrate_project(input: &Path, output: &Path) -> Result<()> {
    anyhow::ensure!(
        !output.exists(),
        "Choose a new output filename; migration never overwrites an existing file"
    );
    initialize()?;
    let mut p: piratewave_core::project::Project = serde_json::from_slice(&std::fs::read(input)?)?;
    for n in &mut p.nodes {
        if let piratewave_core::project::NodeKind::Extension { definition } = &mut n.kind {
            let e = extensions::get(&definition.id).context("Required extension is unavailable")?;
            if definition.state_schema != e.descriptor.state_schema {
                let mut state = e
                    .provider
                    .migrate(&n.extension.state)
                    .map_err(anyhow::Error::msg)?;
                state.schema = e.descriptor.state_schema;
                n.extension.state = state;
            }
            **definition = e.descriptor.clone();
        }
        for spec in &mut n.effects {
            if let Some(old) = &spec.extension_descriptor {
                let e = extensions::get(&old.id)
                    .context("Required extension processor is unavailable")?;
                if old.state_schema != e.descriptor.state_schema {
                    let mut state = e
                        .provider
                        .migrate(
                            spec.extension
                                .as_ref()
                                .context("Missing saved extension state")?,
                        )
                        .map_err(anyhow::Error::msg)?;
                    state.schema = e.descriptor.state_schema;
                    spec.extension = Some(state);
                }
                spec.extension_descriptor = Some(e.descriptor.clone());
            }
        }
    }
    // Port/action/parameter removals require an author-provided new type ID or
    // explicit project edits; never silently discard connections or settings.
    piratewave_core::graph::validate_with_registry(
        &p,
        &piratewave_core::effects::EffectRegistry::default(),
    )
    .map_err(anyhow::Error::msg)?;
    for n in &p.nodes {
        for (d, state, values) in std::iter::once(
            extensions::descriptor(n, None)
                .map(|d| (d, &n.extension.state, &n.extension.parameters)),
        )
        .flatten()
        .chain(n.effects.iter().filter_map(|e| {
            e.extension_descriptor
                .as_ref()
                .zip(e.extension.as_ref())
                .map(|(d, s)| (d, s, &e.parameters))
        })) {
            let entry = extensions::get(&d.id).context("Extension disappeared during migration")?;
            let mut instance = entry
                .provider
                .create(48000., state)
                .map_err(anyhow::Error::msg)?;
            for (index, param) in entry.descriptor.parameters.iter().enumerate() {
                anyhow::ensure!(
                    instance.set(
                        index as u32,
                        *values.get(&param.id).unwrap_or(&param.default)
                    ),
                    "Migrated extension rejected its parameters"
                );
            }
        }
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    use std::io::Write;
    f.write_all(&serde_json::to_vec_pretty(&p)?)?;
    f.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn install_rejects_traversal_duplicate_names_and_invalid_archives() {
        let base = std::env::temp_dir().join(format!(
            "piratewave-extension-invalid-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&base).unwrap();
        for (index, names) in [
            vec!["../escape"],
            vec!["file.txt", "FILE.TXT"],
            vec!["bad\\file"],
            vec!["trailing."],
        ]
        .iter()
        .enumerate()
        {
            let package = base.join(format!("bad{index}.piratewaveext"));
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&package).unwrap());
            for name in names {
                zip.start_file(*name, zip::write::FileOptions::default())
                    .unwrap();
                zip.write_all(b"invalid").unwrap();
            }
            zip.finish().unwrap();
            assert!(install_at(&package, &base.join(format!("root{index}"))).is_err());
            assert!(!base.join("escape").exists());
        }
        std::fs::remove_dir_all(base).unwrap();
    }
}
