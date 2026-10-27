# Voice runtime implementation decision

The ONNX export has not been repaired. Operator isolation using identical real
inputs found small arithmetic differences in convolution and normalization;
the encoder amplifies them before decoder-state storage. Float64 PyTorch itself
also exceeds the agreed hidden-state limits against float32 PyTorch. This
supports numerical sensitivity, rather than proving an ONNX semantic defect.

Experiments preserved under `dist/voice-feasibility/diagnostics/` include isolated
operators and a float64 oracle, normalization arithmetic variants, a precomputed
constant speaker embedding, and float64 normalization in ONNX. None makes the
ONNX export meet the agreed state limits. Tolerances and checkpoint weights have
not changed.

Options considered before changing the implementation:

1. Keep pursuing kernel-specific ONNX equivalence, leaving the feature blocked.
2. Change state acceptance criteria, which would change the agreed requirement.
3. Preserve the original kernels using native LibTorch/TorchScript through Rust.

The implementation uses option 3 for CPU. This changes the planned
inference runtime, while preserving Rust application code, original weights,
local inference, explicit state, and a shipped application without Python.
The full-model DirectML path remains unavailable because its ONNX qualification
still fails. The hybrid DirectML path described below preserves the sensitive
CPU kernels. No silent CPU fallback is permitted when GPU is explicitly requested.

The native Rust probe uses pinned `tch` 0.18.1 and LibTorch 2.5.1 CPU.
The 26 ms profile passes waveform and final-state tolerances on all 14 recordings.
Two CPU threads measured 8.96 ms mean and 9.82 ms p99. One thread measured 12.91 ms
mean and 14.31 ms p99. The 13 ms hop fails its mean-time headroom requirement.
The saved/reloaded model passed every intermediate state and waveform comparison
over 30 minutes with exact equality when reference and candidate both used two
threads. The actual worker differs slightly from the one-thread reference; the
quiet-speech diagnostic becomes exactly equal with a two-thread reference, and
skipping warm-up does not remove the one-thread difference. This identifies
thread-dependent native arithmetic rather than state ordering or warm-up history.
Single-step identical-state and free-running diagnostics both pass.
See VOICE_CHANGER_VALIDATION.json for final worker and integration evidence.

To keep the runtime optional and isolate crashes, the production
integration uses a separate Rust worker executable with its own native DLLs in
the explicitly installed voice pack. PirateWave communicates with it from a supervisor
thread; the graph exchanges only bounded audio blocks. The application itself
does not load LibTorch or invoke Python. Status must identify the actual runtime.

This CPU runtime increases installed pack size substantially (`torch_cpu.dll`
alone is about 249 MB). Model/runtime hashes and notices belong in the pack.
Automated model and integration results do not establish manual listening
acceptance, gaming performance, hardware latency or clean-machine compatibility.
At the user's request, normal launches enable live CPU Beta conversion after
automated qualification on the validated processor. Manual release acceptance
remains false until those checks are attested. The optional --voice-qualification
flag labels test sessions; it does not change backend eligibility.


## Hybrid DirectML runtime

The all-ONNX encoder changes small convolution/normalization rounding errors,
which become amplified in the encoder residual stack. Quiet-speech output still
looked close, but the stored encoder and decoder inputs failed the original
state criteria. The fix retains the original LibTorch CPU prenet, encoder,
label/projection and synthesis kernels. Only the transformer decoder executes
through ONNX Runtime 1.19.2 DirectML. The checkpoint, streaming state layout,
context, causal resamplers and acceptance tolerances are unchanged. In this
one-layer checkpoint, decoder history caches the CPU projections before
attention. Keeping those projections on CPU prevents GPU attention rounding
from feeding back into that cached state. Both identical-state and recurrent
checks still compare every tensor; this is not a change to the error limits.

`tools/voice-dml-worker` is a separate Rust executable in the optional voice pack.
It uses two CPU threads and a sequential DirectML session with memory patterns,
idle spinning and CPU EP fallback disabled. It enumerates DXGI adapters, selects
the qualified RTX 4070 SUPER, warms the model, resets streaming state and uses
the existing private binary audio protocol. PirateWave's graph thread does no model
loading or inference. CPU and Auto continue using the original CPU worker.

