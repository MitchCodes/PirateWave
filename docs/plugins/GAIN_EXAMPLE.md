# Gain plugin walkthrough

The complete beginner example lives in the repository's `examples/gain-plugin/` folder. It contributes **Example Gain Processor** and **Example Gain Node** from one DLL. The existing `crates/piratewave-example-extension` remains the advanced multiport/state-migration fixture.

Portable distributions include the same example in `sdk/examples/gain-plugin/`; run the commands below from the `sdk/` directory instead of the repository root. A Rust Windows x64 MSVC toolchain, C++ build tools and Python 3 are required.

## Build and test

```powershell
cargo test --locked -p piratewave-gain-example
cargo build --release --locked -p piratewave-gain-example
```

The test calls the exported C ABI for both contributed types and verifies stereo gain numerically. The build produces `target/release/piratewave_gain_example.dll`.

## Package

From the repository root or portable SDK root:

```powershell
New-Item -ItemType Directory -Force dist/gain-example | Out-Null
Copy-Item target/release/piratewave_gain_example.dll dist/gain-example/
Copy-Item examples/gain-plugin/manifest.json dist/gain-example/
Copy-Item LICENSE dist/gain-example/
python scripts/package-extension.py dist/gain-example dist/PirateWaveGainExample.piratewaveext --piratewave C:/absolute/path/to/PirateWave.exe
```

Use the compatible PirateWave executable from your dist (for example, repository `dist/PirateWave/PirateWave.exe` or portable `../PirateWave.exe`). The packager scans the actual DLL in a child process and checks its embedded manifest before writing the package. It does not install it.

In PirateWave, choose **Settings → General → PirateWave extensions → Install .piratewaveext…**, select `PirateWaveGainExample.piratewaveext`, then restart. Add **Example Gain Processor** under **Add processor → Examples**, or choose **Example Gain Node** in **Add node**. Connect the node's Audio in/Audio out ports in Routing. Set Gain, press Unity or Toggle quiet, or assign the actions through Keybinds/Loupedeck.

## How it works

`manifest.json` declares stable qualified type IDs, port IDs 10/20, a −60..+12 dB parameter and two declarative actions. `Plugin::create` allocates one independent instance per node/effect. `set_parameter` converts dB to linear gain on the control path. `process` smooths that gain across samples and multiplies both channels equally, with no processing allocations, locks or I/O. Smoothing changes the gain envelope without delaying audio. `reset` settles the current gain immediately.

The node's explicit bypass map copies its input to output. Rack bypass is owned by PirateWave. The example needs no opaque state; schema 1 accepts empty bytes. Use the advanced example when adding private state migrations or more ports.

When making your own plugin, change the publisher ID, both qualified type IDs, Cargo package/DLL names and manifest library filename together. Adjust the public SDK dependency path if you move the folder. Embed the same manifest file you package. Keep stable IDs when updating an existing plugin; follow the [compatibility guide](STATE_AND_COMPATIBILITY.md) for changes to saved state or descriptors. Read the [ABI contract](ABI.md) before adding DSP: native plugins run as trusted in-process code.
