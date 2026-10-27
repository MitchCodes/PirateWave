//! Optional local Voice Creation Beta and independent training storage.
//! No Python, downloads, or training are started by opening this module's UI.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub mod hardware;
pub mod release;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
static AUDIO_RUNNING: AtomicBool = AtomicBool::new(false);
static AUDIO_STARTS: AtomicU64 = AtomicU64::new(0);
pub fn audio_requested() {
    AUDIO_STARTS.fetch_add(1, Ordering::AcqRel);
}
pub fn audio_running(running: bool) {
    AUDIO_RUNNING.store(running, Ordering::Release);
}
pub const GUIDE: &[(&str, &str)] = &[
    ("What creating a voice means", "Beta training learns a consistent vocal identity from recordings. PirateWave prepares a teacher voice, generates paired speech, trains LLVC and checks native playback. Current user-trained results can sound robotic or metallic; the pretrained voice is cleaner in our listening tests. A finished job does not guarantee a convincing voice. Everything stays local. Training tools are optional."),
    ("1. Choose one voice", "Use your own recordings or material you have permission to use for voice training. Record one speaker and one consistent character. If you want both a natural voice and a dramatically different character, prepare separate voices."),
    ("2. Find a quiet space", "Close windows, turn off nearby fans when practical, and use a room with soft furnishings. Echo, background music, other speakers and keyboard noise can become part of the learned voice. Re-record a noisy take instead of trying to repair it with heavy processing."),
    ("3. Record a clean microphone", "Use the same microphone position and distance throughout. Record the raw microphone, without reverb, automatic voice correction, a voice changer or aggressive noise suppression. Keep the level comfortable and leave room for louder words; a clipped, crackling recording cannot be repaired by training."),
    ("4. Vary what you say", "Include natural sentences, questions, soft and firm consonants, different vowels, and comfortable changes in expression. Speak clearly in the character you want to learn. Do not repeat one sentence for the entire recording. Clean, varied speech is more useful than extra noisy material."),
    ("How much should I record?", "This Beta accepts up to eight minutes of included training recordings and thirty minutes across all roles. Use several shorter takes, keeping separate validation and test recordings. Requirements for better LLVC quality remain unmeasured; these limits are practical Beta limits, not a promise of quality. Prefer clean, varied material over extra noisy speech."),
    ("Practice prompts", "Read these naturally, then add your own varied sentences: ‘Please bring the blue jacket before Thursday.’ ‘Did you hear the quiet footsteps outside?’ ‘Six bright birds flew past the window.’ ‘I thought we agreed to meet at half past five.’ Include ordinary conversation as well as reading."),
    ("5. Plan the storage", "Training tools, shared models, source speech, generated pairs, features and checkpoints can use much more space than the final voice. Download size, installed size and peak working space are different numbers. Choose a workspace drive before installing tools. Unknown requirements must be measured before training becomes available."),
    ("6. Give training time", "Training runs locally and can take a long time. A compatible GPU driver is still required for GPU training. CPU, NVIDIA, Intel and DirectML training need separate qualification; a working live conversion backend does not prove training support. PirateWave will pause heavy training while its audio engine runs."),
    ("7. Compare on new speech", "Use headphones and a monitoring output. Compare the original recording, teacher voice and live LLVC result with speech that was not used for training. Test several input speakers, quiet and loud speech, pauses and consonant-heavy sentences. A lower training loss does not necessarily mean a better-sounding voice."),
    ("Troubleshooting: noisy or muffled", "First listen to the original recordings. Remove clips with music, strong echo or heavy processing; improve microphone placement and record them again. More training is not a reliable cure for poor recordings."),
    ("Troubleshooting: missing syllables or unstable character", "Check clean held-out speech and consistent target recordings. Mixed speakers, changing characters and too little varied material can hurt results. Conversion and export must pass streaming validation before a new voice is usable."),
    ("Troubleshooting: storage or interruption", "Keep original recordings. Checkpoints allow training to resume; deleting them removes that ability. Generated pairs and features can be rebuilt, but rebuilding takes time. Tools and shared models may be reused by multiple voices. Never delete a folder while a training job is using it."),
    ("Use your finished voice", "Once qualified, route microphone → light cleanup → Voice Changer → output bus. Keep music separate. Avoid a dry microphone route to the same output, which would play both voices together. Explicit passthrough sends your original voice; automatic conversion failures produce silence."),
];

