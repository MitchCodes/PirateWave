//! Self-authored test plugins; not production instruments. No audio devices are opened.
#![allow(non_snake_case)]
use std::{
    cell::Cell,
    ffi::{c_char, c_void},
    ptr,
    sync::{
        atomic::{AtomicU32, Ordering},
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
        2
    }
    unsafe fn getClassInfo(&self, index: i32, info: *mut PClassInfo) -> tresult {
        if info.is_null() || !(0..2).contains(&index) {
            return kInvalidArgument;
        }
        *info = std::mem::zeroed();
        (*info).cid = if index == 0 { GAIN } else { SYNTH };
        (*info).cardinality = i32::MAX;
        text(&mut (*info).category, "Audio Module Class");
        text(
            &mut (*info).name,
            if index == 0 {
                "PirateWave Test Gain"
            } else {
                "PirateWave Test Instrument"
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
        if id != GAIN && id != SYNTH {
            return kNoInterface;
        }
        let plugin = ComWrapper::new(TestPlugin {
            synth: id == SYNTH,
            gain: Cell::new(0.5),
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
        if info.is_null() || !(0..2).contains(&index) {
            return kInvalidArgument;
        }
        *info = std::mem::zeroed();
        (*info).cid = if index == 0 { GAIN } else { SYNTH };
        (*info).cardinality = i32::MAX;
        text(&mut (*info).category, "Audio Module Class");
        text(
            &mut (*info).name,
            if index == 0 {
                "PirateWave Test Gain"
            } else {
                "PirateWave Test Instrument"
            },
        );
        text(
            &mut (*info).subCategories,
            if index == 0 { "Fx" } else { "Instrument|Synth" },
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
struct TestPlugin {
    synth: bool,
    gain: Cell<f64>,
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
    unsafe fn setState(&self, _stream: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn getState(&self, _stream: *mut IBStream) -> tresult {
        kResultOk
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
    unsafe fn setComponentHandler(&self, _h: *mut IComponentHandler) -> tresult {
        kResultOk
    }
    unsafe fn createView(&self, _name: FIDString) -> *mut IPlugView {
        ptr::null_mut()
    }
}
// One VST2 instrument export exercises the legacy host's note and audio paths.
struct V2Parameters(AtomicU32);
impl vst::plugin::PluginParameters for V2Parameters {
    fn get_parameter(&self, _i: i32) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    fn set_parameter(&self, _i: i32, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
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
}
impl Default for V2 {
    fn default() -> Self {
        Self {
            params: Arc::new(V2Parameters(AtomicU32::new(0.5f32.to_bits()))),
            note: false,
            phase: 0.0,
            rate: 48000.0,
        }
    }
}
impl vst::plugin::Plugin for V2 {
    fn new(_host: vst::plugin::HostCallback) -> Self {
        Self::default()
    }
    fn get_info(&self) -> vst::plugin::Info {
        vst::plugin::Info {
            name: "PirateWave Test VST2 Instrument".into(),
            vendor: "PirateWave tests".into(),
            unique_id: 0x50544348,
            inputs: 0,
            outputs: 2,
            parameters: 1,
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
