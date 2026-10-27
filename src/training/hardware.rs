//! Read the installed NVIDIA driver, without a toolkit or training runtime.
use anyhow::{ensure, Context, Result};

#[cfg(windows)]
pub fn cuda_devices() -> Result<String> {
    let system = std::path::PathBuf::from(
        std::env::var_os("SystemRoot").context("Windows directory unavailable")?,
    )
    .join("System32/nvcuda.dll");
    // Use the absolute system driver path, never a DLL from the project/workspace.
    unsafe {
        let library =
            libloading::Library::new(system).context("NVIDIA CUDA driver is not installed")?;
        let init: libloading::Symbol<unsafe extern "system" fn(u32) -> i32> =
            library.get(b"cuInit\0")?;
        let version: libloading::Symbol<unsafe extern "system" fn(*mut i32) -> i32> =
            library.get(b"cuDriverGetVersion\0")?;
        let count: libloading::Symbol<unsafe extern "system" fn(*mut i32) -> i32> =
            library.get(b"cuDeviceGetCount\0")?;
        let device: libloading::Symbol<unsafe extern "system" fn(*mut i32, i32) -> i32> =
            library.get(b"cuDeviceGet\0")?;
        let name: libloading::Symbol<unsafe extern "system" fn(*mut i8, i32, i32) -> i32> =
            library.get(b"cuDeviceGetName\0")?;
        let memory: libloading::Symbol<unsafe extern "system" fn(*mut usize, i32) -> i32> =
            library.get(b"cuDeviceTotalMem_v2\0")?;
        let status = init(0);
        ensure!(
            status == 0,
            "NVIDIA driver initialization failed (CUDA error {status})"
        );
        let mut driver = 0;
        ensure!(
            version(&mut driver) == 0,
            "CUDA driver version query failed"
        );
        let mut total = 0;
        ensure!(
            count(&mut total) == 0 && total > 0,
            "No CUDA-capable devices are available"
        );
        let mut devices = Vec::new();
        for ordinal in 0..total.min(32) {
            let mut dev = 0;
            let mut text = [0i8; 256];
            let mut bytes = 0usize;
            ensure!(
                device(&mut dev, ordinal) == 0
                    && name(text.as_mut_ptr(), text.len() as i32, dev) == 0
                    && memory(&mut bytes, dev) == 0,
                "CUDA device query failed"
            );
            text[255] = 0;
            devices.push(format!(
                "{} ({} VRAM)",
                std::ffi::CStr::from_ptr(text.as_ptr()).to_string_lossy(),
                super::bytes(bytes as u64)
            ));
        }
        Ok(format!("CUDA driver API {}.{}: {}. The bundled PyTorch worker checks compatibility again before training. Other NVIDIA hardware remains unverified by this Beta's end-to-end tests.", driver / 1000, (driver % 1000) / 10, devices.join(", ")))
    }
}

#[cfg(not(windows))]
pub fn cuda_devices() -> Result<String> {
    anyhow::bail!("This training runtime targets Windows x64")
}

pub fn check() -> super::release::Task {
    super::release::Task::spawn(|_| {
        Ok(format!("CPU: {} logical processors; optional CPU tools required. {}", std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1), match cuda_devices() {
            Ok(info) => info,
            Err(error) => format!("CUDA unavailable: {error:#}. CPU remains a separate choice; no automatic fallback."),
        }))
    })
}
