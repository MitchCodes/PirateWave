# Loupedeck / Logi controller support

PirateWave's Windows plugin controls the mixer directly, including actions with no keyboard shortcut assigned. It supports the current Logi Plugin Service 6.4.1 or newer, on compatible Loupedeck CT/Live/Live S, Razer Stream Controller and MX Creative Console devices through the official SDK. Physical device layouts and button availability depend on your device. This release targets the .NET 10 host used by Logi Plugin Service 6.4.1.

## Install and connect

1. Run PirateWave and enable **Settings → General → Enable external controllers**.
2. Double-click `PirateWave.lplug4` from the GitHub release assets to install it using Logi Plugin Service. This is a separate plugin installation; building PirateWave does not install it or alter profiles.
3. Find **PirateWave Audio Mixer** in your Loupedeck/Logi configuration software. Assign a PirateWave press action to a button or a rotation adjustment to a dial.
4. Choose the node/source, effect/slot/destination, then action or parameter in its editor. For application actions choose PirateWave; for sound pads choose Soundboard.

PirateWave must remain running; its window can be in the background. Plugin and PirateWave can start in either order. Settings displays connection status and any startup error. Only one controller-enabled PirateWave instance is allowed per Windows user. If another instance owns the connection, close/disable that instance and click **Retry connection**. Disabling integration closes the connection.

### Installation troubleshooting

Use plugin version **1.2.1 or newer**. Earlier PirateWave packages could pass the official package verifier but fail installation with error **1627** / **Install plugin method failed** because they lacked the SDK's required `ClientApplication` entry point. This is fixed for Loupedeck CT and the other supported devices. Installing the corrected package does not require PirateWave to be running or change existing profiles.

