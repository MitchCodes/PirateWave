# Contributing to PirateWave

Windows x64 is the supported and tested desktop target. The device-independent `piratewave-core` crate can be developed independently of the desktop UI.

## Set up

Install stable Rust through [rustup](https://rustup.rs/) and Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK. The checked-in `rust-toolchain.toml` requests rustfmt and Clippy. Run commands from the repository root.

```powershell
cargo run --locked
./build.ps1 -Verify
```

The verification script checks formatting, runs the test suite, treats Clippy warnings as errors, and creates a portable Windows build with dependency notices. `Cargo.lock` is committed because this is an application. Include intentional lockfile changes when updating dependencies.

## Code layout

- `crates/piratewave-core`: DSP, effect catalog, routing validation, and project data.
- `src`: native UI, device I/O, playback, persistence, and OS integrations.
- `scripts`: packaging helpers.
- `design/prototype`: the original HTML interaction design; independent of the native app.
- `docs`: architecture and screenshots.

Read [the backend guide](docs/ARCHITECTURE.md) before adding a processor. Add numerical tests for changes to audio processing and routing. Avoid filesystem operations, locks, decoding, and allocations in normal device callback paths. Preserve processor state during ordinary parameter changes.

## Check a change

```powershell
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo run -p piratewave-core --example custom_effect
```

Device and native UI diagnostics are described in [Native diagnostics](docs/DEVELOPMENT.md). Run applicable checks locally when changing those components; hosted CI does not assume an audio device, virtual cable, or interactive desktop. Record measured results and any limitations in your pull request.

Do not commit imported sound files, personal studio configurations, secrets, generated binaries, or `target/` and `dist/`. Put local test projects/assets under ignored `local/`; synthetic test fixtures may be added deliberately with tests. Bug reports should include the app version, Windows version, reproduction steps, and relevant device names; recordings are not required.

GitHub CI validates pull requests and pushes and retains portable build artifacts. Main-branch release publication is opt-in and requires explicit configuration; see [Releasing](docs/RELEASING.md). Builds do not install drivers or controller profiles.
