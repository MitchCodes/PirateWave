//! Optional hybrid worker: original CPU encoder/synthesis, DirectML transformer.
//! No Python, audio devices, graph callbacks or implicit CPU execution fallback.
use anyhow::{ensure, Context, Result};
use ort::{DirectMLExecutionProvider, Session, Tensor as OrtTensor};
use std::{
    io::{Read, Write},
    path::Path,
};
use tch::{CModule, IValue, Tensor};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
const INPUT: usize = 448;
const HOP: usize = 416;
const SHAPES: &[&[i64]] = &[&[1, 512, 510], &[1, 2, 13, 256], &[1, 512, 4], &[1, 1, 24]];

fn tensors(value: IValue) -> Result<Vec<Tensor>> {
    let IValue::Tuple(values) = value else {
        anyhow::bail!("Invalid model tuple")
    };
    values
        .into_iter()
        .map(|v| match v {
            IValue::Tensor(t) => Ok(t),
            _ => anyhow::bail!("Invalid tensor"),
        })
        .collect()
}
fn iv(values: &[Tensor]) -> Vec<IValue> {
    values
        .iter()
        .map(|v| IValue::Tensor(v.shallow_clone()))
        .collect()
}
fn zeros() -> Vec<Tensor> {
    SHAPES
        .iter()
        .map(|s| Tensor::zeros(*s, (tch::Kind::Float, tch::Device::Cpu)))
        .collect()
}
fn adapter() -> Result<i32> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
    for index in 0..32 {
        let Ok(a) = (unsafe { factory.EnumAdapters1(index) }) else {
            break;
        };
        let mut desc = Default::default();
        unsafe { a.GetDesc1(&mut desc)? };
        let name = String::from_utf16_lossy(&desc.Description)
            .trim_end_matches('\0')
            .to_owned();
        if desc.Flags & 2 == 0 && name == "NVIDIA GeForce RTX 4070 SUPER" {
            eprintln!("DirectML adapter {index}: {name}; CPU encoder/synthesis, GPU transformer");
            return Ok(index as i32);
        }
    }
    anyhow::bail!(
        "No qualified DirectML adapter: this profile requires NVIDIA GeForce RTX 4070 SUPER"
    )
}
struct Hybrid {
    front: CModule,
    finish: CModule,
    decoder: Session,
}
impl Hybrid {
    fn open(root: &Path) -> Result<Self> {
        ort::init_from(root.join("onnxruntime.dll").to_string_lossy())
            .with_telemetry(false)
            .commit()?;
        let decoder = Session::builder()?
            .with_parallel_execution(false)?
            .with_memory_pattern(false)?
            .with_intra_threads(1)?
            .with_inter_threads(1)?
            .with_config_entry("session.intra_op.allow_spinning", "0")?
            .with_config_entry("session.inter_op.allow_spinning", "0")?
            .with_config_entry("session.disable_cpu_ep_fallback", "1")?
            .with_execution_providers([DirectMLExecutionProvider::default()
                .with_device_id(adapter()?)
                .build()
                .error_on_failure()])?
            .commit_from_file(root.join("decoder.onnx"))?;
        let mut front = CModule::load(root.join("front.pt"))?;
        let mut finish = CModule::load(root.join("finish.pt"))?;
        front.set_eval();
        finish.set_eval();
        Ok(Self {
            front,
            finish,
            decoder,
        })
    }
    fn step(&self, audio: Tensor, states: &[Tensor]) -> Result<Vec<Tensor>> {
        ensure!(states.len() == 4, "Invalid state count");
        let f = tensors(self.front.forward_is(&iv(&[
            audio,
            states[0].shallow_clone(),
            states[3].shallow_clone(),
        ]))?)?;
        ensure!(f.len() == 6, "Invalid front output");
        let mut inputs = Vec::with_capacity(3);
        for (name, tensor) in [("target", &f[2]), ("memory", &f[3]), ("state", &states[1])] {
            let shape: Vec<usize> = tensor.size().iter().map(|v| *v as usize).collect();
            let data: Vec<f32> = Vec::try_from(tensor.reshape([-1]))?;
            inputs.push((name, OrtTensor::from_array((shape, data))?.into_dyn()));
        }
        let output = self.decoder.run(inputs)?;
        let (_, mask) = output["mask"].try_extract_raw_tensor::<f32>()?;
        let (_, dec) = output["out_state"].try_extract_raw_tensor::<f32>()?;
        ensure!(
            mask.len() == 256 * 26 && dec.len() == 2 * 13 * 256,
            "Invalid GPU tensor contract"
        );
        let s = tensors(self.finish.forward_is(&iv(&[
            f[0].shallow_clone(),
            f[1].shallow_clone(),
            Tensor::from_slice(mask).reshape([1, 256, 26]),
            states[2].shallow_clone(),
        ]))?)?;
        ensure!(s.len() == 2, "Invalid synthesis output");
        Ok(vec![
            s[0].shallow_clone(),
            f[4].shallow_clone(),
            Tensor::from_slice(dec).reshape(SHAPES[1]),
            s[1].shallow_clone(),
            f[5].shallow_clone(),
        ])
    }
}
#[allow(clippy::chunks_exact_to_as_chunks)] // Fixed four-byte wire samples, matching the CPU IPC.
fn main() -> Result<()> {
    let model = std::env::args_os()
        .nth(1)
        .context("Missing pack model path")?;
    let root = Path::new(&model)
        .parent()
        .context("Missing pack directory")?;
    let diagnostic = std::env::args_os()
        .skip(2)
        .any(|s| s == "--diagnostic-states");
    tch::set_num_threads(2);
    tch::set_num_interop_threads(1);
    let _guard = tch::no_grad_guard();
    let runtime = Hybrid::open(root)?;
    let mut states = zeros();
    for _ in 0..24 {
        states = runtime
            .step(
                Tensor::zeros([1, 1, INPUT as i64], (tch::Kind::Float, tch::Device::Cpu)),
                &states,
            )?
            .into_iter()
            .skip(1)
            .collect();
    }
    states = zeros();
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    output.write_all(if diagnostic { b"PATCHVS1" } else { b"PATCHVC1" })?;
    output.flush()?;
    let mut bytes = [0_u8; INPUT * 4];
    loop {
        match input.read_exact(&mut bytes) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        let values: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        ensure!(values.iter().all(|v| v.is_finite()), "Nonfinite input");
        let mut result = runtime.step(
            Tensor::from_slice(&values).reshape([1, 1, INPUT as i64]),
            &states,
        )?;
        let audio: Vec<f32> = Vec::try_from(result.remove(0).reshape([-1]))?;
        ensure!(
            audio.len() == HOP && audio.iter().all(|v| v.is_finite()),
            "Invalid converted audio"
        );
        for sample in audio {
            output.write_all(&sample.to_le_bytes())?;
        }
        states = result;
        if diagnostic {
            for state in &states {
                let data: Vec<f32> = Vec::try_from(state.reshape([-1]))?;
                let mut bytes = Vec::with_capacity(data.len() * 4);
                for value in data {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                output.write_all(&bytes)?;
            }
        }
        output.flush()?;
    }
}
