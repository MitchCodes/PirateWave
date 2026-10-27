# Rack processors

Select a node in **Effects rack**, then **Add processor** (or the top **Add effect** button). All processors can be reordered, disabled or removed. Settings are saved with the project, and parameter changes preserve processing history. Gain was already available and retains its existing ID/settings.

| Processor | Behavior and main controls |
| --- | --- |
| Reverb | Diffuse stereo room reflections with Wet mix (0–100%), Decay (0.1–10 s), Damping (0–100%), Pre-delay (0–200 ms) and Stereo width (0–100%). Start around 10–15% wet and 0.5–1 s decay for speech. Help on the card explains every control and placement. |
| Expander | Direction: **Downward** reduces levels below Threshold; **Upward** boosts levels above it. Ratio, Attack, Release, Maximum change, RMS/Peak detection and Output gain. |
| De-esser | Detects sibilance above Sibilance cutoff. Threshold/Ratio control attenuation, capped by Maximum reduction. Split band applies a dynamic high shelf; Broadband turns down the entire signal. Attack/Release shape the response. |
| Limiter | Instant, stereo-linked **sample-peak** ceiling and adjustable Release. Place last to catch peaks from preceding processors. This is not an oversampled true-peak limiter. |
| Noise Suppression | RNNoise speech denoising via the pure Rust nnnoiseless library. Strength blends processed and delayed original audio. Approximately **20 ms** additional latency at 48 kHz. |
| Invert Polarity | Flips the sign of Both, Left or Right channel. Disable to restore normal polarity. |
| Gain | Existing transparent level adjustment, -60 to +24 dB. No duplicate processor was added. |
| 3 Band Equalizer | Independent Low/Mid/High gains, -24 to +24 dB. Low shelf: 200 Hz; mid bell: 1 kHz, Q approximately 0.707; high shelf: 5 kHz. All-zero gains are flat. |
| Upward Compressor | Raises levels **below** Threshold by the selected Ratio, capped by Maximum boost. Attack, Release, RMS/Peak detection and Output gain. Quieter speech becomes louder; room noise can become louder too. |

## Gate versus suppression

A **Noise gate** turns down the whole signal when the input falls below its threshold. Once speech opens it, background noise comes through with the speech. A downward **Expander** does this more gradually with an adjustable ratio.

