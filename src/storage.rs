use anyhow::{Context, Result};
use piratewave_core::{effects::EffectRegistry, graph::validate_with_registry, project::Project};
use std::path::{Path, PathBuf};
pub fn default_path() -> PathBuf {
    directories::ProjectDirs::from("audio", "PirateWave", "PirateWave")
        .map(|d| d.config_dir().join("studio.json"))
        .unwrap_or_else(|| PathBuf::from("studio.json"))
}
pub fn load(path: &Path) -> Result<Project> {
    let result = load_inner(path);
    match &result {
        Ok(_) => crate::logging::info(format_args!("Storage load: {}", path.display())),
        Err(error) => crate::logging::error(format_args!(
            "Storage load failed for {}: {error:#}",
            path.display()
        )),
    }
    result
}
pub fn load_inner(path: &Path) -> Result<Project> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("Reading {}", path.display()))?;
    let project: Project = serde_json::from_str(&text)?;
    validate_with_registry(&project, &EffectRegistry::default()).map_err(anyhow::Error::msg)?;
    Ok(project)
}
pub fn settings_path() -> PathBuf {
    default_path().with_file_name("settings.json")
}
pub fn load_settings(path: &Path) -> Result<piratewave_core::shortcuts::AppSettings> {
    let result = load_settings_inner(path);
    match &result {
        Ok(_) => crate::logging::info(format_args!("Storage load_settings: {}", path.display())),
        Err(error) => crate::logging::error(format_args!(
            "Storage load_settings failed for {}: {error:#}",
            path.display()
        )),
    }
    result
}
pub fn load_settings_inner(path: &Path) -> Result<piratewave_core::shortcuts::AppSettings> {
    let settings: piratewave_core::shortcuts::AppSettings =
        serde_json::from_str(&std::fs::read_to_string(path)?)?;
    anyhow::ensure!(
        settings.version == 1,
        "Unsupported settings version {}",
        settings.version
    );
    anyhow::ensure!(
        settings.keybinds.values().all(|key| key.len() <= 256),
        "Shortcut text is too long"
    );
    anyhow::ensure!(
        settings.ui.utilities_height.is_finite()
            && (80.0..=2000.0).contains(&settings.ui.utilities_height),
        "Invalid utility pane height"
    );
    Ok(settings)
}
pub fn save_settings(
    path: &Path,
    settings: &piratewave_core::shortcuts::AppSettings,
) -> Result<()> {
    save_json(path, settings)
}
pub fn save(path: &Path, project: &Project) -> Result<()> {
    validate_with_registry(project, &EffectRegistry::default()).map_err(anyhow::Error::msg)?;
    save_json(path, project)
}
pub(crate) fn save_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let result = save_json_inner(path, value);
    match &result {
        Ok(_) => crate::logging::info(format_args!("Storage save_json: {}", path.display())),
        Err(error) => crate::logging::error(format_args!(
            "Storage save_json failed for {}: {error:#}",
            path.display()
        )),
    }
    result
}
pub(crate) fn save_json_inner(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let text = serde_json::to_string_pretty(value)?;
    let temp = path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    // On Windows std::fs::rename replaces an existing file on the same volume.
    std::fs::rename(&temp, path).with_context(|| format!("Saving {}", path.display()))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn global_settings_roundtrip_separately_from_project() {
        use piratewave_core::shortcuts::{AppAction, AppSettings};
        let path =
            std::env::temp_dir().join(format!("piratewave-settings-{}.json", std::process::id()));
        let mut settings = AppSettings {
            startup: piratewave_core::shortcuts::StartupSettings {
                start_with_windows: true,
                start_minimized: true,
                start_audio_automatically: true,
            },
            ..AppSettings::default()
        };
        settings
            .keybinds
            .insert(AppAction::StopSounds, "Ctrl+Shift+F8".into());
        settings
            .keybinds
            .insert(AppAction::ToggleAudio, "Ctrl+Alt+F9".into());
        save_settings(&path, &settings).unwrap();
        assert_eq!(load_settings(&path).unwrap(), settings);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn save_replaces_and_loads() {
        let path =
            std::env::temp_dir().join(format!("piratewave-project-{}.json", std::process::id()));
        let mut p = Project::default();
        save(&path, &p).unwrap();
        p.name = "Updated".into();
        save(&path, &p).unwrap();
        assert_eq!(load(&path).unwrap().name, "Updated");
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn rejects_future_project_versions() {
        let p = Project {
            version: 999,
            ..Project::default()
        };
        let path =
            std::env::temp_dir().join(format!("piratewave-invalid-{}.json", std::process::id()));
        assert!(save(&path, &p).is_err());
    }
}
