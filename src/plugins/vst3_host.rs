//! VST3 COM hosting. All object calls and destruction stay on the owning thread.
use super::{vst3_support::*, *};
use ::vst3::{
    Steinberg::{Vst::*, *},
    *,
};
use std::rc::Rc;
use std::{
    ffi::c_char,
    ptr,
    sync::{atomic::Ordering, Arc, Mutex, OnceLock, Weak},
};
#[cfg(windows)]
#[path = "vst3_worker.rs"]
mod worker;
#[cfg(windows)]
pub(super) use worker::Threaded;

fn ctext(bytes: &[c_char]) -> String {
    String::from_utf8_lossy(
        &bytes
            .iter()
            .take_while(|c| **c != 0)
            .map(|c| *c as u8)
            .collect::<Vec<_>>(),
    )
    .into()
}
fn utext(chars: &[u16]) -> String {
    String::from_utf16_lossy(
        &chars
            .iter()
            .copied()
            .take_while(|c| *c != 0)
            .collect::<Vec<_>>(),
    )
}
fn hex(cid: &TUID) -> String {
    cid.iter().map(|b| format!("{:02x}", *b as u8)).collect()
}
fn cid(text: &str) -> Result<TUID> {
    anyhow::ensure!(text.len() == 32 && text.is_ascii(), "Invalid VST3 class ID");
    let mut id = [0; 16];
    for (i, b) in id.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)? as i8;
    }
    Ok(id)
}
fn binary(path: &Path) -> Result<PathBuf> {
    if path.is_file() {
        return Ok(path.to_owned());
    }
    let folder = path.join("Contents/x86_64-win");
    let expected = folder.join(path.file_name().context("Invalid VST3 bundle name")?);
    if expected.is_file() {
        return Ok(expected);
    }
    let files: Vec<_> = std::fs::read_dir(&folder)
        .context("VST3 bundle has no Contents/x86_64-win folder (32-bit plugins are unsupported)")?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("vst3"))
        })
        .collect();
    anyhow::ensure!(
        files.len() == 1,
        "VST3 bundle must contain one x86_64-win module"
    );
    Ok(files[0].clone())
}
fn modules() -> &'static Mutex<HashMap<PathBuf, Weak<Module>>> {
    static MODULES: OnceLock<Mutex<HashMap<PathBuf, Weak<Module>>>> = OnceLock::new();
    MODULES.get_or_init(|| Mutex::new(HashMap::new()))
}
struct Module {
    key: PathBuf,
    registered: bool,
    factory_lock: Mutex<()>,
    host: ComWrapper<Host>,
    factory: Option<ComPtr<IPluginFactory>>,
    exit: Option<unsafe extern "system" fn() -> bool>,
    _library: libloading::Library,
}
impl Module {
    fn open(path: &Path) -> Result<Arc<Self>> {
        let key = binary(path)?.canonicalize()?;
        loop {
            let mut cache = modules().lock().unwrap_or_else(|e| e.into_inner());
            if let Some(existing) = cache.get(&key) {
                if let Some(module) = existing.upgrade() {
                    return Ok(module);
                }
                // The last reference is being dropped. Wait for ExitDll to finish
                // before creating a new module for the same binary.
                drop(cache);
                std::thread::yield_now();
                continue;
            }
            let mut module = Self::load_module(&key)?;
            module.registered = true;
            let module = Arc::new(module);
            cache.insert(key.clone(), Arc::downgrade(&module));
            return Ok(module);
        }
    }
    fn load_module(path: &Path) -> Result<Self> {
        unsafe {
            let library = libloading::Library::new(binary(path)?)?;
            let init = library
                .get::<unsafe extern "system" fn() -> bool>(b"InitDll\0")
                .ok()
                .map(|s| *s);
            let exit = library
                .get::<unsafe extern "system" fn() -> bool>(b"ExitDll\0")
                .ok()
                .map(|s| *s);
            let get = *library
                .get::<unsafe extern "system" fn() -> *mut IPluginFactory>(b"GetPluginFactory\0")?;
            if let Some(init) = init {
                anyhow::ensure!(init(), "VST3 InitDll failed");
            }
            let mut module = Self {
                key: path.to_owned(),
                registered: false,
                factory_lock: Mutex::new(()),
                host: ComWrapper::new(Host),
                factory: None,
                exit,
                _library: library,
            };
            module.factory = Some(ComPtr::from_raw(get()).context("VST3 returned no factory")?);
            if let Some(factory) = module.factory().cast::<IPluginFactory3>() {
                factory.setHostContext(
                    module
                        .host
                        .as_com_ref::<IHostApplication>()
                        .unwrap()
                        .as_ptr()
                        .cast(),
                );
            }
            Ok(module)
        }
    }
    fn factory(&self) -> &ComPtr<IPluginFactory> {
        self.factory.as_ref().unwrap()
    }
    fn classes(&self) -> Result<Vec<(TUID, String, PluginRole, String)>> {
        let _lock = self.factory_lock.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            let factory = self.factory();
            let count = factory.countClasses();
            anyhow::ensure!((0..=1024).contains(&count), "Invalid factory class count");
            let mut vendor: PFactoryInfo = std::mem::zeroed();
            factory.getFactoryInfo(&mut vendor);
            let mut classes = vec![];
            for index in 0..count {
                let mut info: PClassInfo = std::mem::zeroed();
                if factory.getClassInfo(index, &mut info) != kResultOk {
                    continue;
                }
                if ctext(&info.category) != "Audio Module Class" {
                    continue;
                }
                let mut role = PluginRole::Effect;
                let mut name_vendor = ctext(&vendor.vendor);
                if let Some(f) = factory.cast::<IPluginFactory2>() {
                    let mut info2: PClassInfo2 = std::mem::zeroed();
                    if f.getClassInfo2(index, &mut info2) == kResultOk {
                        if ctext(&info2.subCategories)
                            .split('|')
                            .any(|s| s == "Instrument")
                        {
                            role = PluginRole::Instrument;
                        }
                        let specific = ctext(&info2.vendor);
                        if !specific.is_empty() {
                            name_vendor = specific;
                        }
                    }
                }
                classes.push((info.cid, ctext(&info.name), role, name_vendor));
            }
            anyhow::ensure!(!classes.is_empty(), "No VST3 audio components found");
            Ok(classes)
        }
    }
}
impl Drop for Module {
    fn drop(&mut self) {
        let mut cache = self
            .registered
            .then(|| modules().lock().unwrap_or_else(|e| e.into_inner()));
        drop(self.factory.take());
        if let Some(exit) = self.exit {
            unsafe {
                exit();
            }
        }
        if let Some(cache) = &mut cache {
            cache.remove(&self.key);
        }
    }
}
pub fn scan(path: &Path) -> Result<Vec<PluginDescriptor>> {
    let classes = Module::open(path)?.classes()?;
    let mut result = vec![];
    for (id, _, _, _) in classes {
        result.push(Vst3::load(path, &hex(&id))?.descriptor().clone());
    }
    Ok(result)
}
struct Bus {
    channels: Vec<[f32; BLOCK_SIZE]>,
    active: bool,
}
/// Audio buffers and parameter/event queues transfer once to the engine thread.
/// Raw pointers refer only to allocations owned by these vectors and are never
/// dereferenced by the controller thread after the transfer.
struct AudioState {
    processor: ComPtr<IAudioProcessor>,
    handler: Arc<HandlerState>,
    inputs: Vec<Bus>,
    outputs: Vec<Bus>,
    input_buses: Vec<AudioBusBuffers>,
    output_buses: Vec<AudioBusBuffers>,
    _input_ptrs: Vec<Vec<*mut f32>>,
    _output_ptrs: Vec<Vec<*mut f32>>,
    events: ComWrapper<Events>,
    out_events: ComWrapper<Events>,
    params: ComWrapper<Params>,
    out_params: ComWrapper<Params>,
    time: i64,
}
// Each queue and buffer is owned exclusively by the audio thread after transfer;
// the VST3 processor's process method is expressly callable from an audio thread.
unsafe impl Send for AudioState {}

