# Backend architecture and adding effects

## Ownership and threads

```text
egui / native Windows event loop
    | bounded commands: start, stop, update project, play, stop voices
    v
audio worker: owns device streams, compiled graph, and processor instances
    ^                                  |
    | bounded stereo queues            | bounded stereo queues
CPAL input callbacks                   v
                                  CPAL output callbacks

asset loader thread -> decoded Arc<Vec<[f32; 2]>> -> GUI -> play command
audio worker -> bounded status queue -> GUI meters and diagnostics
```

Input callbacks convert hardware samples to stereo frames and push to bounded queues. Output callbacks resample and copy queued stereo frames into the device format. Their normal sample paths do not acquire a GUI mutex, decode files, allocate, or execute effect code. Error callbacks may allocate error strings.

The worker renders when an output queue needs frames and uses a software clock when no outputs are available. It reads/resamples input queues, adds voices, processes 128-frame graph blocks, and enqueues each output bus. Input and output reserves are approximately 43 ms each (2,048 frames at 48 kHz); input reserve and capacity scale with the capture sample rate. These reserves cover packet-based capture/output delivery and worker scheduling jitter. Starting a capture reader with only 256 frames allowed it to empty between Windows packets and insert brief silence into an otherwise clean direct route. Small occupancy-based rate corrections compensate for independent hardware clocks. Diagnostics separately count input starvation after priming, output underflow callbacks, and dropped queue frames. Initial input buffering is not counted as starvation. Callback progress, stream errors, and asynchronous endpoint discovery drive independent stream recovery. Unavailable assignments remain saved while the rest of the graph continues; see [Audio recovery](AUDIO_RECOVERY.md).

Validation, project cloning, graph compilation, processor creation, and configuration changes occur between processing blocks on the worker. This is buffered soft real time. A future low-latency engine could prepare graph changes on a control thread and swap them at block boundaries.

## Core types

Application Capture adds one shared COM/WASAPI process-loopback worker per selected process ID, with independent subscriber queues for duplicate sources, feeding the existing bounded input queues and source graph. Its saved target contains an executable path and optional exact window-title requirement; runtime PIDs are resolved again after application restarts. Capture errors use a separate status queue and preserve other audio streams. See [implementation and evidence](APPLICATION_CAPTURE.md).

- `Project`: versioned data; nodes, connections, and sound-pad metadata.
- `Node`: stable ID, kind, gain, balance, mute, solo, effect specs, processor bypass, typed keybinds, and canvas position.
- `Graph`: topological order, routing edges, preallocated buffers, and stateful processors.
- `AudioEffect`: mutable DSP instance owned by the worker.
- `EffectFactory`: stable ID, display metadata, parameter schema, and constructor.
- `EffectSpec`: project data identifying a processor, enabled state, and parameter values.

Validation rejects duplicate/missing nodes, unknown effects/parameters, invalid numbers, invalid graph direction, and cycles before compilation. Limits: 64 nodes, 16 effects per node, and 128 total effect instances. Endpoint-aware validation also rejects digital feedback through loopback capture.

Each block initializes mixing accumulators and assigns source blocks, then visits nodes in topological order. Fully overwritten source and unused auxiliary buffers avoid redundant clears. Effects run in listed order; balance/gain/mute/solo apply after the chain. Nodes sum into their downstream buffers. Outputs scrub non-finite values and hard clip to [-1, 1]; this is overload protection, not a mastering limiter. Peaks are measured before output clipping.

Processor nodes ignore legacy channel mute/solo/gain fields and remain balanced. Voice Changer applies its dedicated output-level setting; other processors use unity gain. `effects_disabled` bypasses their whole chain, producing true passthrough without rewriting individual `EffectSpec.enabled` values. Changing this bypass state resets processor histories, so re-enabling a delay does not release stale buffered audio. Source/output behavior is unchanged. Both added node fields use serde defaults to load existing project files.

## Voice conversion

Voice Changer uses the graph's external-processor callback, which returns whether
a host handled the node. Unhandled enabled voice nodes produce silence, including
core-only rendering. Explicit bypass passes the original input. Graph validation
permits one active voice runtime, including mutually exclusive voice nodes on
direct Switch branches. Control-plane selection follows the switch destination;
changing the selected voice invalidates earlier conversion results. A shared converted node buffer fans out to every
destination without additional inference.

