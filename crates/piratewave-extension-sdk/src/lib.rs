//! PirateWave ABI 1. The wire ABI is C; Rust objects never cross the DLL boundary.
use serde::{Deserialize, Serialize};
use std::ffi::c_void;
pub const ABI_VERSION: u32 = 1;
pub const MAX_PORTS: usize = 8;
pub const MAX_PARAMETERS: usize = 64;
pub const MAX_STATE: usize = 65536;
pub type Frame = [f32; 2];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub abi: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub license: String,
    pub library: String,
    pub types: Vec<TypeDescriptor>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeKind {
    Processor,
    Node,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeDescriptor {
    /// Fully qualified, e.g. com.example.audio/width. Never a list position.
    pub id: String,
    pub name: String,
    pub kind: TypeKind,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub help: String,
    #[serde(default = "category")]
    pub category: String,
    #[serde(default)]
    pub inputs: Vec<Port>,
    #[serde(default)]
    pub outputs: Vec<Port>,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default)]
    pub actions: Vec<Action>,
    #[serde(default = "one")]
    pub state_schema: u32,
    #[serde(default)]
    pub default_state: Vec<u8>,
    #[serde(default)]
    pub bypass: Vec<Bypass>,
    #[serde(default)]
    pub latency_frames: u32,
}
fn one() -> u32 {
    1
}
fn category() -> String {
    "Extensions".into()
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Port {
    pub id: u32,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub id: String,
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    #[serde(default)]
    pub unit: String,
    /// Zero means continuous. Positive steps are anchored at min.
    #[serde(default)]
    pub step: f32,
    #[serde(default)]
    pub choices: Vec<String>,
}
impl Parameter {
    pub fn valid(&self, value: f32) -> bool {
        value.is_finite()
            && value >= self.min
            && value <= self.max
            && (self.step == 0.
                || (((value - self.min) / self.step).round() * self.step + self.min - value).abs()
                    <= 0.0001)
    }
    pub fn snap(&self, value: f32) -> f32 {
        let value = value.clamp(self.min, self.max);
        if self.step > 0. {
            (self.min + ((value - self.min) / self.step).round() * self.step)
                .clamp(self.min, self.max)
        } else {
            value
        }
    }
}
/// Declarative actions update persistent parameters through PirateWave's dispatcher.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub id: String,
    pub name: String,
    pub parameter: String,
    pub value: f32,
    #[serde(default)]
    pub toggle: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bypass {
    pub input: u32,
    pub output: u32,
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 160
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-/".contains(&c))
}
fn text(s: &str) -> bool {
    s.len() <= 4096 && !s.contains('\0')
}
impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.abi != ABI_VERSION
            || !valid_id(&self.id)
            || self.id.contains('/')
            || self.types.is_empty()
            || self.types.len() > 32
            || ![&self.name, &self.version, &self.author, &self.license]
                .iter()
                .all(|s| text(s))
        {
            return Err("Invalid manifest or incompatible ABI".into());
        }
        if !self.library.ends_with(".dll")
            || self.library.len() > 160
            || self.library.contains(['/', '\\', ':'])
            || self.library.starts_with('.')
        {
            return Err("Library must be a package-local DLL filename".into());
        }
        let mut ids = std::collections::HashSet::new();
        for t in &self.types {
            t.validate()?;
            if !t.id.starts_with(&format!("{}/", self.id)) || !ids.insert(&t.id) {
                return Err("Duplicate or unqualified type identity".into());
            }
        }
        Ok(())
    }
}
impl TypeDescriptor {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_id(&self.id)
            || !self.id.contains('/')
            || ![&self.name, &self.description, &self.help, &self.category]
                .iter()
                .all(|s| text(s))
            || self.inputs.len() > MAX_PORTS
            || self.outputs.len() > MAX_PORTS
            || self.outputs.is_empty()
            || self.parameters.len() > MAX_PARAMETERS
            || self.actions.len() > 64
            || self.default_state.len() > MAX_STATE
            || self.state_schema == 0
            || self.latency_frames != 0
        {
            return Err("Invalid extension type or nonzero latency".into());
        }
        for ports in [&self.inputs, &self.outputs] {
            let mut ids = std::collections::HashSet::new();
            for p in ports {
                if !ids.insert(p.id) || !text(&p.name) {
                    return Err("Invalid or duplicate port".into());
                }
            }
        }
        if self.kind == TypeKind::Processor && (self.inputs.len() != 1 || self.outputs.len() != 1) {
            return Err("Rack processor needs one stereo input and output".into());
        }
        let mut ids = std::collections::HashSet::new();
        for p in &self.parameters {
            if !valid_id(&p.id)
                || !ids.insert(&p.id)
                || !text(&p.name)
                || !text(&p.unit)
                || !p.min.is_finite()
                || !p.max.is_finite()
                || p.min >= p.max
                || !(p.max - p.min).is_finite()
                || (p.step > 0.
                    && (!((p.max - p.min) / p.step).is_finite() || p.step > p.max - p.min))
                || !p.step.is_finite()
                || p.step < 0.
                || !p.valid(p.default)
                || p.choices.len() > 128
                || p.choices.iter().any(|s| !text(s))
                || (!p.choices.is_empty()
                    && (p.step != 1. || p.min != 0. || p.max != (p.choices.len() - 1) as f32))
            {
                return Err("Invalid extension parameter".into());
            }
        }
        let mut ids = std::collections::HashSet::new();
        let mut hashes = std::collections::HashSet::new();
        for a in &self.actions {
            let p = self
                .parameters
                .iter()
                .find(|p| p.id == a.parameter)
                .ok_or("Action parameter missing")?;
            if !valid_id(&a.id)
                || !ids.insert(&a.id)
                || !hashes.insert(a.id.bytes().fold(0xcbf29ce484222325u64, |h, b| {
                    (h ^ b as u64).wrapping_mul(0x100000001b3)
                }))
                || !text(&a.name)
                || !p.valid(a.value)
            {
                return Err("Invalid extension action".into());
            }
        }
        let mut outputs = std::collections::HashSet::new();
        for b in &self.bypass {
            if !self.inputs.iter().any(|p| p.id == b.input)
                || !self.outputs.iter().any(|p| p.id == b.output)
                || !outputs.insert(b.output)
            {
                return Err("Invalid bypass map".into());
            }
        }
        Ok(())
    }
}

