# Automatic Voice Correction

A native Rust monophonic pitch-correction effect using pitch detection and read-head crossfading reconstructed through focused REA analysis of the supplied Auto-Tune Evo VST2 DLL. It runs without that DLL or a VST host. This reconstructs selected behavior; it is not a bit-identical port of the complete plugin.

## Use

In **Effects rack**, select a microphone or processor node, then **Add effect > Voice Change Effects > Automatic Voice Correction**. For an isolated vocal route, connect **Microphone → Effects processor → Output Bus**.

- Choose the song's **Key** and **Scale**: Major, natural Minor or Chromatic. Chromatic is useful without a known key.
- Select **Voice range**. Start with Alto / tenor for speech; Low male searches lower pitches.
- Lower **Retune time** for immediate snapping; raise it for gradual correction. **Humanize** slows correction on sustained notes.
- **Reference detune** adjusts tuning around A=440 Hz, in cents. **Transpose** shifts corrected notes by whole semitones, up to one octave either way.
- **Correction amount** adjusts how far pitch moves toward its target. **Wet mix** blends corrected and delayed original audio.

The processor adds a nominal **29 ms at 48 kHz** delay (1,390 samples, reduced from the previous 64 ms) in addition to device buffers. Internal dry/wet paths are aligned; parallel graph branches have no automatic latency compensation. Disabling gives immediate passthrough; re-enabling clears history. Existing processor Disable / Enable / Toggle keybinds work on a dedicated correction node.

Use one voice or a monophonic instrument. Chords, noise, vocal fry and large shifts can cause tracking/crossfade artifacts. This version does not preserve formants or reproduce graphical editing, microtonal/custom scales, vibrato generation, throat modeling or MIDI target-note mode. Voice presets now use the recovered acquisition filters and adaptive lag-band margins.

## Implementation and provenance

`crates/piratewave-core/src/voice_correction.rs` implements `AudioEffect`, registered as `automatic_voice_correction`. Parameters serialize through `EffectSpec`; the existing graph preserves histories on parameter changes and clears them on bypass. No project schema migration is needed.

Recovered behavior used in the native implementation:

- Period-domain target selection and midpoint thresholds. Standard major/minor interval arrays and small note-boundary hysteresis are our implementation choices.
- Ratio smoothing: `alpha = 0.5^(5000/(internalSampleRate*retuneMs))`, applied every five original internal samples, with zero retune giving alpha=0. Retune therefore describes a correction half-life in milliseconds. The native implementation now updates on the same five-sample cadence.
- Humanize interpolates from the initial coefficient toward the coefficient for `retuneMs+humanize` over 400 ms on the same target, resetting on target changes. The native implementation evaluates this on each recovered detector update.
- Detune ratio `2^(cents/1200)` and transpose range -12 to +12 semitones. The original normalized retune control maps to `round(400*(49^(1-x)-1)/48)` ms; our UI exposes milliseconds directly.
- Dual variable read heads and three-point quadratic interpolation.

The previous YIN detector and raised-cosine granular shifter have been replaced:

- `voice_correction/detector.rs` translates the recovered two-stage acquisition/adaptive detector. Acquisition applies the original input-type filters, decimates by eight, and searches rolling energy/correlation minima with harmonic-consistency tests. Adaptive tracking updates full-rate lag statistics and refines the period every five samples. Thresholds, input filters, band movement, lock loss and reacquisition follow the observed code.
- `voice_correction/shifter.rs` translates the original two read heads, 5,696-sample ring, nearest-integer/fractional positions, period-sized jumps, quadratic interpolation and **50-sample linear crossfade**. Cursor guards, reset positions and the 1,280-sample preparation delay were recovered explicitly.
- The four upstream anti-alias filters use the recovered RBJ coefficients/process equation and ten-percent ratio-change threshold. Their coefficient updates preserve history.

