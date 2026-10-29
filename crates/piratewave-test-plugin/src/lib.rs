//! Self-authored test plugins; not production instruments. No audio devices are opened.
#![allow(non_snake_case)]
use std::{
    cell::{Cell, RefCell},
    ffi::{c_char, c_void},
    ptr,
    sync::{
        atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering},
        Arc,
    },
};
use vst3::{
    Steinberg::{Vst::*, *},
    *,
};
const GAIN: TUID = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
const SYNTH: TUID = [
    17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
];
const NO_EDITOR: TUID = [
    33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48,
];
fn text(dst: &mut [c_char], value: &str) {
    for (d, c) in dst.iter_mut().zip(value.bytes()) {
        *d = c as c_char;
    }
}
fn text16(dst: &mut [u16], value: &str) {
    for (d, c) in dst.iter_mut().zip(value.encode_utf16()) {
        *d = c;
    }
}
struct Factory;
impl Class for Factory {
    type Interfaces = (IPluginFactory2,);
}
impl IPluginFactoryTrait for Factory {
    unsafe fn getFactoryInfo(&self, info: *mut PFactoryInfo) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }
        *info = std::mem::zeroed();
        text(&mut (*info).vendor, "PirateWave tests");
        kResultOk
    }
    unsafe fn countClasses(&self) -> i32 {
        3
    }
    unsafe fn getClassInfo(&self, index: i32, info: *mut PClassInfo) -> tresult {
        if info.is_null() || !(0..3).contains(&index) {
            return kInvalidArgument;
        }
        *info = std::mem::zeroed();
        (*info).cid = match index {
            0 => GAIN,
            1 => SYNTH,
            _ => NO_EDITOR,
        };
        (*info).cardinality = i32::MAX;
        text(&mut (*info).category, "Audio Module Class");
        text(
            &mut (*info).name,
            match index {
                0 => "PirateWave Test Gain",
                1 => "PirateWave Test Instrument",
                _ => "PirateWave Test Without Editor",
            },
        );
        kResultOk
    }
    unsafe fn createInstance(
        &self,
        id: FIDString,
        iid: FIDString,
        obj: *mut *mut c_void,
    ) -> tresult {
        if id.is_null() || iid.is_null() || obj.is_null() {
            return kInvalidArgument;
        }
        *obj = ptr::null_mut();
        let id = *(id as *const TUID);
        if id != GAIN && id != SYNTH && id != NO_EDITOR {
            return kNoInterface;
        }
        let plugin = ComWrapper::new(TestPlugin {
            synth: id == SYNTH,
            no_editor: id == NO_EDITOR,
            gain: Arc::new(AtomicF64::new(0.5)),
            opaque: Arc::new(AtomicU64::new(0)),
            handler: RefCell::new(None),
            note: Cell::new(false),
            phase: Cell::new(0.0),
            rate: Cell::new(48000.0),
        });
        let unknown = plugin.as_com_ref::<IComponent>().unwrap().as_ptr() as *mut FUnknown;
        ((*(*unknown).vtbl).queryInterface)(unknown, iid as *const TUID, obj)
    }
}
impl IPluginFactory2Trait for Factory {
    unsafe fn getClassInfo2(&self, index: i32, info: *mut PClassInfo2) -> tresult {
        if info.is_null() || !(0..3).contains(&index) {
            return kInvalidArgument;
        }
        *info = std::mem::zeroed();
        (*info).cid = match index {
            0 => GAIN,
            1 => SYNTH,
            _ => NO_EDITOR,
        };
        (*info).cardinality = i32::MAX;
        text(&mut (*info).category, "Audio Module Class");
        text(
            &mut (*info).name,
            match index {
                0 => "PirateWave Test Gain",
                1 => "PirateWave Test Instrument",
                _ => "PirateWave Test Without Editor",
            },
        );
        text(
            &mut (*info).subCategories,
            if index == 1 { "Instrument|Synth" } else { "Fx" },
        );
        text(&mut (*info).vendor, "PirateWave tests");
        kResultOk
    }
}
#[no_mangle]
pub extern "system" fn InitDll() -> bool {
    true
}
#[no_mangle]
pub extern "system" fn ExitDll() -> bool {
    true
}
#[no_mangle]
pub extern "system" fn GetPluginFactory() -> *mut IPluginFactory {
    let factory = ComWrapper::new(Factory)
        .to_com_ptr::<IPluginFactory>()
        .unwrap();
    let ptr = factory.as_ptr();
    std::mem::forget(factory);
    ptr
}
struct AtomicF64(AtomicU64);
impl AtomicF64 {
    fn new(value: f64) -> Self {
        Self(AtomicU64::new(value.to_bits()))
    }
    fn get(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::Relaxed))
    }
    fn set(&self, value: f64) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }
}
struct TestPlugin {
    synth: bool,
    no_editor: bool,
    gain: Arc<AtomicF64>,
    opaque: Arc<AtomicU64>,
    handler: RefCell<Option<ComPtr<IComponentHandler>>>,
    note: Cell<bool>,
    phase: Cell<f64>,
    rate: Cell<f64>,
}
impl Class for TestPlugin {
    type Interfaces = (IComponent, IAudioProcessor, IEditController);
}
impl IPluginBaseTrait for TestPlugin {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }
    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}
