# Changelog

## Unreleased

- Add Cookbook recipes, managed autosaved workspaces, Mixer ordering, themes, audio tools, and effect-tuning displays.
- Add Stream Deck controls alongside Loupedeck, with shared actions and live feedback.
- Recover stalled application capture and disconnected outputs independently, preserving device selections; fix a Windows device-discovery allocation leak.
- Standardize PirateWave branding, `.piratewaveext` packages, SDK exports and controller identifiers; remove obsolete migration code. Controller assignments and extensions created with former identifiers must be recreated or rebuilt.
- Refresh public documentation and distinguish historical qualification from current setup instructions.

## 0.2.0-beta.10

- Add versioned native PirateWave extensions for custom multiport nodes and rack processors, with generated controls, shortcuts, Loupedeck actions/knobs and recording/export integration.
- Preserve unavailable extension settings, ports and bindings; use explicit fallback and offline state migration. Scan packages in isolated child processes and activate installed versions after restart.
- Ship a Rust/C SDK, tested example DLL/package, developer/AI documentation and allocation/integration coverage. Existing built-in and VST processing remain intact.


## 0.2.0-beta.9

- Keep inputs/sources and output buses in independently scrolling Mixer panes; move processors and utility nodes to a remembered, collapsible lower section with category filters.
- Expand Routing around saved node bounds in both directions; preserve access after resizing, support negative coordinates, Find node, Reset view and edge scrolling while dragging.
- Use compact workspace/node headers and grouped rack, VST, Voice Changer and ducking controls with expandable help. Opening editors no longer inserts missing default parameter values into projects.
- Add responsive-layout, navigation-without-audio-updates and settings-compatibility regressions plus native screenshot coverage.

## 0.2.0-beta.8

- Skip Auto Ducking level and speech detection when either input is disconnected, muted or silent. Settled undelayed outputs use direct pass-through copies.
- Preserve pending lookahead audio and smooth Hold/Release transitions; resume detection on fresh analysis windows without rendering allocations.
- Expand ducking regression and allocation benchmarks to cover inactive inputs and transitions.

## 0.2.0-beta.7

- Added Auto Ducking nodes with independent Priority/Ducked inputs and simultaneous Mixed, Priority and Ducked outputs.
- Added level and bundled RNNoise speech detection, stereo-linked attenuation, attack/hold/release, optional aligned lookahead, live meters and bypass.
- Added ducking keybinds and Loupedeck parameter knobs/actions, compatible project defaults, precise input-port routing and allocation-free processing checks.

## 0.2.0-beta.6

- Added recording-node effects racks for slot preview/send playback and cropped WAV/MP3 export, while retaining dry captures and live passthrough.
- Added whole-rack bypass and per-effect enable/disable/toggle controls and shortcuts, including Loupedeck control and deleted-effect shortcut cleanup.
- Isolated effect history per playback, synchronized live rack edits, and kept render-time processing allocation-free.

## 0.2.0-beta.5

- Added the optional Loupedeck / Logi plugin: all existing actions, soundboard and recording slots, volume/pan and built-in/VST parameter knobs, live state feedback and dynamic name-based targets.
- Added a user-restricted local control pipe, project-generation checks, bounded queues, automatic reconnect without action replay, and an integration enable/status control in Settings.
- Added reproducible `.lplug4` packaging, SDK/native transport validation and controller setup documentation.

## 0.2.0-beta.4

- Added recorder-level Stop playing recording controls and shortcut; selection edits and reset stop the slot on both playback outputs and cancel pending plays.

- Add Recording nodes with mixed input and separate live passthrough, preview and send output ports.
- Add persistent recording slots, lookback capture, waveform selection, preview/send playback, and cropped WAV/MP3 export through background file workers.
- Add per-node start/stop and five per-slot global shortcuts with stable identities and deletion cleanup.
- Share bounded rolling histories across downstream recorders and passthrough chains; snapshot on workers without allocations or file I/O in normal rendering.
- Treat active recording as a Voice Changer destination, and keep recorded desktop preview separate from live loopback-feedback validation.
- Recognize Windows sharing violations when selecting separate worker log files.

## 0.2.0-beta.3

