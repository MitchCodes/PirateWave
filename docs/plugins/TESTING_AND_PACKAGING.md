# Testing, packaging and diagnosis

## Commands

From the PirateWave workspace:

```powershell
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo run --release --locked -p piratewave-core --example extension_check
./build.ps1 -Verify -WithLoupedeck
```

The build creates the real example DLL, validates its manifest and callbacks in a child, packages `PirateWaveExamples.piratewaveext`, runs native multiport/effect/controller integration and includes the package and guides in the portable dist. CI runs the same build. The SDK examples are workspace members, so stale APIs break compilation/tests.

Validate an unpacked package directly:

```powershell
./dist/PirateWave/PirateWave.exe --scan-extension C:/absolute/package C:/absolute/report.json
./dist/PirateWave/PirateWave.exe --extension-self-test C:/absolute/example-package
```

The self-test expects the example's two-input/three-output fixture. For visual coverage:

```powershell
./dist/PirateWave/PirateWave.exe --ui-smoke-test --extension-fixture C:/absolute/example-package
```

These fixture commands do not install packages, open audio hardware or alter existing Loupedeck profiles. They exercise real native callbacks, not physical knobs/keys. Scanner nonzero exit, missing report or timeout means validation failed; never accept that as an empty successful catalog.

## Package layout

Use `.piratewaveext` (ZIP), with `manifest.json`, the named Windows x64 DLL, `LICENSE`, notices and optional resources at package-relative paths. No wrapper directory. Package root DLL name must match manifest `library`; embedded DLL metadata must match the parsed manifest exactly. Build with `scripts/package-extension.py <folder> <output> --piratewave <PirateWave.exe>`; it performs native scanner validation and deterministic ZIP creation.

Limits: 64 MiB compressed, 128 MiB expanded, 128 ZIP entries, 64 MiB DLL, 256 KiB manifest. Absolute/traversal paths, symlinks, duplicate/case-colliding entries, and unsafe Windows path suffixes are rejected. Extraction uses a unique staging directory. A failed scan never activates a package. Install only trusted native code; scanning is a reliability check, not a sandbox or authenticity signature.

## Required plugin tests

Verify numerical response, stereo independence, silence, all ports simultaneously, bypass maps, parameter boundaries/discrete choices, reset, sustained processing and zero render allocations. Exercise multiple live/export instances and unrelated graph edits. Test unavailable/changed plugins, state migration failure, legacy project loading, and precise route removal. Test UI, keybind deletion and Loupedeck knob/reset/state synchronization.

Performance checks must compare built-in-only graphs before/after host changes, not just plugin DSP speed. Measure CPU after warm-up over repeated runs; do not infer no regression from a single timing. A zero-latency declaration and allocation check are separate requirements. Native plugin CPU and hangs remain the developer's responsibility.

## Troubleshooting

Settings shows installed/selected versions, active-session state, restart requirements and startup errors. PirateWave's ordinary log records scan/load errors. DSP fault status is surfaced through normal engine telemetry without logging from process callbacks. Missing extension warnings stay visible in the node/effect editor, with saved controls and removal still available.

A failed load can indicate an x86 DLL, ABI mismatch, missing dependency, embedded manifest mismatch, callback failure or timeout. Keep Windows runtime dependencies package-local and resolve resources during preparation, never from process. PirateWave loads the absolute DLL path with restricted dependency search flags. Do not rely on the application's working directory. Reproduce faults in the scanner/debugger before installing another release.
