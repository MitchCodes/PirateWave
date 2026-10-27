# One-voice Voice Changer qualification

Historical ONNX qualification, October 20, 2026: **ONNX release blocked by state validation**. The current scope is one LLVC voice. Additional voices are deferred. Subsequent native LibTorch CPU work resolved reference equivalence without relaxing these limits; see [the current voice guide](VOICE_CHANGER.md) and [runtime decision](VOICE_RUNTIME_DECISION.md). This report preserves the ONNX investigation.

The plan explicitly requires keeping an unvalidated backend unavailable and retaining the prototype if validation fails. Both CPU and DirectML fail the specified state limits at intermediate chunks, with both 13 ms and 26 ms exports. Consequently this change delivers expanded qualification tools and native reproductions, **not** a production Voice Changer node, Voices page, or updated PirateWave application package.

## What was implemented

- Per-chunk comparisons of encoder, decoder, synthesis, and prenet state.
- Separate identical-reference-state inference and free-running recurrent inference.
- Checks for finite values, state shape/order, exact zero-state reset and cloned-input equivalence.
- Native Rust loading of captured reference input states, allowing an individual failing step to be reproduced without Python.
- Tensor-normalized RMS/peak metrics alongside the original per-element diagnostic; old measurements remain preserved.
- CPU optimization on/off and PyTorch MKL-DNN on/off investigations.
- Intermediate operator tracing against matching upstream modules.
- An experimental centered-variance LayerNorm exporter, evaluated against the unchanged upstream reference.
- Fixed 13 ms and 26 ms exports, each checked against upstream `infer_stream`; sequential native CPU one/two-thread and DirectML benchmarks.
- A repeatable qualification script, compact report index with hashes, and tests of the Rust acceptance metrics.

No game assets, model training, additional speaker substitutes, or live microphone capture were used.

## Blocking test

The diagnostic input is the upstream `174-50561-0000` speech recording multiplied by 0.02, the existing `quiet_speech` fixture. It lasts approximately 4.03 seconds after chunk padding. Both implementations start with zero state. Every chunk is checked, not just the final state.

Required state error limits are:

- RMS difference / max(1, reference RMS) <= **0.0001**
- Maximum difference / max(1, reference peak) <= **0.001**

Worst **decoder-state** measurements across the free-running quiet-speech stream:

| Chunk | Backend | Normalized RMS | Normalized maximum | First failing chunk (zero-based) |
| --- | --- | ---: | ---: | ---: |
| 13 ms | CPU | 0.001485 | 0.011792 | 21 |
| 13 ms | DirectML | 0.009245 | 0.074264 | 3 |
| 26 ms | CPU | 0.001034 | 0.007901 | 10 |
| 26 ms | DirectML | 0.009072 | 0.072575 | 1 |

These exceed the agreed limits. Waveform agreement remains close; that does not override a failing state criterion and is not a listening-quality judgment.

The native CPU reproduction for 13 ms chunk 39 receives the exact PyTorch input states. Its decoder normalized RMS error is **0.0001310134**, above 0.0001. Waveform maximum error for that step is only **7.94e-9**. This demonstrates why testing only audio or only the final hidden state can miss the agreed failure. The reproduction has one inference call and zero warm measurements; its timing must not be treated as a throughput benchmark.

## Investigation and limits of the diagnosis

The tensor contract is checked against the ONNX session, including shapes and order. Reference states are copied **before** the upstream call mutates its encoder/decoder buffers. Identical-state tests supply those copies to ONNX; recurrent tests feed back ONNX's own outputs. The native runner reproduces the failure from saved float32 tensors, ruling out Python-only state exchange as the explanation.

Export/reference preparation retains upstream's 32 context samples and 16-sample offline shift, with direct comparison to its `infer_stream` output. Both backends receive identical prepared audio. These tests do not constitute a causal microphone adapter, and no live microphone samples are discarded.

