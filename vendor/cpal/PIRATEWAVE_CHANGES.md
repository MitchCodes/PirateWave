# CPAL 0.15.3 local fix

Source: crates.io cpal 0.15.3 (Apache-2.0, see LICENSE).
The functional change is limited to `src/host/wasapi/device.rs::is_format_supported`.
Two equivalent Rust lint cleanups in that file use `u32::MAX` and an explicit
elided `MutexGuard` lifetime.

Initialize the closest-format out-pointer to null and use the existing
`WaveFormatExPtr` RAII owner to free the Windows-allocated closest match on
all return paths. Preserve format filtering, HRESULT interpretation, supported
format results, stream configuration, and rendering behavior.

Windows requires callers to free this allocation with CoTaskMemFree:
https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-isformatsupported

The old format-enumeration path leaked approximately 85 KB per discovery pass
on the investigated computer. Background recovery repeatedly exercises it.
The repository includes an opt-in native discovery memory soak regression.
