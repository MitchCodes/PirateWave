# Switches, effect shortcuts, logs and AGC validation

> Historical validation record. Counts, versions, measurements, and limitations below describe the builds tested at that time; they are not a fresh qualification of every current build. See the [documentation index](README.md) for current setup guides.

Build: 0.2.0-beta.3, Windows x64, 2026-10-21.

- `build.ps1 -Verify`: passed formatting, workspace/all-target tests (141 passed, 7 opt-in tests ignored), Clippy with warnings denied, release compilation, x86 VST helper compilation and portable packaging.
- Switch DSP tests: exactly one connected destination receives the mixed input; live changes select the other destination; nonconnected selections are rejected; active-route deletion selects the remaining route; route/destination deletion removes associated shortcut data; selection persists through serialization.
- Effect identity tests: existing projects receive IDs without losing settings; bindings persist by identity; same-type effects retain their own DSP history when reordered; removed effect identities ignore stale shortcut dispatches.
- UI interaction tests: switch dropdown selects a destination, switch keybind page lists connected destinations, effect actions still target the right instance after reordering/deletion, and Automatic Voice Correction appears in the expanded Voice Change Effects category.
- Native Windows integration: all 91 shortcut actions registered and delivered through the test's own WM_HOTKEY window. Per-effect enable/disable/toggle and both switch outputs are included. Removing an effect/route unregistered its keys, and the same chords were successfully registered for other actions. Existing recording suspension, conflict rollback, unbinding and node cleanup checks passed. No user keystrokes were synthesized.
- Logging: real native startup, shortcut registration and shutdown events were written to the configured `logs/piratewave.log`, including UTC timestamps, severity and PID. Unit tests verify timestamp formatting, rotation, error persistence, newline sanitization and final flush. Device callbacks and ordinary core DSP have no logging or filesystem calls. Only the bounded writer thread performs disk writes.
- AGC tests: both quiet and loud speech converge toward the target RMS, stereo ratios remain linked, silence/subfloor noise is not boosted, reset is clean, invalid parameters are rejected and irregular blocks produce identical output. Existing Compressor/Upward Compressor were reviewed and each controls only one direction, so a dedicated bidirectional AGC was added.
- Release processing harness: AGC made zero processing allocations and used 0.13% of a one-second realtime budget on this machine. Existing effects also passed zero-allocation checks. Timing varies with hardware/load.
- Native UI smoke: 60 views passed, including switch controls/creation, switch shortcuts, individual effect shortcuts, AGC and its help. Switch controls, effect shortcuts and AGC screenshots were visually inspected.
- Archive verification: ZIP CRCs passed; packaged executable, voice worker and x86 bridge match the portable folder; executable matches the release target; shipped documentation contains switch, AGC and logging instructions.

Artifacts: `dist/shortcut-self-test.txt`, `dist/rack-processors-beta3-performance.txt`, `dist/ui-smoke-result.txt`, `dist/native-switch-controls.png`, `dist/native-switch-keybinds.png`, `dist/native-individual-effect-keybinds.png`, `dist/native-automatic-gain-control.png`.

Portable ZIP SHA-256: `9037070e02d8dc9352165c520d2cdd8fbb2c4d3bc701dff57690e51be9e63fda`.
