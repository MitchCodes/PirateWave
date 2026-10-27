//! Pinned optional model downloads. No network or archive work runs on audio threads.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::{fs, io::Read, path::Path, process::Command};
#[derive(Clone, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
}
pub fn catalog() -> Vec<Entry> {
    serde_json::from_str(include_str!("library.json")).expect("Built-in voice catalog")
}
struct Temporary(std::path::PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        if fs::remove_file(&self.0).is_err() && self.0.exists() {
            if let Ok(exe) = std::env::current_exe() {
                let folder = exe.parent().unwrap().join("voice-downloads");
                if fs::create_dir_all(&folder).is_ok() {
                    let destination = folder.join(self.0.file_name().unwrap());
                    if fs::rename(&self.0, &destination).is_ok() {
                        return;
                    }
                    // The executable may be on a different drive from Windows temp.
                    if fs::copy(&self.0, &destination).is_ok() && fs::remove_file(&self.0).is_ok() {
                        return;
                    }
                }
            }
            crate::logging::error(format_args!(
                "Voice download cleanup could not remove {}",
                self.0.display()
            ));
        }
    }
}
pub fn download(id: &str) -> Result<String> {
    crate::logging::info(format_args!("Voice library installation requested: {id}"));
    let result = download_inner(id);
    match &result {
        Ok(_) => crate::logging::info(format_args!("Voice library installed: {id}")),
        Err(error) => crate::logging::error(format_args!(
            "Voice library installation failed ({id}): {error:#}"
        )),
    }
    result
}
fn download_inner(id: &str) -> Result<String> {
    let entry = catalog()
        .into_iter()
        .find(|v| v.id == id)
        .context("Unknown catalog voice")?;
    if crate::training::release::native_runtime().is_err() {
        let runtime = Entry { id: String::new(), name: String::new(), description: String::new(), url: "https://github.com/MitchCodes/PirateWave/releases/download/support-assets-v1/PirateWave-LLVC-Voice-v1-beta1.zip".into(), bytes: 144300430, sha256: "ee309c0cec085f53b1fae6383d3be38f6e5568f857e94d8e42db92e64e1591a5".into() };
        let archive = fetch(&runtime)?;
        super::pack::install(&archive.0)?;
        crate::training::release::native_runtime()?;
    }
    let temporary = fetch(&entry)?;
    install_archive(&temporary.0, &entry, &super::local::root())?;
    Ok(format!(
        "{} installed. Select it on a Voice Changer node.",
        entry.name
    ))
}
fn fetch(entry: &Entry) -> Result<Temporary> {
    ensure!(
        entry
            .url
            .starts_with("https://github.com/MitchCodes/PirateWave/releases/download/"),
        "Untrusted voice origin"
    );
    let temporary = Temporary(std::env::temp_dir().join(format!(
            "piratewave-voice-{}-{}.zip",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        )));
    // Create our unique destination before launching curl. Credentials are unnecessary.
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary.0)?;
    let mut command = Command::new("curl.exe");
    command
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "20",
            "--max-time",
            "600",
            "--max-filesize",
            &entry.bytes.to_string(),
            "--output",
        ])
        .arg(&temporary.0)
        .arg(&entry.url);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    ensure!(
        command.output()?.status.success(),
        "Voice download failed; check your connection and try again"
    );
    ensure!(
        fs::metadata(&temporary.0)?.len() == entry.bytes
            && crate::training::release::digest(&temporary.0)? == entry.sha256,
        "Voice download checksum mismatch"
    );
    Ok(temporary)
}