The native wrapper still supplies its own standard Major/Minor/Chromatic note tables, small note-boundary hysteresis, correction amount, stereo channel selection and wet/dry mixing. Full original scale-table quantization, vibrato/formant processing and optional modes have not been ported. The original reduces internal sample rates above 50 kHz; our fixed 48 kHz application path needs no such conversion, and behavior at other core sample rates is not claimed to reproduce that wrapper. These remaining differences can affect the complete effect's sound even when the two requested components match.

Buffers and scratch space are preallocated. `process` and `reset` do not allocate, lock or perform I/O. Stereo channels share the recovered read-head motion. Internal wet/dry alignment uses the initial 110-sample head offset plus the 1,280-sample preparation delay; pitch shifting introduces the original period-dependent cursor movement. This is not a promise of fixed phase alignment between shifted and untreated audio.

## REA evidence

Target SHA-256: `278d03b4a88fe6ef59b48e5c1f70bc921b3ae530ea672879ef2ae21f0ce873d5`. PE32 x86, preferred base `0x10000000`. REA 6.4.0 / Ghidra 12.1.4. Addresses are preferred virtual addresses. Decompiled pseudocode is not recovered original source or proof of execution.

| Finding | Address | REA evidence |
| --- | --- | --- |
| VST entry to effect allocation | `0x10022070` → `0x10008c50` | `ev_1e44d6ff7438df76096408bf8b86d201c5fbfa7c6e1f9bae07bdba0d06253ef0` |
| Normalized control mappings | `0x10004830` | `ev_fbb9377af3aa8a95af95588b66a7122867b1e5f8b4440ec953b26f49f95c1b5d` |
| Nonlinear retune curve | `0x1002b7d0` | `ev_81e7c32e3beba9321d6c03958faccef141702dc80a23b13c80e45bddf823999d` |
| Retune / Humanize coefficients | `0x1002e530` | `ev_110f53c9ab75dad1bf07d478db5bd080991634cb84bdfa213dd8608e32ae6811` |
| Floating-point constants | near `0x100393e0` | `ev_7cca85cf4453e1966edcac3a8d8b7e35d9abbe521b253187a81f2f8b350540b8` |
| Period tracking, smoothing and quadratic dual-head output | `0x1002f330` | `ev_7cd2df25ab812a73622ede13ca19e8f5d8baac78b4e9871aaa6d51d5c960e53f` |

Additional evidence from the deeper reconstruction:

| Finding | Address | REA evidence |
| --- | --- | --- |
| Acquisition and adaptive initialization | `0x1002db90`, `0x1002e8f0` | `ev_5648fe8046529f711843c28ee09efc9a6e70f0f61358f800042e71bf969b14ea` |
| Acquisition assembly and rounding | `0x1002db90` | `ev_4d1aaa3958fe2463a06b73c572efcbed483dab71a4bb84cf94b961fb69b9ea6c` |
| Input filters and lag margins | `0x1002eda0` | `ev_104adff2993034e30ca776066af2b54e4fff16e6ea3fb45d19519b343a07e32e` |
| Acquisition reset and adaptive entry | `0x1002d9f0`, `0x1002e440` | `ev_edd7dae1991a606e8d674a2ad078112275a28ea6b773ce5890fa3e6cf6b64ff1` |

- Read-head reset, `0x1002f030`: `ev_c9e3835b8eb3de43b3b1bf31b2e30f9b03aaafe0704d31e41760937c9140ab9f`.

- Preparation delay, `0x100318c0`: `ev_8cf77ca5c60b6a057326f1e85f82d7d2099b69b56f63aa1acec9ce44fdde312a`.

- Anti-alias coefficients, `0x100244b0`: `ev_7465b6d7ef7146e2334ad26d9cbc9c7a0d16375620887bc7b21aab658dbb8830`.

- Filter recurrence, `0x10035000`: `ev_ffbdd30a915bb9fadf188ca41d0b071c23fc7406a88b1a4c9da35342f11fa076`.

Humanize's 0.4 constant at VA `0x1003a9c8` was additionally confirmed by direct PE-byte reading, separate from REA. Scale preparation was traced at `0x1000ddd0`; full scale tables were not decoded. The detector and crossfade paths described above have now been reconstructed and tested. Full formant processing, optional modes and universal sample identity remain unestablished. This establishes selected controls and architecture, not complete sonic equivalence.

