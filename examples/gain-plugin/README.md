# Gain plugin example

A small, copyable PirateWave plugin that adds **Example Gain Processor** to an effects rack and **Example Gain Node** to the routing graph. Both use the same stereo DSP with smooth gain changes, Unity/Quiet actions, generated controls, keybinds and Loupedeck support.

- `manifest.json`: package identity, stable type/port/parameter/action IDs and UI metadata.
- `src/lib.rs`: lifecycle, parameter conversion, allocation-free processing, reset and ABI test.
- `Cargo.toml`: DLL output and the public SDK dependency; no private PirateWave modules.

Read the plugin guide's **Gain plugin walkthrough** (`docs/plugins/GAIN_EXAMPLE.md`) for build, packaging, installation and customization. In portable builds this example is under `sdk/examples/gain-plugin/`.

From the repository root (or the portable `sdk/` workspace):

```powershell
cargo test --locked -p piratewave-gain-example
cargo build --release --locked -p piratewave-gain-example
```

The DLL is `target/release/piratewave_gain_example.dll`. Package it with this manifest and the workspace's MIT `LICENSE` using `scripts/package-extension.py`; pass the absolute path to a running-version-compatible `PirateWave.exe` with `--piratewave`. The walkthrough gives complete commands. Building never installs the example or changes a profile.

When copying this directory into another project, update the SDK dependency path, publisher/type IDs, package name and DLL filename together. Keep the embedded manifest identical to the one packaged beside the DLL.