fn install_archive(archive: &Path, entry: &Entry, root: &Path) -> Result<()> {
    ensure!(
        entry.id.starts_with("local-")
            && entry.id.len() <= 128
            && entry
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "Invalid voice identity"
    );
    ensure!(
        fs::metadata(archive)?.len() == entry.bytes
            && crate::training::release::digest(archive)? == entry.sha256,
        "Voice archive checksum mismatch"
    );
    let mut zip = zip::ZipArchive::new(fs::File::open(archive)?)?;
    let allowed = [
        "voice.json",
        "validation.json",
        "llvc-stream.pt",
        "preview.wav",
        "NOTICES.txt",
        "provenance.json",
    ];
    ensure!(
        zip.len() == allowed.len(),
        "Unexpected voice archive contents"
    );
    let mut files = std::collections::BTreeMap::new();
    let mut total = 0;
    for i in 0..zip.len() {
        let file = zip.by_index(i)?;
        let name = file.name().to_owned();
        ensure!(
            allowed.contains(&name.as_str()) && !files.contains_key(&name),
            "Unexpected or duplicate voice file"
        );
        total += file.size();
        ensure!(total <= 128 * 1024 * 1024, "Voice archive is too large");
        let mut bytes = Vec::new();
        file.take(128 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        files.insert(name, bytes);
    }
    let voice: super::local::Voice = serde_json::from_slice(&files["voice.json"])?;
    ensure!(
        voice.schema == 1
            && voice.id == entry.id
            && voice.name == entry.name
            && voice.worker_sha256 == crate::training::release::NATIVE_WORKER_SHA256,
        "Incompatible voice metadata"
    );
    use sha2::{Digest, Sha256};
    ensure!(
        format!("{:x}", Sha256::digest(&files["llvc-stream.pt"])) == voice.model_sha256
            && format!("{:x}", Sha256::digest(&files["validation.json"])) == voice.report_sha256,
        "Voice model/report mismatch"
    );
    let report: serde_json::Value = serde_json::from_slice(&files["validation.json"])?;
    ensure!(
        report["waveform_and_state_pass"] == true
            && report["timing_pass"] == true
            && report["model_sha256"] == voice.model_sha256,
        "Voice has not passed native export checks"
    );
    fs::create_dir_all(root)?;
    let target = root.join(&entry.id);
    ensure!(
        !target.exists(),
        "Voice already installed; delete it before reinstalling"
    );
    let stage = root.join(format!(
        "staging-library-{}-{}",
        entry.id,
        std::process::id()
    ));
    fs::create_dir(&stage)?;
    let result = (|| {
        for (name, bytes) in files {
            fs::write(stage.join(name), bytes)?;
        }
        fs::rename(&stage, target)?;
        Ok(())
    })();
    if stage.exists() {
        let _ = fs::remove_dir_all(stage);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn fixture(extra: bool) -> (std::path::PathBuf, Entry) {
        use sha2::{Digest, Sha256};
        let root = std::env::temp_dir().join(format!(
            "piratewave-library-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let report = serde_json::to_vec(&serde_json::json!({"waveform_and_state_pass":true,"timing_pass":true,"model_sha256":format!("{:x}",Sha256::digest(b"model"))})).unwrap();
        let voice = super::super::local::Voice {
            schema: 1,
            id: "local-test-library".into(),
            name: "Test Captain".into(),
            model_sha256: format!("{:x}", Sha256::digest(b"model")),
            cpu_identifier: "test".into(),
            logical_processors: "1".into(),
            worker_sha256: crate::training::release::NATIVE_WORKER_SHA256.into(),
            report_sha256: format!("{:x}", Sha256::digest(&report)),
            numerical_seconds: 1.,
            beta: true,
        };
        let mut zip = zip::ZipWriter::new(fs::File::create(root.join("voice.zip")).unwrap());
        let files = [
            ("voice.json", serde_json::to_vec(&voice).unwrap()),
            ("validation.json", report),
            ("llvc-stream.pt", b"model".to_vec()),
            ("preview.wav", vec![]),
            ("NOTICES.txt", b"attribution".to_vec()),
            ("provenance.json", b"{}".to_vec()),
        ];
        for (name, bytes) in files {
            zip.start_file(name, zip::write::FileOptions::default())
                .unwrap();
            zip.write_all(&bytes).unwrap();
        }
        if extra {
            zip.start_file("../outside", zip::write::FileOptions::default())
                .unwrap();
            zip.write_all(b"bad").unwrap();
        }
        zip.finish().unwrap();
        let archive = root.join("voice.zip");
        let entry = Entry {
            id: voice.id,
            name: voice.name,
            description: "test".into(),
            url: String::new(),
            bytes: fs::metadata(&archive).unwrap().len(),
            sha256: crate::training::release::digest(&archive).unwrap(),
        };
        (root, entry)
    }
    #[test]
    fn verified_archive_installs_and_refuses_overwrite() {
        let (root, entry) = fixture(false);
        let target = root.join("installed");
        install_archive(&root.join("voice.zip"), &entry, &target).unwrap();
        assert_eq!(
            fs::read(target.join(&entry.id).join("llvc-stream.pt")).unwrap(),
            b"model"
        );
        assert!(install_archive(&root.join("voice.zip"), &entry, &target).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn malformed_and_tampered_archives_do_not_install() {
        for extra in [false, true] {
            let (root, mut entry) = fixture(extra);
            if !extra {
                entry.sha256 = "wrong".into();
            }
            assert!(
                install_archive(&root.join("voice.zip"), &entry, &root.join("installed")).is_err()
            );
            assert!(!root.join("installed").exists());
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn catalog_has_five_unique_pinned_model_downloads() {
        let entries = catalog();
        assert_eq!(entries.len(), 5);
        let mut ids = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        let mut hashes = std::collections::BTreeSet::new();
        for entry in entries {
            assert!(ids.insert(entry.id));
            assert!(names.insert(entry.name));
            assert!(hashes.insert(entry.sha256.clone()));
            assert_eq!(entry.sha256.len(), 64);
            assert!(entry.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(entry.bytes > 0 && entry.bytes < 128 * 1024 * 1024);
            assert!(entry.url.starts_with(
                "https://github.com/MitchCodes/PirateWave/releases/download/voices-v1/"
            ));
        }
    }
    #[test]
    fn download_rejects_non_github_origins() {
        let entry = Entry {
            id: String::new(),
            name: String::new(),
            description: String::new(),
            url: "http://example.com/voice.zip".into(),
            bytes: 1,
            sha256: String::new(),
        };
        assert!(fetch(&entry).is_err());
    }
    #[test]
    fn temporary_download_is_removed() {
        let (root, _) = fixture(false);
        drop(Temporary(root.join("voice.zip")));
        assert!(!root.join("voice.zip").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
#[cfg(test)]
mod release_tests {
    use super::*;
    #[test]
    #[ignore = "Downloads the published optional GitHub voice assets"]
    fn public_library_downloads_install_and_cleanup() {
        let entries = catalog();
        assert_eq!(entries.len(), 5);
        let root = std::env::temp_dir().join(format!(
            "piratewave-library-native-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for entry in entries {
            let archive = fetch(&entry).unwrap();
            let path = archive.0.clone();
            install_archive(&path, &entry, &root).unwrap();
            assert!(root.join(&entry.id).join("llvc-stream.pt").is_file());
            drop(archive);
            assert!(!path.exists());
        }
        fs::remove_dir_all(root).unwrap();
    }
}
