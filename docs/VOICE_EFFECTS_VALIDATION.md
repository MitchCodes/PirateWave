# Voice effects and recovery validation - 0.2.0-beta.2

> Historical validation record. Counts, versions, measurements, and limitations below describe the builds tested at that time; they are not a fresh qualification of every current build. See the [documentation index](README.md) for current setup guides.

Validated locally on Windows x64, 2026-10-21.

- Workspace tests: 131 passed; 7 opt-in tests excluded from the normal run (hardware/optional voice pack/DLL oracle prerequisites). Two of these opt-in checks were also run explicitly and passed, as described below.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- Formatting and diff whitespace checks: passed.
- Private hardware stream failure injection: microphone failure leaves the original output queue, graph allocation and soundboard playhead intact. Missing microphone retries fail harmlessly, then reconnect when the endpoint is made available to the test. Capture and output callbacks continue. An injected output failure reopens only that output and preserves graph/playback. No actual USB unplug or user project edits are performed.
- Native voice worker lifecycle: passed in 73.28 seconds, including termination of the test's own worker and automatic recovery without pressing Restart voice, plus disable/re-enable and shutdown cleanup.
- Native UI smoke: 54 views rendered successfully, including all ten effect cards and the separate Voice Change Effects picker. Category and Vocoder screenshots visually inspected; all ten effects are also selected through the UI in automated tests.
- DSP: finite output and clean reset at parameter minima/maxima across 8/44.1/48/96/384 kHz; exact irregular-block continuity; pitch octave-up/down spectral checks; distinct vocoder characters and speech-envelope decay; resonator period and tremolo amplitude checks. Existing graph, latency, bypass, persistence, processor and voice tests pass.
- Processing allocation harness: all ten voice effects make zero allocations. Local release timing as percent of one second of audio: Delay 0.02%, Vocoder 2.46%, Resonator 0.27%, Distortion 0.23%, Pitch 0.86%, Flanger 0.42%, Chorus 0.59%, Tremolo 0.15%, Stereo 0.16%, WahWah 0.39%. Timing is machine/load dependent.

The normal-run count is 66 core + 63 desktop + 2 VST2-host tests. Native worker and device-injection checks bring the distinct executed total to 133. Artifacts and detailed behavior: `dist/voice-effects-performance.txt`, `dist/ui-smoke-result.txt`, `dist/native-voice-change-effects.png`, `dist/native-vocoder.png` and `docs/RACK_PROCESSORS.md`.

Rust unwinding cannot recover native access violations in an in-process plugin or OS calls that hang. Recovery claims cover the implemented stream retries, built-in DSP guards, reported plugin failures and voice-worker restart paths, not every possible process crash.