Raw evidence, pseudocode, control sweeps and reference measurements remain in ignored `dist/voice-correction-research/`; they are not packaged or checked in. The supplied DLL is unchanged and not redistributed. No licensing/protection routines were investigated or modified. Owned REA sessions were closed after analysis.

## Validation

The x86 `voice_trace` research oracle invokes the supplied DLL normally and reads its DSP state without writing it. Six one-second traces at 48 kHz cover harmonic pitch raising/lowering, a note transition, a pitch glide, a formant-shaped signal and noise. The latter two also exercise rejection/acquisition behavior; they do not establish successful voiced tracking in every preset.

Across all **288,000 samples**, the reconstructed detector matched the DLL's acquisition/tracking modes exactly; maximum observed period error was below **2e-12 samples**. Given the DLL's prepared audio and selected period/ratio controls, the reconstructed shifter matched output to below **1.5e-8** absolute sample amplitude for all six traces. This isolates the requested detector and crossfading components. It does **not** compare the entire native effect against the complete plugin sample-for-sample.

The opt-in Rust test `recovered_components_match_dll_trace` reproduces those comparisons. A separate committed unit test regenerates a known harmonic fixture and checks detector transitions/period checkpoints from the DLL, so ordinary CI needs no proprietary binary.

Offline 48 kHz stereo comparison against the supplied DLL, using two-second harmonic signals and positive zero crossings over the final second. Retune=0, Humanize=0. These measurements establish steady-state pitch agreement for these fixtures, not voice-quality equivalence.

| Case | Input Hz | DLL output Hz | Native output Hz | Native error cents |
| --- | ---: | ---: | ---: | ---: |
| Chromatic A | 430 | 439.978 | 439.991 | -0.04 |
| Chromatic B | 250 | 246.941 | 246.938 | -0.02 |
| C major | 307 | 293.660 | 293.661 | -0.02 |
| C minor | 307 | 311.122 | 311.123 | -0.02 |
| Transpose +7 | 430 | 659.227 | 659.246 | -0.02 |
| Detune +25 cents | 440 | 446.369 | 446.381 | -0.07 |

The release harness measured approximately 1.0–1.4% of the available real-time budget of one CPU core for one stereo instance on this machine, with zero processing allocations. Timing is machine-dependent, not a CI threshold.

Regressions cover harmonic tones, synthetic formant-shaped voice/note changes, low/high voices, rates 8–384 kHz, scale/key selection, timing laws, stereo polarity, silence/noise, reset, block-size independence, project serialization and routed bypass shortcuts.

Native checks need no DLL or audio device:

```powershell
cargo test -p piratewave-core voice_correction
cargo run --release -p piratewave-core --example voice_correction_check
```

Optional reference probes execute an explicitly supplied local DLL:

```powershell
./scripts/build-vst-bridge.ps1 -Profile release
./target/release/piratewave-vst-bridge32.exe --characterize "C:/path/to/Auto-Tune_Evo_VST.dll" control-mappings.json
./target/release/piratewave-vst-bridge32.exe --voice-reference "C:/path/to/Auto-Tune_Evo_VST.dll" reference-tones.json
```

The reference probes use observed parameter indices specific to this DLL.

Generate a read-only component trace (x86 only; the oracle validates the expected object/vtable profile and is not included in the portable app):

```powershell
cargo run --release -p piratewave-vst2-host --example voice_trace --target i686-pc-windows-msvc -- "C:/path/to/Auto-Tune_Evo_VST.dll" dist/voice-trace 430 0 harmonic
$env:PIRATEWAVE_VOICE_TRACE = (Resolve-Path dist/voice-trace/trace.csv).Path
cargo test -p piratewave-core recovered_components_match_dll_trace -- --ignored --nocapture
```

The oracle also accepts `transition`, `glide`, `vowel`, or `noise` as its last argument, and a negative transpose for the lowering branch. Raw DSP traces and generated audio stay in ignored `dist`; neither the DLL nor its decompiled listing is distributed.
