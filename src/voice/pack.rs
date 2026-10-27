use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Deserialize, Serialize)]
pub struct FileSpec {
    pub sha256: String,
    pub bytes: u64,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Manifest {
    pub schema: u32,
    pub pack: String,
    pub name: String,
    pub model_sha256: String,
    pub cpu_identifier: String,
    #[serde(default)]
    pub cpu_name: String,
    #[serde(default)]
    pub logical_processors: usize,
    pub runtime: String,
    pub hop: usize,
    #[serde(default)]
    pub release_qualified: bool,
    #[serde(default)]
    pub automated_qualified: bool,
    #[serde(default)]
    pub directml_qualified: bool,
    pub files: BTreeMap<String, FileSpec>,
}
pub fn manifest() -> Manifest {
    serde_json::from_str(include_str!("manifest.json")).expect("Bundled voice manifest")
}
/// Manual release acceptance is tracked separately from live Beta availability.
pub fn require_live_profile(manifest: &Manifest) -> Result<()> {
    ensure!(
        manifest.automated_qualified,
        "Live conversion is unavailable: this voice pack has not passed automated CPU qualification."
    );
    Ok(())
}
pub fn require_backend(
    manifest: &Manifest,
    backend: piratewave_core::voice::VoiceBackend,
) -> Result<()> {
    require_live_profile(manifest)?;
    ensure!(
        backend != piratewave_core::voice::VoiceBackend::DirectMl || manifest.directml_qualified,
        "DirectML profile is unavailable: install the qualified GPU voice pack or choose CPU / Auto."
    );
    Ok(())
}
fn root() -> PathBuf {
    crate::storage::default_path().with_file_name("voice-packs")
}
pub fn locate() -> Result<PathBuf> {
    locate_at(&root())
}
fn locate_at(base: &Path) -> Result<PathBuf> {
    let mut bytes = vec![];
    fs::File::open(base.join("active.json"))
        .context("Voice pack is missing. Install LLVC voice on the Voices page.")?
        .take(1025)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024, "Invalid voice pack location");
    let name: String = serde_json::from_slice(&bytes)?;
    ensure!(
        !name.is_empty()
            && name.len() <= 128
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "Invalid voice pack location"
    );
    Ok(base.join(name))
}
fn digest(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn verify(path: &Path) -> Result<()> {
    let expected = manifest();
    ensure!(
        expected.schema == 1
            && expected.pack == piratewave_core::voice::LLVC_PACK
            && expected.hop == 416
            && !expected.files.is_empty(),
        "Unsupported pack contract"
    );
    for (name, spec) in &expected.files {
        let target = path.join(name);
        ensure!(
            fs::metadata(&target).with_context(|| format!("Voice pack is missing or outdated: {name}. Reinstall the matching pack on the Voices page."))?.len() == spec.bytes && digest(&target)? == spec.sha256,
            "Voice pack checksum failed: {name}. Reinstall the matching pack on the Voices page."
        );
    }
    Ok(())
}
pub fn load() -> Result<PathBuf> {
    let path = locate()?;
    verify(&path)?;
    Ok(path)
}
/// Custom voices use the trusted CPU libraries, independently of optional GPU assets.
pub const CPU_RUNTIME_FILES: &[&str] = &[
    "torch_cpu.dll",
    "torch.dll",
    "torch_global_deps.dll",
    "c10.dll",
    "asmjit.dll",
    "fbgemm.dll",
    "libiomp5md.dll",
    "libiompstubs5md.dll",
    "shm.dll",
    "uv.dll",
    "msvcp140.dll",
    "vcruntime140.dll",
    "vcruntime140_1.dll",
];
pub fn load_cpu_runtime() -> Result<PathBuf> {
    let path = locate()?;
    let expected = manifest();
    for &name in CPU_RUNTIME_FILES {
        let spec = expected
            .files
            .get(name)
            .with_context(|| format!("CPU runtime contract missing {name}"))?;
        let file = path.join(name);
        ensure!(
            fs::metadata(&file)
                .with_context(|| format!("CPU runtime missing {name}"))?
                .len()
                == spec.bytes
                && digest(&file)? == spec.sha256,
            "CPU runtime checksum mismatch: {name}. Install the matching voice pack."
        );
    }
    Ok(path)
}
pub fn load_cpu_voice() -> Result<PathBuf> {
    let path = load_cpu_runtime()?;
    let expected = manifest();
    for name in [
        "PatchVoiceWorker.exe",
        "llvc-stream.pt",
        "tensor-contract.json",
        "model-config.json",
    ] {
        let spec = &expected.files[name];
        let file = path.join(name);
        ensure!(
            fs::metadata(&file)
                .with_context(|| format!("CPU voice file missing: {name}"))?
                .len()
                == spec.bytes
                && digest(&file)? == spec.sha256,
            "CPU voice checksum mismatch: {name}"
        );
    }
    Ok(path)
}
pub fn install(archive: &Path) -> Result<PathBuf> {
    install_at(archive, &root())
}
fn install_at(archive: &Path, base: &Path) -> Result<PathBuf> {
    let expected = manifest();
    let mut zip = zip::ZipArchive::new(fs::File::open(archive)?)?;
    ensure!(
        zip.len() == expected.files.len() + 1,
        "Unexpected voice pack contents"
    );
    let supplied: Manifest = serde_json::from_reader(zip.by_name("manifest.json")?.take(65537))?;
    ensure!(
        serde_json::to_vec(&supplied)? == serde_json::to_vec(&expected)?,
        "This pack does not match this PirateWave build"
    );
    if let Ok(existing) = locate_at(base) {
        if verify(&existing).is_ok() {
            return Ok(existing);
        }
    }
    for name in expected.files.keys() {
        ensure!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                && name != "."
                && name != "..",
            "Invalid manifest path"
        );
    }
    fs::create_dir_all(base)?;
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let name = format!("llvc-{nonce}");
    let target = base.join(&name);
    fs::create_dir(&target)?;
    let result = (|| -> Result<()> {
        for (name, spec) in &expected.files {
            let entry = zip.by_name(name)?;
            ensure!(entry.size() == spec.bytes, "Voice file size mismatch");
            let mut output = fs::File::create(target.join(name))?;
            let copied = std::io::copy(&mut entry.take(spec.bytes + 1), &mut output)?;
            ensure!(copied == spec.bytes, "Truncated voice file");
            output.flush()?;
        }
        verify(&target)?;
        // Publish only a fully verified installation. Existing directories are kept
        // intact, including an older/corrupt installation, until a cleanup is requested.
        crate::storage::save_json(&base.join("active.json"), &name)?;
        Ok(())
    })();
    if let Err(error) = result {
        // Clean only known files in the new directory. Never recursively delete
        // or modify an older installation.
        for file in expected.files.keys() {
            let _ = fs::remove_file(target.join(file));
        }
        let _ = fs::remove_dir(&target);
        return Err(error);
    }
    Ok(target)
}
#[cfg(test)]
pub fn cpu_matches() -> bool {
    let expected = manifest();
    std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_default() == expected.cpu_identifier
        && !expected.cpu_name.is_empty()
        && cpu_name() == expected.cpu_name
        && std::env::var("NUMBER_OF_PROCESSORS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            == Some(expected.logical_processors)
}
#[cfg(windows)]
pub(super) fn cpu_name() -> String {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    let key: Vec<u16> = "HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0\0"
        .encode_utf16()
        .collect();
    let value: Vec<u16> = "ProcessorNameString\0".encode_utf16().collect();
    let mut buffer = [0_u16; 512];
    let mut bytes = std::mem::size_of_val(&buffer) as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if result != 0 {
        return String::new();
    }
    let end = buffer.iter().position(|v| *v == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end]).trim().into()
}
#[cfg(not(windows))]
pub(super) fn cpu_name() -> String {
    String::new()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires the CPU on which this optional voice profile was qualified"]
    fn native_qualification_cpu_identity() {
        assert!(
            cpu_matches(),
            "Native CPU identity differs from the embedded qualification profile"
        );
    }
    fn sandbox() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "piratewave-voice-pack-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        path
    }
    #[test]
    fn malformed_pack_never_publishes_and_pointer_cannot_escape() {
        let base = sandbox();
        let archive = base.join("bad.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        let mut supplied = manifest();
        supplied.pack = "wrong-pack".into();
        zip.start_file("manifest.json", zip::write::FileOptions::default())
            .unwrap();
        zip.write_all(&serde_json::to_vec(&supplied).unwrap())
            .unwrap();
        for name in supplied.files.keys() {
            zip.start_file(name, zip::write::FileOptions::default())
                .unwrap();
        }
        drop(zip.finish().unwrap());
        assert!(install_at(&archive, &base).is_err());
        assert!(!base.join("active.json").exists());
        fs::write(base.join("active.json"), "\"../elsewhere\"").unwrap();
        assert!(locate_at(&base).is_err());
        fs::remove_file(base.join("active.json")).unwrap();
        fs::remove_file(archive).unwrap();
        fs::remove_dir(base).unwrap();
    }
    #[test]
    #[ignore = "Requires the built optional archive; writes only to a new temporary sandbox"]
    fn native_pack_installation() {
        let archive = PathBuf::from(
            std::env::var("PIRATEWAVE_VOICE_TEST_ARCHIVE")
                .expect("Set PIRATEWAVE_VOICE_TEST_ARCHIVE"),
        );
        let base = sandbox();
        let first = install_at(&archive, &base).unwrap();
        verify(&first).unwrap();
        assert_eq!(locate_at(&base).unwrap(), first);
        assert_eq!(install_at(&archive, &base).unwrap(), first);
        fs::write(first.join("preview.wav"), b"corrupt").unwrap();
        assert!(verify(&first).is_err());
        let second = install_at(&archive, &base).unwrap();
        assert_ne!(first, second);
        assert_eq!(locate_at(&base).unwrap(), second);
        verify(&second).unwrap();
        for directory in [first, second] {
            assert_eq!(directory.parent(), Some(base.as_path()));
            for name in manifest().files.keys() {
                fs::remove_file(directory.join(name)).unwrap();
            }
            fs::remove_dir(directory).unwrap();
        }
        fs::remove_file(base.join("active.json")).unwrap();
        fs::remove_dir(base).unwrap();
    }
}
