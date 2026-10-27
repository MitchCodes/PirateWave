# Effect processors, parameters and actions

A processor descriptor has `kind: "processor"`, exactly one stereo input/output and zero declared latency. PirateWave adds it under Add processor using its `category` (default `Extensions`). It can be used wherever PirateWave rack effects are supported, including recorder playback and export. The host's existing rack enable/disable/toggle shortcuts and Loupedeck effect commands work without a plugin-specific implementation.

## Parameter contract

Each parameter has stable `id`, `name`, finite `min`, `max`, `default`, optional `unit`, `step` and `choices`. IDs are bounded ASCII alphanumeric plus `.`, `_`, `-`, `/`. Min must be less than max; defaults and saved values must be in range.

- `step: 0` means continuous. Knobs default to 1% of the range per detent.
- A positive step means a discrete grid anchored at min. Defaults/values must fall on that grid. Knobs use that step; sensitivity remains configurable.
- Choice parameters require min 0, max `choices.length - 1`, step 1 and at least two choices. Provide labels in order.
- Knob press resets to default. UI controls include Reset. Updates through UI, keybind actions and Loupedeck share persistent values, validation, graph updates and state feedback.

At most 64 parameters and 128 choice labels per parameter are allowed. The host resolves IDs to descriptor indices on the control plane. DSP must not search string maps each block. Avoid coefficient rebuilding on every sample; update bounded coefficients once and smooth audible changes. The example width processor smoothly converges to its target without a delay block.

## Declarative actions

An action is `{ "id": "mono", "name": "Mono", "parameter": "width", "value": 0.0 }`. With `toggle: true`, execution switches from the specified value to the parameter's default, or from any other value to the specified value. Choose a value distinct from default for a useful toggle.

Action IDs and numeric values are validated against parameters. Full IDs are persisted with bindings; dispatcher handles derive stable numeric keys, with descriptor collision checks. Never persist action array positions. Removing an effect/node unregisters its actions along with ordinary bypass bindings. Missing/incompatible plugins retain saved bindings but cannot execute them. Native events and Loupedeck commands use the same dispatcher as UI action buttons.

## DSP and recording

Implement `Processor::set_parameter`, `process`, and `reset`, allocate histories in `Plugin::create`, and export with the SDK macro. Do not share mutable histories across instances. PirateWave can create several instances for one recorded slot: preview/send processing and offline export must be independent. A recorder captures the upstream dry recording according to existing graph routing; its own rack is applied during playback/save, not capture. Each export processes the selected crop through a fresh configured chain.

Reported callback errors or NaN/infinity outputs latch a processor fault and preserve the dry block. Reset/recovery stays on normal control paths. Passing automated tests does not make arbitrary native memory writes safe; authors must obey the ABI and realtime contract.