impl IComponentTrait for TestPlugin {
    unsafe fn getControllerClassId(&self, _id: *mut TUID) -> tresult {
        kResultFalse
    }
    unsafe fn setIoMode(&self, _mode: i32) -> tresult {
        kResultOk
    }
    unsafe fn getBusCount(&self, media: i32, dir: i32) -> i32 {
        if media == 0 {
            if dir == 0 && self.synth {
                0
            } else {
                1
            }
        } else if media == 1 && dir == 0 && self.synth {
            1
        } else {
            0
        }
    }
    unsafe fn getBusInfo(&self, media: i32, dir: i32, index: i32, bus: *mut BusInfo) -> tresult {
        if bus.is_null() || index != 0 || self.getBusCount(media, dir) == 0 {
            return kInvalidArgument;
        }
        *bus = std::mem::zeroed();
        (*bus).mediaType = media;
        (*bus).direction = dir;
        (*bus).channelCount = if media == 0 { 2 } else { 16 };
        (*bus).flags = 1;
        kResultOk
    }
    unsafe fn getRoutingInfo(
        &self,
        _input: *mut RoutingInfo,
        _output: *mut RoutingInfo,
    ) -> tresult {
        kNotImplemented
    }
    unsafe fn activateBus(&self, _media: i32, _dir: i32, _index: i32, _active: u8) -> tresult {
        kResultOk
    }
    unsafe fn setActive(&self, _active: u8) -> tresult {
        kResultOk
    }
    unsafe fn setState(&self, stream: *mut IBStream) -> tresult {
        if stream.is_null() {
            return kInvalidArgument;
        }
        let mut bytes = [0u8; 16];
        let mut read = 0;
        if ((*(*stream).vtbl).read)(stream, bytes.as_mut_ptr().cast(), 16, &mut read) != kResultOk
            || read != 16
        {
            return kResultFalse;
        }
        let value = f64::from_le_bytes(bytes[..8].try_into().unwrap());
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return kInvalidArgument;
        }
        self.gain.set(value);
        self.opaque.store(
            u64::from_le_bytes(bytes[8..].try_into().unwrap()),
            Ordering::Relaxed,
        );
        kResultOk
    }
    unsafe fn getState(&self, stream: *mut IBStream) -> tresult {
        if stream.is_null() {
            return kInvalidArgument;
        }
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&self.gain.get().to_le_bytes());
        bytes[8..].copy_from_slice(&self.opaque.load(Ordering::Relaxed).to_le_bytes());
        let mut written = 0;
        let result =
            ((*(*stream).vtbl).write)(stream, bytes.as_ptr().cast_mut().cast(), 16, &mut written);
        if result == kResultOk && written == 16 {
            kResultOk
        } else {
            kResultFalse
        }
    }
}
impl IAudioProcessorTrait for TestPlugin {
    unsafe fn setBusArrangements(
        &self,
        _inputs: *mut u64,
        ni: i32,
        _outputs: *mut u64,
        no: i32,
    ) -> tresult {
        if ni == if self.synth { 0 } else { 1 } && no == 1 {
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn getBusArrangement(&self, _dir: i32, _index: i32, arr: *mut u64) -> tresult {
        if arr.is_null() {
            return kInvalidArgument;
        }
        *arr = 3;
        kResultOk
    }
    unsafe fn canProcessSampleSize(&self, size: i32) -> tresult {
        if size == 0 {
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn getLatencySamples(&self) -> u32 {
        0
    }
    unsafe fn setupProcessing(&self, setup: *mut ProcessSetup) -> tresult {
        if setup.is_null() {
            return kInvalidArgument;
        }
        self.rate.set((*setup).sampleRate);
        kResultOk
    }
    unsafe fn setProcessing(&self, _active: u8) -> tresult {
        kResultOk
    }
    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        if data.is_null() {
            return kInvalidArgument;
        }
        let data = &mut *data;
        if let Some(params) = ComRef::from_raw(data.inputParameterChanges) {
            for index in 0..params.getParameterCount() {
                if let Some(q) = ComRef::from_raw(params.getParameterData(index)) {
                    let mut offset = 0;
                    let mut value = 0.0;
                    if q.getParameterId() == 0
                        && q.getPoint(0, &mut offset, &mut value) == kResultOk
                    {
                        self.gain.set(value);
                    }
                }
            }
        }
        if let Some(events) = ComRef::from_raw(data.inputEvents) {
            for index in 0..events.getEventCount() {
                let mut e: Event = std::mem::zeroed();
                if events.getEvent(index, &mut e) == kResultOk {
                    if e.r#type == 0 {
                        self.note.set(true);
                    } else if e.r#type == 1 {
                        self.note.set(false);
                    }
                }
            }
        }
        if data.numSamples == 0 {
            return kResultOk;
        }
        if data.numOutputs < 1 {
            return kInvalidArgument;
        }
        let out = &mut *data.outputs;
        let op = out.__field0.channelBuffers32;
        let ip = if data.numInputs > 0 {
            (*data.inputs).__field0.channelBuffers32
        } else {
            ptr::null_mut()
        };
        for i in 0..data.numSamples as usize {
            let tone = if self.note.get() {
                self.phase.get().sin() as f32 * 0.2 * self.gain.get() as f32
            } else {
                0.0
            };
            for channel in 0..out.numChannels as usize {
                *(*op.add(channel)).add(i) = if self.synth {
                    tone
                } else {
                    *(*ip.add(channel)).add(i) * self.gain.get() as f32
                };
            }
            self.phase.set(
                (self.phase.get() + std::f64::consts::TAU * 440.0 / self.rate.get())
                    % std::f64::consts::TAU,
            );
        }
        out.silenceFlags = 0;
        kResultOk
    }
    unsafe fn getTailSamples(&self) -> u32 {
        0
    }
}
impl IEditControllerTrait for TestPlugin {
    unsafe fn setComponentState(&self, _s: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn setState(&self, _s: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn getState(&self, _s: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn getParameterCount(&self) -> i32 {
        1
    }
    unsafe fn getParameterInfo(&self, index: i32, info: *mut ParameterInfo) -> tresult {
        if index != 0 || info.is_null() {
            return kInvalidArgument;
        }
        *info = std::mem::zeroed();
        text16(&mut (*info).title, "Gain");
        (*info).defaultNormalizedValue = 0.5;
        (*info).flags = 1;
        kResultOk
    }
    unsafe fn getParamStringByValue(&self, _id: u32, _v: f64, _s: *mut String128) -> tresult {
        kNotImplemented
    }
    unsafe fn getParamValueByString(&self, _id: u32, _s: *mut u16, _v: *mut f64) -> tresult {
        kNotImplemented
    }
    unsafe fn normalizedParamToPlain(&self, _id: u32, v: f64) -> f64 {
        v
    }
    unsafe fn plainParamToNormalized(&self, _id: u32, v: f64) -> f64 {
        v
    }
    unsafe fn getParamNormalized(&self, _id: u32) -> f64 {
        self.gain.get()
    }
    unsafe fn setParamNormalized(&self, _id: u32, v: f64) -> tresult {
        self.gain.set(v);
        kResultOk
    }
    unsafe fn setComponentHandler(&self, h: *mut IComponentHandler) -> tresult {
        *self.handler.borrow_mut() = ComRef::from_raw(h).map(|handler| handler.to_com_ptr());
        kResultOk
    }
    unsafe fn createView(&self, name: FIDString) -> *mut IPlugView {
        if self.no_editor
            || name.is_null()
            || std::ffi::CStr::from_ptr(name).to_bytes() != b"editor"
        {
            return ptr::null_mut();
        }
        let wrapper = ComWrapper::new(FixtureView {
            attached: Cell::new(false),
            width: Cell::new(360),
            height: Cell::new(200),
            frame: RefCell::new(None),
            view: Cell::new(ptr::null_mut()),
            gain: self.gain.clone(),
            opaque: self.opaque.clone(),
            handler: self.handler.borrow().clone(),
        });
        let view = wrapper.to_com_ptr::<IPlugView>().unwrap();
        wrapper.view.set(view.as_ptr());
        let result = view.as_ptr();
        std::mem::forget(view);
        result
    }
}
struct FixtureView {
    attached: Cell<bool>,
    width: Cell<i32>,
    height: Cell<i32>,
    frame: RefCell<Option<ComPtr<IPlugFrame>>>,
    view: Cell<*mut IPlugView>,
    gain: Arc<AtomicF64>,
    opaque: Arc<AtomicU64>,
    handler: Option<ComPtr<IComponentHandler>>,
}
impl Class for FixtureView {
    type Interfaces = (IPlugView,);
}
impl IPlugViewTrait for FixtureView {
    unsafe fn isPlatformTypeSupported(&self, platform: FIDString) -> tresult {
        if !platform.is_null() && std::ffi::CStr::from_ptr(platform).to_bytes() == b"HWND" {
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn attached(&self, parent: *mut c_void, platform: FIDString) -> tresult {
        if parent.is_null()
            || self.isPlatformTypeSupported(platform) != kResultOk
            || self.attached.replace(true)
        {
            kResultFalse
        } else {
            self.gain.set(0.75);
            self.opaque.store(0x12345678, Ordering::Relaxed);
            if let Some(handler) = &self.handler {
                handler.beginEdit(0);
                handler.performEdit(0, 0.75);
                handler.endEdit(0);
            }
            if let Some(frame) = self.frame.borrow().as_ref() {
                let mut size = ViewRect {
                    left: 0,
                    top: 0,
                    right: 420,
                    bottom: 240,
                };
                if frame.resizeView(self.view.get(), &mut size) != kResultOk
                    || self.width.get() != 420
                    || self.height.get() != 240
                {
                    return kResultFalse;
                }
            }
            // Simulate an editor performing a slow or modal operation during attachment.
            std::thread::sleep(std::time::Duration::from_millis(1500));
            kResultOk
        }
    }
    unsafe fn removed(&self) -> tresult {
        if self.attached.replace(false) {
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn onWheel(&self, _: f32) -> tresult {
        kResultFalse
    }
    unsafe fn onKeyDown(&self, _: u16, code: i16, _: i16) -> tresult {
        if code == VirtualKeyCodes_::KEY_F2 as i16 {
            self.gain.set(0.625);
            if let Some(handler) = &self.handler {
                handler.beginEdit(0);
                handler.performEdit(0, 0.625);
                handler.endEdit(0);
            }
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn onKeyUp(&self, _: u16, _: i16, _: i16) -> tresult {
        kResultFalse
    }
    unsafe fn getSize(&self, rect: *mut ViewRect) -> tresult {
        if rect.is_null() {
            return kInvalidArgument;
        }
        *rect = ViewRect {
            left: 0,
            top: 0,
            right: self.width.get(),
            bottom: self.height.get(),
        };
        kResultOk
    }
    unsafe fn onSize(&self, rect: *mut ViewRect) -> tresult {
        if rect.is_null() {
            kInvalidArgument
        } else {
            self.width.set((*rect).right - (*rect).left);
            self.height.set((*rect).bottom - (*rect).top);
            kResultOk
        }
    }
    unsafe fn onFocus(&self, _: u8) -> tresult {
        kResultOk
    }
    unsafe fn setFrame(&self, frame: *mut IPlugFrame) -> tresult {
        *self.frame.borrow_mut() = ComRef::from_raw(frame).map(|frame| frame.to_com_ptr());
        kResultOk
    }
    unsafe fn canResize(&self) -> tresult {
        kResultOk
    }
    unsafe fn checkSizeConstraint(&self, rect: *mut ViewRect) -> tresult {
        if rect.is_null() {
            kInvalidArgument
        } else {
            kResultOk
        }
    }
}
// One VST2 instrument export exercises the legacy host's note and audio paths.
struct V2Parameters {
    gain: AtomicU32,
    opaque: AtomicU32,
    program: AtomicI32,
}
struct V2Editor {
    open: bool,
    idle_once: bool,
    host: vst::plugin::HostCallback,
    params: Arc<V2Parameters>,
}
impl vst::editor::Editor for V2Editor {
    fn key_down(&mut self, key: vst::editor::KeyCode) -> bool {
        if matches!(key.key, vst::editor::Key::F2) {
            use vst::host::Host;
            self.params
                .gain
                .store(0.625f32.to_bits(), Ordering::Relaxed);
            self.host.automate(0, 0.625);
            true
        } else {
            false
        }
    }
    fn size(&self) -> (i32, i32) {
        (360, 200)
    }
    fn position(&self) -> (i32, i32) {
        (0, 0)
    }
    fn open(&mut self, parent: *mut c_void) -> bool {
        self.open = !parent.is_null();
        self.idle_once = false;
        self.open
    }
    fn is_open(&mut self) -> bool {
        self.open
    }
    fn close(&mut self) {
        self.open = false;
    }
    fn idle(&mut self) {
        if self.open && !self.idle_once {
            use vst::host::Host;
            self.idle_once = true;
            self.params.gain.store(0.75f32.to_bits(), Ordering::Relaxed);
            self.params.opaque.store(0x12345678, Ordering::Relaxed);
            self.params.program.store(2, Ordering::Relaxed);
            self.host.automate(0, 0.75);
            self.host.size_window(420, 240);
            // Simulate a vendor editor spending time in a modal action.
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
    }
}
impl vst::plugin::PluginParameters for V2Parameters {
    fn change_preset(&self, preset: i32) {
        self.program.store(preset, Ordering::Relaxed);
    }
    fn get_preset_num(&self) -> i32 {
        self.program.load(Ordering::Relaxed)
    }
    fn get_bank_data(&self) -> Vec<u8> {
        if std::env::var_os("PIRATEWAVE_TEST_VST2_PROGRAM_ONLY").is_some() {
            return Vec::new();
        }
        if self.opaque.load(Ordering::Relaxed) == 0x12345678 {
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
        self.get_preset_data()
    }
    fn load_bank_data(&self, data: &[u8]) {
        self.load_preset_data(data);
    }
    fn get_preset_data(&self) -> Vec<u8> {
        let mut data = self.gain.load(Ordering::Relaxed).to_le_bytes().to_vec();
        data.extend_from_slice(&self.opaque.load(Ordering::Relaxed).to_le_bytes());
        if self.opaque.load(Ordering::Relaxed) == 0x87654321 {
            // Exercise bulk bridge transfers without requiring an external plugin.
            data.resize(24 * 1024 * 1024, 0x5a);
        }
        data
    }
    fn load_preset_data(&self, data: &[u8]) {
        if data.len() == 8 {
            let value = f32::from_bits(u32::from_le_bytes(data[..4].try_into().unwrap()));
            if value.is_finite() && (0.0..=1.0).contains(&value) {
                self.gain.store(value.to_bits(), Ordering::Relaxed);
                self.opaque.store(
                    u32::from_le_bytes(data[4..].try_into().unwrap()),
                    Ordering::Relaxed,
                );
            }
        }
    }
    fn get_parameter(&self, _i: i32) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }
    fn set_parameter(&self, _i: i32, v: f32) {
        self.gain.store(v.to_bits(), Ordering::Relaxed);
    }
    fn get_parameter_name(&self, _i: i32) -> String {
        "Gain".into()
    }
}
struct V2 {
    params: Arc<V2Parameters>,
    note: bool,
    phase: f32,
    rate: f32,
    host: vst::plugin::HostCallback,
}
impl Default for V2 {
    fn default() -> Self {
        Self {
            params: Arc::new(V2Parameters {
                gain: AtomicU32::new(0.5f32.to_bits()),
                opaque: AtomicU32::new(0),
                program: AtomicI32::new(0),
            }),
            note: false,
            phase: 0.0,
            rate: 48000.0,
            host: Default::default(),
        }
    }
}
impl vst::plugin::Plugin for V2 {
    fn get_editor(&mut self) -> Option<Box<dyn vst::editor::Editor>> {
        if std::env::var_os("PIRATEWAVE_TEST_VST2_NO_EDITOR").is_some() {
            return None;
        }
        Some(Box::new(V2Editor {
            open: false,
            idle_once: false,
            host: self.host,
            params: self.params.clone(),
        }))
    }
    fn new(host: vst::plugin::HostCallback) -> Self {
        Self {
            host,
            ..Self::default()
        }
    }
    fn get_info(&self) -> vst::plugin::Info {
        vst::plugin::Info {
            name: "PirateWave Test VST2 Instrument".into(),
            vendor: "PirateWave tests".into(),
            unique_id: 0x50544348,
            inputs: 0,
            outputs: 2,
            parameters: 1,
            presets: 3,
            preset_chunks: true,
            category: vst::plugin::Category::Synth,
            ..Default::default()
        }
    }
    fn get_parameter_object(&mut self) -> Arc<dyn vst::plugin::PluginParameters> {
        self.params.clone()
    }
    fn set_sample_rate(&mut self, rate: f32) {
        self.rate = rate;
    }
    fn process_events(&mut self, events: &vst::api::Events) {
        for event in events.events() {
            if let vst::event::Event::Midi(m) = event {
                match m.data[0] & 0xf0 {
                    0x90 => self.note = m.data[2] > 0,
                    0x80 => self.note = false,
                    _ => {}
                }
            }
        }
    }
    fn process(&mut self, buffer: &mut vst::buffer::AudioBuffer<f32>) {
        use vst::plugin::PluginParameters;
        let (_, mut outputs) = buffer.split();
        for i in 0..outputs[0].len() {
            let value = if self.note {
                self.phase.sin() * 0.2 * self.params.get_parameter(0)
            } else {
                0.0
            };
            for channel in 0..outputs.len() {
                outputs[channel][i] = value;
            }
            self.phase =
                (self.phase + std::f32::consts::TAU * 440.0 / self.rate) % std::f32::consts::TAU;
        }
    }
}
vst::plugin_main!(V2);
