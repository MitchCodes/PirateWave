# PirateWave 0.2.0 Beta

Extract this entire folder, then run `PirateWave.exe`. Windows x64 is supported.

## Start mixing

1. Select microphone/input and output devices in Mixer. Use headphones for monitoring.
2. Open Routing to connect sources, optional processors and output buses.
3. Start audio. Keep your raw microphone and converted voice from reaching the
   same destination together unless you deliberately want both.
4. Effects rack Help explains each processor. Soundboard imports audio clips;
   optional recorded shortcuts play them globally.

Audio starts stopped by default. Settings → General offers Start with Windows,
Start minimized and Start audio automatically. After moving this folder, toggle
Start with Windows off/on to update its registered executable path.

## Voice Creation — experimental Beta

Install the matching optional LLVC voice pack on Voices, then choose Create voice.
Record or import dry speech, reserve separate validation/test takes, explicitly
install CPU or CUDA tools plus shared assets, train, compare and install a CPU
voice revision. Tools are optional; no system Python, pip, compiler or CUDA toolkit
is required. CUDA requires a compatible installed NVIDIA driver.

Current trained voices can sound robotic or metallic. This release exposes the
existing process and does not claim improved quality. Custom voices pass short
native numerical/timing checks; complete clean-Windows, long-run custom-model
and gaming-impact qualification remain pending. Custom DirectML playback is
unavailable until separately validated. The curated voice has its own hardware
profiles, independently of training.
The curated live profiles currently target the i9-10900K and RTX 4070 SUPER;
custom CPU voices are checked locally against the training machine's processor.

The Beta accepts up to eight minutes of training takes and thirty minutes overall.
Read `docs/VOICE_TRAINING.md` for download sizes, storage, recording recommendations,
pause/resume, previews and troubleshooting. Heavy training pauses when PirateWave audio
starts; resume is explicit. Recordings and results stay local. Use recordings you
have permission to use.

For offline tools, put every matching `.partNNN` file for your selected backend
and shared bundle in one folder and select it in Tools/storage. Do not extract
individual parts. The application verifies its pinned checksums before execution.

## Other help

- `docs/VOICE_CHANGER.md`: routing and live voice conversion.
- `docs/RACK_PROCESSORS.md`: built-in audio processors.
- `docs/EFFECT_TUNING.md`: comparison controls, tuning plots and visibility-scoped live analysis.
- `docs/VST_PLUGINS.md`: optional VST instruments/effects.
- `docs/APPLICATION_CAPTURE.md`: capturing a selected application.
- `docs/VOICE_TRAINING_NOTICES.txt`: shared models and source-speech attribution.
- `THIRD_PARTY_NOTICES.md` and `VOICE_WORKER_THIRD_PARTY_NOTICES.md`: native dependencies.

The optional tools include additional dependency licenses and reproducible file
manifests. No virtual audio driver is installed automatically. Configure your
chosen virtual cable separately when sending audio to Discord or OBS.

Recording nodes support recent-history capture, independent passthrough/preview/send ports, waveform cropping, slot keybinds and WAV/MP3 export. **Slot effects** racks process playback and saved files, with rack bypass and individual effect keybinds. Captures remain dry; see `docs/RECORDING.md`.

## Loupedeck

Download the separate `PirateWave.lplug4` release asset, then double-click it to install the optional controller plugin. In PirateWave, enable Settings > General > Enable external controllers. See `docs/LOUPEDECK.md` for assigning actions, volume/effect knobs and recording slots. Names must uniquely match in the current project.

## Auto Ducking

Add Auto Ducking to lower background audio when a Priority signal is active. Two named inputs feed Mixed, Priority and Ducked outputs. Level/Speech detection, timing, optional lookahead, keybinds and Loupedeck controls are described in `docs/AUTO_DUCKING.md`.

## Workspace layout

Mixer keeps input/source strips and output buses in independent scrolling panes. Expand **Processors & utility nodes** below for other nodes. Routing has a growing canvas with two scrollbars, Find node and Reset view. Details are in `docs/WORKSPACE_LAYOUT.md`.

## PirateWave extensions

Optional `PirateWaveExamples.piratewaveext` demonstrates a stereo-width effect and multiport node. Install trusted packages in Settings → General → PirateWave extensions, then restart. The build does not install anything automatically. See [developer guide](PLUGIN_DEVELOPMENT.md) for creating plugins.

For Stream Deck buttons and Stream Deck+ dials, download and install the separate `PirateWave.streamDeckPlugin` release asset and follow [Stream Deck setup](STREAM_DECK.md). Loupedeck and Stream Deck can be used together.

Open **Tools** in the sidebar for microphone checks, spectrum/latency/stereo tests, test tones, voice auditions, references and controller-icon export. See [Tools](TOOLS.md). Tests use separate streams and leave your project unchanged.

## Example workspaces

Open **Cookbook** in the sidebar to import microphone-cleaning, music-ducking, switchable-voice, recording, and all-in-one recipes. Each import creates a separate workspace. See [Cookbook setup](COOKBOOK.md).