impl AudioState {
    fn set_parameter(&mut self, id: u32, value: f64) {
        self.params.set(id, value);
    }
    fn note(&mut self, note: u8, velocity: f32, on: bool) {
        let mut events = self.events.events.borrow_mut();
        if events.len() >= 256 {
            return;
        }
        events.push(Event {
            busIndex: 0,
            sampleOffset: 0,
            ppqPosition: 0.0,
            flags: 1,
            r#type: if on { 0 } else { 1 },
            __field0: if on {
                Event__type0 {
                    noteOn: NoteOnEvent {
                        channel: 0,
                        pitch: note as i16,
                        tuning: 0.0,
                        velocity,
                        length: 0,
                        noteId: note as i32,
                    },
                }
            } else {
                Event__type0 {
                    noteOff: NoteOffEvent {
                        channel: 0,
                        pitch: note as i16,
                        velocity: 0.0,
                        noteId: note as i32,
                        tuning: 0.0,
                    },
                }
            },
        });
    }
    fn process(&mut self, frames: &mut [Frame]) -> Result<()> {
        unsafe {
            anyhow::ensure!(frames.len() == BLOCK_SIZE, "Unexpected VST3 block size");
            // Dynamic I/O changes need a fresh instance; keep the graph's saved stereo contract.
            let restart = self.handler.restart.swap(0, Ordering::AcqRel);
            anyhow::ensure!(
                restart & 2 == 0,
                "Plugin changed its bus layout; restart audio"
            );
            for bus in &mut self.inputs {
                let mono = bus.channels.len() == 1;
                for (channel, buffer) in bus.channels.iter_mut().enumerate() {
                    for (i, frame) in frames.iter().enumerate() {
                        buffer[i] = if !bus.active {
                            0.0
                        } else if mono {
                            (frame[0] + frame[1]) * 0.5
                        } else {
                            frame[channel]
                        };
                    }
                }
            }
            for bus in &mut self.outputs {
                for channel in &mut bus.channels {
                    channel.fill(0.0);
                }
            }
            for bus in &mut self.output_buses {
                bus.silenceFlags = 0;
            }
            let mut context: ProcessContext = std::mem::zeroed();
            context.sampleRate = SAMPLE_RATE as f64;
            context.projectTimeSamples = self.time;
            context.continousTimeSamples = self.time;
            context.tempo = 120.0;
            context.timeSigNumerator = 4;
            context.timeSigDenominator = 4;
            context.state = 2 | 1024 | 8192 | 131072;
            let mut data = ProcessData {
                processMode: 0,
                symbolicSampleSize: 0,
                numSamples: BLOCK_SIZE as i32,
                numInputs: self.input_buses.len() as i32,
                numOutputs: self.output_buses.len() as i32,
                inputs: self.input_buses.as_mut_ptr(),
                outputs: self.output_buses.as_mut_ptr(),
                inputParameterChanges: self
                    .params
                    .as_com_ref::<IParameterChanges>()
                    .unwrap()
                    .as_ptr(),
                outputParameterChanges: self
                    .out_params
                    .as_com_ref::<IParameterChanges>()
                    .unwrap()
                    .as_ptr(),
                inputEvents: self.events.as_com_ref::<IEventList>().unwrap().as_ptr(),
                outputEvents: self.out_events.as_com_ref::<IEventList>().unwrap().as_ptr(),
                processContext: &mut context,
            };
            let result = self.processor.process(&mut data);
            self.events.events.borrow_mut().clear();
            self.out_events.events.borrow_mut().clear();
            self.params.active.borrow_mut().clear();
            self.out_params.active.borrow_mut().clear();
            self.time += BLOCK_SIZE as i64;
            anyhow::ensure!(result == kResultOk, "VST3 process returned {result}");
            let index = self.outputs.iter().position(|b| b.active).unwrap();
            let bus = &self.outputs[index];
            for (i, frame) in frames.iter_mut().enumerate() {
                *frame = [bus.channels[0][i], bus.channels[bus.channels.len() - 1][i]];
                for (channel, sample) in frame.iter_mut().enumerate() {
                    if self.output_buses[index].silenceFlags
                        & (1 << channel.min(bus.channels.len() - 1))
                        != 0
                    {
                        *sample = 0.0;
                    }
                }
            }
            Ok(())
        }
    }
    fn flush_parameter_changes(&mut self) -> Result<()> {
        if self.params.active.borrow().is_empty() {
            return Ok(());
        }
        unsafe {
            let mut data = ProcessData {
                processMode: 0,
                symbolicSampleSize: 0,
                numSamples: 0,
                numInputs: 0,
                numOutputs: 0,
                inputs: ptr::null_mut(),
                outputs: ptr::null_mut(),
                inputParameterChanges: self
                    .params
                    .as_com_ref::<IParameterChanges>()
                    .unwrap()
                    .as_ptr(),
                outputParameterChanges: self
                    .out_params
                    .as_com_ref::<IParameterChanges>()
                    .unwrap()
                    .as_ptr(),
                inputEvents: ptr::null_mut(),
                outputEvents: ptr::null_mut(),
                processContext: ptr::null_mut(),
            };
            anyhow::ensure!(
                self.processor.process(&mut data) == kResultOk,
                "VST3 parameter flush was rejected"
            );
            self.params.active.borrow_mut().clear();
            self.out_params.active.borrow_mut().clear();
        }
        Ok(())
    }
}
pub struct Vst3 {
    #[cfg(windows)]
    editor_values: std::collections::BTreeMap<u32, f64>,
    #[cfg(windows)]
    editor_scale: f32,
    #[cfg(windows)]
    editor_focused: bool,
    #[cfg(windows)]
    editor: Option<(
        ComPtr<IPlugView>,
        ComWrapper<EditorFrame>,
        Rc<piratewave_vst2_host::window::EditorWindow>,
    )>,
    component: Option<ComPtr<IComponent>>,
    processor: Option<ComPtr<IAudioProcessor>>,
    controller: Option<ComPtr<IEditController>>,
    controller_separate: bool,
    component_initialized: bool,
    controller_initialized: bool,
    active: bool,
    processing: bool,
    inputs: Vec<Bus>,
    outputs: Vec<Bus>,
    input_buses: Vec<AudioBusBuffers>,
    output_buses: Vec<AudioBusBuffers>,
    input_ptrs: Vec<Vec<*mut f32>>,
    output_ptrs: Vec<Vec<*mut f32>>,
    handler: ComWrapper<Handler>,
    events: ComWrapper<Events>,
    out_events: ComWrapper<Events>,
    params: ComWrapper<Params>,
    out_params: ComWrapper<Params>,
    descriptor: PluginDescriptor,
    time: i64,
    _module: Arc<Module>,
}
impl Vst3 {
    fn take_audio_state(&mut self) -> AudioState {
        AudioState {
            processor: self.processor.as_ref().unwrap().clone(),
            handler: self.handler.state.clone(),
            inputs: std::mem::take(&mut self.inputs),
            outputs: std::mem::take(&mut self.outputs),
            input_buses: std::mem::take(&mut self.input_buses),
            output_buses: std::mem::take(&mut self.output_buses),
            _input_ptrs: std::mem::take(&mut self.input_ptrs),
            _output_ptrs: std::mem::take(&mut self.output_ptrs),
            events: std::mem::replace(&mut self.events, ComWrapper::new(Events::new())),
            out_events: std::mem::replace(&mut self.out_events, ComWrapper::new(Events::new())),
            params: std::mem::replace(
                &mut self.params,
                ComWrapper::new(Params::new(std::iter::empty())),
            ),
            out_params: std::mem::replace(
                &mut self.out_params,
                ComWrapper::new(Params::new(std::iter::empty())),
            ),
            time: self.time,
        }
    }
    pub fn load(path: &Path, class_id: &str) -> Result<Self> {
        unsafe {
            let module = Module::open(path)?;
            let wanted = cid(class_id)?;
            let (_, name, mut role, vendor) = module
                .classes()?
                .into_iter()
                .find(|(id, _, _, _)| *id == wanted)
                .context("VST3 class is missing; rescan the plugin")?;
            let handler = ComWrapper::new(Handler {
                state: Arc::new(HandlerState::default()),
            });
            let mut raw = ptr::null_mut();
            let result = {
                let _lock = module
                    .factory_lock
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                module.factory().createInstance(
                    wanted.as_ptr(),
                    IComponent::IID.as_ptr().cast(),
                    &mut raw,
                )
            };
            anyhow::ensure!(
                result == kResultOk && !raw.is_null(),
                "VST3 component creation failed ({result})"
            );
            let component = ComPtr::<IComponent>::from_raw(raw.cast()).unwrap();
            let mut this = Self {
                #[cfg(windows)]
                editor_values: Default::default(),
                #[cfg(windows)]
                editor_scale: 0.0,
                #[cfg(windows)]
                editor_focused: false,
                #[cfg(windows)]
                editor: None,
                component: Some(component),
                processor: None,
                controller: None,
                controller_separate: false,
                component_initialized: false,
                controller_initialized: false,
                active: false,
                processing: false,
                inputs: vec![],
                outputs: vec![],
                input_buses: vec![],
                output_buses: vec![],
                input_ptrs: vec![],
                output_ptrs: vec![],
                handler,
                events: ComWrapper::new(Events::new()),
                out_events: ComWrapper::new(Events::new()),
                params: ComWrapper::new(Params::new(std::iter::empty())),
                out_params: ComWrapper::new(Params::new(std::iter::empty())),
                descriptor: PluginDescriptor {
                    path: path.to_string_lossy().into(),
                    class_id: class_id.into(),
                    format: PluginFormat::Vst3,
                    role,
                    name,
                    vendor,
                    parameters: vec![],
                },
                time: 0,
                _module: module,
            };
            let mut parameter_ids = vec![];
            let component = this.component.as_ref().unwrap();
            let context = this
                ._module
                .host
                .as_com_ref::<IHostApplication>()
                .unwrap()
                .as_ptr()
                .cast();
            anyhow::ensure!(
                component.initialize(context) == kResultOk,
                "VST3 initialization failed"
            );
            this.component_initialized = true;
            this.processor = Some(
                component
                    .cast::<IAudioProcessor>()
                    .context("VST3 class has no audio processor")?,
            );
            if let Some(controller) = component.cast::<IEditController>() {
                this.controller = Some(controller);
            } else {
                let mut id = [0; 16];
                if component.getControllerClassId(&mut id) == kResultOk {
                    let mut raw = ptr::null_mut();
                    if {
                        let _lock = this
                            ._module
                            .factory_lock
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        this._module.factory().createInstance(
                            id.as_ptr(),
                            IEditController::IID.as_ptr().cast(),
                            &mut raw,
                        )
                    } == kResultOk
                    {
                        this.controller = ComPtr::from_raw(raw.cast());
                        this.controller_separate = true;
                    }
                }
            }
            if let Some(controller) = &this.controller {
                if this.controller_separate {
                    anyhow::ensure!(
                        controller.initialize(context) == kResultOk,
                        "VST3 controller initialization failed"
                    );
                    this.controller_initialized = true;
                }
                controller.setComponentHandler(
                    this.handler
                        .as_com_ref::<IComponentHandler>()
                        .unwrap()
                        .as_ptr(),
                );
                if this.controller_separate {
                    if let (Some(a), Some(b)) = (
                        component.cast::<IConnectionPoint>(),
                        controller.cast::<IConnectionPoint>(),
                    ) {
                        a.connect(b.as_ptr());
                        b.connect(a.as_ptr());
                    }
                }
                let stream = ComWrapper::new(MemoryStream::default());
                let ptr = stream.as_com_ref::<IBStream>().unwrap().as_ptr();
                if component.getState(ptr) == kResultOk {
                    stream.rewind();
                    controller.setComponentState(ptr);
                }
                let count = controller.getParameterCount();
                anyhow::ensure!((0..=4096).contains(&count), "Too many VST3 parameters");
                for index in 0..count {
                    let mut info: ParameterInfo = std::mem::zeroed();
                    if controller.getParameterInfo(index, &mut info) != kResultOk {
                        continue;
                    }
                    parameter_ids.push(info.id);
                    if info.flags & (2 | 16) == 0 {
                        this.descriptor.parameters.push(PluginParameter {
                            id: info.id,
                            name: utext(&info.title),
                            unit: utext(&info.units),
                            default: controller.getParamNormalized(info.id).clamp(0.0, 1.0),
                            steps: info.stepCount.max(0) as u32,
                        });
                    }
                }
            }
            this.descriptor.validate().map_err(anyhow::Error::msg)?;
            this.params =
                ComWrapper::new(Params::new(this.descriptor.parameters.iter().map(|p| p.id)));
            this.out_params = ComWrapper::new(Params::new(parameter_ids.into_iter()));
            let processor = this.processor.as_ref().unwrap();
            anyhow::ensure!(
                processor.canProcessSampleSize(0) == kResultOk,
                "Plugin requires 64-bit sample processing; PirateWave hosts 32-bit float audio"
            );
            if role == PluginRole::Effect
                && component.getBusCount(0, 0) == 0
                && component.getBusCount(1, 0) > 0
            {
                role = PluginRole::Instrument;
                this.descriptor.role = role;
            }
            let mut stereo_in = [3u64];
            let mut stereo_out = [3u64];
            // Ask for a main stereo bus. If declined, preserve the plugin's reported topology.
            processor.setBusArrangements(
                stereo_in.as_mut_ptr(),
                if role == PluginRole::Instrument { 0 } else { 1 },
                stereo_out.as_mut_ptr(),
                1,
            );
            for direction in 0..2 {
                let count = component.getBusCount(0, direction);
                anyhow::ensure!((0..=16).contains(&count), "Unsupported VST3 bus count");
                let mut buses = vec![];
                let mut found_main = false;
                for index in 0..count {
                    let mut info: BusInfo = std::mem::zeroed();
                    anyhow::ensure!(
                        component.getBusInfo(0, direction, index, &mut info) == kResultOk,
                        "Cannot read VST3 bus"
                    );
                    anyhow::ensure!(
                        (0..=64).contains(&info.channelCount),
                        "Unsupported VST3 channel count"
                    );
                    let active = !found_main && info.busType == 0 && info.channelCount > 0;
                    if active {
                        anyhow::ensure!(
                            info.channelCount <= 2,
                            "Plugin main bus is not mono/stereo"
                        );
                        found_main = true;
                    }
                    component.activateBus(0, direction, index, u8::from(active));
                    buses.push(Bus {
                        channels: vec![[0.0; BLOCK_SIZE]; info.channelCount as usize],
                        active,
                    });
                }
                if direction == 0 {
                    this.inputs = buses;
                } else {
                    this.outputs = buses;
                }
            }
            anyhow::ensure!(
                this.outputs.iter().any(|b| b.active),
                "VST3 has no main audio output"
            );
            if role == PluginRole::Effect {
                anyhow::ensure!(
                    this.inputs.iter().any(|b| b.active),
                    "Effect has no audio input; rescan as an instrument"
                );
            }
            let event_count = component.getBusCount(1, 0);
            for index in 0..event_count {
                component.activateBus(
                    1,
                    0,
                    index,
                    u8::from(index == 0 && role == PluginRole::Instrument),
                );
            }
            if role == PluginRole::Instrument {
                anyhow::ensure!(event_count > 0, "Instrument has no note-event input");
            }
            let mut setup = ProcessSetup {
                processMode: 0,
                symbolicSampleSize: 0,
                maxSamplesPerBlock: BLOCK_SIZE as i32,
                sampleRate: SAMPLE_RATE as f64,
            };
            anyhow::ensure!(
                processor.setupProcessing(&mut setup) == kResultOk,
                "VST3 rejected 48 kHz / 128-frame processing"
            );
            anyhow::ensure!(
                component.setActive(1) == kResultOk,
                "VST3 activation failed"
            );
            this.active = true;
            anyhow::ensure!(
                processor.setProcessing(1) == kResultOk,
                "VST3 processing activation failed"
            );
            this.processing = true;
            for bus in &mut this.inputs {
                this.input_ptrs
                    .push(bus.channels.iter_mut().map(|c| c.as_mut_ptr()).collect());
            }
            for bus in &mut this.outputs {
                this.output_ptrs
                    .push(bus.channels.iter_mut().map(|c| c.as_mut_ptr()).collect());
            }
            for (bus, pointers) in this.inputs.iter().zip(&mut this.input_ptrs) {
                this.input_buses.push(AudioBusBuffers {
                    numChannels: bus.channels.len() as i32,
                    silenceFlags: 0,
                    __field0: AudioBusBuffers__type0 {
                        channelBuffers32: pointers.as_mut_ptr(),
                    },
                });
            }
            for (bus, pointers) in this.outputs.iter().zip(&mut this.output_ptrs) {
                this.output_buses.push(AudioBusBuffers {
                    numChannels: bus.channels.len() as i32,
                    silenceFlags: 0,
                    __field0: AudioBusBuffers__type0 {
                        channelBuffers32: pointers.as_mut_ptr(),
                    },
                });
            }
            Ok(this)
        }
    }
}
impl Vst3 {
    #[cfg(windows)]
    fn handle_editor_key(
        &self,
        message: &windows_sys::Win32::UI::WindowsAndMessaging::MSG,
    ) -> bool {
        let Some((view, _, window)) = &self.editor else {
            return false;
        };
        if message.hwnd.cast::<std::ffi::c_void>() != window.handle() {
            return false;
        }
        let Some(key) = piratewave_vst2_host::window::editor_key(message) else {
            return false;
        };
        unsafe {
            let result = if key.down {
                view.onKeyDown(key.character, key.code, key.modifiers)
            } else {
                view.onKeyUp(key.character, key.code, key.modifiers)
            };
            result == kResultOk
        }
    }
    fn take_versioned_editor_edits(&mut self) -> Vec<(u32, f64, u64)> {
        let mut edits = Vec::new();
        while let Some((id, value, sequence)) = self.handler.state.edits.pop() {
            if self.handler.state.current(id, sequence) {
                edits.push((id, value, sequence));
            }
        }
        #[cfg(windows)]
        if self.editor.is_some() {
            if let Some(controller) = &self.controller {
                for parameter in &self.descriptor.parameters {
                    if self.handler.state.host_pending(parameter.id) {
                        continue;
                    }
                    let value = unsafe { controller.getParamNormalized(parameter.id) };
                    if value.is_finite() && (0.0..=1.0).contains(&value) {
                        let changed = self
                            .editor_values
                            .insert(parameter.id, value)
                            .is_some_and(|old| old != value);
                        if changed && !edits.iter().any(|(id, _, _)| *id == parameter.id) {
                            let sequence = self.handler.state.record(parameter.id);
                            edits.push((parameter.id, value, sequence));
                        }
                    }
                }
            }
            for &(id, value, _) in &edits {
                self.editor_values.insert(id, value);
            }
        }
        for &(id, value, _) in &edits {
            self.params.set(id, value);
        }
        edits
    }
}
impl Hosted for Vst3 {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }
    #[cfg(windows)]
    fn open_editor(&mut self) -> Result<()> {
        self.editor_idle();
        if let Some((_, _, window)) = &self.editor {
            window.show();
            return Ok(());
        }
        unsafe {
            let controller = self
                .controller
                .as_ref()
                .context("Plugin has no editor controller")?;
            let view = ComPtr::<IPlugView>::from_raw(controller.createView(ViewType::kEditor))
                .context("This plugin has no vendor editor")?;
            anyhow::ensure!(
                view.isPlatformTypeSupported(kPlatformTypeHWND) == kResultOk,
                "Plugin editor does not support Windows windows"
            );
            let mut rect: ViewRect = std::mem::zeroed();
            anyhow::ensure!(
                view.getSize(&mut rect) == kResultOk,
                "Plugin editor did not provide a size"
            );
            let window = Rc::new(piratewave_vst2_host::window::EditorWindow::new(
                &self.descriptor.name,
                rect.right
                    .checked_sub(rect.left)
                    .context("Invalid plugin editor width")?,
                rect.bottom
                    .checked_sub(rect.top)
                    .context("Invalid plugin editor height")?,
                view.canResize() == kResultOk,
            )?);
            let frame = ComWrapper::new(EditorFrame {
                window: window.clone(),
            });
            self.editor_scale = window.scale();
            if let Some(scale) = view.cast::<IPlugViewContentScaleSupport>() {
                scale.setContentScaleFactor(self.editor_scale);
            }
            anyhow::ensure!(
                view.setFrame(frame.as_com_ref::<IPlugFrame>().unwrap().as_ptr()) == kResultOk,
                "Plugin editor rejected its host frame"
            );
            if view.attached(window.handle(), kPlatformTypeHWND) != kResultOk {
                view.setFrame(ptr::null_mut());
                anyhow::bail!("Plugin editor could not attach to its window");
            }
            window.show();
            self.editor = Some((view, frame, window));
            self.editor_values = self
                .descriptor
                .parameters
                .iter()
                .map(|p| (p.id, controller.getParamNormalized(p.id)))
                .collect();
            Ok(())
        }
    }
    #[cfg(windows)]
    fn close_editor(&mut self) {
        if let Some((view, frame, window)) = self.editor.take() {
            unsafe {
                if self.editor_focused {
                    view.onFocus(0);
                    self.editor_focused = false;
                }
                view.removed();
                view.setFrame(ptr::null_mut());
            }
            drop(frame);
            drop(window);
        }
    }
    #[cfg(windows)]
    fn editor_idle(&mut self) {
        if self
            .editor
            .as_ref()
            .is_some_and(|(_, _, window)| !window.visible())
        {
            self.close_editor();
        }
        if let Some((view, _, window)) = &self.editor {
            unsafe {
                let focused = window.focused();
                if focused != self.editor_focused {
                    self.editor_focused = focused;
                    view.onFocus(u8::from(focused));
                }
                let scale = window.scale();
                if scale != self.editor_scale && scale > 0.0 {
                    self.editor_scale = scale;
                    if let Some(support) = view.cast::<IPlugViewContentScaleSupport>() {
                        support.setContentScaleFactor(scale);
                    }
                }
                let (width, height) = window.client_size();
                let mut actual: ViewRect = std::mem::zeroed();
                if width > 0 && height > 0 && view.getSize(&mut actual) == kResultOk {
                    let Some(actual_width) = actual.right.checked_sub(actual.left) else {
                        return;
                    };
                    let Some(actual_height) = actual.bottom.checked_sub(actual.top) else {
                        return;
                    };
                    if actual_width == width && actual_height == height {
                        return;
                    }
                    let mut requested = ViewRect {
                        left: 0,
                        top: 0,
                        right: width,
                        bottom: height,
                    };
                    let mut accepted = false;
                    if view.canResize() == kResultOk
                        && view.checkSizeConstraint(&mut requested) == kResultOk
                    {
                        if let (Some(new_width), Some(new_height)) = (
                            requested.right.checked_sub(requested.left),
                            requested.bottom.checked_sub(requested.top),
                        ) {
                            if (1..=8192).contains(&new_width) && (1..=8192).contains(&new_height) {
                                if new_width != width || new_height != height {
                                    let _ = window.resize(new_width, new_height);
                                }
                                accepted = view.onSize(&mut requested) == kResultOk;
                            }
                        }
                    }
                    if !accepted {
                        let _ = window.resize(actual_width, actual_height);
                    }
                }
            }
        }
    }
    #[cfg(windows)]
    fn editor_open(&self) -> bool {
        self.editor.is_some()
    }
    #[cfg(windows)]
    fn editor_size(&self) -> (i32, i32) {
        self.editor
            .as_ref()
            .map_or((0, 0), |(_, _, window)| window.client_size())
    }
    fn take_editor_edits(&mut self) -> Vec<(u32, f64)> {
        self.take_versioned_editor_edits()
            .into_iter()
            .map(|(id, value, _)| (id, value))
            .collect()
    }
    fn flush_parameter_changes(&mut self) -> Result<()> {
        if self.params.active.borrow().is_empty() {
            return Ok(());
        }
        unsafe {
            let mut data = ProcessData {
                processMode: 0,
                symbolicSampleSize: 0,
                numSamples: 0,
                numInputs: 0,
                numOutputs: 0,
                inputs: ptr::null_mut(),
                outputs: ptr::null_mut(),
                inputParameterChanges: self
                    .params
                    .as_com_ref::<IParameterChanges>()
                    .unwrap()
                    .as_ptr(),
                outputParameterChanges: self
                    .out_params
                    .as_com_ref::<IParameterChanges>()
                    .unwrap()
                    .as_ptr(),
                inputEvents: ptr::null_mut(),
                outputEvents: ptr::null_mut(),
                processContext: ptr::null_mut(),
            };
            anyhow::ensure!(
                self.processor.as_ref().unwrap().process(&mut data) == kResultOk,
                "VST3 parameter flush was rejected"
            );
            self.params.active.borrow_mut().clear();
            self.out_params.active.borrow_mut().clear();
        }
        Ok(())
    }
    fn capture_state(&mut self) -> Result<Option<piratewave_core::plugins::PluginState>> {
        unsafe {
            let stream = ComWrapper::new(MemoryStream::default());
            let ptr = stream.as_com_ref::<IBStream>().unwrap().as_ptr();
            let result = self.component.as_ref().unwrap().getState(ptr);
            if result == kNotImplemented {
                return Ok(None);
            }
            anyhow::ensure!(
                result == kResultOk,
                "VST3 component state capture failed ({result})"
            );
            let controller = if let Some(controller) = &self.controller {
                let stream = ComWrapper::new(MemoryStream::default());
                let ptr = stream.as_com_ref::<IBStream>().unwrap().as_ptr();
                let result = controller.getState(ptr);
                if result == kNotImplemented {
                    None
                } else {
                    anyhow::ensure!(
                        result == kResultOk,
                        "VST3 controller state capture failed ({result})"
                    );
                    Some(stream.bytes())
                }
            } else {
                None
            };
            Ok(Some(piratewave_core::plugins::PluginState {
                component: stream.bytes().into(),
                controller: controller.map(Into::into),
                vst2_bank: false,
                vst2_program: None,
            }))
        }
    }
    fn restore_state(&mut self, state: &piratewave_core::plugins::PluginState) -> Result<()> {
        unsafe {
            let stream =
                ComWrapper::new(MemoryStream::from_bytes(state.component.as_ref().clone()));
            let ptr = stream.as_com_ref::<IBStream>().unwrap().as_ptr();
            anyhow::ensure!(
                self.component.as_ref().unwrap().setState(ptr) == kResultOk,
                "VST3 component rejected saved state"
            );
            if let Some(controller) = &self.controller {
                stream.rewind();
                let result = controller.setComponentState(ptr);
                anyhow::ensure!(
                    result == kResultOk || result == kNotImplemented,
                    "VST3 controller rejected component state ({result})"
                );
                if let Some(bytes) = &state.controller {
                    let stream = ComWrapper::new(MemoryStream::from_bytes(bytes.as_ref().clone()));
                    let ptr = stream.as_com_ref::<IBStream>().unwrap().as_ptr();
                    anyhow::ensure!(
                        controller.setState(ptr) == kResultOk,
                        "VST3 controller rejected saved state"
                    );
                }
            }
            Ok(())
        }
    }
    fn set_parameter(&mut self, id: u32, value: f64) {
        #[cfg(windows)]
        if self.editor.is_some() {
            self.editor_values.insert(id, value);
        }
        unsafe {
            if let Some(controller) = &self.controller {
                controller.setParamNormalized(id, value);
            }
        }
        self.params.set(id, value);
    }
    fn note(&mut self, note: u8, velocity: f32, on: bool) {
        let mut events = self.events.events.borrow_mut();
        if events.len() >= 256 {
            return;
        }
        events.push(Event {
            busIndex: 0,
            sampleOffset: 0,
            ppqPosition: 0.0,
            flags: 1,
            r#type: if on { 0 } else { 1 },
            __field0: if on {
                Event__type0 {
                    noteOn: NoteOnEvent {
                        channel: 0,
                        pitch: note as i16,
                        tuning: 0.0,
                        velocity,
                        length: 0,
                        noteId: note as i32,
                    },
                }
            } else {
                Event__type0 {
                    noteOff: NoteOffEvent {
                        channel: 0,
                        pitch: note as i16,
                        velocity: 0.0,
                        noteId: note as i32,
                        tuning: 0.0,
                    },
                }
            },
        });
    }
    fn process(&mut self, frames: &mut [Frame]) -> Result<()> {
        unsafe {
            anyhow::ensure!(frames.len() == BLOCK_SIZE, "Unexpected VST3 block size");
            // Dynamic I/O changes need a fresh instance; keep the graph's saved stereo contract.
            let restart = self.handler.state.restart.swap(0, Ordering::AcqRel);
            anyhow::ensure!(
                restart & 2 == 0,
                "Plugin changed its bus layout; restart audio"
            );
            for bus in &mut self.inputs {
                let mono = bus.channels.len() == 1;
                for (channel, buffer) in bus.channels.iter_mut().enumerate() {
                    for (i, frame) in frames.iter().enumerate() {
                        buffer[i] = if !bus.active {
                            0.0
                        } else if mono {
                            (frame[0] + frame[1]) * 0.5
                        } else {
                            frame[channel]
                        };
                    }
                }
            }
            for bus in &mut self.outputs {
                for channel in &mut bus.channels {
                    channel.fill(0.0);
                }
            }
            for bus in &mut self.output_buses {
                bus.silenceFlags = 0;
            }
            let mut context: ProcessContext = std::mem::zeroed();
            context.sampleRate = SAMPLE_RATE as f64;
            context.projectTimeSamples = self.time;
            context.continousTimeSamples = self.time;
            context.tempo = 120.0;
            context.timeSigNumerator = 4;
            context.timeSigDenominator = 4;
            context.state = 2 | 1024 | 8192 | 131072;
            let mut data = ProcessData {
                processMode: 0,
                symbolicSampleSize: 0,
                numSamples: BLOCK_SIZE as i32,
                numInputs: self.input_buses.len() as i32,
                numOutputs: self.output_buses.len() as i32,
                inputs: self.input_buses.as_mut_ptr(),
                outputs: self.output_buses.as_mut_ptr(),
                inputParameterChanges: self
                    .params
                    .as_com_ref::<IParameterChanges>()
                    .unwrap()
                    .as_ptr(),
                outputParameterChanges: self
                    .out_params
                    .as_com_ref::<IParameterChanges>()
                    .unwrap()
                    .as_ptr(),
                inputEvents: self.events.as_com_ref::<IEventList>().unwrap().as_ptr(),
                outputEvents: self.out_events.as_com_ref::<IEventList>().unwrap().as_ptr(),
                processContext: &mut context,
            };
            let result = self.processor.as_ref().unwrap().process(&mut data);
            self.events.events.borrow_mut().clear();
            self.out_events.events.borrow_mut().clear();
            self.params.active.borrow_mut().clear();
            self.out_params.active.borrow_mut().clear();
            self.time += BLOCK_SIZE as i64;
            anyhow::ensure!(result == kResultOk, "VST3 process returned {result}");
            let index = self.outputs.iter().position(|b| b.active).unwrap();
            let bus = &self.outputs[index];
            for (i, frame) in frames.iter_mut().enumerate() {
                *frame = [bus.channels[0][i], bus.channels[bus.channels.len() - 1][i]];
                for (channel, sample) in frame.iter_mut().enumerate() {
                    if self.output_buses[index].silenceFlags
                        & (1 << channel.min(bus.channels.len() - 1))
                        != 0
                    {
                        *sample = 0.0;
                    }
                }
            }
            Ok(())
        }
    }
}
impl Drop for Vst3 {
    fn drop(&mut self) {
        self.close_editor();
        unsafe {
            if let Some(processor) = &self.processor {
                if self.processing {
                    processor.setProcessing(0);
                }
            }
            if let Some(component) = &self.component {
                if self.active {
                    component.setActive(0);
                }
            }
            if let Some(controller) = &self.controller {
                controller.setComponentHandler(ptr::null_mut());
                if self.controller_separate {
                    if let Some(component) = &self.component {
                        if let (Some(a), Some(b)) = (
                            component.cast::<IConnectionPoint>(),
                            controller.cast::<IConnectionPoint>(),
                        ) {
                            a.disconnect(b.as_ptr());
                            b.disconnect(a.as_ptr());
                        }
                    }
                    if self.controller_initialized {
                        controller.terminate();
                    }
                }
            }
            if self.component_initialized {
                if let Some(component) = &self.component {
                    component.terminate();
                }
            }
            drop(self.controller.take());
            drop(self.processor.take());
            drop(self.component.take());
        }
    }
}
