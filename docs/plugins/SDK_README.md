# Standalone PirateWave extension SDK

This folder contains only the public SDK and working examples, independent of PirateWave's application source. Start with `examples/gain-plugin/` for a minimal processor/node, or `crates/piratewave-example-extension/` for the advanced multiport fixture. Read [the developer guide](../PLUGIN_DEVELOPMENT.md) first.

From this folder, with Rust's Windows x64 MSVC toolchain and Python 3 installed:

```powershell
cargo test --workspace --all-targets
cargo build --release -p piratewave-example-extension
./scripts/build-extensions.ps1 -PirateWaveExe ../PirateWave.exe
```

The script produces and verifies `dist/PirateWaveExamples.piratewaveext`, runs the native integration fixture, and does not install it. Copy/rename the example crate and use your own namespace, manifest, library filename and license for your plugin. The SDK dependency is local and versioned; it does not link private PirateWave modules or Logi/VST host SDK assemblies.

The ABI is C version 1, SDK crate version 0.1.0. This bundle and its examples use the MIT license. Individual third-party plugins can choose their own licenses.
