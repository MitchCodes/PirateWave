# Recording slot effects validation — 0.2.0-beta.6

> Historical validation record. Counts, versions, measurements, and limitations below describe the builds tested at that time; they are not a fresh qualification of every current build. See the [documentation index](README.md) for current setup guides.

Validated locally on Windows on 2026-10-22.

- `./build.ps1 -Verify -WithLoupedeck` passed formatting, all 173 workspace tests (seven opt-in tests ignored), Clippy with warnings denied, optimized desktop and 32-bit bridge builds, and official SDK package verification.
- Audio regressions verify wet preview/send output, dry rolling capture and live passthrough, immutable takes, live parameter changes, individual and whole-rack bypass, rack removal, independent delay history and bypass-history reset.
- File-worker regressions encode/decode cropped WAV and MP3 exports with effects, individual effect bypass and whole-rack bypass. Fresh delay history and a partial final block preserve the exact selected sample count before encoding. MP3 duration assertions allow encoder padding.
- UI regressions add a recording effect through the real add menu, exercise rack and effect shortcuts, verify slot shortcut-page scoping, and reject actions for removed effects. Existing crop-edit, recorder-wide stop and deferred-playback cancellation tests continue passing.
- Native Windows shortcut validation registered/delivered 105 mappings through WM_HOTKEY, including all recording-rack and individual-effect actions. Removing a recorder effect releases its OS key for reuse. No user keystrokes were synthesized.
- The native UI smoke test rendered 69 views, including recording racks and their keybinds.
- `recording_check` measured zero rendering allocations with four recording nodes, twenty 60-second slots and slot gain/delay processing. One shared history uses 23,414,016 bytes; this run used 0.325% of the realtime CPU budget. `rack_processors_check` also confirmed zero processing allocations for all ten voice-change effects. These are synthetic measurements, not physical-device latency guarantees.
- The existing Loupedeck integration suite passed real SDK construction and simulated callbacks against the native named-pipe host, including reconnect, dynamic slots, knobs, resets, generation changes and bounded queues. Physical controller testing is separate.
- ZIP integrity passed. Desktop/worker/bridge executable bytes, the verified plugin package and updated recording/controller guides match the portable directory and build inputs. PirateWave.exe matches the compiled release binary.

Playback and export retain the exact selection duration: delay/reverb tails beyond it are trimmed, and effect latency remains within that duration. Exports snapshot settings when requested; capture assets and waveforms remain dry with respect to the recording rack. Upstream processors still affect the captured signal.

Artifact: `dist/PirateWave-v0.2.0-beta.6-windows-x64.zip`

SHA-256: `fa026809baebe9fb4bf046e1ec3393fa8a96cb6d825de301f66178a27e72be24`

Detailed local reports and native screenshots are in the ignored `dist/recording-effects-*` and `dist/native-recording-*` files. These checks use synthetic recordings and native Windows facilities; no physical microphone or Loupedeck input was used.