/// ABI functions return 0 on success and nonzero on failure. All pointers are
/// borrowed for that call. State is immutable declarative configuration.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Api {
    pub version: u32,
    pub size: u32,
    pub describe: unsafe extern "C" fn(*mut u32) -> *const u8,
    pub create: unsafe extern "C" fn(u32, f32, u32, u32, *const u8, u32) -> *mut c_void,
    pub migrate: unsafe extern "C" fn(u32, u32, *const u8, u32, *mut u8, *mut u32) -> i32,
    pub destroy: unsafe extern "C" fn(*mut c_void),
    pub set_parameter: unsafe extern "C" fn(*mut c_void, u32, f32) -> i32,
    pub reset: unsafe extern "C" fn(*mut c_void) -> i32,
    pub process:
        unsafe extern "C" fn(*mut c_void, *const *const f32, u32, *const *mut f32, u32, u32) -> i32,
}
/// Safe authoring wrapper. No heap allocation occurs when constructing a Block.
pub struct Block<'a> {
    inputs: &'a [*const f32],
    outputs: &'a [*mut f32],
    frames: usize,
}
impl Block<'_> {
    pub fn frames(&self) -> usize {
        self.frames
    }
    pub fn input(&self, port: usize) -> &[Frame] {
        unsafe { std::slice::from_raw_parts(self.inputs[port].cast(), self.frames) }
    }
    pub fn output(&mut self, port: usize) -> &mut [Frame] {
        unsafe { std::slice::from_raw_parts_mut(self.outputs[port].cast(), self.frames) }
    }
    pub fn input_count(&self) -> usize {
        self.inputs.len()
    }
    pub fn output_count(&self) -> usize {
        self.outputs.len()
    }
}
pub trait Processor: Send {
    fn set_parameter(&mut self, index: u32, value: f32) -> bool;
    fn process(&mut self, block: &mut Block<'_>) -> bool;
    fn reset(&mut self);
}
pub trait Plugin {
    fn migrate(_type_index: u32, _old_schema: u32, _state: &[u8]) -> Option<Vec<u8>> {
        None
    }
    const MANIFEST: &'static str;
    fn create(
        type_index: u32,
        rate: f32,
        block_size: u32,
        schema: u32,
        state: &[u8],
    ) -> Option<Box<dyn Processor>>;
}
struct Handle(Box<dyn Processor>);
fn guarded(f: impl FnOnce() -> bool) -> i32 {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(false) {
        0
    } else {
        -1
    }
}
/// SDK-internal ABI implementation. Callers must follow ABI.md.
pub mod export {
    use super::*;
    /// # Safety
    /// `len` must point to a writable u32.
    pub unsafe extern "C" fn describe<P: Plugin>(len: *mut u32) -> *const u8 {
        if len.is_null() {
            return std::ptr::null();
        }
        *len = P::MANIFEST.len() as u32;
        P::MANIFEST.as_ptr()
    }
    /// # Safety
    /// State must be readable for len bytes (or null when len is zero).
    pub unsafe extern "C" fn create<P: Plugin>(
        index: u32,
        rate: f32,
        block: u32,
        schema: u32,
        state: *const u8,
        len: u32,
    ) -> *mut c_void {
        if len as usize > MAX_STATE || (len > 0 && state.is_null()) {
            return std::ptr::null_mut();
        }
        let state = if len == 0 {
            &[]
        } else {
            std::slice::from_raw_parts(state, len as usize)
        };
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            P::create(index, rate, block, schema, state)
                .map(|p| Box::into_raw(Box::new(Handle(p))).cast())
                .unwrap_or(std::ptr::null_mut())
        }))
        .unwrap_or(std::ptr::null_mut())
    }
    /// # Safety
    /// State is readable for len bytes. Output points to MAX_STATE writable bytes,
    /// output_len points to a writable u32. Called only on the control plane.
    pub unsafe extern "C" fn migrate<P: Plugin>(
        index: u32,
        schema: u32,
        state: *const u8,
        len: u32,
        out: *mut u8,
        out_len: *mut u32,
    ) -> i32 {
        if len as usize > MAX_STATE || state.is_null() || out.is_null() || out_len.is_null() {
            return -1;
        }
        guarded(|| {
            let input = std::slice::from_raw_parts(state, len as usize);
            if let Some(bytes) = P::migrate(index, schema, input) {
                if bytes.len() > MAX_STATE {
                    return false;
                }
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len());
                *out_len = bytes.len() as u32;
                true
            } else {
                false
            }
        })
    }
    /// # Safety
    /// Handle must be a live instance created by this module, destroyed once.
    pub unsafe extern "C" fn destroy(h: *mut c_void) {
        if !h.is_null() {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                drop(Box::from_raw(h.cast::<Handle>()))
            }));
        }
    }
    /// # Safety
    /// Handle is live, exclusively owned, and not processing concurrently.
    pub unsafe extern "C" fn set(h: *mut c_void, i: u32, v: f32) -> i32 {
        if h.is_null() {
            return -1;
        }
        guarded(|| (*h.cast::<Handle>()).0.set_parameter(i, v))
    }
    /// # Safety
    /// Handle is live and exclusively owned.
    pub unsafe extern "C" fn reset(h: *mut c_void) -> i32 {
        if h.is_null() {
            return -1;
        }
        guarded(|| {
            (*h.cast::<Handle>()).0.reset();
            true
        })
    }
    /// # Safety
    /// All buffers are valid interleaved stereo arrays; outputs do not alias
    /// any buffer. Handle is live, exclusively owned. Counts match its type.
    pub unsafe extern "C" fn process(
        h: *mut c_void,
        inputs: *const *const f32,
        ni: u32,
        outputs: *const *mut f32,
        no: u32,
        n: u32,
    ) -> i32 {
        if h.is_null()
            || ni as usize > MAX_PORTS
            || no as usize > MAX_PORTS
            || n > 128
            || inputs.is_null()
            || outputs.is_null()
        {
            return -1;
        }
        let inputs = std::slice::from_raw_parts(inputs, ni as usize);
        let outputs = std::slice::from_raw_parts(outputs, no as usize);
        if inputs.iter().any(|p| p.is_null()) || outputs.iter().any(|p| p.is_null()) {
            return -1;
        }
        guarded(|| {
            (*h.cast::<Handle>()).0.process(&mut Block {
                inputs,
                outputs,
                frames: n as usize,
            })
        })
    }
}
#[macro_export]
macro_rules! export_plugin {
    ($plugin:ty) => {
        static PIRATEWAVE_API: $crate::Api = $crate::Api {
            version: $crate::ABI_VERSION,
            size: std::mem::size_of::<$crate::Api>() as u32,
            describe: $crate::export::describe::<$plugin>,
            create: $crate::export::create::<$plugin>,
            migrate: $crate::export::migrate::<$plugin>,
            destroy: $crate::export::destroy,
            set_parameter: $crate::export::set,
            reset: $crate::export::reset,
            process: $crate::export::process,
        };
        #[no_mangle]
        pub extern "C" fn piratewave_extension_api() -> *const $crate::Api {
            &PIRATEWAVE_API
        }
    };
}

