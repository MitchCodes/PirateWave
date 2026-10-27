# Voice Changer — live one-voice Beta

PirateWave now includes a dedicated Voice Changer node, optional voice installation,
monitor-only previews, node controls, recorded shortcuts and beginner help.
**Live CPU conversion is enabled in normal launches** on the validated processor after installing the voice pack. No special launcher is required.
An enabled unqualified node produces silence; explicit passthrough sends the
original input. Existing audio routes and effects work normally.

## Runtime and validation

The CPU implementation uses a separate **Rust worker with native LibTorch 2.5.1**,
through pinned `tch` 0.18.1. It does not invoke or distribute Python. This changes
the planned ONNX runtime: the original ONNX path still fails state acceptance.
See [the runtime decision](VOICE_RUNTIME_DECISION.md).

The 26 ms native CPU profile passes all 14 original reference recordings.
Separately, the saved/reloaded model passed a continuous 30-minute comparison
against the original PyTorch model with both using two threads, checking every
state tensor and 1,800 one-second waveform windows. Those errors were zero. Two-thread native inference measured 8.96 ms mean and 9.82 ms p99 on
the i9-10900K. A repeated benchmark under concurrent validation load measured
10.56 ms mean and 15.94 ms p99, also passing. The 13 ms native profile failed its 6.5 ms mean-time requirement.
The optional GPU profile uses a **DirectML transformer + CPU encoder/synthesis**
on the qualified RTX 4070 SUPER. The full-model ONNX path remains unavailable. The hybrid worker passed a 30-minute
state comparison and a separate 30-minute single-stream live run with zero
underruns, stable memory, 89.6 ms first-ready time and a maximum inference/IPC
wait of 20.6 ms. Its earlier native benchmark measured 9.2 ms mean and 14.3 ms
p99 including IPC and transfers. Independent concurrent conversion workers
caused recoveries in stress tests; keep other GPU voice-conversion tools stopped
when using this profile. Gaming impact still requires manual testing.

A developer-load test exposed one underrun with one reserved model hop. The
adapter now reserves two hops to cover Windows delivery/render bursts. That
configuration passed a 30-minute run with zero underruns, stable private memory
and 88.3 ms to first converted output. The final worker's repeat also passed
30 minutes with zero underruns and 89.5 ms first-ready time;
see VOICE_CHANGER_VALIDATION.json for exact binaries and results. Software
first-ready time is distinct from complete microphone-to-output latency.

An independent SciPy/upstream-reference comparison also verifies the causal
resampling, context alignment and output fade. The actual two-thread worker has
small, acceptable rounding differences from a one-thread reference; a matching
two-thread quiet-speech reference is exactly equal. State tolerances are unchanged.

## Install and audition

1. Open **Voices** and select **Install voice pack**.
2. Choose the matching `PirateWave-LLVC-Voice-v1.zip` supplied with this PirateWave build.
3. Select headphones or speakers under **Preview through**, then play the preview.
   Preview opens a separate device stream and is never inserted into mixer nodes.
4. Read **Help** for routing and monitoring guidance.

