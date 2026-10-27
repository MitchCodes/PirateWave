# Instructions for AI-assisted plugin development

Read `docs/PLUGIN_DEVELOPMENT.md`, `ABI.md` and `PROCESSORS.md` before editing. Use the actual SDK and example source as the API authority. Do not invent host callbacks, depend on egui, call desktop internals, or add a NodeKind enum variant for each third-party node.

## Repository map

- `examples/gain-plugin`: beginner template contributing a simple node and rack processor; build and package it using the Gain plugin walkthrough.
- `crates/piratewave-extension-sdk/src/lib.rs`: manifest validation, C table, safe Rust authoring wrapper and export macro.
- `crates/piratewave-example-extension`: buildable width processor and multiport node, with exact manifest/source agreement.
- `crates/piratewave-core/src/extensions.rs`: frozen catalog, validation, fallback, action dispatch and preallocated runtime adapters.
- `crates/piratewave-core/src/graph.rs`: compilation to numeric ports, rendering and state preservation.
- `src/extensions.rs`: native loader, isolated scanner, package installation, migration and integration fixture.
- `src/ui/extensions.rs`: generated controls and settings.
- `scripts/package-extension.py`: native verification and deterministic packaging.

## Implementation checklist

1. Decide processor versus node; assign unique stable qualified IDs.
2. Write bounded, complete descriptor metadata and finite parameter ranges/defaults. Declare latency zero. Set an explicit nonzero state schema.
3. Keep manifest JSON embedded in the DLL identical to the package's manifest, including contribution order.
4. Implement per-instance DSP state. Allocate only in create; use fixed buffers and numeric parameter indices while processing. Input buffers are read-only; populate every output.
5. Smooth parameter changes. Cover silence, reset, bypass, all parameter extrema and discrete values.
6. Use declarative actions for persistent controls. Let PirateWave own keyboard/controller registration and project mutation.
7. Test numerical outputs and multiple independent instances. Run workspace tests, Clippy and allocation/performance checks. Verify the built DLL, not only a mock trait implementation.
8. Package with the official repository script. Test installation/restart, absence, migration failure and restoration. Document limitations honestly.

Never introduce render logging, heap growth, locks, waits, disk/network operations, asset loading, serialization, or custom IPC queues. Do not claim native access violations/hangs are recoverable. Do not claim arbitrary third-party DSP has zero CPU cost. Do not overwrite a loaded version or silently drop old settings during migration. Do not add buffering to solve a control-plane problem.

When changing the SDK/host itself, retain old project fixtures and baseline allocation/CPU evidence, keep built-in DSP paths intact, update this documentation and both working examples, and build a refreshed dist including the verified Loupedeck package. State physical-device validation separately from simulated callbacks.
