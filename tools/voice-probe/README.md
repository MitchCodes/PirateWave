# Voice conversion feasibility probe

This is a historical, gated feasibility prototype. It is an offline **Rust** streaming ONNX runner, not a production PirateWave node. It does not capture a microphone, change Windows devices, or transmit audio. It is separate from the current native LibTorch conversion runtime.

## Reproduce

Use Python 3.9 for the pinned development environment, Git, and Rust 1.88 or later. From the repository root:

```powershell
./scripts/voice-feasibility/run.ps1
```

The script prepares the pinned upstream checkout/checkpoint under ignored `.tools/voice-feasibility`, exports the network, generates independent PyTorch reference fixtures, compiles this isolated crate, and measures CPU with one/two threads and DirectML with one thread. It writes artifacts under ignored `dist/voice-feasibility`. Python is needed only for export/reference generation. The resulting Rust executable uses the bundled native ONNX Runtime and DirectML DLLs without Python.

For an already prepared environment, use `-SkipSetup`. Existing fixtures are preserved; use another `-OutputRoot` to regenerate them. `-ChunkFactor 2` or `4` exports a separate fixed-shape profile; never feed different-sized packets to an existing export. The upstream default hop is 208 samples at 16 kHz (13 ms), with 32 samples of context. The source's 16-sample input shift is preserved in the reference test; it is not silently treated as a production microphone adapter.

After preparing fixtures, run native inference directly:

```powershell
./dist/voice-feasibility/bin/piratewave-voice-probe.exe `
  ./dist/voice-feasibility/hop1 `
  "$PWD/dist/voice-feasibility/bin/onnxruntime.dll" `
  cpu 1 ./dist/voice-feasibility/hop1/cpu-1.json
```

Use `directml 1` for the GPU check. DXGI enumeration selects the NVIDIA adapter when present; the report names the actual adapter. DirectML session creation disables memory patterns and parallel execution and **disables CPU execution-provider fallback**. Unsupported GPU execution fails explicitly. Keep the matching `DirectML.dll` beside the executable: loading only `onnxruntime.dll` from another directory can resolve an incompatible system DirectML dependency.

## Outputs and acceptance

- `fixture.json`: revisions, checkpoint/config/ONNX hashes, exact tensor shapes, case sample counts, and reference dependency versions.
- `cpu-1.json`, `cpu-2.json`, `directml-1.json`: cold-load/first-chunk timing, warmed mean/p99/max, audio error, and final hidden-state error for each case. Logs retain failures before report creation.
- `*.cpu.wav`, `*.directml.wav`: float32 converted speech for listening review. These are generated only from the upstream public fixtures; they are not microphone recordings.
- Each case starts with fresh zero state; every subsequent call feeds the prior state outputs back as inputs. Inputs include ten speech fixtures, silence, quiet/loud speech, and a continuous concatenated multi-speaker stream. Reference preparation is checked against the upstream `infer_stream` function itself.

Waveform tolerance is maximum absolute error <= 1e-4 and RMS error <= 1e-5. Hidden states use `abs(actual-reference) <= 1e-4 + 1e-4 * abs(reference)` per element; reports also preserve raw maximum errors. These strict diagnostic thresholds are explicit and must not be confused with listening approval. Timing requires mean <= half the hop and p99 < one hop, excluding the first 20 chunks of each case. Time includes input preparation, state exchange and sample checks, but excludes file I/O. This is an offline throughput test, not a measurement of complete microphone latency or game impact. A nonzero exit indicates an unmet parity/timing check; inspect the saved report.

## Intermediate-state qualification

Prepare both `hop1` and `hop2` fixtures with `run.ps1 -ChunkFactor 1` and `run.ps1 -ChunkFactor 2`, then run:

```powershell
./scripts/voice-feasibility/qualify.ps1
```

Qualification writes to a new timestamped directory and refuses to reuse a report directory. It compares PyTorch and ONNX at **every chunk** of the quiet-speech fixture, with both identical reference input states and independent recurrent states. It reproduces the first failing identical-state step in Rust, then benchmarks all six chunk/backend/thread profiles sequentially. Do not run export, compilation, or other performance workloads alongside benchmarks.

Each state tensor must satisfy RMS error / max(1, reference RMS) <= 1e-4 and peak error / max(1, reference peak) <= 1e-3. The original per-element diagnostic remains in reports. The native runner adds `final_states` and `final_tensor_state_pass` without changing the legacy `parity_pass`. A single-step reproduction reports zero warm measurements and cannot pass timing qualification.

`diagnose_states.py` accepts `--optimization off|basic|all`, `--mkldnn on|off`, and `--trace-chunk N`. Its DirectML diagnostic defaults to DXGI adapter 0 (the RTX 4070 SUPER on the measured machine); use `--device-id` if your intended adapter has another index and compare with the native report's adapter name. Operator tracing exposes intermediate Conv/LayerNormalization/Gemm outputs and corresponding upstream module outputs. `--native-repro DIR` saves exact inputs and reference states for a failing step. `export_llvc.py --stable-layernorm` tests a centered-variance export; it is experimental and did **not** resolve acceptance failures.

`summarize_qualification.py --reports REPORT_DIR --diagnostics DIAGNOSTIC_DIR --out REPORT.json` creates a compact evidence index with hashes of full reports.

## Historical ONNX release gate

The ONNX investigation targeted **one** validated pretrained voice; additional voices were deferred. Both chunk sizes fail state acceptance on CPU and DirectML, including native Rust reproductions. See [the current report](../../docs/VOICE_CONVERSION_QUALIFICATION.md). Those were prototype-era blockers, not a description of current feature availability. The native LibTorch runtime now supplies production conversion and a completed reference comparison; local voice training has a separate optional embedded runtime. See [current voice support](../../docs/VOICE_CHANGER.md) and [Voice Creation](../../docs/VOICE_TRAINING.md) for remaining hardware and quality limits.

## Sources

- [LLVC source and streaming reference](https://github.com/KoeAI/LLVC/tree/1627c5d358cf9bb2b92b0ccc513d8b36807c923d)
- [Pinned model repository](https://huggingface.co/KoeAI/llvc_models/tree/ebfe8c0fdeb974a7eeb463b3abbc8ce42a0e3851)
- [DirectML session constraints](https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html)

Upstream's speech fixtures originate from LibriSpeech; retain upstream dataset attribution when sharing generated samples. This repository does not commit checkpoints, ONNX graphs, reference samples, or runtime DLLs. The model source, runtime binaries and respective notices remain separate from PirateWave's MIT source license.
