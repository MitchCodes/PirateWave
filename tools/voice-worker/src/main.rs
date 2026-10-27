//! Optional native CPU inference process. Binary stdin/stdout are private IPC.
use anyhow::{ensure, Result};
use std::io::{Read, Write};
use tch::{CModule, IValue, Tensor};
const HOP: usize = 416;
const INPUT: usize = 448;
fn main() -> Result<()> {
    let model_path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Missing model"))?;
    // Explicit developer-only IPC mode. PirateWave never supplies these arguments.
    let diagnostic_states = std::env::args_os()
        .skip(2)
        .any(|s| s == "--diagnostic-states");
    let no_warm = std::env::args_os().skip(2).any(|s| s == "--no-warm");
    tch::set_num_threads(2);
    tch::set_num_interop_threads(1);
    let _guard = tch::no_grad_guard();
    // Open through Rust's Unicode-aware Windows filesystem API. Passing a path
    // directly to LibTorch fails for some non-ASCII workspace/user names.
    let mut model_file = std::fs::File::open(std::path::PathBuf::from(model_path))?;
    let mut model = CModule::load_data(&mut model_file)?;
    model.set_eval();
    let shapes: &[&[i64]] = &[&[1, 512, 510], &[1, 2, 13, 256], &[1, 512, 4], &[1, 1, 24]];
    let mut states: Vec<_> = shapes
        .iter()
        .map(|s| Tensor::zeros(*s, (tch::Kind::Float, tch::Device::Cpu)))
        .collect();
    // Compile/warm native kernels before accepting live audio, then reset all
    // streaming buffers. Startup work stays in this optional worker process.
    for _ in 0..if no_warm { 0 } else { 24 } {
        let mut args = vec![IValue::Tensor(Tensor::zeros(
            [1, 1, INPUT as i64],
            (tch::Kind::Float, tch::Device::Cpu),
        ))];
        args.extend(states.drain(..).map(IValue::Tensor));
        let IValue::Tuple(result) = model.forward_is(&args)? else {
            anyhow::bail!("Invalid warmup output");
        };
        states = result
            .into_iter()
            .skip(1)
            .map(|v| match v {
                IValue::Tensor(t) => Ok(t),
                _ => anyhow::bail!("Invalid warmup state"),
            })
            .collect::<Result<Vec<_>>>()?;
    }
    states = shapes
        .iter()
        .map(|s| Tensor::zeros(*s, (tch::Kind::Float, tch::Device::Cpu)))
        .collect();
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    output.write_all(if diagnostic_states {
        b"PATCHVS1"
    } else {
        b"PATCHVC1"
    })?;
    output.flush()?;
    let mut packet = [0_u8; INPUT * 4];
    let mut state_copy = if diagnostic_states {
        vec![0_f32; 269848]
    } else {
        vec![]
    };
    loop {
        match input.read_exact(&mut packet) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        let values: Vec<_> = packet
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        ensure!(values.iter().all(|v| v.is_finite()), "Nonfinite input");
        let mut args = vec![IValue::Tensor(Tensor::from_slice(&values).reshape([
            1,
            1,
            INPUT as i64,
        ]))];
        args.extend(states.drain(..).map(IValue::Tensor));
        let IValue::Tuple(result) = model.forward_is(&args)? else {
            anyhow::bail!("Invalid model output");
        };
        ensure!(result.len() == 5, "Wrong output count");
        let mut tensors = result
            .into_iter()
            .map(|v| match v {
                IValue::Tensor(t) => Ok(t),
                _ => anyhow::bail!("Invalid tensor"),
            })
            .collect::<Result<Vec<_>>>()?;
        let converted: Vec<f32> = Vec::try_from(tensors.remove(0).reshape([-1]))?;
        ensure!(
            converted.len() == HOP && converted.iter().all(|v| v.is_finite()),
            "Invalid audio output"
        );
        states = tensors;
        let mut bytes = [0_u8; HOP * 4];
        for (i, value) in converted.into_iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        output.write_all(&bytes)?;
        if diagnostic_states {
            ensure!(
                cfg!(target_endian = "little"),
                "Diagnostic float stream requires little-endian host"
            );
            for state in &states {
                let flat = state.reshape([-1]);
                let count = flat.numel();
                ensure!(count <= state_copy.len(), "Invalid diagnostic state size");
                flat.copy_data(&mut state_copy[..count], count);
                // f32 has no padding; the checked initialized slice is valid as
                // bytes for this little-endian diagnostic protocol.
                let raw = unsafe {
                    std::slice::from_raw_parts(state_copy.as_ptr().cast::<u8>(), count * 4)
                };
                output.write_all(raw)?;
            }
        }
        output.flush()?;
    }
}
