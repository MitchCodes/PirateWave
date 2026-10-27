# Voice conversion: measured feasibility milestone

Historical first milestone on October 20, 2026: **the production integration gate did not pass**. This report and its JSON preserve the original three-voice investigation. The user subsequently selected a **one-voice** release; additional voices are no longer a prerequisite. The ONNX intermediate-state failures in [the historical qualification report](VOICE_CONVERSION_QUALIFICATION.md) blocked that prototype. The subsequent native LibTorch path supplies the current Voice Changer node and Voices page; see [current setup and limits](VOICE_CHANGER.md).

## Implemented

- A pinned LLVC checkout/checkpoint, functional stateful ONNX export, and structural ONNX validation. No game assets or model training were used.
- Reference generation from the original PyTorch model, including an independent comparison to upstream's own `infer_stream` function. Caller-visible state mutations are functionalized at the export boundary without editing upstream model math.
- A native Rust ONNX runner with four persistent state tensors, explicit state reset between recordings, sample checks, WAV outputs and machine-readable reports. Running the executable does not require Python.
- CPU one/two-thread profiles with sequential execution and idle spinning disabled; DirectML with sequential execution, memory patterns disabled and CPU-provider fallback disabled.
- DXGI hardware adapter identification, correct native DLL packaging, checksum rejection, pinned Rust dependencies, export/reproduction scripts, and compiler/linter checks.

## Measurements

Hardware: Intel i9-10900K (10 cores/20 logical processors), NVIDIA RTX 4070 SUPER, driver `32.0.16.1656`. Runtime: ONNX Runtime DirectML **1.19.2**. Export: PyTorch/torchaudio **2.5.1+cpu**, ONNX **1.16.2**, NumPy **1.23.5**, SpeechBrain **0.5.16**. Rust bindings: `ort`/`ort-sys` **2.0.0-rc.8**, matched to the runtime's API 19.

The export contains 2,860 ONNX nodes. Input packets are 240 samples: 208 new samples plus 32 context samples at 16 kHz. Each call produces 208 samples (13 ms). The four hidden-state tensors total 269,848 float32 elements. All state is carried forward between calls. The reference's 16-sample input shift, padding and context are preserved.

Fourteen cases cover ten upstream speech recordings, silence, quiet speech, loud speech, and a 107.5-second continuous concatenated speech stream. Each profile processes 13,077 chunks, approximately 170 seconds of audio. Timings exclude the first 20 chunks of each case and include input/state exchange and sample checks; they exclude disk I/O.

| Backend | Mean per 13 ms chunk | p99 | Maximum | Timing gate |
| --- | ---: | ---: | ---: | --- |
| CPU, 1 thread | 7.04 ms | 8.35 ms | 19.78 ms | Fails mean <= 6.5 ms |
| CPU, 2 threads | 6.72 ms | 8.34 ms | 21.81 ms | Fails mean <= 6.5 ms |
| DirectML, RTX 4070 SUPER | 1.31 ms | 1.81 ms | 12.28 ms | Passes |

CPU is faster than real time but falls slightly short of the agreed 2x average headroom. DirectML achieves approximately 10x offline throughput with CPU fallback disabled. Neither figure establishes complete microphone latency, GPU utilization, or game frame-time impact.

### Audio versus hidden-state agreement

Every case on every backend meets the waveform criteria: maximum sample error <= 1e-4, RMS error <= 1e-5. Across all cases, CPU maximum error is **1.384e-5**, DirectML maximum error **1.204e-5**, and worst RMS error is below **4.063e-7**. Outputs remain finite, and silence stays silent.

The stricter per-element hidden-state test **does not pass for all cases**. It requires `abs(actual-reference) <= 1e-4 + 1e-4 * abs(reference)`. CPU fails three cases and DirectML fails six, particularly on quiet speech. Maximum raw state differences are approximately 0.0123 (CPU) and 0.1513 (DirectML); hidden activations are not normalized waveform samples. These discrepancies are retained in the reports, not suppressed or declared inaudible. Their effect on longer use and the appropriate state-equivalence criteria need investigation before production acceptance. Close waveform agreement alone is not listening approval.

## Original preset asset gate (superseded by one-voice scope)

The pinned official model repository contains `llvc`, `llvc_hfg` and `llvc_nc` checkpoints. These are architecture/training variants for the same target, not three speaker presets: the upstream experiment configurations identify `f_8312_ls360`, and the model hardcodes its label input rather than exposing a speaker selector. An RVC checkpoint or the game's 192-value embedding is not interchangeable with LLVC.

The official model card labels its repository MIT, and the upstream code carries an MIT license. A bounded Hugging Face `llvc` model search returned the official repository and a separate `yenqkr/llvc_video` project; neither established three compatible, accepted identities with documented distribution terms. This is not a claim that no other voices exist anywhere.

**Original missing requirement:** two additional distinct, compatible pretrained target voices, plus listening acceptance of the library. Additional voices have since been deferred; this is no longer a release blocker. The one-voice release still requires state, performance, integration, and listening qualification.

## Reproduction and evidence

Use [the probe instructions](../tools/voice-probe/README.md) or run `./scripts/voice-feasibility/run.ps1` from the repository root. Generated model, fixtures, converted WAVs and raw results are under `dist/voice-feasibility/`; runtime binaries sit in its `bin/` folder. The JSON reports and logs intentionally report unmet checks with a nonzero native probe exit code.

The committed [measurement snapshot](VOICE_CONVERSION_FEASIBILITY.json) retains hardware identity, complete per-case results and these pins:

| Artifact | Identity |
| --- | --- |
| LLVC source commit | `1627c5d358cf9bb2b92b0ccc513d8b36807c923d` |
| Model repository revision | `ebfe8c0fdeb974a7eeb463b3abbc8ce42a0e3851` |
| Checkpoint SHA-256 | `cceb7ab9621f84d62d283725ae3281cacb04f762d6a088fe3e97c1a29d4b8c0e` |
| Configuration SHA-256 | `530f18a80fc1b233382801f3a8ba16ba13f4b8b412e9e4dde296c65e755f97ba` |
| Exported ONNX SHA-256 | `6fe0a3607d35a20bd4170afb6b1e69b4be4679c0f8f30eefeb454284235e400e` |
| ONNX Runtime DLL SHA-256 | `8702a9e02b0e4bc111ca64eb34fb6c9c863ef8ec8e4b21f1a8e4b792edbabf30` |

Validation completed: original streaming-reference agreement, ONNX checker, native CPU/DirectML runs across all 14 cases, malformed model-checksum rejection, Rust formatting and Clippy with warnings denied, and Python/PowerShell syntax checks.

Not completed or claimed: microphone-to-output integration/latency, subjective listening approval, three accepted voices, the 30-minute production streaming run, game/Discord frame-time comparisons, or a new portable PirateWave feature build. Those depend on passing the preceding gate. PirateWave's existing application executable, settings, audio routes and effects remain unchanged.

Sources: [pinned LLVC](https://github.com/KoeAI/LLVC/tree/1627c5d358cf9bb2b92b0ccc513d8b36807c923d), [pinned model repository](https://huggingface.co/KoeAI/llvc_models/tree/ebfe8c0fdeb974a7eeb463b3abbc8ce42a0e3851), [official LLVC comparison of variants](https://koeai.github.io/llvc-demo/), [DirectML constraints](https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html).
