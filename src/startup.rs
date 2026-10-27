//! Per-user Windows sign-in registration. No installer or elevation required.
use anyhow::Result;

pub fn configure(enabled: bool) -> Result<()> {
    #[cfg(windows)]
    {
        let command = if enabled {
            Some(platform::command(&std::env::current_exe()?)?)
        } else {
            None
        };
        platform::write(platform::RUN_KEY, command.as_deref())
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
        anyhow::bail!("Start with Windows is only available on Windows");
    }
}

pub fn registered() -> Result<bool> {
    #[cfg(windows)]
    {
        Ok(platform::read(platform::RUN_KEY)?.is_some())
    }
    #[cfg(not(windows))]
    {
        Ok(false)
    }
}

/// Roll back Windows registration if saving preferences fails.
pub fn apply(
    previous: &piratewave_core::shortcuts::StartupSettings,
    next: &piratewave_core::shortcuts::StartupSettings,
    mut register: impl FnMut(bool) -> Result<()>,
    save: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let changed = previous.start_with_windows != next.start_with_windows;
    if changed {
        register(next.start_with_windows)?;
    }
    if let Err(error) = save() {
        if changed {
            if let Err(rollback) = register(previous.start_with_windows) {
                anyhow::bail!("Could not save settings: {error:#}. Could not restore Windows startup registration: {rollback:#}");
            }
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{os::windows::ffi::OsStrExt, path::Path, ptr};
    use windows_sys::Win32::{Foundation::ERROR_FILE_NOT_FOUND, System::Registry::*};
    pub(super) const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    fn check(status: u32) -> Result<()> {
        if status != 0 {
            return Err(std::io::Error::from_raw_os_error(status as i32).into());
        }
        Ok(())
    }
    pub(super) fn command(exe: &Path) -> Result<Vec<u16>> {
        let mut command = vec![b'"' as u16];
        command.extend(exe.as_os_str().encode_wide());
        command.push(b'"' as u16);
        anyhow::ensure!(command.len() <= 260, "Windows startup requires the executable path to fit within 258 characters. Move PirateWave to a shorter path first.");
        anyhow::ensure!(
            !command[1..command.len() - 1].contains(&(b'"' as u16)) && !command.contains(&0),
            "Invalid executable path"
        );
        command.push(0);
        Ok(command)
    }
    pub(super) fn read(path: &str) -> Result<Option<Vec<u16>>> {
        read_named(path, "PirateWave")
    }
    fn read_named(path: &str, name: &str) -> Result<Option<Vec<u16>>> {
        unsafe {
            let mut handle = ptr::null_mut();
            let status = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                wide(path).as_ptr(),
                0,
                KEY_QUERY_VALUE,
                &mut handle,
            );
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            check(status)?;
            let key = Key(handle);
            let mut size = 0;
            let status = RegQueryValueExW(
                key.0,
                wide(name).as_ptr(),
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut size,
            );
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            check(status)?;
            anyhow::ensure!(size <= 32768, "Invalid Windows startup value");
            let mut data = vec![0u16; (size as usize).div_ceil(2)];
            check(RegQueryValueExW(
                key.0,
                wide(name).as_ptr(),
                ptr::null(),
                ptr::null_mut(),
                data.as_mut_ptr().cast(),
                &mut size,
            ))?;
            Ok(Some(data))
        }
    }
    pub(super) fn write(path: &str, command: Option<&[u16]>) -> Result<()> {
        write_named(path, "PirateWave", command)
    }
    fn write_named(path: &str, name: &str, command: Option<&[u16]>) -> Result<()> {
        unsafe {
            let mut handle = ptr::null_mut();
            if command.is_some() {
                check(RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    wide(path).as_ptr(),
                    0,
                    ptr::null(),
                    0,
                    KEY_SET_VALUE,
                    ptr::null(),
                    &mut handle,
                    ptr::null_mut(),
                ))?;
            } else {
                let status = RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    wide(path).as_ptr(),
                    0,
                    KEY_SET_VALUE,
                    &mut handle,
                );
                if status == ERROR_FILE_NOT_FOUND {
                    return Ok(());
                }
                check(status)?;
            }
            let key = Key(handle);
            if let Some(command) = command {
                check(RegSetValueExW(
                    key.0,
                    wide(name).as_ptr(),
                    0,
                    REG_SZ,
                    command.as_ptr().cast(),
                    std::mem::size_of_val(command) as u32,
                ))
            } else {
                let status = RegDeleteValueW(key.0, wide(name).as_ptr());
                if status == ERROR_FILE_NOT_FOUND {
                    Ok(())
                } else {
                    check(status)
                }
            }
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn quotes_unicode_paths_and_roundtrips_in_isolated_registry_key() {
            let path = format!(r"Software\PirateWave-Startup-Test-{}", std::process::id());
            let command = command(Path::new(r"C:\Apps with spaces\音声\PirateWave.exe")).unwrap();
            assert_eq!(
                String::from_utf16_lossy(&command[..command.len() - 1]),
                "\"C:\\Apps with spaces\\音声\\PirateWave.exe\""
            );
            write(&path, Some(&command)).unwrap();
            assert_eq!(read(&path).unwrap(), Some(command));
            write(&path, None).unwrap();
            write(&path, None).unwrap();
            assert!(read(&path).unwrap().is_none());
            unsafe {
                check(RegDeleteKeyW(HKEY_CURRENT_USER, wide(&path).as_ptr())).unwrap();
            }
            assert!(command_for_long_path_is_rejected());
        }
        fn command_for_long_path_is_rejected() -> bool {
            command(Path::new(&"x".repeat(259))).is_err()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use piratewave_core::shortcuts::StartupSettings;
    #[test]
    fn old_settings_load_with_startup_disabled() {
        let settings: piratewave_core::shortcuts::AppSettings =
            serde_json::from_str(r#"{"version":1,"keybinds":{}}"#).unwrap();
        assert_eq!(settings.startup, StartupSettings::default());
    }
    #[test]
    fn saves_are_independent_and_registration_is_rolled_back_on_failure() {
        let old = StartupSettings::default();
        let next = StartupSettings {
            start_minimized: true,
            start_audio_automatically: true,
            ..old.clone()
        };
        apply(
            &old,
            &next,
            |_| panic!("Unchanged startup must not touch Windows"),
            || Ok(()),
        )
        .unwrap();
        let next = StartupSettings {
            start_with_windows: true,
            ..next
        };
        let mut calls = vec![];
        assert!(apply(
            &old,
            &next,
            |enabled| {
                calls.push(enabled);
                Ok(())
            },
            || anyhow::bail!("Disk unavailable")
        )
        .is_err());
        assert_eq!(calls, [true, false]);
        let mut saved = false;
        assert!(apply(
            &old,
            &next,
            |_| anyhow::bail!("Access denied"),
            || {
                saved = true;
                Ok(())
            }
        )
        .is_err());
        assert!(!saved);
    }
}