If installation still fails, check `%LOCALAPPDATA%\Logi\LogiPluginService\Logs\sentry\` for installation errors and `Logs\plugin_logs\PirateWave.log` for load errors. The official tool can also report the installer result: `logiplugintool install C:/absolute/path/to/PirateWave.lplug4`.

For dropdowns, use plugin **1.2.4 or newer**. Logi uses lowercase editor identifiers and exact renderer cache keys. Earlier PirateWave versions could populate the node list but send dependent refresh/reset messages under different keys, leaving the visible effect/slot/destination and parameter lists empty. Controls, selection resets and refresh events now share the same lowercase identifiers; legacy saved assignments remain readable. Version 1.2.2 also removed repeated dropdown invalidations on every live state packet: choices refresh when available targets change, while live values update separately. If the first dropdown says **PirateWave offline**, run PirateWave and enable **Settings → General → Enable external controllers**. **No matching controls** means this project has no targets for that particular action type. The plugin log records connection failures and the number of controls received from PirateWave.

## Actions and knobs

- **Application:** start/stop/toggle audio and stop all sounds (also stops recording playback).
- **Nodes:** every existing node shortcut action without needing an actual key binding: mute/unmute/toggle, solo, pan presets, volume steps, output listening, processor/recording-rack bypass, instrument audition notes/release/all-notes-off, and recorder start/stop/stop playing recording. Only supported actions appear for each node type.
- **Effects:** enable, disable or toggle a particular effect within a processor rack.
- **Switch outputs:** activate a named connected destination.
- **Soundboard:** play any named pad at its saved volume.
- **Auto Ducking:** enable/disable/toggle ducking under Nodes; the Auto Ducking parameters action provides detection mode, level threshold, speech probability threshold, duck amount, attack/hold/release and lookahead knobs. Knob press restores the parameter default. Targets use exact node names and live feedback follows UI and shortcut changes.
- **Recording slots:** capture, preview, send, preview & send, and delete the take. Empty slots are available for capture. Capture requires an armed recorder, running audio and some buffered samples. Playback uses the selected crop, recording node effects rack and configured preview/send ports. Recording racks also expose individual effect actions and built-in parameter knobs. Recorder Stop playing recording stops every slot on both ports. Changing crop selection cancels that slot's active and deferred playback as usual.
- **Volume & pan:** source and bus mixer volume; source pan. Volume defaults to 1 dB/detent and is clamped to -60..+12 dB; pan defaults to 5%/detent and clamps at full left/right. Press volume to toggle mute; press pan to center.
- **Extension actions:** custom node and processor actions declared by installed PirateWave extensions, alongside node/rack bypass. **Extension node parameters** provides knobs/reset for their parameters. Extension processor parameters appear under **Effect parameters**. Assignments retain qualified type identity and exact current-project names. Unavailable or changed extensions cannot execute actions.
- **Effect parameters:** any built-in or extension processor parameter. Continuous parameters default to 1% of their range per detent. Choice parameters use one choice per detent and display the choice label. Press to restore the parameter's default.
- **VST parameters:** exposed editable VST2/VST3 parameters on effects or instruments, including plugins hosted through the 32-bit bridge. Continuous parameters default to 0.01 on PirateWave's normalized 0..1 scale. Stepped parameters use their declared step size. Press to restore the saved descriptor default. Values match PirateWave's own parameter controls; the plugin does not add a native-editor automation/readback path or expose parameters that PirateWave cannot edit.

Knob **Sensitivity** multiplies the default step (0.01x..100x); its default is 1x. For example, 0.5x makes a volume knob change 0.5 dB/detent. Very small sensitivity on discrete choices can round to no change. Displays follow the application's current state rather than an optimistic local counter.

Plugin **1.3.0** adds state-colored button text: **green for ON**, **red for OFF**, plus an ON/OFF label and matching underline. This follows the actual switch being controlled: mute on is green, solo on is green, and an enabled rack/effect or ducking control is green. Audio running, listening, recorder arming, selected switch destinations and extension toggle actions also show their current state. Volume's mute-on-press image uses this feedback; ordinary volume/pan/parameter rotation remains neutral. Offline, missing, ambiguous and operation-error states stay neutral. Changes made in PirateWave or through shortcuts update the device automatically. Use matching current PirateWave and plugin builds for explicit boolean feedback.

## Names and dynamic targets

Bindings match **exact, case-sensitive names in the currently open project**, together with node type. Recording bindings additionally match the slot name; effects match effect kind; switch destinations match name/type. VST bindings also require the same format, class identity, plugin name and parameter ID.

Names are structured data: Unicode, quotes and separators are supported. Reordering nodes or slots and changing numeric IDs do not redirect a binding. Opening another project with the same names makes it usable there. Renaming a target requires updating the assignment.

Missing or duplicate matches display **Missing target** or **Ambiguous target** and do nothing. Two instances of the same effect kind in a rack are ambiguous; use separately named processor nodes when you need to control those instances independently. The plugin never chooses the first matching occurrence. Add/remove/rename changes refresh the editor choices and displays automatically.

Capture is asynchronous: **Accepted** means the command was queued, not that a take has been written. Slot state changes to **Capture queued**, then **Captured** or **Error: ...** when the worker replies. Errors and connection problems appear in device feedback. Deleted slots and versioned worker replies retain PirateWave's existing cleanup protections.

## Connection and performance

Protocol v1 uses a local Windows named pipe whose name includes the current user's SID. A protected ACL permits that user; remote pipe clients are rejected. No network listener, pairing token, key synthesis or administrator privileges are required.

Messages have request IDs, a version handshake and a 16 MiB frame limit. The app command queue holds 64 entries; the plugin queue holds 128. Queue overflow reports rejection. Only adjacent, same-direction knob changes for the same target/sensitivity/project may merge; buttons and resets preserve ordering. Incoming commands execute on the UI/control plane and share keyboard action handlers; IPC and catalog serialization never run in an audio callback. State is published at up to 10 Hz.

Requests are stamped with the current project's generation and rejected after a project switch. A disconnected client loses pending requests; the plugin clears pending actions and reconnects automatically. It does not replay old sounds, captures, toggles or knob movements. These are at-most-once attempts, not guaranteed execution across a crash. PirateWave state/autosave remains authoritative.

## Build and validate

Install the .NET 10 SDK and Logi Plugin Service 6.4.1+ for its `PluginApi.dll`. The build references this SDK but does not package host assemblies.

```powershell
./scripts/build-loupedeck.ps1 -Verify
./build.ps1 -Verify -WithLoupedeck
```

The first command expects `target/debug/piratewave-desktop.exe` built by `cargo build --locked`, or accepts `-PirateWaveExe` for another compiled binary. `-PluginApiDir` and `-DotnetExe` support non-default SDK/tool locations. Every plugin build checks the SDK loader entry points, offline installation lifecycle and dependent dropdown/refresh behavior before packaging. The script installs a pinned official Logi Plugin Tool into the ignored `.tools/` directory if necessary, packs and verifies `dist/PirateWave.lplug4`, and never installs it into the running host.

Rust tests cover target resolution, shared action dispatch, recording readiness, parameter constraints, duplicates and generation rejection. The C# integration executable constructs the real SDK action types and exercises the plugin client against an isolated native pipe fixture, without user projects or audio hardware. Native VST host checks remain in `scripts/test-plugins.ps1`. Physical button/dial behavior must be qualified separately on the user's device; simulated callbacks and package verification are not a physical-device test.

For an optional check against the installed, running service, close action editor dialogs, run PirateWave with integration enabled, install the matching plugin build, then run:

```powershell
python -m pip install websocket-client
python scripts/test-loupedeck-editor.py
```

This checks selection-change responses and refresh events using the renderer's exact state/list cache keys, then follows the dependent dropdowns. It does not save profile bindings or execute PirateWave actions. The deterministic SDK checks also verify these outgoing message keys before every package build.

SDK references: [Action editors](https://logitech.github.io/actions-sdk-docs/csharp/plugin-features/action-editor-actions/), [plugin packaging](https://logitech.github.io/actions-sdk-docs/csharp/plugin-development/distributing-the-plugin/), [current SDK examples](https://github.com/Logitech/actions-sdk).

Stream Deck can connect simultaneously using the same external-controller setting. See [Stream Deck support](STREAM_DECK.md). Protocol v1 remains supported on the PirateWave controller endpoint. Use matching current app/plugin builds; controller and extension identifiers now use the PirateWave namespace. Reassign controls created with older identifiers.
