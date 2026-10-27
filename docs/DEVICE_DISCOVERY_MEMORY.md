# Device discovery memory leak — 2026-10-26

## Observed live process

The running PirateWave process initially used 4,487,557,120 bytes of working-set
memory and 7,396,167,680 bytes of private committed memory. Private memory kept
growing while its existing audio session continued. Read-only Windows memory-map
inspection showed that the bulk was committed private writable memory, rather
than mapped files. No debugger attachment, suspension, injected code, or process
termination was used. The original process later exited independently; the recent
Windows Application log contained no corresponding PirateWave crash event.

## Cause and correction

The recovery watcher periodically calls `audio_io::devices()`. CPAL 0.15.3's
`HostTrait::input_devices` / `output_devices` filter endpoints by enumerating
supported formats. Its WASAPI `is_format_supported` calls `IAudioClient::IsFormatSupported`
with an output pointer for the closest format. Windows allocates that format on
`S_FALSE`; the previous code did not release it. Repeated scans therefore leaked
many small native allocations even with no playback or UI.

[Microsoft's API contract](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-isformatsupported)
requires releasing these buffers with `CoTaskMemFree`.

PirateWave now pins a locally maintained copy of CPAL 0.15.3 under `vendor/cpal`.
The narrowly scoped change starts the out-pointer at null and owns it with the
existing `WaveFormatExPtr` RAII destructor before interpreting the result. Exact
matches, rejected formats, invalidated devices, and other error paths keep their
previous return behavior. The polling interval, reconnect behavior, stream
formats, DSP, and audio callbacks are unchanged. Apache-2.0 licensing and the local
modification notice are included in generated distribution notices.

## Reproduction and measurements

An isolated native test performs 201 discovery passes using the same production
functions, without opening audio streams or changing endpoint settings.

| Private committed bytes | Original CPAL | Fixed CPAL |
|---|---:|---:|
| After first pass | 2,846,720 | 2,846,720 |
| After 25 further passes | 5,140,480 | 2,920,448 |
| After 100 further passes | 11,575,296 | 3,162,112 |
| After 200 further passes | 20,058,112 | 3,084,288 |

The original retained about 84 KiB per pass. The fixed run stabilized after small
initialization/cache fluctuations; its final private memory was only 160 KiB above
the 25-pass warmup sample. Handles were stable after warmup. This establishes and
fixes a reproducible leak consistent with prolonged growth. It is not an overnight
soak of every UI, driver, plugin, and audio configuration, nor proof that every
byte in the original process came from this one defect.

Run the regression on Windows with a stable device inventory:

```powershell
cargo test -p piratewave-desktop native_device_discovery_memory_soak -- --ignored --nocapture
```

It prints memory/handle samples and fails if retained private memory grows more
than 2 MiB or handles more than 32 between passes 25 and 200. It is opt-in because
it queries installed drivers for several minutes; it opens no playback/capture
streams. Normal workspace tests, formatting, and Clippy also pass.

The running old executable cannot retroactively free allocations it has lost.
Use the rebuilt executable on the next launch. Device-format diagnostic output
is compared against the original build before delivery.
