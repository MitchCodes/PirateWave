//! Local immutable model revisions. Executable workers always come from PirateWave.
use crate::training::release::{digest, save, Job, NATIVE_WORKER_SHA256};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Clone, Serialize, Deserialize)]
pub struct Voice {
    pub schema: u32,
    pub id: String,
    pub name: String,
    pub model_sha256: String,
    pub cpu_identifier: String,
    pub logical_processors: String,
    pub worker_sha256: String,
    pub report_sha256: String,
    pub numerical_seconds: f64,
    pub beta: bool,
}
pub fn root() -> PathBuf {
    crate::storage::default_path().with_file_name("local-voices")
}
pub fn list() -> Vec<Voice> {
    fs::read_dir(root())
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name().to_str().is_some_and(valid_id) && e.file_type().is_ok_and(|f| f.is_dir())
        })
        .filter_map(|e| fs::read(e.path().join("voice.json")).ok())
        .filter_map(|bytes| serde_json::from_slice::<Voice>(&bytes).ok())
        .filter(|v| valid_id(&v.id))
        .collect()
}
fn valid_id(id: &str) -> bool {
    id.starts_with("local-")
        && id.len() <= 128
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
pub fn resolve(id: &str) -> Result<PathBuf> {
    ensure!(valid_id(id), "Unknown local voice identifier");
    let path = root().join(id);
    let voice: Voice = serde_json::from_slice(
        &fs::read(path.join("voice.json"))
            .context("Custom voice is missing; reinstall its training job")?,
    )?;
    ensure!(
        voice.id == id && voice.schema == 1 && voice.worker_sha256 == NATIVE_WORKER_SHA256,
        "Voice revision is incompatible with this worker"
    );
    ensure!(
        voice.cpu_identifier == std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_default()
            && voice.logical_processors
                == std::env::var("NUMBER_OF_PROCESSORS").unwrap_or_default(),
        "This CPU differs from the custom voice validation machine; revalidate the training job"
    );
    ensure!(
        digest(&path.join("llvc-stream.pt"))? == voice.model_sha256
            && digest(&path.join("validation.json"))? == voice.report_sha256,
        "Custom voice checksum mismatch; reinstall the completed job"
    );
    crate::training::release::native_worker()?;
    Ok(path.join("llvc-stream.pt"))
}
pub fn install(folder: &Path) -> Result<String> {
    let job = Job::load(folder)?;
    ensure!(
        job.status == "complete",
        "Complete and validate training before installation"
    );
    let report_path = folder.join("native/report.json");
    let report: serde_json::Value = serde_json::from_slice(&fs::read(&report_path)?)?;
    ensure!(
        report["waveform_and_state_pass"] == true
            && report["timing_pass"] == true
            && report["worker_sha256"] == NATIVE_WORKER_SHA256,
        "Custom voice did not pass native CPU checks"
    );
    let model = folder.join("native/llvc-stream.pt");
    let hash = digest(&model)?;
    ensure!(
        report["model_sha256"] == hash,
        "Model differs from the validated export"
    );
    let id = format!("local-{}-{}", job.id, &hash[..12]);
    ensure!(valid_id(&id), "Invalid voice revision identifier");
    let path = root().join(&id);
    if path.exists() {
        resolve(&id)?;
        return Ok(id);
    }
    fs::create_dir_all(root())?;
    let staging = root().join(format!("staging-{}", job.id));
    ensure!(
        !staging.exists(),
        "Previous installation staging exists; inspect it before retrying"
    );
    fs::create_dir(&staging)?;
    fs::copy(&model, staging.join("llvc-stream.pt"))?;
    fs::copy(&report_path, staging.join("validation.json"))?;
    fs::copy(
        folder.join("review/3-llvc-offline.wav"),
        staging.join("preview.wav"),
    )?;
    let voice = Voice {
        schema: 1,
        id: id.clone(),
        name: job.name,
        model_sha256: hash,
        cpu_identifier: std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_default(),
        logical_processors: std::env::var("NUMBER_OF_PROCESSORS").unwrap_or_default(),
        worker_sha256: NATIVE_WORKER_SHA256.into(),
        report_sha256: digest(&report_path)?,
        numerical_seconds: report["seconds"].as_f64().unwrap_or(0.),
        beta: true,
    };
    save(&staging.join("voice.json"), &voice)?;
    save(
        &staging.join("provenance.json"),
        &serde_json::json!({"recipe":job.recipe,"job":job.id,"backend":job.backend,"recordings":job.recordings,
        "limitations":"Local Beta: short numerical and CPU timing checks only; not a 30-minute or gaming-impact qualification. DirectML is not qualified for this model."}),
    )?;
    fs::write(staging.join("NOTICES.txt"),"PirateWave user-trained LLVC Beta. LLVC: Koe AI, MIT. RVC: RVC contributors, MIT. Source speech: LibriSpeech, CC BY 4.0, Vassil Panayotov and Daniel Povey. Target recordings supplied locally by the user. Training tools contain full dependency notices. This pack contains model data only; no executable or training checkpoints.\n")?;
    fs::rename(staging, path)?;
    Ok(id)
}
pub fn check_pipeline(id: &str, speech: &Path) -> Result<serde_json::Value> {
    use super::{Rack, PRIMING, READY};
    use piratewave_core::{
        project::{Connection, Node, NodeKind, Project},
        BLOCK_SIZE,
    };
    use std::{
        sync::atomic::Ordering,
        thread,
        time::{Duration, Instant},
    };
    let _audio = crate::audio_scheduling::AudioThread::register();
    let mut project = Project::default();
    project.nodes.clear();
    project.connections.clear();
    let mut node = Node::new(
        2,
        "Custom voice validation",
        NodeKind::VoiceChanger,
        [0.; 2],
    );
    node.voice.get_or_insert_default().pack = id.into();
    project.nodes.push(node.clone());
    project.nodes.push(Node::new(
        3,
        "Bus",
        NodeKind::Output { device: None },
        [0.; 2],
    ));
    project.connections.push(Connection {
        input: Default::default(),
        port: Default::default(),
        from: 2,
        to: 3,
    });
    let mut rack = Rack::new(&project);
    let timeout = Instant::now() + Duration::from_secs(20);
    while rack.shared.state.load(Ordering::Acquire) != PRIMING && Instant::now() < timeout {
        thread::sleep(Duration::from_millis(5));
    }
    ensure!(
        rack.shared.state.load(Ordering::Acquire) == PRIMING,
        "Custom voice could not prime: {:?}",
        rack.status()
    );
    let clip = crate::assets::load(speech)?;
    let start = Instant::now();
    let mut deadline = start;
    let mut frames = 0;
    let mut first_ready = None;
    let mut nonzero = false;
    while start.elapsed() < Duration::from_secs(30) {
        let mut block = std::array::from_fn::<_, BLOCK_SIZE, _>(|i| {
            clip.frames[(frames + i) % clip.frames.len()]
        });
        rack.process(&node, &mut block);
        ensure!(
            block.iter().flatten().all(|v| v.is_finite()),
            "Non-finite custom voice output"
        );
        nonzero |= block.iter().flatten().any(|v| v.abs() > 1e-5);
        if rack.shared.state.load(Ordering::Acquire) == READY {
            first_ready.get_or_insert(start.elapsed().as_secs_f64() * 1000.);
        }
        if first_ready.is_some() {
            ensure!(
                rack.shared.state.load(Ordering::Acquire) == READY,
                "Custom voice lost readiness: {:?}",
                rack.status()
            );
        }
        frames += BLOCK_SIZE;
        deadline += Duration::from_secs_f64(BLOCK_SIZE as f64 / 48000.);
        if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
    }
    ensure!(
        nonzero && first_ready.is_some(),
        "Custom voice never produced converted audio"
    );
    let underruns = rack.shared.underruns.load(Ordering::Acquire);
    ensure!(
        underruns == 0,
        "Custom live pipeline underruns: {underruns}"
    );
    Ok(
        serde_json::json!({"seconds":30,"finite_output":true,"nonzero_output":nonzero,"first_ready_ms":first_ready,
        "underruns":underruns,"maximum_inference_ms":rack.shared.max_inference_us.load(Ordering::Acquire) as f64/1000.,"pass":true}),
    )
}