- Move Automatic Voice Correction into Voice Change Effects.
- Add Switch nodes with one active connected output, a destination dropdown and per-output global shortcuts, with cleanup when routes or nodes are deleted.
- Add per-effect enable/disable/toggle shortcuts with stable identities, safe deletion and correct processor history after reordering.
- Add asynchronous rotating diagnostic logs, error/panic reporting and bounded audio-health summaries while keeping device callbacks and normal DSP free of log I/O.
- Add Automatic Gain Control for bidirectional speech leveling, with target RMS, boost/reduction limits and a noise-floor guard.

## 0.2.0-beta.2

- Recover microphone, output and Listen endpoint disconnects independently with automatic retries and stalled-callback detection. Preserve the graph and playing soundboard clips, and keep processing while all outputs are unavailable.
- Bypass and retry faulty built-in effects, reload reported plugin failures, automatically restart failed voice workers with backoff, and preserve the last valid setup on rejected live edits.
- Add the Voice Change Effects category: Delay, Vocoder (four characters), Resonator, Distortion, Pitch, Flanger, Chorus, Tremolo, Stereo and WahWah, with controls, persistence and beginner help.

## 0.2.0-beta.1

- Expose local CPU/CUDA Voice Creation as Beta with the current experimental quality. Add raw recording, audio import/review, optional verified tools, persistent supervised jobs, audio-start pause, monitoring comparisons, immutable custom CPU voices and storage guidance. Training no longer requires a separate Python installation. Custom models must pass short native waveform/state and CPU timing checks; long-run and hardware acceptance remain outstanding.
- Keep curated CPU/DirectML behavior and existing shortcuts. Custom voices use the shared trusted runtime and remain CPU-only until separately validated.

The entries below describe earlier investigation builds and their then-closed training gate.

- Add voice-creation recording guidance and training storage preferences/measurement. Retain a gated CPU/CUDA training/export feasibility prototype with resumable checkpoints and native state comparisons; user training and runtime downloads remain unavailable pending recipe, quality and portable-runtime qualification. See `docs/VOICE_TRAINING.md`.

## Development history before 0.2.0-beta.1

These entries record prototype work and its gates at that time; current feature availability is described above and in the user guides.

- Add embedded Python 3.12 CPU/CUDA training runtime candidates, speaker-separated evaluation, pitch-conditioned teacher comparisons and native model export checks. Preserve the user-training gate pending accepted voice quality, full application workflow and release qualification.

- Voice Changer release candidate: dedicated routing/mixer node, Voices installation page, monitor-only speech previews, recorded processor shortcuts, output level and beginner help. Native Rust/LibTorch CPU worker preserves the original LLVC kernels; 30-minute model equivalence passed with zero error. Bounded asynchronous queues, causal resampling, generation resets and process cleanup isolate conversion from other audio routes. Live CPU routing is enabled in normal launches after automated qualification; manual release acceptance remains pending. The optional hybrid DirectML profile uses the RTX 4070 SUPER for the transformer while preserving the sensitive CPU encoder/synthesis; Auto still prefers CPU.

- Extend voice qualification with per-chunk state checks, identical-state versus recurrent comparisons, operator tracing, 13/26 ms profiles, and native Rust reproductions of failing steps. One voice is now the release scope. ONNX CPU and DirectML fail state acceptance; their reports remain available alongside the native CPU implementation above.

- Add an isolated Rust/ONNX LLVC feasibility probe, pinned streaming exporter, reference-audio/state comparisons and CPU/DirectML benchmarks. The ONNX implementation remains a gated prototype; the separate native CPU path above supplies the one-voice candidate.

- Stereo Reverb processor with wet mix, decay, damping, pre-delay and width controls, plus a beginner tutorial in the effect card's Help popup. Preallocated delay networks and smoothed live adjustments reuse the existing routing, saved project settings and bypass behavior.

- Fix intermittent gaps/crackling in direct input-to-output routes by increasing packet/jitter reserves and scaling capture buffers with the device sample rate. Surface input underruns separately in diagnostics; add continuous-signal drift/jitter regression tests and a native virtual-device continuity check.

