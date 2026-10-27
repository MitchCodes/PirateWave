# Recording node validation — 0.2.0-beta.4

> Historical validation record. Counts, versions, measurements, and limitations below describe the builds tested at that time; they are not a fresh qualification of every current build. See the [documentation index](README.md) for current setup guides.

Validated locally on Windows on 2026-10-21.

- `./build.ps1 -Verify`: formatting, workspace tests, Clippy with warnings denied, release executable, 32-bit plugin bridge and portable ZIP passed. 161 tests passed; seven opt-in tests were ignored by the standard suite.
- Core tests cover shared maximum histories, capture request timing, per-recorder start epochs, mixed contributors, switch gating, passthrough chains, port routing and backward-compatible project data.
- Desktop tests cover engine capture/playback/crop routing, recording-only clock pacing, asynchronous WAV/MP3 export and decoding, Unicode paths and overwrite, scoped shortcut cleanup, stale worker replies and saved armed-node startup. Recorder stop buttons and shortcuts, reset selection, both waveform boundary drags, and cancellation of deferred playback are covered by regression tests.
- Native UI smoke rendered 67 views, including the recording page, waveform/crop selection, node and slot shortcut pages, mixer, add menu and three routing ports. Waveform fixtures use synthetic audio.
- Native shortcut self-test registered and delivered 99 Windows mappings through WM_HOTKEY, including all three recording-node actions and all five slot actions. Deleted slots release keys for reuse. No user keystrokes were synthesized.
- `cargo run --release --locked -p piratewave-core --example recording_check`: four recorders with twenty 60-second slots share one 23,414,016-byte history; zero render allocations; measured 0.141% of the realtime CPU budget on this machine. This is a synthetic benchmark, not a latency guarantee.
- Native VST2/VST3 integration checks passed, including the 32-bit bridge. A simultaneous native UI/shortcut run verified the process-specific log fallback without stderr errors.
- Final ZIP integrity was checked; its three executable files match the portable directory, and PirateWave.exe matches the compiled release binary. The included recording guide contains the final software-clock behavior.

These checks use synthetic signal fixtures and native Windows facilities. They do not constitute a physical microphone unplug/replug or microphone recording test.

## Artifact

`dist/PirateWave-v0.2.0-beta.4-windows-x64.zip`

SHA-256: `91377a3c2802bafbbb83f0e51787251c759936187daaa4f64cabd25381529465`

Local diagnostic outputs are in the ignored `dist/` folder: `recording-fix-build.txt`, `recording-performance.txt`, `recording-fix-shortcuts.txt`, `ui-smoke-result.txt`, and `native-recording-*.png`.
