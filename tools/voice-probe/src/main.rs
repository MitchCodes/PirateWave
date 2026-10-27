//! Offline stateful ONNX parity/performance probe. No microphone or routing side effects.
use anyhow::{ensure, Context, Result};
use ort::{DirectMLExecutionProvider, DynValue, Session, Tensor};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, time::Instant};
mod metrics;

#[derive(Deserialize)]
struct Fixture {
    schema: u32,
    onnx_sha256: String,
    sample_rate: usize,
    hop: usize,
    shapes: Vec<Vec<usize>>,
    input_names: Vec<String>,
    output_names: Vec<String>,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    name: String,
    chunks: usize,
    #[serde(default)]
    initial_state_file: Option<String>,
}
#[derive(Serialize)]
struct Measurement {
    name: String,
    samples: usize,
    max_audio_error: f32,
    rms_audio_error: f64,
    max_state_error: f32,
    max_state_scaled_error: f32,
    final_states: Vec<metrics::StateError>,
    final_tensor_state_pass: bool,
    parity_pass: bool,
}
#[derive(Serialize)]
struct Report {
    adapter: Option<String>,
    runtime: String,
    model_sha256: String,
    backend: String,
    threads: usize,
    hop_ms: f64,
    session_load_ms: f64,
    first_chunk_ms: f64,
    mean_ms: f64,
    p99_ms: f64,
    max_ms: f64,
    timing_pass: bool,
    warm_measurements: usize,
    parity_pass: bool,
    cases: Vec<Measurement>,
}
fn floats(path: &Path) -> Result<Vec<f32>> {
    let bytes = fs::read(path).with_context(|| path.display().to_string())?;
    ensure!(bytes.len() % 4 == 0, "Invalid float fixture");
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect())
}
#[cfg(windows)]
fn directml_adapter() -> Result<(i32, String)> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
    let mut devices = vec![];
    for index in 0..32 {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
            break;
        };
        let mut description = Default::default();
        unsafe { adapter.GetDesc1(&mut description)? };
        if description.Flags & 2 != 0 {
            continue;
        }
        let name = String::from_utf16_lossy(&description.Description)
            .trim_end_matches('\0')
            .to_owned();
        devices.push((index as i32, name));
    }
    // A virtual display may precede the hardware accelerator in DXGI order.
    let selected = devices
        .iter()
        .find(|(_, name)| name.contains("NVIDIA"))
        .or_else(|| {
            devices
                .iter()
                .find(|(_, name)| name.contains("AMD") || name.contains("Intel"))
        })
        .or_else(|| devices.first())
        .context("No hardware DXGI adapter")?;
    Ok(selected.clone())
}
#[cfg(not(windows))]
fn directml_adapter() -> Result<(i32, String)> {
    anyhow::bail!("DirectML is Windows-only")
}
fn states(fixture: &Fixture) -> Result<Vec<DynValue>> {
    fixture.shapes[1..]
        .iter()
        .map(|s| Ok(Tensor::from_array((s.clone(), vec![0.0_f32; s.iter().product()]))?.into_dyn()))
        .collect()
}
fn percentile(values: &[f64], fraction: f64) -> f64 {
    values[((values.len() - 1) as f64 * fraction).ceil() as usize]
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(args.len() == 6, "usage: piratewave-voice-probe <fixture-dir> <onnxruntime.dll> <cpu|directml> <threads> <report.json>");
    let input_path = Path::new(&args[1]);
    let fixture_path = if input_path.is_dir() {
        input_path.join("fixture.json")
    } else {
        input_path.to_owned()
    };
    let dir = fixture_path.parent().context("Missing fixture directory")?;
    let fixture: Fixture = serde_json::from_slice(&fs::read(&fixture_path)?)?;
    ensure!(
        fixture.schema == 1
            && fixture.shapes.len() == 5
            && fixture.input_names.len() == 5
            && fixture.output_names.len() == 5,
        "Unsupported fixture contract"
    );
    ensure!(
        fixture.sample_rate > 0 && fixture.hop > 0 && !fixture.cases.is_empty(),
        "Empty fixture"
    );
    let digest = format!(
        "{:x}",
        Sha256::digest(fs::read(dir.join("llvc-stream.onnx"))?)
    );
    ensure!(digest == fixture.onnx_sha256, "ONNX checksum mismatch");
    let threads: usize = args[4].parse()?;
    ensure!((1..=2).contains(&threads), "Use one or two CPU threads");
    ensure!(args[3] == "cpu" || args[3] == "directml", "Unknown backend");
    ort::init_from(&args[2]).with_telemetry(false).commit()?;
    let started = Instant::now();
    let mut adapter_name = None;
    let mut builder = Session::builder()?
        .with_parallel_execution(false)?
        .with_intra_threads(threads)?
        .with_inter_threads(1)?;
    builder = builder
        .with_config_entry("session.intra_op.allow_spinning", "0")?
        .with_config_entry("session.inter_op.allow_spinning", "0")?;
    if args[3] == "directml" {
        let (adapter, name) = directml_adapter()?;
        println!("DirectML adapter {adapter}: {name}");
        adapter_name = Some(name);
        builder = builder
            .with_memory_pattern(false)?
            .with_execution_providers([DirectMLExecutionProvider::default()
                .with_device_id(adapter)
                .build()
                .error_on_failure()])?;
        // Fail explicitly rather than reporting CPU fallback as GPU inference.
        builder = builder.with_config_entry("session.disable_cpu_ep_fallback", "1")?;
    }
    let session = builder.commit_from_file(dir.join("llvc-stream.onnx"))?;
    let load_ms = started.elapsed().as_secs_f64() * 1000.0;
    ensure!(
        session
            .inputs
            .iter()
            .map(|i| &i.name)
            .eq(fixture.input_names.iter()),
        "Input contract mismatch"
    );
    ensure!(
        session
            .outputs
            .iter()
            .map(|i| &i.name)
            .eq(fixture.output_names.iter()),
        "Output contract mismatch"
    );
    let mut measurements = vec![];
    let mut times = vec![];
    let mut first = None;
    for case in &fixture.cases {
        let audio = floats(&dir.join(format!("{}.input.f32", case.name)))?;
        let reference = floats(&dir.join(format!("{}.reference.f32", case.name)))?;
        let reference_state = floats(&dir.join(format!("{}.states.f32", case.name)))?;
        let packet: usize = fixture.shapes[0].iter().product();
        ensure!(
            audio.len() == case.chunks * packet && reference.len() == case.chunks * fixture.hop,
            "Invalid case lengths"
        );
        let mut memory = states(&fixture)?;
        if let Some(file) = &case.initial_state_file {
            let initial = floats(&dir.join(file))?;
            let mut offset = 0;
            memory.clear();
            for shape in &fixture.shapes[1..] {
                let count = shape.iter().product::<usize>();
                ensure!(offset + count <= initial.len(), "Initial state truncated");
                memory.push(
                    Tensor::from_array((shape.clone(), initial[offset..offset + count].to_vec()))?
                        .into_dyn(),
                );
                offset += count;
            }
            ensure!(offset == initial.len(), "Initial state has extra values");
        }
        let mut max_error = 0.0_f32;
        let mut squares = 0.0;
        let mut rendered = Vec::with_capacity(reference.len());
        for (index, input) in audio.chunks_exact(packet).enumerate() {
            let started = Instant::now();
            let tensor =
                Tensor::from_array((fixture.shapes[0].clone(), input.to_vec()))?.into_dyn();
            let mut values = vec![(fixture.input_names[0].as_str(), tensor)];
            values.extend(
                fixture.input_names[1..]
                    .iter()
                    .zip(memory.drain(..))
                    .map(|(n, v)| (n.as_str(), v)),
            );
            let mut output = session.run(values)?;
            let (_, converted) =
                output[fixture.output_names[0].as_str()].try_extract_raw_tensor::<f32>()?;
            ensure!(converted.len() == fixture.hop, "Output length mismatch");
            rendered.extend_from_slice(converted);
            for (&actual, &expected) in converted.iter().zip(&reference[index * fixture.hop..]) {
                ensure!(
                    actual.is_finite() && expected.is_finite(),
                    "Nonfinite audio"
                );
                let difference = (actual - expected).abs();
                max_error = max_error.max(difference);
                squares += f64::from(difference).powi(2);
            }
            memory = fixture.output_names[1..]
                .iter()
                .map(|n| output.remove(n.as_str()).context("Missing output state"))
                .collect::<Result<_>>()?;
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            if first.is_none() {
                first = Some(ms);
            }
            if index >= 20 {
                times.push(ms);
            }
        }
        let mut max_state = 0.0_f32;
        let mut max_state_scaled = 0.0_f32;
        let mut offset = 0;
        let mut final_states = Vec::new();
        for (name, value) in fixture.output_names[1..].iter().zip(&memory) {
            let (_, actual) = value.try_extract_raw_tensor::<f32>()?;
            ensure!(
                offset + actual.len() <= reference_state.len(),
                "State length mismatch"
            );
            final_states.push(metrics::state_error(
                name,
                actual,
                &reference_state[offset..offset + actual.len()],
            )?);
            for (&a, &b) in actual.iter().zip(&reference_state[offset..]) {
                ensure!(a.is_finite() && b.is_finite(), "Nonfinite state");
                max_state = max_state.max((a - b).abs());
                // Hidden activations have arbitrary scale, unlike audio samples.
                // Use explicit absolute + relative float32 tolerances.
                max_state_scaled =
                    max_state_scaled.max((a - b).abs() / (0.0001 + b.abs() * 0.0001));
            }
            offset += actual.len();
        }
        ensure!(offset == reference_state.len(), "State length mismatch");
        let rms = (squares / reference.len() as f64).sqrt();
        let mut wav = hound::WavWriter::create(
            dir.join(format!("{}.{}.wav", case.name, args[3])),
            hound::WavSpec {
                channels: 1,
                sample_rate: fixture.sample_rate as u32,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        for value in rendered {
            wav.write_sample(value)?;
        }
        wav.finalize()?;
        println!(
            "{}: audio max={max_error:.8}, rms={rms:.8}, state max={max_state:.8}",
            case.name
        );
        measurements.push(Measurement {
            name: case.name.clone(),
            samples: reference.len(),
            max_audio_error: max_error,
            rms_audio_error: rms,
            max_state_error: max_state,
            max_state_scaled_error: max_state_scaled,
            final_tensor_state_pass: final_states.iter().all(|s| s.tensor_pass),
            final_states,
            parity_pass: max_error <= 0.0001 && rms <= 0.00001 && max_state_scaled <= 1.0,
        });
    }
    let warm_measurements = times.len();
    // A saved single-step state reproduction has no warm timing samples.
    // Report its first-call time, explicitly disqualifying it as a benchmark.
    if times.is_empty() {
        times.push(first.context("No inference measurements")?);
    }
    times.sort_by(f64::total_cmp);
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    let p99 = percentile(&times, 0.99);
    let hop_ms = fixture.hop as f64 * 1000.0 / fixture.sample_rate as f64;
    let report = Report {
        adapter: adapter_name,
        runtime: ort::info().into(),
        model_sha256: digest,
        backend: args[3].clone(),
        threads,
        hop_ms,
        session_load_ms: load_ms,
        first_chunk_ms: first.unwrap(),
        mean_ms: mean,
        p99_ms: p99,
        max_ms: *times.last().unwrap(),
        timing_pass: warm_measurements > 0 && mean <= hop_ms * 0.5 && p99 < hop_ms,
        warm_measurements,
        parity_pass: measurements.iter().all(|m| m.parity_pass),
        cases: measurements,
    };
    let encoded = serde_json::to_string_pretty(&report)?;
    fs::write(&args[5], &encoded)?;
    println!("{encoded}");
    ensure!(report.parity_pass, "Reference parity failed; report saved");
    ensure!(report.timing_pass, "Timing gate failed; report saved");
    Ok(())
}
