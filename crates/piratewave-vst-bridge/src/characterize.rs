//! Explicit offline compatibility probes; never run during ordinary playback.
use anyhow::{Context, Result};
use piratewave_vst2_host::{Hosted, Vst2};
use std::path::Path;
pub fn run(args: &[String]) -> Result<bool> {
    let Some(command) = args.get(1) else {
        return Ok(false);
    };
    if command != "--characterize" && command != "--voice-reference" {
        return Ok(false);
    }
    let path = Path::new(args.get(2).context("Missing plugin path")?);
    let output = args
        .get(3)
        .context("Missing characterization report path")?;
    if command == "--characterize" {
        let path = Path::new(args.get(2).context("Missing plugin path")?);
        let output = args
            .get(3)
            .context("Missing characterization report path")?;
        let mut plugin = Vst2::load(path)?;
        let descriptor = plugin.descriptor().clone();
        let mut parameters = vec![];
        for parameter in descriptor
            .parameters
            .iter()
            .filter(|p| !p.name.starts_with("xxx"))
        {
            let mut values = vec![];
            for step in 0..=28 {
                let normalized = step as f64 / 28.0;
                plugin.set_parameter(parameter.id, normalized);
                values.push(serde_json::json!({"normalized":normalized,"text":plugin.parameter_text(parameter.id)}));
            }
            plugin.set_parameter(parameter.id, parameter.default);
            parameters.push(serde_json::json!({"id":parameter.id,"name":parameter.name,"values":values,"default_text":plugin.parameter_text(parameter.id)}));
        }
        std::fs::write(
            output,
            serde_json::to_vec_pretty(
                &serde_json::json!({"plugin":descriptor.name,"parameters":parameters}),
            )?,
        )?;
    } else {
        let mut cases = vec![];
        for (name, input, scale, detune, transpose) in [
            ("chromatic_a", 430.0, 2.0, 0.0, 0.0),
            ("chromatic_b", 250.0, 2.0, 0.0, 0.0),
            ("c_major", 307.0, 0.0, 0.0, 0.0),
            ("c_minor", 307.0, 1.0, 0.0, 0.0),
            ("transpose", 430.0, 2.0, 0.0, 7.0),
            ("detune", 440.0, 2.0, 25.0, 0.0),
        ] {
            let mut plugin = Vst2::load(path)?;
            for (id, value) in [
                (1, scale / 28.0),
                (2, 0.0),
                (3, (detune + 100.0) / 200.0),
                (4, 1.0),
                (61, 0.0),
                (74, (transpose + 12.0) / 24.0),
            ] {
                plugin.set_parameter(id, value);
            }
            let mut rendered = Vec::with_capacity(96000);
            for block in 0..750 {
                let mut frames = std::array::from_fn::<_, 128, _>(|i| {
                    let phase = std::f32::consts::TAU * input * (block * 128 + i) as f32 / 48000.0;
                    let sample =
                        0.2 * phase.sin() + 0.08 * (2.0 * phase).sin() + 0.04 * (3.0 * phase).sin();
                    [sample; 2]
                });
                plugin.process(&mut frames)?;
                rendered.extend_from_slice(&frames);
            }
            let steady = &rendered[48000..];
            let crossings: Vec<f32> = steady
                .windows(2)
                .enumerate()
                .filter(|(_, w)| w[0][0] <= 0.0 && w[1][0] > 0.0)
                .map(|(i, w)| i as f32 - w[0][0] / (w[1][0] - w[0][0]))
                .collect();
            anyhow::ensure!(crossings.len() > 2, "No voiced output for {name}");
            let hz =
                48000.0 * (crossings.len() - 1) as f32 / (crossings.last().unwrap() - crossings[0]);
            cases.push(serde_json::json!({"name":name,"input_hz":input,"output_hz":hz}));
        }
        std::fs::write(
            output,
            serde_json::to_vec_pretty(&serde_json::json!({"cases":cases}))?,
        )?;
    }
    Ok(true)
}
