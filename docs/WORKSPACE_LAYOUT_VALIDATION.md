# Workspace layout validation — 0.2.0-beta.9

> Historical validation record. Counts, versions, measurements, and limitations below describe the builds tested at that time; they are not a fresh qualification of every current build. See the [documentation index](README.md) for current setup guides.

Validated locally on Windows on 2026-10-22.

- `./build.ps1 -Verify -WithLoupedeck` passed formatting, 197 workspace tests (seven opt-in tests ignored), Clippy with warnings denied, optimized desktop/32-bit bridge builds, and official Logi plugin packaging and verification.
- Seven new layout tests cover legacy settings defaults and round trips; collapsed/expanded Mixer at 1000×700, 1380×960, 1600×960 and 1920×1080 with 1×/1.5×/2× rendering scales; many sources/buses and long names; utility filters/reveal; large and negative routing bounds; Find node/Reset view; and edge dragging with scrolling in both axes.
- Tests compare serialized projects and intercept engine commands: navigation, viewport changes, disclosure/filter changes and viewing missing default parameters do not change project audio data or send engine updates. Actual node dragging changes only saved positions. Existing port/wire, removal, shortcut, recording cancellation and controller tests remain passing.
- Native UI smoke renders 85 views, including the new collapsed/expanded and narrow Mixer, enlarged/negative routing scenes, grouped recorder/ducking/voice/VST pages and increased UI scale. Visual inspection found a short-window sidebar overlap; navigation now scrolls independently above its persistent status/Settings area. Final screenshots were inspected after that correction.
- Native Windows shortcut validation passed all 108 mappings, including deletion cleanup. Real Logi SDK/native named-pipe integration passed button/dial/reset callbacks, dynamic recording slots, feedback, project matching, disconnect/reconnect and no replay. These are simulated SDK callbacks; no physical Loupedeck was operated.
- `ducking_check` passed all 32 detector/lookahead/activity combinations with zero graph-rendering allocations (approximately 0.06–1.30% of the realtime CPU budget). `recording_check` passed shared-history/processed-slot playback with zero render allocations (approximately 0.21%). `rack_processors_check` passed all 20 processors with zero processing allocations (approximately 0.00–2.46%). These are synthetic local measurements, not physical-device latency guarantees.
- Final archive integrity and byte comparisons verify the desktop executable, worker, bridge, verified `PirateWave.lplug4` and updated workspace guide. No installation, profile changes or release upload was performed.

Artifact: `dist/PirateWave-v0.2.0-beta.9-windows-x64.zip`

SHA-256: `7ee738ddf886d6e30b734f4a1e244b2eab99bd39529423092bad57c238638c96`

Ignored local evidence: `dist/layout-build.txt`, `dist/ui-smoke-result.txt`, `dist/native-*.png` and `dist/shortcut-self-test.txt`.