At 13 ms CPU chunk 39, intermediate tracing shows small differences increasing through the dilated encoder's convolution/normalization layers. Among the traced operators, the final encoder block's pointwise convolution exceeds the tensor RMS limit before the decoder attention computation. The encoder-to-decoder memory projection carries that difference into decoder state. This is evidence of numerical amplification upstream of attention, not evidence that state tensors are swapped.

Disabling all ONNX graph optimizations produces essentially the same failure. Disabling PyTorch MKL-DNN also leaves the observed results unchanged. An alternative LayerNorm export using explicit centered variance does **not** fix the issue; the 13 ms CPU free-running decoder normalized RMS reaches approximately 0.001507 and normalized maximum 0.012360.

No export or state-management repair has been demonstrated. The trace localizes the discrepancy, but it does not establish one particular low-level kernel as the sole cause. The tensor tolerances have not been relaxed, and no backend has been declared equivalent or qualified.

## Native timing measurements

The target machine is an i9-10900K and RTX 4070 SUPER, with the matched ONNX Runtime DirectML 1.19.2 libraries. Each profile processes all 14 reference cases (about 170 seconds of audio), carrying state within each recording. Timings exclude the first 20 chunks per case and include input/state exchange and waveform checks.

| Chunk | Backend | Mean per chunk | p99 | Timing gate |
| --- | --- | ---: | ---: | --- |
| 13 ms | CPU, 1 thread | 7.14 ms | 8.38 ms | Fails mean <=6.5 ms |
| 13 ms | CPU, 2 threads | 6.85 ms | 8.48 ms | Fails mean <=6.5 ms |
| 13 ms | DirectML | 1.34 ms | 1.88 ms | Passes |
| 26 ms | CPU, 1 thread | 8.67 ms | 10.23 ms | Passes |
| 26 ms | CPU, 2 threads | 7.79 ms | 9.25 ms | Passes |
| 26 ms | DirectML | 1.47 ms | 2.10 ms | Passes |

The 13 ms one-thread row uses the isolated repeat (`hop1-cpu-1-isolated.json`); the initial run briefly overlapped a compiler check and remains preserved separately. All six profiles meet the whole-recording waveform limits: worst maximum error is 1.422e-5 and worst RMS error is 4.063e-7.

These are offline throughput measurements, not conversion-latency or game-impact measurements. All profiles still fail state validation, so none is eligible for Auto selection or live routing.

## Evidence and reproduction

The original feasibility snapshot remains in [VOICE_CONVERSION_FEASIBILITY.json](VOICE_CONVERSION_FEASIBILITY.json). Current evidence is indexed in [VOICE_CONVERSION_QUALIFICATION.json](VOICE_CONVERSION_QUALIFICATION.json), including full-report hashes, per-state maxima, first failures, native reproductions, and timing profiles.

Full generated evidence is under `dist/voice-feasibility/qualification-one-voice/`, with optimization/trace/export experiments under `dist/voice-feasibility/diagnostics/`. Model and fixture binaries remain ignored by Git. The original `hop1` reports are preserved; the new `hop2` directory holds the 26 ms export and reference fixtures.

Verification completed: three Rust metric tests, Rust formatting and Clippy with warnings denied, Python/PowerShell syntax checks, ONNX structural checks, upstream reference agreement, the six native timing profiles, and native reproduction of intermediate failures for both chunk sizes/backends. Expected qualification failures produce saved reports and nonzero exit codes.

See [probe instructions](../tools/voice-probe/README.md) for setup and `scripts/voice-feasibility/qualify.ps1` for the sequential diagnostic/benchmark matrix. The checkpoint, upstream source, runtime, and dependency pins remain those of the original feasibility report.

## Release requirements still outstanding

Resolve state acceptance before production integration. The separate inference worker, causal resampler/context adapter, bounded queues, generation handling, fail-silent graph integration, node controls, installation/preview workflow, tutorial, and portable application packaging remain unimplemented.

The 30-minute continuous qualification, one-second waveform windows throughout that run, integrated latency <=100 ms, allocation/isolation tests, user-operated game/Discord impact <=5%, and manual listening acceptance have not been performed or claimed. A four-second state failure already prevents qualification; running longer cannot erase that failure.
