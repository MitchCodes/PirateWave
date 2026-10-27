# VST support

PirateWave hosts installed 64-bit Windows VST3 plugins and 32-/64-bit VST 2.4 DLLs. Plugins are not bundled. VST3 folders contain `Contents/x86_64-win/<plugin>.vst3`; flat `.vst3` modules work too.

## Add and route plugins

1. Open **VST Plugins**. Use **Add plugin file** for a VST2 DLL/flat VST3 module, or **Add VST3 bundle** for a `.vst3` folder. VST3 normally installs under `C:/Program Files/Common Files/VST3`.
2. PirateWave inspects the plugin in a separate process (30-second limit). Its name, vendor, format, type, and exposed parameters appear in the library. Multiple VST3 audio classes get separate entries.
3. Click **Add instrument node** / **Add effect node**, or select either type in **Add node**. Connect it in **Routing**. Instruments have only an outgoing audio port; effects process an incoming mix. Both can feed processors and multiple output buses.
4. Start audio. Open **Plugin / Play notes** or **Plugin controls** in Mixer, or select the node in Effects rack. Select an imported plugin of the matching type, edit parameter sliders, and play the instrument keyboard. Hold a keyboard button with the mouse; releasing anywhere releases that note. **Play audition note** stays on until **Stop all notes** or its release shortcut.

Instruments have volume, pan, mute and solo controls. VST effects have **Disable (passthrough)**, bypassing the plugin and additional built-in effects while preserving saved parameters. Manage extra built-in effects below Plugin controls in Effects rack. Output Bus mute/gain and Listen apply to the resulting mix as usual.

## Global shortcuts

All bindings are optional and initially unbound. Use the node's **Keybinds** page and **Apply keybinds**.

- Instruments: all 14 source actions (mute, unmute, toggle mute, solo on/off/toggle, volume up/down/reset and five balance positions), plus **Play audition note**, **Release audition note**, and **Stop all notes**.
- Effects: **Disable (passthrough)**, **Enable effects**, and **Toggle effects**.

The audition note (MIDI 0–127, default 60 / C4) and velocity are saved per instrument. Events use channel 1 and never start automatically on audio start or project load. Transport/soundboard shortcuts remain unchanged.

## Persistence and compatibility

Projects store plugin paths, class identities, parameter metadata/values, audition settings and shortcuts. `plugins.json`, beside `settings.json`, stores the imported library. Removing a library entry keeps existing nodes. After moving a plugin, import its new location and select it on affected nodes. Each node has a separate instance. Parameter edits and graph-only changes preserve active instances/notes; device-stream restarts recreate plugins.

Current scope: float32 mono/stereo main audio at 48 kHz / 128 frames, up to 4,096 parameters, generic normalized controls, and channel-1 notes from PirateWave. Vendor editor windows, external MIDI input, MIDI routing/output, presets or opaque state chunks, sample loading through vendor UIs, sidechains, surround, VST2 shells, 32-bit VST3, sample-accurate automation and latency compensation are not supported. Plugins requiring these may not be usable yet. Parallel branches containing plugins with latency may be misaligned.

32-bit VST2 is automatically hosted in `piratewave-vst-bridge32.exe`, bundled beside `PirateWave.exe`. No plugin conversion or administrator access is needed. Keep both executables together. The helper uses authenticated loopback IPC with bounded audio/parameter messages and a 500 ms processing deadline. This adds CPU/IPC overhead. Windows job objects terminate helpers on host/scanner exit; helper failure uses the existing silent-instrument / passthrough-effect fallback.

Scanning is process-isolated. 64-bit playback remains in process: missing plugins and reported API errors display messages and use silent-instrument / passthrough-effect fallback; a native plugin crash can terminate PirateWave. Reported failures have bounded automatic reload/recovery; restarting audio is also available for troubleshooting. See [recovery behavior](RACK_PROCESSORS.md#recovery-behavior). Import plugins you trust. PirateWave does not install, license or activate them.

## Development checks

From a source checkout:

```powershell
./scripts/test-plugins.ps1
```

Builds self-authored 64-bit and 32-bit fixture plugins and checks isolated scans, instrument → effect → effect → output audio, live parameters, bypass, mute, notes, project restoration, invalid-plugin errors and shared module lifetime. CI runs native and bridged VST2 checks without audio hardware. Source builds require MSVC x86 and x64 build tools; `build.ps1` / `scripts/build-vst-bridge.ps1` install the i686 Rust standard library when needed. Results: `dist/plugin-self-test.txt`.

Optional check requiring existing VB-Audio CABLE Input and Line 1 Virtual Audio Cable playback devices:

```powershell
./target/debug/piratewave-desktop.exe --plugin-audio-self-test 'dist/plugin-fixtures/PirateWave Test.vst3'
```

Checks note commands, instrument mute and effect bypass through both real WASAPI loopbacks while preserving Windows settings. Results: `dist/plugin-audio-self-test.txt`.

`--plugin-probe <effect-path>` checks a third-party effect in memory using a quiet sine and, when present, its Pre Gain parameter. Validation used Wolf Shaper VST2/VST3 in addition to fixtures; third-party plugins are not redistributed. The UI smoke test captures fifteen views with display-only plugin metadata, loads no plugins, starts no audio and saves no user settings.

References: [Steinberg VST3 interfaces](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/), [Rust VST3 bindings](https://github.com/coupler-rs/vst3-rs), [VST2 adapter library](https://github.com/RustAudio/vst-rs).

VST is a trademark of Steinberg Media Technologies GmbH.
