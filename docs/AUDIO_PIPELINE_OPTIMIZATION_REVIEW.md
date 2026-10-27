# Audio pipeline optimization assessment

## Implementation completed 2026-10-24

The selected conservative CPU optimizations are implemented. The original
assessment below describes the pre-optimization code.

- Creative effects resolve parameters once per block; distortion output gain and
  vocoder envelope coefficients are computed once per block. Fixed vocoder band
  coefficients are initialized once; WahWah still updates its moving filter per
  sample. The original floating-point equations and operation order are retained.
- Session source/device, soundboard destination, and graph/output mappings are
  compiled outside rendering, refreshed after graph edits, device recovery, and
  panic recovery. Soundboard voice accumulation order is unchanged. The general
  Graph API retains its flexible input-ID lookup for arbitrary caller ordering.
- Recorder sync indexes histories by source/output, preserving shared histories,
  switch-target silence, and snapshot lifetime. Taps no longer scan all histories.
- Only relevant auxiliary buffers are cleared. Source blocks that are completely
  overwritten no longer receive a redundant clear; missing sources still clear.
- Graph fan-out borrows disjoint source/destination buffers instead of explicitly
  copying each routed block. Output queue writes also borrow their source blocks.
- Stable unity gain bypasses ramp arithmetic and multiplication, while still
  metering, sanitizing, and applying output clipping. Gain transitions retain the
  original ramp.

No effect fusion, safety-guard removal, resampling changes, queue-size changes, or
DSP approximations were introduced. Algorithmic latency remains unchanged.

### Validation and measurements

`cargo test --workspace --locked --quiet`: 240 tests passed; 7 existing tests
ignored. `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo fmt --all -- --check`, and `git diff --check` passed.

A test-only frozen copy of the original creative DSP verifies bit-for-bit sample
equality in debug and release builds at 8, 44.1, 48, 96, and 384 kHz, across
parameter extrema/edits, reset, varying block lengths, stereo noise, silence and
nonfinite input. Regression tests additionally exercise reordered/missing source
audio, backward-index fan-out, stream mapping refresh, and recording-history
resize, stop, and recreation. Existing routing, effects recovery, recording,
controller, ducking, and extension tests continue to pass.

Same-machine release microbenchmarks (percentage of available audio duration):

| Case | Before | After |
|---|---:|---:|
| Vocoder | 2.47% | 0.25% |
| Resonator | 0.27% | 0.15% |
| Distortion | 0.23% | 0.11% |
| Flanger | 0.43% | 0.39% |
| Chorus | 0.60% | 0.52% |
| Tremolo | 0.15% | 0.14% |
| Stereo | 0.16% | 0.12% |
| WahWah | 0.39% | 0.39% |
| Routing | 0.094963% | 0.093047% |
| Gain/delay/gate rack | 0.150544% | 0.144190% |
| Level ducking | 0.341098% | 0.334784% |
| Speech ducking | 0.779051% | 0.787763% |
| Rack with live tap | 0.149417% | 0.144980% |

Both release benchmarks report zero processing allocations. The graph benchmark
now also covers recording histories (0.106752%, zero render allocations; no
pre-change baseline for this new case). Small timing differences are subject to
measurement noise; these are synthetic runs, not hardware latency measurements
or promises about total application CPU. Native device listening and physical
controller qualification were not repeated for this change.

## Original assessment

Scope: source review of the current engine, graph, built-in DSP, recording chains,
CPAL queues, VST adapters and voice-worker adapter. Release microbenchmarks were
run locally. This is an assessment, not an implementation; no audio behavior was
changed. Timings below are observations on this machine, not promised speedups.

## Pipeline and latency

The engine uses stereo f32 frames at 48 kHz in 128-frame blocks (2.667 ms).
Capture callbacks convert/sanitize device samples and push bounded frame queues.
The engine's QueueReader resamples and corrects independent hardware clock drift.
The engine fills source blocks and mixes soundboard voices, then traverses a
validated topological graph. Each node receives its upstream mix, runs external
processing where applicable, runs enabled rack effects in order, handles special
ports, applies node gain/pan, sanitizes/meters, clips output buses, taps recording
histories, and adds outputs into downstream input accumulators. Output callbacks
consume another queue through a QueueReader and convert to the device format.

References: src/audio_io.rs (QueueReader, build_input, build_output),
src/engine.rs (Session::render, render_ready), crates/piratewave-core/src/graph.rs
(process_recording_impl), crates/piratewave-core/src/lib.rs.

A normal sequence of in-place effects does NOT add one block of latency per
effect. All process the same block before it is published. More effects consume
CPU time and reduce scheduling headroom; only effects with buffering/lookahead
add algorithmic signal delay.