**Noise Suppression** analyzes the audio to attenuate unwanted noise even while speech is present. The RNNoise model is intended for speech, not a full music mix; it can attenuate intentional non-speech sounds or color the voice. It does not guarantee removal of all clicks, other people or loud background sounds. This distinction also appears in the [OBS filters guide](https://obsproject.com/kb/filters-guide) and [OBS noise suppression documentation](https://obsproject.com/kb/noise-suppression-filter).

A practical starting chain for a microphone is **Noise Suppression → 3 Band Equalizer → De-esser → Compressor → Expander → Limiter**. Add Gain if level adjustment is needed. Upward compression can replace or complement ordinary compression when quieter words need lifting. Tune thresholds for your own mic level rather than treating the defaults as universal presets.

Upward expansion and upward compression differ: with threshold -30 dB and ratio 2:1, upward expansion takes an above-threshold -20 dB signal toward -10 dB; upward compression takes a below-threshold -40 dB signal toward -35 dB. Maximum change/boost limits bound these adjustments.

## Backend and verification

`piratewave-core::reverb` supplies an algorithmic stereo reverb through the same factory interface. Eight damped feedback combs per channel feed four all-pass diffusion stages, with different left/right reflection times, fractional pre-delay and wet-only stereo width. Decay sets the undamped feedback's approximate -60 dB time; damping shortens high-frequency decay. The dry path has no added delay; wet reflections begin after the chosen pre-delay plus the room's earliest reflection (about 30 ms). Parameters and feedback gains smooth over about 20 ms during live changes. Delay buffers are preallocated at the processor sample rate; processing and reset do not allocate. Bypass/reset clears the room tail. Existing projects require no migration.

`piratewave-core::rack_processors` registers seven new factories through the existing extensible AudioEffect/EffectFactory interface. It supplies named categorical choices to the rack for direction, RMS/Peak detection, de-esser processing and polarity channels. No project schema migration is needed. Existing source/processor/output routes, bypass and chain ordering apply unchanged.

Dynamics and limiting share stereo gain to avoid channel balance shifts. EQ uses RBJ biquads with persistent filter histories; the de-esser uses a high-pass sidechain and a dynamic high shelf or linked broadband attenuation. No look-ahead delay is added by those processors.

RNNoise works in 480-sample blocks at 48 kHz, independently per stereo channel. The adapter buffers arbitrary graph block sizes and provides streaming linear conversion for other supported core rates; PirateWave's normal engine is 48 kHz. Dry/wet paths account for framing/overlap delay. Parallel routing paths do not receive automatic latency compensation. RNNoise neural/filter histories are fully reconstructed on reset during control-plane graph reconfiguration; ordinary process calls reuse allocated memory. FFT plans are warmed at construction on the processing thread.

These are PirateWave implementations with OBS-inspired controls; they are not claimed to reproduce every OBS backend or preset. No OBS code was copied.

Numerical regressions cover directional dynamics curves, bounded upward boost, limiter transient ceilings/recovery and stereo ratios, exact polarity, flat/EQ band responses, frequency-selective de-essing, stationary-noise reduction, noise-suppression dry latency, reset, parameter validation, project serialization, block-size independence and finite output across core sample rates. An offline release harness checks processing allocations and CPU timing for every requested processor:

```powershell
cargo test -p piratewave-core rack_processors
cargo run --release -p piratewave-core --example rack_processors_check
```

Native smoke screenshots include every requested rack processor. RNNoise/nnnoiseless and transitive dependency notices are generated into the portable package by the existing build script.

## Voice Change Effects

Use **Add processor > Voice Change Effects** or the categorized effect picker:

| Effect | Controls / character |
| --- | --- |
| Delay | Time (1-1000 ms), feedback, wet/dry. Existing delay projects stay compatible. |
| Vocoder | Twelve speech-envelope bands; Classic saw, Robot, Chord, Pulse; carrier pitch, release, wet/dry. |
| Resonator | Tuned feedback comb; resonance pitch, feedback, damping, wet/dry. |
| Distortion | Warm saturation, Hard clip, Foldback; drive, output trim, wet/dry. |
| Pitch | Fixed +/-24 semitone voice transposition, fractional shifts and wet/dry. Shares the tested voice-correction detector, phase-aligned shifter and antialias filters with note snapping disabled. About 29 ms latency at 48 kHz; no formant preservation. |
| Flanger | Short stereo swept delay; rate, depth, signed feedback, wet/dry. |
| Chorus | Two modulated delay voices; rate, depth, wet/dry. |
| Tremolo | Sine or Square volume modulation; rate, depth. |
| Stereo | Mid/side width, optional right-channel Haas delay, balance. Check mono compatibility. |
| WahWah | Swept resonant band-pass; rate, base frequency, octave depth, resonance, wet/dry. |

Every processor includes a beginner Help guide and persists through the existing project format. Processing buffers are preallocated. Tests cover parameter extremes at 8/44.1/48/96/384 kHz, reset, irregular blocks, octave transposition, vocoder characters/envelope decay, resonator tuning and tremolo depth. The allocation harness includes all ten effects. These are native PirateWave effects, not claimed copies of Voicemod presets; no new Voicemod inspection was needed.

## Recovery behavior

Capture, main output and Listen endpoints have separate callback/error health tracking. The engine checks and retries missing endpoints every 500 ms; a driver that silently stops callbacks is detected after two seconds. Reopening an endpoint rebuilds its queue/resampler while preserving the graph, soundboard playheads and other streams. Missing sources feed silence. When no outputs are available, processing continues on a paced software clock until an output returns. Device selection and the saved project remain unchanged. An explicit Stop cancels the session and its retries.

A Rust effect panic or nonfinite output bypasses only that effect, preserving the dry block. The control plane attempts reset every two seconds. Reported plugin failures, Rust processing panics and invalid plugin output similarly preserve dry audio; failed plugin instances reload with saved parameters and cleared MIDI notes. Unavailable plugin loads retry with a capped backoff. Voice worker failures retry the same configured backend with a 1-30 second backoff; user stop/configuration changes cancel a pending retry, and a missing pack still requires installation. Invalid live edits leave the previous valid setup running.

This handles recoverable stream, DSP and worker faults. Native access violations in in-process plugins, process termination and OS/driver failures that hang a call cannot be recovered by Rust unwinding. PirateWave does not claim to recover every possible crash. Hardware validation injects errors into private test streams; it does not physically unplug the user's microphone.

## Switch routing and effect shortcuts (0.2.0-beta.3)

Add a **Switch** from **Add audio node**, then connect its output port to each candidate processor or output bus in Routing. The switch mixes its incoming routes and forwards the result to exactly one connected destination. Choose that destination using **Active output** on the mixer card or the switch's Effects rack page. The first connected output is selected automatically; removing the active route selects the first remaining route, or silence when none remain. Existing branches can also receive audio from other independently connected sources.

The switch's **Keybinds** page has an **Activate <destination>** action for each connected output. These bindings disappear and unregister when the route or destination is removed. Switches do not host effects themselves: connect them to effect processors to choose a voice chain.

An effect processor's **Keybinds** page (also reachable with **Effect keybinds** in the rack) includes **Enable effect**, **Disable effect**, and **Toggle effect** for every effect instance. These work independently of the node's whole-chain bypass. Record or type a key and choose **Apply keybinds**. Shortcuts use stable effect identities, including when two effects have the same type. Reordering preserves both the correct effect's DSP history and its bindings. Removing an effect or node unregisters its keys; deleted identities ignore stale events. Existing projects receive identities when loaded, without losing effect settings. Automatic Voice Correction now appears in the **Voice Change Effects** submenu.

## Automatic Gain Control

The existing Compressor only reduces loud input; Upward Compressor boosts quiet input. Neither provides a bidirectional target-level controller on its own. **Automatic Gain Control** is a dedicated linked-stereo RMS controller that raises quiet speech and lowers loud speech toward a Target RMS level, within separate boost and reduction limits. Below Noise floor it returns toward unity instead of increasing gain on room noise. Reduce gain time and Raise gain time control how quickly levels adapt. It changes audio inside PirateWave, not the Windows microphone hardware volume. Place it after cleanup and before creative effects; add a limiter afterward for peaks. Processing is allocation-free, and no audio samples are logged.

## Application logs

**Settings > Audio & diagnostics** displays the actual log path. Normally it is the project configuration folder's `logs/piratewave.log` (on Windows, `%APPDATA%/PirateWave/PirateWave/config/logs/piratewave.log`, depending on the platform directory provider). Use the path shown by the app as authoritative. Concurrent scanners or diagnostics use a process-specific file if another process already owns `piratewave.log`.

Logs include UTC timestamps, severity and PID. Startup/shutdown, project/settings reads and writes, shortcut registration failures, audio session setup, device failures/recovery, periodic audio-health summaries, plugin/voice-worker errors, recovered DSP faults and Rust panics are recorded. Error handling does not throw away a working audio session just because the logger cannot start. The logger has a bounded 1,024-event queue and a dedicated file-writer thread; producers never wait for disk, and queue saturation reports dropped events. Normal device callbacks and per-sample DSP do not format logs or access files. Files flush every second, on errors and at orderly shutdown. Each writer rotates its file at 5 MiB and keeps three backups. Logs contain diagnostic messages and file paths, not microphone samples or recorded keystrokes. A forced process kill can lose buffered entries.

## Tuning displays

Effect racks now include comparison snapshots, defaults, logarithmic frequency/time sliders, effect-specific guides and an optional live before/after spectrum and waveform. See [Effect tuning](EFFECT_TUNING.md) for what each display measures and when analysis runs.
