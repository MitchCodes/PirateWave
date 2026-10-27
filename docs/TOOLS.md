# Tools

Open **Tools** in PirateWave's sidebar, search or filter the catalog, and open a tool's
workspace. Every workspace has an **All tools** button. Tool settings are separate
from project audio data; browsing, measuring and experimenting do not dirty the
project or change its routing/effects. No test audio starts just by opening a page.

## Workspaces

| Tool | Purpose |
| --- | --- |
| Sound Check | Native endpoint overview, microphone check, quiet-floor check and acoustic return measurement. |
| Signal Check | Five-second microphone test or continuous level/waveform display, peak, clipping and tonal energy. |
| Room Hush | Five-second quiet-input measurement and peak-to-floor span. |
| Spectral Lens | Log-frequency stereo spectrum, 2048/8192-point windows, peak hold, freeze and frequency/value cursor. |
| Listen Back | Independent microphone-to-headphones monitoring with adjustable gain. |
| Voice Sandbox | Eight character auditions using PirateWave's existing DSP, with live effect parameters. |
| Echo Clock | A real chirp-and-return test between a selected output and input. |
| Tone Forge | Frequency presets, 20 Hz–20 kHz adjustment, sine/square/triangle/sawtooth waveforms and level control. |
| Channel Compass | Left-only, right-only, center and matched/opposite-polarity speaker tests. |
| Device Atlas | Input/output identities, default-name matches, advertised formats and endpoint errors. |
| Clock Scope | Shared-mode endpoint rates/channels/formats alongside PirateWave's 48 kHz processing clock. |
| Buffer Budget | Custom frame counts, rate presets, stage counts and a comparison table. |
| Vocal Map | Interactive voice-frequency regions and a calibrated bell-EQ response preview. |
| Level Guide | Digital amplitude/dBFS exploration and separate approximate acoustic landmarks. |
| Key Canvas | Original geometric controller icons, colors, gradients, transparency, labels and PNG/ZIP export. |

## Audio lifecycle

Start a test explicitly. Choose devices in the tool's selectors; **Windows default**
resolves when the test starts. Refresh device choices after attaching hardware.
Device choices are disabled during an active test. Stop before changing endpoints.

**Stop tool audio**, leaving the workspace, application shutdown, and the shared
**Stop all sounds** action cancel test streams. Audio-stop actions also cancel tool
streams. Timed microphone/polarity checks end automatically; continuous tools end
after ten minutes. Tool streams and analysis belong to a separate worker; existing
engine callbacks gain no work when Tools is idle. Device loss and processor faults
stop the affected test and report an error. Stop/close work stays off the UI thread.

Use headphones for Listen Back and Voice Sandbox. The confirmation checkbox is
required before monitoring starts; removing confirmation stops monitoring. Tones
default to a low digital level and are capped at −6 dBFS. Physical playback volume
still depends on the device and its amplifier. Output gain does not calibrate SPL.
The voice sandbox uses DSP only; it does not install AI models, preserve formants,
clone voices, or add effects to the project.

## What measurements mean

- Levels are digital amplitudes relative to full scale. Stereo RMS uses separate
  channels; clipping counts frames containing an absolute sample of at least
  0.999. This is not true-peak metering, LUFS or acoustic SPL.
- Quiet floor is the power average of the quietest quarter of 20 ms windows,
  bounded to the last 30 seconds. Speak first and then stay quiet during Signal
  Check; stay quiet throughout Room Hush. The displayed peak-to-floor span is a
  descriptive comparison, not a calibrated SNR. Silence can read below −120 dBFS.
- Tool input bypasses PirateWave's project processors. OS/interface processing can
  remain active; PirateWave does not silently change device enhancements or gain.
- Spectrum uses a Hann window and takes the greater magnitude of the two channel
  transforms. Opposite-polarity stereo therefore remains visible. A bin-centered
  sine has calibrated amplitude; signals between bins spread over adjacent bins.
  Display frequency is capped by both 20 kHz and the input's Nyquist frequency.
- Echo Clock measures from enqueueing a known chirp to observing its matching
  input samples. It includes the tool's output reserve, Windows/device buffering,
  acoustic travel and input/worker delivery. It is a **separate test loop**, not a
  direct measurement of the project's graph, ASIO latency or converter delay.
  Results are rounded to milliseconds; callback/worker scheduling limits precision.
  A missing/weak match yields no value. Dropped samples invalidate the match.
  Use audible speakers, a nearby mic and a quiet room; stop unrelated playback and
  monitoring first. Headphones generally provide no usable acoustic return.
- Endpoint details are shared-mode defaults and advertised capabilities. They do
  not prove a converter's internal clock or the format an exclusive application
  negotiated. CPAL's endpoint identities are name-based; duplicate names make a
  default-name match ambiguous. Unknown buffer sizes are shown as unknown.
- Buffer Budget is arithmetic, not a hardware measurement. Vocal Map is a response
  preview, not a change to a rack. Acoustic landmarks in Level Guide are approximate
  and cannot be converted directly into digital meter readings.

Audio stays in bounded memory and is not saved or uploaded. Only explicit artwork
exports write files. Key Canvas exports 144, 288 or 576-pixel PNGs, or a ZIP with ten
matching action symbols. It reuses PirateWave's bundled UI font and original geometric
artwork; no PirateWaved artwork, logos or source code are included. Import icons into
controller software explicitly; existing controller profiles are never modified.

## Research and implementation choices

The reference catalog was reviewed at [the original tools catalog](https://patchd.studio/tools).
These are independent native implementations, with new names and PirateWave's existing
visual style. Functional references:

- Microphone checks: [mic test](https://patchd.studio/tools/mic-test),
  [quiet floor](https://patchd.studio/tools/mic-noise-floor),
  [spectrum](https://patchd.studio/tools/spectrum-analyzer),
  [monitor](https://patchd.studio/tools/mic-monitor),
  [voice effects](https://patchd.studio/tools/online-voice-changer).
- Setup: [diagnostics](https://patchd.studio/tools/audio-diagnostic),
  [return timing](https://patchd.studio/tools/audio-latency-test),
  [test tones](https://patchd.studio/tools/tone-generator),
  [stereo tests](https://patchd.studio/tools/stereo-channel-test),
  [device inspection](https://patchd.studio/tools/audio-device-inspector),
  [sample rates](https://patchd.studio/tools/sample-rate-checker).
- References: [buffer math](https://patchd.studio/tools/buffer-size-latency-calculator),
  [EQ regions](https://patchd.studio/tools/eq-frequency-chart),
  [decibel scales](https://patchd.studio/tools/db-loudness-reference).
- Controller artwork: [key icons](https://patchd.studio/tools/streamdeck-icons).

Webcam-frame, OBS-scene and Twitch-panel designers were omitted because they do not
fit PirateWave's audio/control workflow.

## Developer checks

`cargo test -p piratewave-desktop tools` covers calibrated levels/noise, anti-phase FFT,
chirp correlation and absent echoes, bounded stereo generators/fades, navigation,
project/engine isolation, EQ math and actual PNG/ZIP exports. Workspace tests,
formatting and Clippy remain the normal checks.

`PirateWave --tools-ui-smoke` creates native screenshots for the catalog and all tools,
including narrow layouts. It uses generated signal/endpoint fixtures, opens no
audio streams, never loads/saves the user's project or settings, and writes only
under `dist`. Physical microphone, acoustic echo and speaker wiring checks require
the user's hardware and are separate from generated fixtures.
