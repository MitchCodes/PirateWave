# Native diagnostics

Run normal checks from the repository root:

```powershell
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo run -p piratewave-core --example custom_effect
cargo run --release --locked -p piratewave-core --example rack_processors_check
```

The last example reports effect processing allocations and CPU cost. Hardware diagnostics are opt-in: use an idle test setup and read each test's requirements first. Reports and screenshots are written under ignored `dist/`.

```powershell
./dist/PirateWave/PirateWave.exe --devices
./dist/PirateWave/PirateWave.exe --audio-self-test
./dist/PirateWave/PirateWave.exe --shortcut-self-test
./dist/PirateWave/PirateWave.exe --transport-self-test
./dist/PirateWave/PirateWave.exe --ui-smoke-test
./dist/PirateWave/PirateWave.exe --cable-self-test
./dist/PirateWave/PirateWave.exe --listen-self-test
./dist/PirateWave/PirateWave.exe --continuity-self-test
```

- Device reporting enumerates installed endpoints.
- The audio test opens default input/output streams without connecting the input to the output.
- Shortcut tests use native OS registration and message delivery; they do not synthesize user keystrokes.
- Transport tests check real-engine start, stop, and toggle with silent outputs.
- UI smoke tests render native views without starting audio or saving workspace/settings changes.
- Cable, listening, and continuity tests require existing virtual audio endpoints and send test audio. Listening/continuity checks use both VB-Audio CABLE Input and Line 1 playback endpoints. They preserve Windows device settings.

Additional ignored Rust tests document hardware requirements in their source. See [Audio recovery](AUDIO_RECOVERY.md) and [Device discovery memory](DEVICE_DISCOVERY_MEMORY.md) for targeted checks. Hosted CI does not establish physical-device, native desktop, or overnight reliability qualification.