The render-side adapter exchanges preallocated 128-frame blocks through bounded
queues with a supervisor. The supervisor owns pack verification, native child
lifetime, continuous resampling and IPC. The optional Rust/LibTorch child owns
model warm-up and inference. No model load, native session destruction, inference
or thread join occurs on the render worker. Generation/sequence tags reject
stale speech; a 250 ms age limit bounds recovery.

Auto and CPU use the original LibTorch worker. The optional hybrid DirectML
worker retains LibTorch encoder/projection/synthesis kernels and offloads the
transformer decoder through an ONNX Runtime session with CPU fallback disabled.
It uses the same graph queues and IPC, so backend changes restart a generation.
Pack manifests independently gate GPU qualification. Render and supervisor
threads register with Windows MMCSS during audio work; node diagnostics retain
maximum inference/IPC wait and render-call gap. See VOICE_RUNTIME_DECISION.md
for numerical sensitivity, backend evidence and the qualified hardware profile.

See [the voice guide](VOICE_CHANGER.md), [runtime decision](VOICE_RUNTIME_DECISION.md)
and [validation record](VOICE_CHANGER_VALIDATION.json) for qualification, explicit
passthrough, fail-silent behavior and the unavailable DirectML backend.

## Shortcut actions and application settings

`shortcuts.rs` defines platform-independent `AppAction`, `NodeAction`, `ShortcutAction`, and `AppSettings` types. `NodeAction::for_kind` is the shared supported-action catalog used by the UI, registry, and project validation; `NodeAction::apply` is the same mutation path used by native events and tested numerically through the audio graph. Sources have 14 actions, VST instruments have 17, processors/VST effects have three, and output buses have eight; all node bindings default to unbound.

Application bindings live in a separate versioned settings file; node bindings live on their project nodes. Sound-pad `Sound.shortcut` data and its Soundboard editing surface are unchanged. New bindings start empty except the existing default stop-sounds key.

The desktop shortcut manager compiles one canonical binding plan for all three scopes, catches aliases/conflicts before OS calls, registers additions before releasing old keys, and rolls additions back if registration fails. Deleted target IDs are pruned even when another binding is invalid. Project loads clear stale registrations before installing the new project's map.

Pressed native events dispatch typed actions. Transport actions enter the engine command queue: StartIfStopped is idempotent and Toggle checks the worker's actual session rather than UI telemetry. Reconfigure never starts a stopped engine; ordinary node updates are queued even during startup to avoid losing a key press. Stop drops the session and all voices. The UI has explicit Apply/Revert controls for new settings and node keybind pages.

Ordinary control changes preserve processor histories. Structural updates build a replacement graph and transfer compatible histories by node ID and effect position/type. Reordering or replacing effects can reset affected state. Bypass resets a processor; bypassed delays do not continue a wet tail. Gain/pan/mute/solo transitions ramp over one block.

## Add a new processor

Run the complete stereo-width example without desktop dependencies:

```powershell
cargo run -p piratewave-core --example custom_effect
```

1. Add a Rust type holding DSP state. Allocate buffers in its constructor.
2. Implement `AudioEffect::set_parameter`, `process`, and `reset`.
3. Describe stable parameter IDs, ranges, defaults, labels, and units with `ParameterDescriptor`.
4. Register an `EffectFactory` in `EffectRegistry::default()` with a unique stable ID.
5. Add numerical tests for signal response, reset/bypass, and boundary values.

The desktop uses this registry, so your processor appears in both Add processor menus without UI edits. Scalar parameters receive sliders. Projects store the stable ID and parameter map. Missing values use defaults; invalid or unknown values fail validation. Keep IDs stable when changing labels.

The registry validates ranges, defaults, duplicate parameter/effect IDs, and sample rates. Custom `set_parameter` must accept every value allowed by its descriptors. `process` and `reset` must remain bounded, must not panic, and must not do I/O, locking, or per-block allocations. Use preallocated buffers for delay and separate per-channel histories.

Built-in processors remain linked Rust code. VST nodes use the injectable Graph::process_with_plugins hook, called in topological order before built-in effects and channel gain. Metadata-only validation never loads external binaries.

## Desktop adapters

