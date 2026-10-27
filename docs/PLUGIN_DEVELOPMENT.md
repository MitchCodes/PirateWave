# Developing PirateWave extensions

PirateWave extensions add node types and effect-rack processors without rebuilding PirateWave. This is separate from VST2/VST3 support, which remains available through VST Plugins. ABI 1 targets Windows x64, stereo `float32`, 48 kHz and 128-frame blocks. Only zero-algorithmic-latency contributions are accepted.

## Beginner example

Start with the complete `examples/gain-plugin/` plugin: it adds a gain processor and a gain node using the same short DSP implementation. Follow the [Gain plugin walkthrough](plugins/GAIN_EXAMPLE.md) to build, package, install and customize it. Portable builds include its source at `sdk/examples/gain-plugin/`.

## Quick start (Rust)

Use a Rust installation with the `x86_64-pc-windows-msvc` toolchain and Visual Studio C++ build tools. Python 3 is used for packaging. Build and test PirateWave first:

```powershell
./build.ps1 -Verify -WithLoupedeck
```

This builds `crates/piratewave-example-extension`, verifies its DLL in a scanner child, runs real native graph integration, and produces `dist/PirateWaveExamples.piratewaveext`. Installation is optional: open **Settings → General → PirateWave extensions → Install .piratewaveext…**, select the package, then restart PirateWave. The package contributes **SDK Stereo Width** under Add processor → Extensions and **SDK Multi Mix** under Add node. Nothing is installed automatically by the build.

For your own project, copy `crates/piratewave-example-extension` as a starting point, keep `crate-type = ["cdylib", "rlib"]`, reference `piratewave-extension-sdk`, replace the manifest IDs/library filename, and implement `Plugin` and `Processor`. The example is runnable source, not pseudocode. Its width processor includes smoothing, a choice parameter, persistent actions and an explicit state migration. Multi Mix demonstrates two named inputs, three named outputs, separate recording histories and a declared bypass map.

The portable dist also includes a standalone `sdk/` source workspace and build/package scripts; it needs no private PirateWave application sources. See `sdk/README.md` in the extracted dist.

For an independent plugin repository, vendor the versioned SDK crate or depend on an exact PirateWave Git revision. Do not depend on PirateWave's private desktop/core modules or a moving branch. The ABI itself is C, so compiler Rust versions do not have to match the host. The SDK version here is 0.1.0 and native ABI version is 1.

```powershell
cargo build --release -p piratewave-example-extension
# Put manifest.json, your DLL, LICENSE, and optional resources in one folder.
python scripts/package-extension.py my-package dist/MyPlugin.piratewaveext --piratewave dist/PirateWave/PirateWave.exe
```

Package validation loads native code in a separate scanner with a timeout. Install only trusted packages: active DSP runs inside PirateWave. Rust panics reported through the SDK and invalid numerical outputs can be contained; native access violations, process termination and hangs cannot be safely recovered in-process.

## Choose a contribution

- **Processor:** one stereo input and one stereo output, inserted into an effect chain. PirateWave owns enabled/bypass/reorder/remove controls and effect shortcuts. Instances are independent for live processing, recording preview/send and saved WAV/MP3 exports. Capture remains dry.
- **Node:** zero to eight named stereo inputs and one to eight outputs. All incoming routes to each input are summed. All outputs can be connected concurrently. The initial SDK exposes these as utility nodes with generated controls and bypass actions; hardware device ownership and custom native windows remain in PirateWave.

Both use named parameters and declarative actions. Actions set a parameter or toggle it between a specified value and its default, through the same dispatcher as keybinds and Loupedeck. They do not directly mutate the project or issue arbitrary host commands. The initial SDK does not offer arbitrary stateful command callbacks, MIDI/event ports, custom UI code, scripts, dynamic port topology or access to PirateWave internals.

## Read next

1. [ABI and lifecycle](plugins/ABI.md), including ownership and callback threads.
2. [Node definitions and ports](plugins/NODES.md).
3. [Processor DSP and parameters](plugins/PROCESSORS.md).
4. [State and compatibility](plugins/STATE_AND_COMPATIBILITY.md).
5. [Testing, packaging and diagnosis](plugins/TESTING_AND_PACKAGING.md).
6. [AI implementation checklist](plugins/AI_IMPLEMENTATION_GUIDE.md).
7. [C/C++ ABI header](plugins/piratewave_extension.h).

The SDK and example are MIT licensed. Plugin licenses are independent; include your own license and notices. Give every publisher a stable reverse-domain namespace, such as `com.example.audio`, and every contribution a qualified identity, such as `com.example.audio/width`. Labels can be descriptive; identities must never depend on menu positions or DLL filesystem paths.
