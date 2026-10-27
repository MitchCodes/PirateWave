//! COM objects owned by one VST hosting thread. Buffers are bounded and reused.
#![allow(non_snake_case)]
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    ptr,
};
use vst3::{
    Steinberg::{Vst::*, *},
    *,
};

pub struct Host;
impl Class for Host {
    type Interfaces = (IHostApplication,);
}
impl IHostApplicationTrait for Host {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        if name.is_null() {
            return kInvalidArgument;
        }
        *name = [0; 128];
        for (i, c) in "PirateWave".encode_utf16().enumerate() {
            (*name)[i] = c;
        }
        kResultOk
    }
    unsafe fn createInstance(
        &self,
        cid: *mut TUID,
        iid: *mut TUID,
        obj: *mut *mut c_void,
    ) -> tresult {
        if obj.is_null() || cid.is_null() || iid.is_null() {
            return kInvalidArgument;
        }
        *obj = ptr::null_mut();
        if *cid == IMessage::IID.map(|b| b as i8) && *iid == IMessage::IID.map(|b| b as i8) {
            let value = ComWrapper::new(Message {
                id: RefCell::new(std::ffi::CString::default()),
                attributes: ComWrapper::new(Attributes::default()),
            })
            .to_com_ptr::<IMessage>()
            .unwrap();
            *obj = value.as_ptr().cast();
            std::mem::forget(value);
            kResultOk
        } else if *cid == IAttributeList::IID.map(|b| b as i8)
            && *iid == IAttributeList::IID.map(|b| b as i8)
        {
            let value = ComWrapper::new(Attributes::default())
                .to_com_ptr::<IAttributeList>()
                .unwrap();
            *obj = value.as_ptr().cast();
            std::mem::forget(value);
            kResultOk
        } else {
            kNoInterface
        }
    }
}
pub struct Handler {
    pub restart: Cell<i32>,
}
impl Class for Handler {
    type Interfaces = (IComponentHandler,);
}
impl IComponentHandlerTrait for Handler {
    unsafe fn beginEdit(&self, _id: u32) -> tresult {
        kResultOk
    }
    unsafe fn performEdit(&self, _id: u32, _value: f64) -> tresult {
        kResultOk
    }
    unsafe fn endEdit(&self, _id: u32) -> tresult {
        kResultOk
    }
    unsafe fn restartComponent(&self, flags: i32) -> tresult {
        self.restart.set(self.restart.get() | flags);
        kResultOk
    }
}
pub struct Events {
    pub events: RefCell<Vec<Event>>,
}
impl Events {
    pub fn new() -> Self {
        Self {
            events: RefCell::new(Vec::with_capacity(256)),
        }
    }
}
impl Class for Events {
    type Interfaces = (IEventList,);
}
impl IEventListTrait for Events {
    unsafe fn getEventCount(&self) -> i32 {
        self.events.borrow().len() as i32
    }
    unsafe fn getEvent(&self, index: i32, e: *mut Event) -> tresult {
        if e.is_null() {
            return kInvalidArgument;
        }
        match self.events.borrow().get(index as usize) {
            Some(event) => {
                *e = *event;
                kResultOk
            }
            None => kInvalidArgument,
        }
    }
    unsafe fn addEvent(&self, e: *mut Event) -> tresult {
        if e.is_null() {
            return kInvalidArgument;
        }
        let mut events = self.events.borrow_mut();
        if events.len() >= 256 {
            return kOutOfMemory;
        }
        // Output events are accepted for compatibility, but PirateWave does not forward MIDI.
        events.push(*e);
        kResultOk
    }
}
pub struct ParamQueue {
    pub id: u32,
    pub value: Cell<f64>,
}
impl Class for ParamQueue {
    type Interfaces = (IParamValueQueue,);
}
impl IParamValueQueueTrait for ParamQueue {
    unsafe fn getParameterId(&self) -> u32 {
        self.id
    }
    unsafe fn getPointCount(&self) -> i32 {
        1
    }
    unsafe fn getPoint(&self, index: i32, offset: *mut i32, value: *mut f64) -> tresult {
        if index != 0 || offset.is_null() || value.is_null() {
            return kInvalidArgument;
        }
        *offset = 0;
        *value = self.value.get();
        kResultOk
    }
    unsafe fn addPoint(&self, _offset: i32, value: f64, index: *mut i32) -> tresult {
        self.value.set(value);
        if !index.is_null() {
            *index = 0;
        }
        kResultOk
    }
}
pub struct Params {
    pub queues: Vec<ComWrapper<ParamQueue>>,
    pub active: RefCell<Vec<usize>>,
}
impl Params {
    pub fn new(ids: impl Iterator<Item = u32>) -> Self {
        let queues: Vec<_> = ids
            .map(|id| {
                ComWrapper::new(ParamQueue {
                    id,
                    value: Cell::new(0.0),
                })
            })
            .collect();
        let active = RefCell::new(Vec::with_capacity(queues.len()));
        Self { queues, active }
    }
    pub fn set(&self, id: u32, value: f64) {
        if let Some(index) = self.queues.iter().position(|q| q.id == id) {
            self.queues[index].value.set(value);
            let mut active = self.active.borrow_mut();
            if !active.contains(&index) {
                active.push(index);
            }
        }
    }
}
impl Class for Params {
    type Interfaces = (IParameterChanges,);
}
impl IParameterChangesTrait for Params {
    unsafe fn getParameterCount(&self) -> i32 {
        self.active.borrow().len() as i32
    }
    unsafe fn getParameterData(&self, index: i32) -> *mut IParamValueQueue {
        self.active
            .borrow()
            .get(index as usize)
            .map_or(ptr::null_mut(), |i| {
                self.queues[*i]
                    .as_com_ref::<IParamValueQueue>()
                    .unwrap()
                    .as_ptr()
            })
    }
    unsafe fn addParameterData(&self, id: *const u32, index: *mut i32) -> *mut IParamValueQueue {
        if id.is_null() {
            return ptr::null_mut();
        }
        let Some(queue) = self.queues.iter().position(|q| q.id == *id) else {
            return ptr::null_mut();
        };
        let mut active = self.active.borrow_mut();
        let position = match active.iter().position(|i| *i == queue) {
            Some(i) => i,
            None => {
                active.push(queue);
                active.len() - 1
            }
        };
        if !index.is_null() {
            *index = position as i32;
        }
        self.queues[queue]
            .as_com_ref::<IParamValueQueue>()
            .unwrap()
            .as_ptr()
    }
}
#[derive(Default)]
pub struct MemoryStream {
    data: RefCell<Vec<u8>>,
    position: Cell<usize>,
}
impl MemoryStream {
    pub fn rewind(&self) {
        self.position.set(0);
    }
}
impl Class for MemoryStream {
    type Interfaces = (IBStream,);
}
impl IBStreamTrait for MemoryStream {
    unsafe fn read(&self, buffer: *mut c_void, bytes: i32, read: *mut i32) -> tresult {
        if buffer.is_null() || bytes < 0 {
            return kInvalidArgument;
        }
        let data = self.data.borrow();
        let start = self.position.get().min(data.len());
        let n = (bytes as usize).min(data.len() - start);
        ptr::copy_nonoverlapping(data.as_ptr().add(start), buffer.cast(), n);
        self.position.set(start + n);
        if !read.is_null() {
            *read = n as i32;
        }
        if n > 0 || bytes == 0 {
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn write(&self, buffer: *mut c_void, bytes: i32, written: *mut i32) -> tresult {
        if buffer.is_null() || bytes < 0 {
            return kInvalidArgument;
        }
        let start = self.position.get();
        let end = start.saturating_add(bytes as usize);
        if end > 64 * 1024 * 1024 {
            return kOutOfMemory;
        }
        let mut data = self.data.borrow_mut();
        if data.len() < end {
            data.resize(end, 0);
        }
        ptr::copy_nonoverlapping(
            buffer.cast::<u8>(),
            data.as_mut_ptr().add(start),
            bytes as usize,
        );
        self.position.set(end);
        if !written.is_null() {
            *written = bytes;
        }
        kResultOk
    }
    unsafe fn seek(&self, pos: i64, mode: i32, result: *mut i64) -> tresult {
        let base = match mode {
            0 => 0,
            1 => self.position.get() as i64,
            2 => self.data.borrow().len() as i64,
            _ => return kInvalidArgument,
        };
        let Some(next) = base
            .checked_add(pos)
            .filter(|n| (0..=64 * 1024 * 1024).contains(n))
        else {
            return kInvalidArgument;
        };
        self.position.set(next as usize);
        if !result.is_null() {
            *result = next;
        }
        kResultOk
    }
    unsafe fn tell(&self, pos: *mut i64) -> tresult {
        if pos.is_null() {
            return kInvalidArgument;
        }
        *pos = self.position.get() as i64;
        kResultOk
    }
}

// VST3 components/controllers use host-allocated messages while connecting.
struct Message {
    id: RefCell<std::ffi::CString>,
    attributes: ComWrapper<Attributes>,
}
impl Class for Message {
    type Interfaces = (IMessage,);
}
impl IMessageTrait for Message {
    unsafe fn getMessageID(&self) -> FIDString {
        self.id.borrow().as_ptr()
    }
    unsafe fn setMessageID(&self, id: FIDString) {
        if !id.is_null() {
            *self.id.borrow_mut() = std::ffi::CStr::from_ptr(id).to_owned();
        }
    }
    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        self.attributes
            .as_com_ref::<IAttributeList>()
            .unwrap()
            .as_ptr()
    }
}
enum Attribute {
    Int(i64),
    Float(f64),
    String(Vec<u16>),
    Binary(Vec<u8>),
}
#[derive(Default)]
struct Attributes(RefCell<std::collections::HashMap<Vec<u8>, Attribute>>);
unsafe fn key(id: *const std::ffi::c_char) -> Option<Vec<u8>> {
    (!id.is_null()).then(|| std::ffi::CStr::from_ptr(id).to_bytes().to_vec())
}
impl Class for Attributes {
    type Interfaces = (IAttributeList,);
}
impl IAttributeListTrait for Attributes {
    unsafe fn setInt(&self, id: *const std::ffi::c_char, value: i64) -> tresult {
        let Some(id) = key(id) else {
            return kInvalidArgument;
        };
        self.0.borrow_mut().insert(id, Attribute::Int(value));
        kResultOk
    }
    unsafe fn getInt(&self, id: *const std::ffi::c_char, value: *mut i64) -> tresult {
        if value.is_null() {
            return kInvalidArgument;
        }
        let Some(id) = key(id) else {
            return kInvalidArgument;
        };
        if let Some(Attribute::Int(v)) = self.0.borrow().get(&id) {
            *value = *v;
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn setFloat(&self, id: *const std::ffi::c_char, value: f64) -> tresult {
        let Some(id) = key(id) else {
            return kInvalidArgument;
        };
        self.0.borrow_mut().insert(id, Attribute::Float(value));
        kResultOk
    }
    unsafe fn getFloat(&self, id: *const std::ffi::c_char, value: *mut f64) -> tresult {
        if value.is_null() {
            return kInvalidArgument;
        }
        let Some(id) = key(id) else {
            return kInvalidArgument;
        };
        if let Some(Attribute::Float(v)) = self.0.borrow().get(&id) {
            *value = *v;
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn setString(&self, id: *const std::ffi::c_char, value: *const u16) -> tresult {
        let Some(id) = key(id) else {
            return kInvalidArgument;
        };
        if value.is_null() {
            return kInvalidArgument;
        }
        let mut string = Vec::new();
        for i in 0..65536 {
            let c = *value.add(i);
            string.push(c);
            if c == 0 {
                self.0.borrow_mut().insert(id, Attribute::String(string));
                return kResultOk;
            }
        }
        kInvalidArgument
    }
    unsafe fn getString(
        &self,
        id: *const std::ffi::c_char,
        value: *mut u16,
        bytes: u32,
    ) -> tresult {
        if value.is_null() || bytes < 2 {
            return kInvalidArgument;
        }
        let Some(id) = key(id) else {
            return kInvalidArgument;
        };
        if let Some(Attribute::String(string)) = self.0.borrow().get(&id) {
            let count = string.len().min(bytes as usize / 2);
            ptr::copy_nonoverlapping(string.as_ptr(), value, count);
            *value.add(count - 1) = 0;
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn setBinary(
        &self,
        id: *const std::ffi::c_char,
        data: *const c_void,
        bytes: u32,
    ) -> tresult {
        if data.is_null() || bytes > 64 * 1024 * 1024 {
            return kInvalidArgument;
        }
        let Some(id) = key(id) else {
            return kInvalidArgument;
        };
        self.0.borrow_mut().insert(
            id,
            Attribute::Binary(
                std::slice::from_raw_parts(data.cast::<u8>(), bytes as usize).to_vec(),
            ),
        );
        kResultOk
    }
    unsafe fn getBinary(
        &self,
        id: *const std::ffi::c_char,
        data: *mut *const c_void,
        bytes: *mut u32,
    ) -> tresult {
        if data.is_null() || bytes.is_null() {
            return kInvalidArgument;
        }
        let Some(id) = key(id) else {
            return kInvalidArgument;
        };
        if let Some(Attribute::Binary(value)) = self.0.borrow().get(&id) {
            *data = value.as_ptr().cast();
            *bytes = value.len() as u32;
            kResultOk
        } else {
            kResultFalse
        }
    }
}
