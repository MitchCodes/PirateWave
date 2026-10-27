# Stream Deck

PirateWave's Windows Stream Deck plugin controls the current project directly. Keyboard
shortcuts are optional. Loupedeck and Stream Deck can connect at the same time.

## Install

1. Install Stream Deck **7.1 or newer** on Windows 10/11.
2. Run the matching PirateWave build and enable **Settings > General > External
   controllers > Enable external controllers**. Only one PirateWave instance per
   Windows user can own controller support.
3. Open **PirateWave.streamDeckPlugin** from the separate GitHub release asset
   and confirm installation in Stream Deck. Installation is never automatic.
4. Drag an action from **PirateWave Audio Mixer** to a key or, for adjustable controls,
   a Stream Deck+ dial. Select the node, effect/slot/destination, and action or
   parameter in its property inspector.

Existing profiles are not modified. PirateWave must already be running; the plugin
does not launch it. Ordinary keys work on supported Stream Deck devices; dials and
touch-strip controls require Stream Deck+. Neo Infobar customization is excluded.

## Actions

Application controls start/stop/toggle audio and stop sounds. Node actions cover
mute, solo, output listening, effects bypass, recorder start/stop/stop playback,
Auto Ducking enable/disable/toggle and VST instrument audition/release/all notes
off. Separate actions select individual effects, switch destinations, soundboard
pads, recording slots and extension actions. Lists come from PirateWave's catalog, so
new supported dynamic actions do not need hardcoded keyboard assignments.

Recording slots expose capture, preview, send, preview & send and delete. Empty
slots remain selectable for capture. Arm recording and wait for buffered audio
before capture. A successful command response means accepted; the slot's live
status reports capture completion or failure. Changing a selection stops its
playback; recorder-wide stop also cancels deferred playback.

Volume, pan, effect/VST parameters, Auto Ducking parameters and extension node
parameters work on keys and dials:

| Control | Default step | Dial press / touch tap |
| --- | --- | --- |
| Volume | 1 dB | Toggle mute |
| Pan | 5% | Center |
| Built-in continuous parameter | 1% of range | Reset default |
| Continuous VST parameter | 0.01 normalized | Reset default |
| Discrete parameter | Declared step/choice | Reset default |

Sensitivity multiplies the default step, from 0.01x to 100x. PirateWave applies existing
ranges and validation. Key controls offer increase, decrease or reset; each key
press executes once. A reset key restores the parameter default, including volume.
Muted volume displays its mute state alongside its value. Buttons use green text
and an **ON** label for enabled flags, red and **OFF** for disabled flags. A mute
flag being ON means muted. State comes from PirateWave, including changes made using
other controllers or shortcuts; there are no optimistic local toggles.

## Target matching and troubleshooting

Assignments store exact, case-sensitive node type/name and applicable effect kind,
slot name, sound name or destination name. VST assignments also include format,
class identity and parameter ID. No list positions or project-specific IDs are
persisted. Another project with matching names can use the same assignments.
Renaming requires reassignment. Missing or duplicate matches display **Missing
target** or **Ambiguous target** and do nothing. Repeated effects of the same kind
within a rack are ambiguous. Filtering the editor does not change the assignment.

If lists are empty, confirm PirateWave is running under the same Windows account,
external controllers are enabled and the project contains a matching control.
For a missing parameter, check that its plugin is available and still exposes it.
An **Upgrade PirateWave** message requires a PirateWave build supporting control protocol v2.
Use PirateWave's application log and Stream Deck's plugin logs for connection errors.

Connections recover automatically after either application restarts. Pending
commands are discarded on disconnect and never replayed. A full controller queue
reports an error instead of silently dropping presses. Slow/disconnected clients
do not block other controllers. The local pipe rejects remote clients and is
restricted to the current Windows user. No additional network listener is opened.

## Development and verification

Source lives in `integrations/streamdeck`. Node.js 24+ and Python are required to
build. Run `scripts/build-streamdeck.ps1`; add `-Verify -PirateWaveExe <absolute-path>`
for the isolated native named-pipe fixture. Run
`build.ps1 -Verify -WithLoupedeck -WithStreamDeck` for the full distribution.

The TypeScript adapter uses Elgato SDK v3 and the SDK's host WebSocket. Its single
Node pipe client uses lossless string generations in protocol v2. Loupedeck keeps
its compatible v1 protocol on the same endpoint. Shared catalogs, dispatcher,
validation and autosave remain in PirateWave. All transport and rendering work runs
outside the audio engine. Settings retain structured selectors with schema 1.

`npm run check`, `npm run build`, `npm test` and `npm run validate` check types,
simulated production SDK callbacks, selector behavior, framing, queue behavior,
and the official manifest. `test/native.mjs` runs simultaneous v1/v2 clients,
project-generation rejection, ownership, reconnect and command-discard checks
against PirateWave's isolated fixture without using user settings or audio hardware.
The official CLI packages the SDKVersion 2 installer for direct distribution,
without Marketplace processing. Locked dependencies and archive allowlists keep
development files and credentials out of packages.

After a successful main-branch push, release automation publishes the portable ZIP, PirateWave.lplug4 and
PirateWave.streamDeckPlugin and an optional CPU-training ZIP plus checksums. See [release configuration](RELEASING.md). Build scripts never install or link a
plugin. Marketplace publication is a separate operation.

Native Stream Deck host installation and physical device tests are distinct from
simulated SDK callbacks. Record results for key displays/presses, dial direction,
press/tap, touch-strip values, multiple devices, both controller applications and
restart recovery before claiming hardware qualification. The implementation
environment has no Stream Deck host/device available, so these are not claimed.

## Verified implementation checks (2026-10-23)

- Workspace tests, Rust formatting and Clippy passed; optional voice-pack tests
  remain explicitly ignored by their existing test definitions.
- All 15 Stream Deck tests passed, including production SDK callbacks, the actual
  property-inspector script and a simulated host starting the bundled backend.
- Native Windows pipe tests passed against the release executable with extension
  fixtures: simultaneous v1/v2 clients, eight-client limit, exclusive ownership,
  large generations, clamping, discrete choices, defaults, empty-slot actions,
  project-switch rejection, a stalled subscriber and restart without replay.
- Loupedeck SDK/editor/native integration passed with extension fixtures.
- Native VST2/VST3 and 32-bit bridge checks passed. Voice correction, rack
  processors, shared recording histories, both ducking detectors/lookahead and
  extension graph checks reported zero audio-render processing allocations.
- Official plugin validation/packaging and package-content checks passed. Public
  release tests require both controller packages, verify embedded copies and
  protect main-branch publication. No plugins or profiles were installed.