- `plugins/`: VST2/VST3 adapters, isolated scanner, shared VST3 module lifetime and live per-node instances.
- `piratewave-vst2-host`: shared native VST2 adapter, bounded PE architecture detection and versioned bridge protocol.
- `piratewave-vst-bridge`: 32-bit helper using that same adapter; no GUI or audio-device dependencies.
- `ui/plugins.rs`: plugin library, generic parameter editor and instrument keyboard.
- `audio_io.rs`: discovery, sample conversion, WASAPI loopback, queue readers, counters.
- `engine.rs`: session lifetime, voice playback, commands, buffered execution, telemetry.
- `assets.rs`: asynchronous decoding/conversion to 48 kHz stereo.
- `hotkeys.rs`: global registration and sound-pad mapping, owned by the native event-loop thread.
- `ui/keybinds.rs`: generalized settings, per-node shortcut editors, and typed event dispatch.
- `storage.rs`: validated JSON load and temporary-file/rename save.
- `windows_audio.rs`: read-only endpoint mute/volume diagnostics.
- `ui.rs` / `ui/mixer.rs`: desktop controls; canvas positions never affect DSP.

Audio starts stopped and never follows a project load automatically. Streams are dropped on stop/exit. No input samples are written to disk.

## Tests and future work

Tests cover DSP, mixing, clipping, balance, graph rejection, processor state preservation/passthrough, switching ramps, old-project migration, settings persistence, every node action, decoding/resampling, queue recovery, and project saves. Headless egui mouse tests exercise Mixer Keybinds navigation, Disable, removal of every node kind, and editable-field visibility. Optional native diagnostics exercise WASAPI devices, measured loopback signals, all shortcut mappings, authoritative transport commands, and fifteen rendered views.

Future work: band-limited adaptive resampling, timestamped latency measurement, precompiled graph swaps, endpoint-ID persistence, and plugin latency compensation. Application Capture, a rack limiter, and independent device reconnection are implemented. A new virtual audio driver is a separate Windows driver-distribution project.


### Output listening

`Node.listen_enabled` and `Node.listen_device` default to false and system default when reading older v1 projects. Only output buses expose them. `Session` owns secondary Listen streams separately from primary outputs. Live project updates reconcile changed Listen destinations, preserving primary streams, DSP history and active voices. Each secondary receives the same post-processing frames as its bus primary, with its own queue, resampling and clock correction. An assigned, opened primary is required. Destinations identical to that bus primary are deduplicated after resolving the default alias; separate buses may still share a device intentionally.

Listen errors use per-stream queues: a failed secondary is removed while primary playback continues. Failed destinations are remembered so ordinary fader updates do not repeatedly reopen them; toggle off/on or change destination to retry. Default aliases are resolved at startup or live update; refresh/restart audio after changing system defaults to reopen primary endpoints. Feedback validation includes active Listen destinations as well as primary outputs. Listen properties and node bindings persist in the project; global transport bindings remain in application settings.


### 32-bit VST2 bridge

Before loading a VST2 DLL, PirateWave validates its PE header and selects native x64 or the x86 helper. The shared adapter preserves Windows loader errors instead of collapsing them to InvalidPath. Each bridged node owns a helper, authenticated localhost connection, preallocated binary audio/event buffers and pending parameter changes. Only loopback addresses are used; a random per-instance token authenticates the child before any plugin is loaded. Frames and parameter changes travel in bounded, versioned little-endian packets. The engine's worker waits for each processed block (not a device callback); timeout/disconnection becomes a reported plugin failure and the normal fallback.

The helper is built for i686-pc-windows-msvc, has an explicit asInvoker manifest, and is renamed piratewave-vst-bridge32.exe beside both development and packaged executables. Its Windows job uses kill-on-close, so a killed scanner or host does not leave child plugins running. Native x64 VST2 and the x86 helper share one implementation; fixture tests verify both paths, parameter changes, routing, notes and saved projects. The DLL supplied during diagnosis was PE machine 0x14c / optional-header magic 0x10b, explaining the original x64 loader failure; no plugin binary changes were required.

## Automatic Voice Correction

`piratewave-core::voice_correction` registers a built-in effect through the existing factory interface. It preallocates analysis, audio and timestamped control rings; analyzes one linked stereo pitch; and applies a dual-read-head shifter with the recovered 1,280-sample preparation delay and period-based read-head movement. Retune/Humanize ratio smoothing follows recovered binary control laws. Categorical key/scale/voice choices are exported for the effects rack. History survives control-plane graph updates and clears on bypass. Parallel graph branches have no automatic latency compensation. See [evidence, DSP design and limitations](AUTOMATIC_VOICE_CORRECTION.md).