TARGET_FRAMES is 2048: approximately 42.67 ms on each queue side. Input targets
scale with the device rate to retain that duration. Input plus output targets
therefore represent roughly 85.3 ms of nominal buffering, before device/backend
latency and route-specific DSP. This is not a measured end-to-end latency:
occupancy, callback bursts, startup and clock correction affect actual delay.
Output queues start with silence. The 8192-frame queue capacity is not the normal
latency target. Lowering BLOCK_SIZE alone will not remove these reserves.

There is an explicit regression test showing that the earlier 256-input /
1024-output reserve starves under simulated packet delivery. Do not simply reduce
these constants and call it an optimization. Native device stress and loopback
latency measurements are needed for a selectable low-latency mode.

Additional route-dependent delay includes about 20 ms for noise suppression,
1390 frames (28.96 ms) for automatic voice correction, optional Auto Ducking
lookahead, intentional delay effects, and third-party plugin behavior. The voice
converter is asynchronous and additionally primes its output queue (21 graph
blocks at startup) and uses a separate inference worker. Its total latency cannot
be inferred from graph block size alone.

## Measurements

Commands: cargo run -p piratewave-core --release --example builtin_graph_check --locked
and cargo run -p piratewave-core --release --example rack_processors_check --locked.

| Fixture | Percentage of real-time audio duration spent processing |
|---|---:|
| Simple routing graph | 0.109% |
| Gain/delay/gate rack graph | 0.169% |
| Same rack with live effect tap | 0.169% |
| Level ducking graph | 0.380% |
| Speech ducking graph | 0.842% |
| Individual vocoder | 2.84% |
| Individual noise suppression | 1.18% |
| Individual pitch | 0.96% |
| Individual chorus | 0.63% |
| Individual reverb | 0.40% |
| Individual distortion | 0.25% |

All measured fixtures reported zero processing/render allocations. These are
synthetic microbenchmarks, not whole-application CPU percentages or deadline-tail
measurements. They exclude device callbacks, IPC delays, recovery, command
handling, UI work and most scheduling effects. No before/after speedup has been
measured because no optimization was implemented. Source-level repeated work
may already be partly removed by the optimizer; profiling must confirm impact.

## Prioritized opportunities

### 1. Cache invariant DSP work in creative effects

In crates/piratewave-core/src/voice_effects.rs, VoiceEffect::value scans descriptors by
string. It is called repeatedly inside the sample loop. Distortion computes its
output gain with powf per sample. Vocoder computes band frequencies and envelope
coefficients within its sample processing; Band::run calculates trigonometric
filter coefficients for both speech and carrier bands. Vocoder band centers and
Q are fixed, so they are strong coefficient-caching candidates.

Resolve values into typed locals once per block, or cache derived values when a
parameter changes. Cache fixed vocoder band coefficients at construction. Retain
independent filter/envelope state for each channel and each band. WahWah sweeps
frequency per sample, so its coefficients cannot simply be made block-constant.
The shared creative processor also calculates a generic LFO and maintains delay
storage across kinds; specialize work by kind where that state is demonstrably
unused. Do not replace sine/tanh/interpolation with approximations in this phase.

Other candidates: EQ coefficients and compressor/gate constants in effects.rs
are recomputed per block. ThreeBand already updates coefficients on parameter
changes and processes its three filters inside one sample loop; use it as a
local design precedent. Expected benefit: lower CPU, no queue-latency reduction.

### 2. Remove graph searches from sample loops

Session::render loops over soundboard voices, every sample, every source block,
and then searches graph.nodes to determine whether the block is a soundboard.
Precompile the soundboard block indices when the graph/source configuration
changes. Preserve the existing voice summation order and distribution to every
soundboard node. This removes graph traversal from each sample without changing
DSP or summation arithmetic.

Likewise compile source/input and output/node bindings rather than repeatedly
searching node/stream vectors per block. Recorder::tap currently scans every
history for every tapped output; build an indexed tap-to-histories lookup during
Recorder::sync while preserving switch-target matching and shared histories.
These are especially useful as node/voice/recording counts grow.

### 3. Reduce unnecessary clears and copies

Graph::process_recording_impl clears five stereo block buffers for every node:
main, preview, send, priority and ducked. Ordinary nodes only need their main
buffer; recording and ducking nodes need their respective extra buffers. Clear
only buffers read by the compiled node/port paths. Source main buffers can be
assigned directly when an input block exists, with an explicit missing-input
silence path. Session::render also clears source blocks before fully overwriting
them from an input reader; retain clearing only for additive/missing sources.