The profile is explicitly called **DirectML transformer + CPU encoder/synthesis**;
it is not an all-GPU model. Its CPU and GPU components share the same streaming
lifecycle. A backend change restarts the generation; errors produce silence,
with deliberate passthrough available. Unsupported hardware does not secretly
switch to CPU. The measured GPU driver is 32.0.16.1656; other GPUs have not been
qualified. Manual game-impact, listening, hardware latency and clean-Windows
acceptance remain separate from the automated checks.

See DIRECTML_QUALIFICATION.json inside the pack and the combined
VOICE_CHANGER_VALIDATION.json for exact models, runtime libraries, worker hashes,
GPU execution witness, intermediate-state checks, performance and continuous-run
results. Reproduction: build the Rust helper against the pinned LibTorch wheel,
run `directml_hybrid.py`, the actual-worker state/protocol/case/benchmark probes,
and the ignored `native_voice_pipeline` with PIRATEWAVE_VOICE_TEST_BACKEND=directml.
Only `build_directml_pack.py --qualify` publishes a GPU-enabled pack, after its
required reports pass. [DirectML session requirements](https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html)
are enforced by both development and native inference sessions.


One deliberately contended qualification attempt recovered after approximately
215 seconds while separate state, fixture, benchmark and restart workers were
sharing the GPU. Its report is retained as
`diagnostics/directml-pipeline-contended.stdout` / `.stderr`; a separate sustained
run after the model-loading/restart tests is required for qualification. The
repeat runs one live pipeline alongside the offline state verifier; the final
fixture comparison was also finishing in the background at its start. This is not a passing game-impact test, and
concurrent GPU model compilation is not representative of PirateWave's enforced
one-stream lifecycle. The existing bounded-queue recovery behavior silences
only the converted route on starvation.


A further unscheduled run recovered at approximately 807 seconds. Its logs are
retained as `diagnostics/directml-pipeline-unscheduled.stdout` / `.stderr`.
The engine and voice supervisor now register their working threads with Windows
MMCSS Pro Audio, releasing registration when their session ends. This changes
thread scheduling only, without changing system settings, model math, error
limits or the audio reserve. Node diagnostics include maximum IPC/inference wait
and render-call gap so a recovery can be investigated rather than hidden by
increasing latency. The native integration fixture uses the same render-thread
registration. [Microsoft MMCSS documentation](https://learn.microsoft.com/en-us/windows/win32/procthread/multimedia-class-scheduler-service).


## Qualified single-stream result

After every other GPU conversion worker exited, the final one-stream profile
completed 1,800 seconds / 675,000 graph blocks with zero underruns and no resets
beyond initial configuration. First-ready time was 89.6405 ms; maximum
inference/IPC wait was 20.611 ms and maximum render gap was 20.84 ms. Worker
private memory stayed within a 6.6 MB range after startup (about 482 MB steady).
The separate native state check passed 1,800 one-second waveform windows, with
maximum waveform error 8.03e-7 and maximum one-second RMS error 5.01e-8. Encoder
and prenet state matched exactly; the other states remained inside the unchanged
limits. All 28 individual recurrent/single-step state checks and all 14 normal
protocol fixtures passed. The native helper benchmark, including GPU transfers
and IPC, measured 9.21 ms mean and 14.32 ms p99 for a 26 ms chunk.

This enables the **hybrid** DirectML Beta profile for the qualified machine.
It does not qualify concurrent GPU conversion sessions or game/Discord impact.
PirateWave runs one conversion runtime at a time, selecting the active direct Switch branch when multiple voice nodes are configured. Auto still prefers CPU,
and explicit GPU never silently falls back to a CPU decoder. Use CPU/Auto if
other GPU inference workloads cause recovery. The numerical fix is the retained
CPU encoder, not a relaxation of state criteria; MMCSS improves scheduling but
did not make competing inference streams stable within this reserve.
