# Input and output audio formats

PirateWave can use different sample rates and sample formats for each microphone input, output bus, and Listen output. Open **Mixer**, choose a device, and expand **Audio format**.

Leave the sample rate on **Auto — device default** and sample format on **Auto — best supported** unless you have a reason to choose another supported format. With both on Auto, this follows the device's Windows shared-mode default. With an explicit rate, Auto format prefers the default encoding when supported at that rate, otherwise another supported encoding. The lists show combinations the audio backend reports as supported; changing the sample rate resets the sample-format choice to Auto. Use **Refresh formats** or Mixer **Refresh Devices** after changing Windows device settings.

The active-format line appears while audio is running. It reports the opened application stream, whether conversion is needed, and any fallback. It is not a claim about the DAC's physical bit depth. In shared mode Windows can perform additional conversions. PirateWave does not change Windows' global sound settings or take exclusive control.

If a requested format is unavailable, PirateWave temporarily uses the device default and retains your preference. If opening that stream also fails, the previous healthy stream continues during a live format edit and the change retries after a delay. Missing devices retain their assignments and preferences for reconnection.

## What changes and what stays the same

- Input/output format changes prepare a replacement in the background and replace only that stream. Other streams, effects, voice models, playback, and recording timelines continue. The affected device can have a brief interruption while buffers fill.
- The processing graph remains **48 kHz, stereo, 32-bit float**, in 128-frame blocks. A 96 kHz device does not make effects or recordings run at 96 kHz. Channel mapping is unchanged: mono input is duplicated, and mono output averages left/right.
- A 44.1 kHz input is converted once into the graph; a 96 kHz output is converted once after it. There are no new converters between ordinary effects or nodes.
- The first available primary output paces the engine. A 48 kHz pacing output transfers samples directly. Other device clocks remain synchronized even when their nominal rate is also 48 kHz. If the pacing device disappears, another output takes over; without outputs, the software timeline continues.
- Application Capture continues using its existing 48 kHz capture contract. Desktop loopback follows its playback endpoint. These sources do not expose misleading independent hardware-format controls.
- Recorder slots, multitrack takes, soundboard decoding, LLVC model conversion, plugins, scenes, automation, and controller assignments retain their existing contracts. Saved-file formats are separate from device formats.

## Quality and latency

Rate conversion and clock synchronization use one band-limited sinc stage per boundary, with shared timing for left/right. The filter and scratch buffers are prepared before processing. Matching synchronized rates use a direct path. Unrelated mixer edits preserve converter state.

Converters use 128 output-frame chunks and a 128-tap filter. The filter delay is approximately 64 input frames (1.33 ms with 48 kHz input), plus chunk staging of up to 128 output frames. These delays are separate from the existing approximately 43 ms input/output queue reserves and the Windows driver buffers; the reserves have not been increased. Bypassed direct transfer has no converter delay.

Integer samples are decoded once on input and quantized once on output. Output is checked for nonfinite samples and clamped after resampling to contain filter overshoot. 8- and 16-bit PCM outputs use rounded quantization with triangular dither; exact digital silence remains silent. Float32 output has no quantization/dither stage. Float64 streams still pass through the float32 graph.

Only sample encodings supported by the current backend are offered. Packed 24-bit PCM is not implemented. A 32-bit PCM stream/container must not be interpreted as proof of 32 valid hardware bits; the CPAL abstraction does not expose the endpoint's separate valid-bit count. Windows hardware precision remains a device/driver property.

## Troubleshooting

**A rate is missing:** Windows shared mode or the driver may not accept it. Leave Auto selected, or change the endpoint's format in Windows Sound settings and refresh. A rate shown for another device need not be available here.

**The active rate differs from the saved choice:** The choice is unavailable or failed to open. The status line explains the default fallback. Reconnecting the device or selecting a supported combination retries negotiation.

**Two 48 kHz devices still show synchronization:** Their physical clocks can differ slightly. Synchronization prevents their queues from slowly emptying or overflowing; equal rate labels alone do not prove equal clocks.

**A live change briefly interrupts one device:** Stream reopening and buffering cannot be guaranteed gapless. Primary output startup and teardown have short fades. Other graph paths are not restarted.

## Developer validation

`cargo test --workspace --all-targets --locked` covers format defaults/negotiation, stale-result rejection, sample encoding, exact direct transfer, stereo alignment, filter passband/alias rejection, queue starvation recovery, and simulated packet delivery with clock drift. Existing DSP/workflow/controller tests remain required.

`cargo run --release --example audio_format_check --locked` opens no hardware. It checks heap allocations, underruns, and CPU time for direct, adaptive equal-rate, and 44.1/48/96/192 kHz conversions. Preparation allocations are excluded intentionally. On the development PC, the initial run measured 0.09% of a core for direct transfer and 0.40–1.39% per converting stereo stream; results depend on hardware.

An optional silent native-output test is available with `cargo test --bin piratewave-desktop native_shared_output_formats_and_owner_shutdown -- --ignored --nocapture`. It opens only shared playback streams and does not change Windows settings. This is not a microphone, listening-quality, unplug/replug, or overnight hardware qualification.

Relevant implementation: `src/audio_io/{format,resample,encode,owner}.rs`, `src/engine_formats.rs`, and `src/ui/audio_format.rs`. Device formats are defaulted workspace fields; scenes do not capture them. One pending format replacement per session and a 32-entry UI capability cache bound background work/storage. Late results for changed/deleted targets are discarded, and stream destruction happens on the owning thread.

Development-machine native check: the default shared output reported 96 kHz with float32, unsigned 8-bit, signed 16-bit, and signed 32-bit streams. All four opened, delivered callbacks, and closed successfully with silence. Other nominal rates were tested in simulation; physical multi-device drift and overnight hot-plug qualification remain separate manual checks.
