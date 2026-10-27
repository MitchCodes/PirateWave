# Optional native voice worker

Rust executable using pinned `tch` 0.18.1 and native LibTorch 2.5.1 CPU. It loads
the pinned fixed-shape LLVC TorchScript export, warms the kernels, resets all four
states, then serves binary audio over private standard-input/output pipes.
It does not load Python or run inside device callbacks or the graph worker.

The process is installed only as part of the voice pack. PirateWave owns a Windows
kill-on-close job, a bounded audio adapter and the supervisor. Models are not
discovered or downloaded automatically. Do not replace the pinned model with an
arbitrary TorchScript file; a different adapter/profile requires qualification.

From the source checkout with the pinned development wheel prepared:

```powershell
$env:LIBTORCH=(Resolve-Path .tools/voice-feasibility/python/Lib/site-packages/torch).Path
cargo build --release --locked --manifest-path tools/voice-worker/Cargo.toml --target-dir target/voice-torch-probe
.tools/voice-feasibility/python/Scripts/python.exe scripts/voice-feasibility/build_pack.py
./build.ps1 -Verify
```

Finish any running worker tests before rebuilding the pack; Windows locks loaded
native files. Packaging updates the manifest embedded in PirateWave, so rebuild the
application after the pack. Current live routing depends on installed packs and automated runtime qualification; custom models have separate checks. See
[the voice guide](../../docs/VOICE_CHANGER.md) for qualification and manual acceptance.
