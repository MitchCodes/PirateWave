# State, identities and compatibility

Projects retain the qualified type ID, descriptor snapshot, stable instance ID, parameter map, action bindings and optional opaque state bytes. State is immutable declarative configuration supplied to create, not an automatic snapshot of delay histories or transient DSP internals. Actions modify persistent parameters. ABI 1 does not permit a plugin to write arbitrary project state during rendering.

State schema is a positive integer (default 1); bytes are limited to 65,536 per instance. Provide `default_state` for new instances. A release that changes serialized private configuration must increment its schema and explicitly implement `Plugin::migrate`. The example supports a schema-1-to-2 migration while preserving empty declarative state.

The host compares installed metadata with saved snapshots. Mismatches remain unavailable until explicitly migrated; no label-based replacement or destructive defaulting occurs. In particular, changing descriptor metadata requires updating saved snapshots through migration even when its DSP is compatible. Publishing only a new package version with identical type descriptors works with existing projects immediately after restart.

Missing/incompatible nodes silence outputs; unavailable rack effects pass through. Settings, routes and bindings survive save/load. Missing old built-in effects still fail validation as before; namespaced extension effects are retained as unavailable entries. Opening/saving projects performs metadata validation without creating native DSP instances. Saving a recording with an enabled unavailable/faulted effect reports an error; disable that effect or bypass its rack to explicitly export dry audio.

## Explicit offline migration

Close the project or stop audio. With the required new extension versions installed and enabled for next launch, run:

```powershell
./PirateWave.exe --migrate-extension-project old-studio.json migrated-studio.json
```

Use a new output filename. The command scans installed packages, invokes declared state migrations only where schemas differ, updates descriptor snapshots, validates the entire resulting graph/parameters/actions, and writes a separate project. Existing files are never overwritten. On any unsupported migration or validation error, the original remains untouched. Open the new project normally to audition it.

Migration does not silently remove ports, routes, actions or parameter values. Structural incompatibilities need a new contribution ID or explicit project edits. Keep stable identities when behavior remains compatible. Do not reuse a removed port/parameter/action ID for a different meaning. A plugin's name, DLL filename or directory is never a substitute for its qualified identity.

## Package changes

Installation selects a new version directory, never overwrites an installed version or loaded DLL. Restart applies installation, enable/disable and version changes. Old libraries stay pinned while graphs, exports and callbacks can reference them. Installed packages live beside settings under `extensions/<plugin-id>/<version>/`; `selection.json` records selected versions and disabled IDs. Do not edit loaded DLLs or selection while PirateWave runs.