- Record shortcuts directly in application settings, node keybinds and soundboard pads, including modifier combinations; cancel/focus-loss handling and temporary global-shortcut suspension prevent recordings from triggering existing actions.

- General settings with independent Start with Windows, Start minimized and Start audio automatically preferences; per-user sign-in registration without an installer and safe handling of unavailable startup workspaces.

- Help buttons on every built-in effect card, with beginner tutorials, step-by-step starting settings, explanations of every control and troubleshooting tips in a scrollable popup.

- Windows Application Capture source with executable/window selection, optional exact-title matching, process-tree audio and automatic restart reconnection.
- Application routing, effects, mixer controls and source keybinds; native isolation/engine/reconnection checks and OBS REA findings.

- Seven new native rack processors: bidirectional Expander, De-esser, Limiter, RNNoise Noise Suppression, Invert Polarity, 3 Band Equalizer and Upward Compressor; existing Gain remains compatible.
- Named mode/channel/detection controls, saved parameters, numerical DSP regressions, processing allocation checks and native screenshots for every requested processor.

- Replaced the provisional voice detector and Hann crossfading with the DLL-derived acquisition/adaptive tracker and period-synchronous, 50-sample linear crossfade.
- Recovered input filters, head initialization/jumps, preparation delay and anti-alias recurrence; reduced nominal voice-correction latency to about 29 ms at 48 kHz.
- Read-only DLL component oracle and sample-level comparisons across pitch raising/lowering, transitions, glides and rejection fixtures; ordinary CI uses regenerated detector checkpoints.

- Native Automatic Voice Correction with key/scale, voice range, retune, Humanize, detune, transpose, correction strength and aligned wet/dry controls.
- Focused REA evidence for control/smoothing behavior, with recovered pitch detection/crossfading and explicit fidelity limits.
- Offline pitch/reference probes, no-allocation DSP check, synthetic-voice regressions, routed bypass checks and native UI smoke coverage.

- Automatic 32-bit VST2 hosting through a bundled x86 helper, fixing import of legacy DLLs such as Auto-Tune Evo.
- Explicit PE architecture checks, detailed Windows loader errors and an asInvoker helper manifest.
- Shared native VST2 adapter, bounded authenticated bridge IPC, processing timeouts and helper cleanup via Windows jobs.
- Build/package/run scripts and CI now include the x86 helper and 32-bit plugin regression checks.

- VST Plugins library with isolated scanning of 64-bit VST2 DLLs and VST3 files/bundles.
- VST Instrument sources and VST Effect processors integrated with routing, mixer controls and passthrough.
- Generic per-instance parameters and saved settings; instrument keyboard and audition note/velocity.
- Instrument source/note shortcuts and VST effect enable/disable/toggle shortcuts, all initially unbound.
- Native plugin fixtures, CI DSP checks, third-party probes and two-device WASAPI VST checks.
- Output-bus Listen checkbox and saved secondary playback device, with live toggling and feedback checks.
- Output-bus mute/unmute, volume up/down/reset, and listening shortcuts; source volume-reset shortcut.
- Listen failure isolation and duplicate-device suppression, verified against two virtual playback endpoints.

- Processor cards in Mixer with whole-chain Disable/passthrough and no channel controls.
- Right-click node removal directly from Mixer, including output buses.
- Generalized Settings page with customizable stop-sounds and optional start/stop/toggle audio shortcuts.
- Per-node Keybinds pages: 14 source actions, eight output-bus actions and three processor actions, all unbound by default.
- Shared global shortcut validation, transactional registration, and deleted-target cleanup.
- Backward-compatible project fields for node bindings/bypass and separate application settings persistence.
- Native event-delivery, transport, audio passthrough, and headless mouse interaction checks.

## 0.1.0

Initial native Rust version.

- Device mixer with physical/virtual inputs, output buses, and WASAPI desktop loopback.
- Gain, balance, mute, source solo, and live meters.
- Six built-in processors and a reusable effect/parameter interface.
- Directed acyclic routing editor with draggable nodes and feedback checks.
- File-based soundboard, global shortcuts, and polyphonic playback.
- Versioned JSON projects and audio diagnostics.
- Windows portable packaging and automated checks.

See the README for current scope and limitations.
