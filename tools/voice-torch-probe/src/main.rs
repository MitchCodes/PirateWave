//! Native-only evaluation of a fixed-shape LLVC TorchScript candidate.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, time::Instant};
use tch::{CModule, IValue, Tensor};
#[path = "../../voice-probe/src/metrics.rs"]
mod metrics;

#[derive(Deserialize)]
struct Fixture {
    hop: usize,
    sample_rate: usize,
    shapes: Vec<Vec<usize>>,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    name: String,
    chunks: usize,
}
#[derive(Serialize)]
struct Measurement {
    name: String,
    max_audio_error: f64,
    rms_audio_error: f64,
    states: Vec<metrics::StateError>,
    pass: bool,
}
fn floats(path: &Path) -> Result<Vec<f32>> {
    let bytes = fs::read(path)?;
    ensure!(bytes.len() % 4 == 0, "Invalid float file");
    Ok(bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect())
}
fn tensor(data: &[f32], shape: &[usize]) -> Tensor {
    Tensor::from_slice(data).reshape(shape.iter().map(|&v| v as i64).collect::<Vec<_>>())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() == 5,
        "usage: piratewave-voice-torch-probe <fixture-dir> <model.pt> <threads> <report.json>"
    );
    let dir = Path::new(&args[1]);
    let fixture: Fixture = serde_json::from_slice(&fs::read(dir.join("fixture.json"))?)?;
    ensure!(
        fixture.shapes.len() == 5 && fixture.hop > 0,
        "Invalid fixture"
    );
    let threads: i32 = args[3].parse()?;
    ensure!((1..=2).contains(&threads), "Use 1 or 2 threads");
    tch::set_num_threads(threads);
    tch::set_num_interop_threads(1);
    let mut model = CModule::load(&args[2])?;
    model.set_eval();
    let _guard = tch::no_grad_guard();
    let mut cases = vec![];
    let mut times = vec![];
    for case in &fixture.cases {
        let input = floats(&dir.join(format!("{}.input.f32", case.name)))?;
        let reference = floats(&dir.join(format!("{}.reference.f32", case.name)))?;
        let reference_state = floats(&dir.join(format!("{}.states.f32", case.name)))?;
        let packet = fixture.shapes[0].iter().product::<usize>();
        ensure!(
            input.len() == packet * case.chunks && reference.len() == fixture.hop * case.chunks,
            "Invalid case lengths"
        );
        let mut states: Vec<_> = fixture.shapes[1..]
            .iter()
            .map(|s| tensor(&vec![0.0; s.iter().product()], s))
            .collect();
        let mut peak = 0.0_f64;
        let mut squares = 0.0;
        for (i, block) in input.chunks_exact(packet).enumerate() {
            let started = Instant::now();
            let mut values = vec![IValue::Tensor(tensor(block, &fixture.shapes[0]))];
            values.extend(states.drain(..).map(IValue::Tensor));
            let IValue::Tuple(output) = model.forward_is(&values)? else {
                anyhow::bail!("Expected output tuple");
            };
            ensure!(output.len() == 5, "Wrong output count");
            let mut tensors = output
                .into_iter()
                .map(|v| match v {
                    IValue::Tensor(t) => Ok(t),
                    _ => anyhow::bail!("Expected tensor"),
                })
                .collect::<Result<Vec<_>>>()?;
            let converted: Vec<f32> = Vec::try_from(tensors.remove(0).reshape([-1]))?;
            ensure!(converted.len() == fixture.hop, "Wrong output size");
            states = tensors;
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            if i >= 20 {
                times.push(elapsed);
            }
            for (&a, &b) in converted.iter().zip(&reference[i * fixture.hop..]) {
                ensure!(a.is_finite() && b.is_finite(), "Nonfinite audio");
                let d = f64::from(a) - f64::from(b);
                peak = peak.max(d.abs());
                squares += d * d;
            }
        }
        let mut offset = 0;
        let mut errors = vec![];
        for (name, value) in ["encoder", "decoder", "synthesis", "prenet"]
            .iter()
            .zip(states)
        {
            let actual: Vec<f32> = Vec::try_from(value.reshape([-1]))?;
            ensure!(
                offset + actual.len() <= reference_state.len(),
                "Truncated reference state"
            );
            errors.push(metrics::state_error(
                name,
                &actual,
                &reference_state[offset..offset + actual.len()],
            )?);
            offset += actual.len();
        }
        ensure!(offset == reference_state.len(), "State length mismatch");
        let rms = (squares / reference.len() as f64).sqrt();
        println!("{}: max={peak}, RMS={rms}", case.name);
        cases.push(Measurement {
            name: case.name.clone(),
            max_audio_error: peak,
            rms_audio_error: rms,
            pass: peak <= 1e-4 && rms <= 1e-5 && errors.iter().all(|e| e.tensor_pass),
            states: errors,
        });
    }
    ensure!(!times.is_empty(), "No warm timings");
    times.sort_by(f64::total_cmp);
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    let p99 = times[((times.len() - 1) as f64 * 0.99).ceil() as usize];
    let hop_ms = fixture.hop as f64 * 1000.0 / fixture.sample_rate as f64;
    let parity = cases.iter().all(|c| c.pass);
    let timing = mean <= hop_ms / 2.0 && p99 < hop_ms;
    let report = serde_json::json!({"runtime":"LibTorch 2.5.1 CPU", "threads":threads,
        "cpu_identifier":std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_default(),
        "model_sha256":format!("{:x}",Sha256::digest(fs::read(&args[2])?)),
        "hop_ms":hop_ms,"mean_ms":mean,"p99_ms":p99,"max_ms":times.last(),
        "parity_pass":parity,"timing_pass":timing,"cases":cases});
    fs::write(&args[4], serde_json::to_string_pretty(&report)?)?;
    println!("{report}");
    ensure!(
        parity && timing,
        "Native qualification failed; report saved"
    );
    Ok(())
}
