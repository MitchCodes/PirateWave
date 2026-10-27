# PirateWave extension validation — 0.2.0-beta.10

> Historical validation record. Counts, versions, measurements, and limitations below describe the builds tested at that time; they are not a fresh qualification of every current build. See the [documentation index](README.md) for current setup guides.

Validated locally on Windows on 2026-10-22.

- `./build.ps1 -Verify -WithLoupedeck` passed formatting, 216 workspace tests (seven opt-in tests ignored), Clippy with warnings denied, optimized desktop/32-bit bridge builds, native extension integration and official Logi package verification. `PirateWave.lplug4` package version is 1.3.0; PirateWave extension ABI is 1 and SDK crate version is 0.1.0.
- The beginner `examples/gain-plugin` contributes a gain node and rack processor. Its exported-ABI test verifies stereo gain for both types; its release DLL passes native package scanning. The refreshed archive includes its complete source and walkthrough in the portable SDK.
- The real example DLL contributes a stereo-width processor and a two-input/three-output node. Native checks verify non-default stereo-to-mono processing, independent input/output routing, separate recording taps, project round trips, missing-plugin silence/pass-through, typed actions/knob/reset controls, cross-project names with different numeric IDs, duplicate/missing-target rejection, independent recording/export instances, and explicit state migration.
- Installation is tested in a disposable settings root, including selected-version persistence, exact DLL bytes and rejection of overwriting an installed version. Unit tests reject traversal, case-colliding names, unsafe Windows paths, invalid ABI/latency/parameters/ports/actions/state, and SDK callback panics. Existing versions and user settings were not installed/modified by validation.
- Regression tests cover port-schema changes without resolving new IDs in the old graph, local faults when a plugin rejects configuration, NaN-output silence, declared node bypass maps and explicit dry export when a missing effect is disabled or its rack bypassed. Project load/save performs metadata-only validation; native instances are not constructed by storage.
- Native Windows shortcut validation registered/delivered 117 mappings through WM_HOTKEY, including custom extension actions and removal cleanup. No user keystrokes were synthesized.
- Native UI smoke rendered 94 views. Extension settings, creation, processor and node controls, named ports, keybinds, Mixer cards and unavailable-target warnings were visually inspected. UI regressions verify that merely opening controls does not dirty the project/send engine updates and that Reset uses the ordinary update path.
- Real Logi SDK/native-pipe integration passed extension command callbacks, node/effect knob adjustment, reset and live feedback, alongside the existing startup/reconnect/no-replay tests. These were simulated SDK callbacks; no physical Loupedeck was operated.
- Native VST2/VST3 and x86 VST2 bridge regression checks passed scanning, numerical DSP routing, bypass/parameters, MIDI audition and project restoration. The pre-existing custom Rust effect example still runs.
- The distributed standalone SDK workspace compiled independently with `cargo test --offline --locked --workspace --all-targets --manifest-path dist/PirateWave/sdk/Cargo.toml --target-dir target/sdk-bundle-check` (six passing tests).
- `extension_check` measured zero host graph render allocations at approximately 0.160% of the realtime CPU budget. This check uses a deterministic provider to measure host-side routing/adapter allocations; the separate native integration exercises the actual DLL. Recording, all 20 rack processors, all 32 ducking combinations and pitch-correction allocation checks also passed with zero processing allocations. No new buffering stage or algorithmic delay is introduced by the extension host.

## Built-in-only CPU comparison

The previous commit's core was extracted into an ignored, isolated benchmark workspace. The exact same `builtin_graph_check.rs` harness was compiled with the same release/LTO settings against both versions. Five alternating before/after runs each rendered 50,000 blocks per case after warm-up, with zero render allocations in all runs.

| Graph | Before median (% realtime budget) | After median |
|---|---:|---:|
| Routing | 0.092225 | 0.091100 |
| Rack | 0.146436 | 0.145610 |
| Level ducking | 0.334210 | 0.330621 |
| Speech ducking | 0.773307 | 0.760066 |

These synthetic measurements show no regression in the tested built-in paths. They are local timing evidence, not guarantees for every CPU/project. Initial measurements exposed a small overhead; preserving existing enum tags, using compact compiled ports, lazily storing extension settings and specializing the no-extension render path removed it. Active third-party DSP remains responsible for its own CPU cost and declared latency.

## Artifact and limitations

Final archive integrity and byte comparisons verified all shipped binaries, standalone SDK sources, guides, embedded example package/DLL and verified `PirateWave.lplug4`. Six release-publication safeguard tests also passed; no release upload was made.

Artifact: `dist/PirateWave-v0.2.0-beta.10-windows-x64.zip`

SHA-256: `e34760cd80d3c3c68787dedc2ace2214c588ac0de36a7ccb985ef73809d87848`

Native extensions are trusted in-process code. The SDK catches unwind panics/reported errors and the host handles invalid audio, but cannot contain arbitrary native memory corruption, aborts or hangs. ABI 1 uses fixed stereo ports, host-generated controls, declarative parameter actions and immutable configuration blobs; custom native UI, MIDI/event ports and hardware endpoint ownership are outside this version. Descriptor/schema changes require explicit offline migration, retaining the original project. No physical-device audio latency qualification was performed.

Ignored local evidence is under `dist/extensions-*`, `dist/native-extension-*.png`, `dist/shortcut-self-test.txt` and `dist/ui-smoke-result.txt`.