Routing selects whole arrays by value and copies the main output into a local
array before fan-out. Borrow source and destination through disjoint node slices
where possible to avoid materializing per-edge block copies. A block is only
1024 bytes and compiler optimization may already eliminate some copies: inspect
optimized profiles before adding complexity. Do not replace fan-in addition with
assignment indiscriminately; mixing order, signed-zero/nonfinite behavior and
multiple connections must remain defined.

### 4. Specialize stable unity gain, keeping metering and safety

Every node runs gain interpolation, multiplies both channels, checks finiteness,
updates peak and optionally clips. Ordinary processor nodes target unity gain.
When current and target gains are exactly unity, avoid redundant ramp/multiply
operations while retaining the necessary metering/sanitation/output protection.
Do not skip an entire silent node: delay/reverb tails, oscillators, plugins and
stateful detectors may still produce output or need to advance state.

### 5. Improve scheduling and fault paths before reducing reserves

Engine::run drains commands before rendering and performs recovery, plugin
message pumping and status publication on the same worker. These may be outside
Session::render's measured duration yet still delay the next block. Bound control
work per audio deadline and prepare heavy work away from rendering where thread
ownership permits. Device/plugin APIs have thread-affinity constraints: do not
blindly move all operations to an arbitrary background thread.

The x86 bridge's process call synchronously writes and reads IPC, with a reported
500 ms response deadline. A stalled bridged plugin can therefore hold up the
shared render worker far longer than the 2.667 ms block duration. A bounded async
adapter could isolate this, but requires explicit latency, discontinuity,
parameter/MIDI ordering and recovery behavior. It is not a transparent loop
optimization. On plugin failure, Rack::process also formats/logs an error and
removes/drops the plugin on the rendering call path; defer reporting/destruction
to a safe control context while preserving immediate dry fallback.

Measure full render-to-render gaps, not only the current DSP load metric.

## Can effects be fused into one loop?

Yes for carefully selected native processors, but fewer loops does not by itself
mean lower latency or faster code. Each stereo block is only 1 KiB and normally
fits in cache. Per-sample dynamic dispatch across effects could be slower than
the current once-per-block trait calls. Use specialized compiled kernels rather
than a generic sample virtual call for every effect.

A first candidate is adjacent scalar gain/polarity operations. However, algebraic
fusion (multiplying gains first) changes floating-point rounding and intermediate
overflow behavior. Keeping operations in the same order per sample is closer to
the original, but still must preserve effect-level fault behavior and observation.

GuardedEffect currently copies dry audio, processes an effect, then scans the
whole output for invalid values. If any sample is invalid or processing panics,
it restores that effect's WHOLE input block and latches a fault. Later effects
then see that restored block. A single guard around a fused rack changes this
contract. A speculative sample pipeline that lets later stateful effects consume
bad samples cannot reproduce existing rollback without restoring their state.

Also preserve individual enable/disable/reset behavior, stable IDs, per-effect
before/after monitoring and parameter/keybind/controller updates. A monitored
effect should form a fusion boundary or provide exact intermediate taps. Keep
third-party VST and extension effects opaque, block-based boundaries. Their
planar/interleaved conversion passes are part of their API contract, not simply
redundant loops. Recording slots must keep separate effect histories; preview
and send already reuse one processed slot block, which is correct and efficient.

Recommendation: defer fusion until coefficient caching, compiled bindings and
buffer specialization have been profiled. Do not remove safety passes merely to
make a benchmark faster.

## Validation and rollout

Implement small independent changes, first targeting deterministic equivalence.
For cached constants/search removal/buffer specialization, compare full rendered
outputs against the current implementation, preferably bit-for-bit on the same
build/CPU. Exercise parameter automation, resets, bypass, effect reorder/removal,
missing inputs, fan-in/out, stereo phase opposition, recording preview/send,
controller commands and the live monitor. Include panic/NaN/Inf injection and
verify whole-block dry fallback plus later effect state. Keep tail behavior and
sample-count latency identical. Use tolerances only for explicitly justified
floating-point changes, with spectral/null tests and listening checks.

Expand benchmarks to realistic speech/music, 1/16/64 nodes, long chains, multiple
simultaneous sound pads, recording histories, and VST2/VST3/x86 plugins. Measure
p50/p95/p99/max block time, render gaps, underruns, dropped frames and allocation
counts under UI/controller traffic and device disconnect/reconnect.

Only after that should a separate latency project introduce selectable reserves,
measured device packet sizes, event-driven scheduling or adaptive buffering.
Validate physical loopback latency and dropout rates at 44.1/48/96/192 kHz,
independent clock drift and CPU contention. Keep the current conservative setting
as the default until those results justify a change.
