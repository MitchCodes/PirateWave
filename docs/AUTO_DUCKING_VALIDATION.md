# Auto Ducking inactive-input fast paths — 0.2.0-beta.8

> Historical validation record. Counts, versions, measurements, and limitations below describe the builds tested at that time; they are not a fresh qualification of every current build. See the [documentation index](README.md) for current setup guides.

Validated locally on Windows on 2026-10-22.

- `./build.ps1 -Verify -WithLoupedeck` passed formatting, all 190 workspace tests (seven opt-in tests ignored), Clippy with warnings denied, optimized app/32-bit bridge builds and official plugin package verification.
- New regression tests use detector-sample and inference-frame counters to verify zero level/speech detector work for disconnected or entirely silent input combinations. Output samples pass through unchanged when no delay or transition is required. Disconnected-port presence is cached when building the graph; connected inputs share one sanitation/peak/silence scan.
- Tests cover mute/disconnect/reconnect, fresh analysis windows after idle, control-plane model maintenance, pending lookahead audio, and aligned Priority/Mixed output. Existing ducking timing, graph preservation and three-output recording tests remain passing.
- Hold/Release continues when Priority disappears while background audio remains audible. Settled unity gain snaps to exact pass-through; bypass still advances Hold in bulk, preventing stale activity after later settings changes. No audible background means obsolete attenuation can clear without a click. Delayed background tails finish before detection is paused.
- The expanded `ducking_check` passed all 32 combinations of Level/Speech detection, lookahead off/50 ms, active/silent/disconnected inputs and transitions with zero graph-rendering allocations. Final Speech-mode undelayed measurements: both active 1.277%, Priority-only disconnected 0.078%, Ducked-only disconnected 0.077%, one silent input 0.093%, neither connected 0.062% of the realtime CPU budget. These are synthetic whole-graph measurements, not physical-device latency guarantees.
- Native smoke rendered 74 views. Loupedeck SDK/native-pipe integration passed, including ducking knobs/reset/mode/bypass and reconnect without replay. No physical controller or microphone was driven.
- Final ZIP integrity and byte comparisons verify the compiled desktop executable, worker, 32-bit bridge, verified plugin and updated public ducking guide.

Fast-path eligibility uses actual zero-valued mixed blocks, not the voice-activity probability: nonzero noise can still require normal detection. Copies, signal metering and routing are necessary; enabled delay and outstanding gain transitions retain their required work. Speech inference and rolling RMS work are skipped whenever there is no Priority signal or no remaining background signal to duck. Existing output latency is not increased.

Artifact: `dist/PirateWave-v0.2.0-beta.8-windows-x64.zip`

SHA-256: `5ca3bf5f62cb91d3f0eb8defb3f3c97abbb7826585284960e8d2a12933269cf4`

Detailed local reports are in ignored `dist/ducking-fast-*` files.

---

# Auto Ducking validation — 0.2.0-beta.7

Validated locally on Windows on 2026-10-22.

- `./build.ps1 -Verify -WithLoupedeck` passed formatting, 186 workspace tests (seven opt-in tests ignored), Clippy with warnings denied, optimized desktop/32-bit bridge builds, and official Logi package verification. The plugin package version is 1.1.0.
- Core tests cover independent input mixing, simultaneous Mixed/Priority/Ducked outputs, stereo linking, fixed duck amount, exponential attack/release, Hold, level/probability hysteresis, silence, bypass and live changes. Lookahead tests cover 0/10/50 ms alignment, ring wrap, refill/fade, and state preservation across unrelated graph rebuilds.
- Persistence tests cover saved nodes and port identities, legacy default input/output fields, invalid parameters, duplicate routes, cycles and precise disconnection. Switch tests verify all routes to the selected destination remain active and controller catalogs deduplicate multiport destinations.
- Downstream recording tests verify three distinct processed histories and the sample-wise Mixed = Priority + Ducked identity. Existing recording capture, playback, crop and export tests remain passing.
- The real RNNoise integration test recognizes the bundled attributed LibriSpeech fixture, leaves Priority audio unchanged and does not duck on silence. Deterministic tests verify probability decisions, the level guard and fallback/recovery state. These checks do not guarantee classification accuracy for every voice, noise or music source.
- UI tests create the node, toggle bypass, show Speech controls, dispatch all node actions, remove the node safely, connect both destination inputs through real hit targets and delete only one selected wire.
- Native UI smoke rendered 74 views. The new Level/Speech settings, input/output labels, keybinds and add menu screenshots were visually inspected. An existing screenshot-fixture bug that inserted effects into the first sorted node was corrected to use only the selected compatible rack node.
- Native Windows shortcut validation registered and delivered 108 mappings through WM_HOTKEY, including all three ducking actions. Removing the ducking node released its OS key for reuse. No user keystrokes were synthesized.
- SDK integration passed real action construction and simulated ducking knob, default-reset, discrete Speech-mode and bypass callbacks against the native user-scoped pipe. Existing dynamic-slot, generation, reconnect and no-replay checks passed. Physical controller input remains separately testable.
- Native VST2/VST3 DSP integration and the 32-bit bridge passed after the connection-model change.
- `ducking_check` measured zero graph-rendering allocations in Level/Speech modes, each with lookahead disabled and at 50 ms. Measured realtime CPU budgets were 0.316%/0.307% for Level and 1.637%/1.408% for Speech respectively. `recording_check` retained one 23,414,016-byte shared history with zero render allocations (0.208% realtime CPU budget). These synthetic measurements are not hardware latency guarantees.
- ZIP integrity and byte comparisons verified executable/worker/bridge/plugin copies and all updated public guides. PirateWave.exe matches the compiled release binary. Publication-script tests and a local release dry run passed; nothing was published or pushed.

Artifact: `dist/PirateWave-v0.2.0-beta.7-windows-x64.zip`

SHA-256: `3a135241203f24dff04dce37313521ab5ebf582527874a9884064a6e00f360cc`

The portable archive includes `PirateWave.lplug4` and `docs/AUTO_DUCKING.md`. Detailed reports and screenshots remain in ignored `dist/ducking-*`, `dist/native-ducking-*` and `dist/ui-smoke-result.txt`. No microphone, external audio endpoint or physical Loupedeck was driven by these checks.