pub fn parse_manifest(bytes: &[u8]) -> Result<Manifest, String> {
    if bytes.len() > 262144 {
        return Err("Manifest too large".into());
    }
    let m: Manifest = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    m.validate()?;
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Manifest {
        parse_manifest(include_bytes!(
            "../../piratewave-example-extension/manifest.json"
        ))
        .unwrap()
    }
    #[test]
    fn rejects_abi_latency_duplicate_identity_and_unsafe_library() {
        let mut m = fixture();
        m.abi = 99;
        assert!(m.validate().is_err());
        m = fixture();
        m.types[0].latency_frames = 1;
        assert!(m.validate().is_err());
        m = fixture();
        m.types.push(m.types[0].clone());
        assert!(m.validate().is_err());
        m = fixture();
        m.library = "../plugin.dll".into();
        assert!(m.validate().is_err());
    }
    #[test]
    fn validates_choice_steps_ranges_ports_actions_and_state_limits() {
        let mut m = fixture();
        m.types[0].parameters[1].step = 0.;
        assert!(m.validate().is_err());
        m = fixture();
        m.types[0].parameters[0].default = 3.;
        assert!(m.validate().is_err());
        m = fixture();
        let duplicate = m.types[1].inputs[0].clone();
        m.types[1].inputs.push(duplicate);
        assert!(m.validate().is_err());
        m = fixture();
        m.types[0].actions[0].parameter = "missing".into();
        assert!(m.validate().is_err());
        m = fixture();
        m.types[0].default_state = vec![0; MAX_STATE + 1];
        assert!(m.validate().is_err());
    }
    #[test]
    fn discrete_snapping_anchors_at_minimum() {
        let p = Parameter {
            id: "x".into(),
            name: "x".into(),
            min: 10.,
            max: 20.,
            default: 10.,
            step: 2.,
            unit: "".into(),
            choices: vec![],
        };
        assert_eq!(p.snap(13.), 14.);
        assert!(!p.valid(13.));
        assert!(p.valid(14.));
        assert_eq!(p.snap(-999.), 10.);
        assert_eq!(p.snap(999.), 20.);
    }
    struct Panicking;
    impl Processor for Panicking {
        fn set_parameter(&mut self, _: u32, _: f32) -> bool {
            panic!("fixture set")
        }
        fn process(&mut self, _: &mut Block<'_>) -> bool {
            panic!("fixture process")
        }
        fn reset(&mut self) {
            panic!("fixture reset")
        }
    }
    #[test]
    fn rust_callback_panics_return_errors_without_crossing_abi() {
        let h = Box::into_raw(Box::new(Handle(Box::new(Panicking)))).cast();
        let input = [[0.; 2]; 128];
        let mut output = [[0.; 2]; 128];
        let inputs = [input.as_ptr().cast()];
        let outputs = [output.as_mut_ptr().cast()];
        unsafe {
            assert_ne!(export::set(h, 0, 0.), 0);
            assert_ne!(
                export::process(h, inputs.as_ptr(), 1, outputs.as_ptr(), 1, 128),
                0
            );
            assert_ne!(export::reset(h), 0);
            assert_ne!(
                export::process(h, inputs.as_ptr(), 99, outputs.as_ptr(), 1, 128),
                0
            );
            export::destroy(h);
        }
    }
}