#[derive(Clone, Serialize, Deserialize)]
pub struct Settings {
    pub schema: u32,
    pub workspace: PathBuf,
}
impl Default for Settings {
    fn default() -> Self {
        let base = directories::ProjectDirs::from("audio", "PirateWave", "PirateWave")
            .map(|p| p.data_local_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("local"));
        Self {
            schema: 1,
            workspace: base.join("voice-training"),
        }
    }
}
impl Settings {
    pub fn path() -> PathBuf {
        crate::storage::default_path().with_file_name("training-settings.json")
    }
    pub fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let settings: Self = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        ensure!(
            settings.schema == 1 && settings.workspace.is_absolute(),
            "Unsupported voice-training settings"
        );
        Ok(settings)
    }
    pub fn save(&self) -> Result<()> {
        ensure!(
            self.workspace.is_absolute(),
            "Choose an absolute workspace path"
        );
        crate::storage::save_json(&Self::path(), self)
    }
}

#[derive(Default)]
pub struct StorageUsage {
    pub tools: u64,
    pub shared: u64,
    pub recordings: u64,
    pub jobs: u64,
    pub downloads: u64,
    pub other: u64,
    pub free: Option<u64>,
}
fn size(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let mut total = 0_u64;
    let mut pending = vec![path.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        // Windows junctions/reparse points must not lead a storage scan outside
        // the selected workspace, even if file_type does not call them symlinks.
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                continue;
            }
        }
        if metadata.is_dir() {
            for entry in std::fs::read_dir(path)? {
                pending.push(entry?.path());
            }
        } else if metadata.is_file() {
            total = total
                .checked_add(metadata.len())
                .context("Storage size overflow")?;
        }
    }
    Ok(total)
}
pub fn scan(root: &Path) -> Result<StorageUsage> {
    let mut usage = StorageUsage::default();
    for (name, value) in [
        ("tools", &mut usage.tools),
        ("shared", &mut usage.shared),
        ("recordings", &mut usage.recordings),
        ("jobs", &mut usage.jobs),
        ("downloads", &mut usage.downloads),
    ] {
        *value = size(&root.join(name))?;
    }
    let known = usage
        .tools
        .saturating_add(usage.shared)
        .saturating_add(usage.recordings)
        .saturating_add(usage.jobs)
        .saturating_add(usage.downloads);
    usage.other = size(root)?.saturating_sub(known);
    let mut existing = root;
    while !existing.exists() {
        existing = existing
            .parent()
            .context("Workspace drive is unavailable")?;
    }
    usage.free = free_space(existing);
    Ok(usage)
}
#[cfg(windows)]
fn free_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0;
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available)
}
#[cfg(not(windows))]
fn free_space(_: &Path) -> Option<u64> {
    None
}

/// Required *additional* bytes, including staging supplied by the caller.
pub fn required_space(peak_additional: u64) -> Option<u64> {
    peak_additional.checked_add((2 * 1024_u64.pow(3)).max(peak_additional.div_ceil(10)))
}
pub fn bytes(value: u64) -> String {
    if value >= 1024_u64.pow(3) {
        format!("{:.2} GiB", value as f64 / 1024_f64.powi(3))
    } else if value >= 1024_u64.pow(2) {
        format!("{:.1} MiB", value as f64 / 1024_f64.powi(2))
    } else {
        format!("{value} bytes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_scan_counts_categories_without_double_counting() {
        let root = std::env::temp_dir().join(format!(
            "piratewave-training-storage-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("tools")).unwrap();
        std::fs::create_dir(root.join("jobs")).unwrap();
        std::fs::write(root.join("tools/runtime"), [0; 17]).unwrap();
        std::fs::write(root.join("jobs/checkpoint"), [0; 23]).unwrap();
        std::fs::write(root.join("notes"), [0; 5]).unwrap();
        let usage = scan(&root).unwrap();
        assert_eq!((usage.tools, usage.jobs, usage.other), (17, 23, 5));
        assert_eq!((usage.shared, usage.downloads, usage.recordings), (0, 0, 0));
        #[cfg(windows)]
        assert!(usage.free.is_some_and(|n| n > 0));
        for name in ["tools/runtime", "jobs/checkpoint", "notes"] {
            std::fs::remove_file(root.join(name)).unwrap();
        }
        for name in ["tools", "jobs"] {
            std::fs::remove_dir(root.join(name)).unwrap();
        }
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn storage_budget_keeps_headroom_and_rejects_overflow() {
        assert_eq!(required_space(0), Some(2 * 1024_u64.pow(3)));
        assert_eq!(
            required_space(100 * 1024_u64.pow(3)),
            Some(110 * 1024_u64.pow(3))
        );
        assert_eq!(required_space(u64::MAX), None);
    }
    #[test]
    fn guide_covers_recording_storage_evaluation_and_failures() {
        let text = GUIDE
            .iter()
            .map(|(_, text)| *text)
            .collect::<Vec<_>>()
            .join(" ");
        for word in [
            "eight minutes",
            "clipped",
            "checkpoints",
            "loss",
            "monitoring",
            "permission",
            "silence",
            "DirectML",
        ] {
            assert!(text.contains(word), "Missing guidance: {word}");
        }
    }
}
