# Workflow implementation guide

See [the user guide](WORKFLOWS.md) for behavior and [validation evidence](WORKFLOW_VALIDATION.md) for qualification limits.

## Control plane and persistence

`piratewave-core::workflows` contains defaulted workspace collections for scenes, session configurations and rules. Empty workflows are omitted from JSON, and existing workspaces load with empty collections. Use `workflows::new_id()` for new references. Workspace-local references use stable IDs; controller selectors still use exact names and require one match.

`piratewave-core::actions::Action` provides Invoke, Set and bounded Batch requests. The desktop's `dispatch_typed`/`dispatch_action` path is shared by UI actions, global shortcuts, both controllers and automation. Absolute settings use the same scene-control validation and ordinary engine update path. Batch validation checks every target before dispatch; execution is ordered, with a 32-command/eight-level bound. Runtime operations are not a rollback transaction: a later device/playback failure does not undo already accepted work.

An operation context carries workspace generation, operation ID, origin and a fixed eight-entry causal chain. `Command::Perform` stamps engine requests; old-generation requests are rejected. Engine completion events, recording-slot tokens and take-file replies feed the workflow control worker. A reply accepting work is distinct from its completion event. Start recording is acknowledged after engine attachment; Stop waits for file finalization. Record captures and scene recalls advance an automation sequence only on their authoritative outcomes.

Runtime master-arm state, history arming, progress, UI selection, crop/preview and file-transfer queues are not persisted as armed/running state. Workspace changes cancel runtime work and invalidate queued automation epochs. New templates must also start with the master disarmed.

## Scene transitions

`Scene::capture` captures eligible scalar controls and built-in parameters. Effect-kind identities prevent a replacement processor reusing an ID from silently accepting an old scene. `Scene::apply` validates the full included target set against a temporary project before applying it. Exclusions are stored per control, including individual Auto Ducking parameters.

`scene_runtime::Transition::prepare` resolves node/effect indices, reserves parameter entries and prepares any resets outside rendering. An unchanged setting is omitted from the runtime steps. Recalling an identical scene must not dip or reset an effect tail.

At block boundaries, prepared steps update existing DSP instances. Gain interpolates in dB, pan linearly. `effects::scene_transition` is the central reviewed-parameter metadata: unreviewed or discrete parameters default to a short guarded fade. Review this policy when adding a built-in parameter; do not mark a value safe merely because it is numeric. The guarded fade is four 128-frame blocks (about 10.7 ms at 48 kHz). No second effect chain or steady-state lookahead is introduced.

Switch transitions retain the current destination weights when interrupted, including a temporary three-destination blend. Per-sample route ramps preserve destination-based selection, all connected destination input ports and downstream recording taps. Mute/solo masks preserve ongoing gain ramps. Listening fades use separate secondary-stream gain rather than fading the main bus. Existing voice-model activation/priming behavior remains separate from mixer interpolation.

A manual edit cancels only matching owned controls. Optimistic UI endpoints and control acknowledgments are merged so newer edits are not overwritten. Controller relative changes use engine-reported interpolated values. Structural graph changes cancel incompatible compiled transitions; plugin/voice state is never serialized into a scene.

## Capture points and histories

A capture point is `(node ID, output port, Raw/Processed stage)`. Raw source taps occur after instrument generation but before built-in source effects and mixer controls. Processed taps include bus output and distinct recorder, ducking and extension ports. Routing edits never change a capture point's identity by list position.

The engine's monotonic sample timeline uses 128-frame boundaries. All live tracks enter one packet per boundary. A retrospective snapshot freezes one common block interval; its start is the newest required origin among all selected tracks and its end is shared. Plugin/converter latency is preserved as heard.

`Recorder` merges requirements by capture point and retains the longest requested history. The 512 MiB ledger follows actual `Arc<History>` lifetimes, including pending snapshots and workspace changes. Validate prospective memory before replacing buffers; unshared obsolete buffers are released before new allocations. Rendering writes atomic, version-stamped blocks. Off-thread readers verify each block before and after copying. An overwritten/unavailable range aborts the whole capture rather than yielding independently aligned tracks.

`Graph::process_workflows` uses the original processing path when workflow taps are disabled. Raw copies and extra switch-route taps are enabled only when required. Capture mappings are rebuilt/remapped outside rendering. Removing a live capture point stops the take with an explicit fault.