Installation verifies the exact file set, sizes and SHA-256 checksums against the
manifest embedded in PirateWave. The pack contains the model, native helper, LibTorch and app-local Microsoft C++ runtime DLLs,
four preview clips, original configuration, tensor contract, qualification record and license notices. It is approximately 132 MB
compressed and roughly 350 MB installed. No additional voices are downloaded. Model/code notices and attribution for
[LibriSpeech preview sources (CC BY 4.0)](https://www.openslr.org/12) are included. The worker's app-local C++ DLLs do not
change PirateWave's existing host runtime requirements: a clean machine needs the
[Microsoft Visual C++ Redistributable](https://learn.microsoft.com/cpp/windows/latest-supported-vc-redist)
for PirateWave (x64), and x86 as well when using the 32-bit VST bridge. Python is not
required. Clean-machine acceptance includes checking these prerequisites.

To use live voice, start audio and route **microphone → cleanup processor → Voice
Changer → output bus**. Select Auto, CPU or GPU (DirectML + CPU), adjust output level, and use Restart
voice to recover. Enable/disable/toggle shortcuts start unbound and use the existing
recorder. Avoid sending both original and converted speech to the same bus.

Missing packs keep their saved identity and settings. Old projects without voice
configuration still load. One conversion runtime runs at a time. Voice nodes on
separate direct Switch outputs can stay enabled; the selected branch owns the
runtime. Switching voices reloads conversion and can briefly interrupt audio.
Other simultaneous enabled voice routes are rejected. Additional nodes can remain
in passthrough. Bypassed or disconnected nodes stop inference.

## Architecture

The graph exchanges preallocated 128-frame blocks with a supervisor through bounded
queues. The supervisor verifies packs, manages the child process, resamples and
performs IPC. The native worker owns loading, warm-up and stateful model inference.
Neither device callbacks nor the graph adapter load models, wait for inference,
join threads or destroy native sessions. A Windows job terminates the child if
PirateWave exits or crashes.

Conversion uses continuous 255-tap windowed-sinc filters for 48 kHz stereo to
16 kHz mono and back. The adapter retains all incoming samples; upstream's offline
16-sample shift is not implemented by dropping microphone data. Model context is
carried between hops. Generation/sequence tags reject old results after resets,
source changes or reconfiguration. Backlog age is limited to 250 ms; recovery
flushes the generation. One model stream feeds every connected destination.

## Remaining acceptance

Live Beta use is enabled at the user's request after automated CPU qualification.
Complete hardware latency, repeatable game/Discord comparison, manual listening
and clean-Windows acceptance remain unverified; the release acceptance record
stays false. `docs/VOICE_ACCEPTANCE.example.json` records those human checks.
Copy it to an ignored local folder before filling it in.

Launch PirateWave.exe normally for live conversion. The existing **Voice Qualification.cmd**
and `--voice-qualification` flag remain optional labels for manual test sessions;
they no longer unlock additional functionality. Missing/corrupt packs, unqualified
processors and unavailable GPU profiles still produce an explanation and silence.

For the game test, run three baseline captures and three captures with the native
pipeline qualification workload active. Keep PirateWave/Discord open in both conditions,
use the same scene and capture length, and warm up first. The development command is:

```powershell
$env:PIRATEWAVE_VOICE_TEST_PACK="$PWD/dist/voice-pack/llvc-f8312-v1"
$env:PIRATEWAVE_VOICE_TEST_SECONDS='180'
cargo test --release -p piratewave-desktop native_voice_pipeline -- --ignored --nocapture
```

This fixture runs the actual conversion/queue/resampling path without emitting
audio to devices or modifying user projects. Capture the game's frame times with
your existing benchmark/capture tool. Analyze CSVs using the explicit game process
and a frame-time column in milliseconds:

```powershell
python scripts/voice-feasibility/compare_frame_times.py --baseline off1.csv off2.csv off3.csv --enabled on1.csv on2.csv on3.csv --column MsBetweenPresents --process game.exe --out local/game-comparison.json
```

The report retains input hashes, per-run metrics, baseline variation and the
comparison method. A noisy baseline is inconclusive. Manual game activity and
listening have not been inferred from offline model throughput.

## Manual acceptance procedure

For listening, audition all four previews, then speak through the live Voice Changer using normal, quiet, loud and consonant-heavy speech. Include pauses,
whispers and a second speaker if available. Accept only intelligible speech
without persistent metallic artifacts or chopped syllables; record observations
in the local acceptance file. Reference equality does not establish sound quality.

For latency, use a synchronized multitrack recording or calibrated hardware
loopback to capture the microphone reference and returned monitoring output.
Measure repeated speech onsets first with explicit passthrough, then with CPU
conversion, keeping the devices, sample rates and buffering identical. Record
both the full active microphone-to-output delay and the increase over passthrough.
The added delay must be at most 100 ms. Document synchronization/calibration,
measurement method, devices, buffer settings and results. The software startup
reserve measurement is not a substitute for this hardware check.

On clean Windows, extract the portable package, satisfy its existing C++ runtime
prerequisites, install the optional pack, audition to a physical monitoring device,
and exercise live conversion without Python installed. Check old
projects, missing/corrupt-pack messages, bypass, restart and shortcuts. Record
Windows version and observations. No clean-machine or listening acceptance is
pre-filled by the build scripts.

## Development reproduction

Set `LIBTORCH` to the pinned wheel's `torch` directory and build the isolated
`tools/voice-worker/Cargo.toml` in `target/voice-torch-probe`. Then run
`scripts/voice-feasibility/build_pack.py` in the prepared development environment.
It builds the explicit-install archive and updates `src/voice/manifest.json`.
Rebuild PirateWave afterward so its embedded checksums match the archive. Development
Python is used for export, reference tests and packaging only. The pack builder
also uses the matching licensed Visual Studio redistributable CRT directory
(`--crt-dir` overrides its default). These DLLs are pinned against the runtime
libraries used during qualification. See [Microsoft deployment guidance](https://learn.microsoft.com/cpp/windows/redistributing-visual-cpp-files).

For DirectML, also build `tools/voice-dml-worker`, export the hybrid candidate
with `directml_hybrid.py`, and run the native state, case, protocol, benchmark
and 30-minute pipeline checks. Then run
`build_directml_pack.py --candidate dist/voice-feasibility/directml-hybrid-v2 --qualify`
and rebuild PirateWave. This extends the existing CPU pack without changing its
qualified model or worker. Reinstall this updated matching pack through Voices;
older installations lack the GPU models and runtime DLLs. Auto continues using
CPU, while explicit GPU requires the qualified RTX 4070 SUPER. Model warm-up and
GPU compilation happen before the node becomes ready.

After completing the manual record and all automated gates, rebuild the pack with
`build_pack.py --acceptance local/voice-acceptance.json`, then rebuild PirateWave.
The script checks model/CPU/worker identities, measured game results, latency,
listening, clean Windows acceptance, causal alignment and all continuous tests. Without that record it always
produces a live Beta with manual release acceptance still pending. The repository's example records no acceptance.