## File writer and media lifecycle

`session_io` owns all file access, encoding, waveforms, recovery and export. Its live pool is preallocated on the file worker, with at most 32 tracks per packet. The pool is below 27 MiB; concurrent live/retrospective writer buffers and preview staging keep pending audio transfers under 32 MiB. A previous pool must release before another is allocated. Queue exhaustion stops the take, never blocks the render thread and never silently omits frames.

Float files use 48 kHz stereo IEEE float32. A 92-byte header reserves a ds64/JUNK chunk and contains fmt, fact and data chunks. Finalization switches RIFF to RF64 when necessary. Tests exercise the >4 GiB header boundary without allocating a multi-gigabyte test fixture.

Each take directory has an incomplete manifest during recording and a final `take.json` containing track identities, engine frame range, markers and discontinuities. Checkpoints preserve recoverable metadata. Recovery trims to the shortest physical track, repairs all headers and retains the original engine start frame. Export streams a common crop into a new directory; it does not rewrite the source audio.

Media stays outside workspace JSON. Preserve shared workspace references on deletion. Pending replies require the current generation and an existing target before attachment. Preview streams selected lanes through an explicitly selected device. Take browsing uses paged summaries and one opened waveform set, rather than caching all recordings.

## Automation worker

The deterministic core scheduler accepts an injected millisecond clock. `workflow_runtime::Worker` owns the bounded control queue and runs outside audio rendering. It shares immutable project snapshots, refreshing them after edits rather than cloning the workspace every level tick. It consumes 20 Hz summaries only when armed rules need levels/activity.

Level rules use transition detection, hysteresis, hold and cooldown. A rule has at most one running sequence; overlapping triggers increment a skipped counter. Steps execute in order with acknowledgments between operations. Wait stores a deadline and never sleeps on the audio thread. Missing targets remain unresolved with an error; transient operation failures stop that sequence and may retry on a later valid trigger.

Carry the initiating context through completions and generated actions. Settling-window level crossings retain that context through their hold so direct audio feedback can be detected. Attribution is conservative; rules should preferably measure an upstream signal rather than alter their own detector path. A repeated causal ID, depth overflow or queue saturation disarms the master and cancels sequences, leaving the current mix alone.

The worker bounds events to 256, rule steps to 32, causal depth to eight and history to 128 recent execution records. A separate epoch gate invalidates already queued steps immediately on disarm/reset. Restart, workspace changes and imports never replay actions.

## Extending and testing

When adding an action or condition:

1. Add a typed definition, persistence defaults and target validation in core.
2. Route execution through the shared dispatcher and preserve parameter validation/autosave.
3. Decide whether it is synchronous or requires an authoritative completion/failure.
4. Carry generation, operation and causal metadata through every worker handoff.
5. Add a core catalog entry and exact-name resolver. Add controller action-group definitions only when a new group is needed.
6. Test deletion, interruption, disconnect, disarm, stale replies and saturation, not just the successful callback.

Do not add render-thread allocation, locks, logging, file access, serialization, rule evaluation or device discovery. Foreign plugin internals remain outside the allocation guarantees of PirateWave's own workflow code. Keep control-only resources and cleanup on control/file workers.

Useful checks:

```powershell
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo run --release --locked -p piratewave-core --example workflow_check
cargo run --release --locked -p piratewave-core --example builtin_graph_check
cargo run --release --locked -p piratewave-core --example ducking_check
cargo test --bin piratewave-desktop sustained_stress:: --locked -- --ignored --nocapture
./scripts/build-loupedeck.ps1 -Verify -PirateWaveExe ./target/debug/piratewave-desktop.exe
./scripts/build-streamdeck.ps1 -Verify -PirateWaveExe ./target/debug/piratewave-desktop.exe
./target/debug/piratewave-desktop.exe --workflows-ui-smoke
```

The sustained test writes about 737 MB of temporary audio and cleans it up. Native UI smoke mode renders fixtures without starting audio or saving user workspaces/settings. Physical controller and real-device/overnight recording qualification must be recorded separately from these simulations.

See [release validation and hardware qualification limits](WORKFLOW_VALIDATION.md).
