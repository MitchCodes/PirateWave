# Voice Creation — Beta

PirateWave 0.2.0-beta.1 exposes local voice creation with optional embedded Python 3.12
CPU/CUDA tools. No system Python, pip, Conda, compiler or CUDA toolkit is needed.
CUDA needs a compatible installed NVIDIA driver. Other training backends remain
unavailable; the curated voice's live DirectML conversion is separate.

**Current quality is experimental.** User-trained LLVC voices can sound robotic or metallic and did not match the cleanliness of the pretrained voice in the recorded listening tests. This Beta exposes the training workflow with known quality limitations; those tests do not establish production-quality results.

## Create a voice

1. On Voices, install the matching optional LLVC voice pack. It supplies the
   shared, application-trusted native inference libraries.
2. Open **Create voice → Introduction**. Read the Beta limitations, choose CPU or
   CUDA, name the voice and create a persistent training project. Choose the
   workspace in Settings → Voice Training & Storage before creating the project.
3. Import WAV, FLAC, MP3 or Ogg, or record the selected raw microphone. Recordings
   bypass the effects graph. Imported audio is copied to application-owned WAV;
   originals are preserved. Imports are limited to 30 minutes per file, raw takes
   to five minutes. Make separate training, validation and test recordings.
   This Beta limits included training speech to eight minutes and included audio
   across all roles to thirty minutes. Use shorter takes to allow clip exclusions.
4. Review levels and quiet/clipped sections; audition only through a selected
   monitoring output. Exclude poor clips and assign their roles before training.
   Dataset edits lock when training starts. Create a new project to change data.
5. In **Tools/storage**, explicitly download or import the selected runtime and
   shared bundle. For offline installation, put all matching part files in one
   folder. The app checks hashes pinned into this build, combined archive hashes,
   file hashes, paths, installed sizes and disk headroom before execution.
6. In **Train**, start/resume the fixed Beta recipe: no-F0 RVC teacher (250
   updates), 100 generated source pairs and LLVC adaptation (1,000 updates).
   Heavy work pauses when audio starts; resume is explicit. A worker that misses
   the five-second pause deadline is terminated, retaining the prior checkpoint.
7. Compare held-out original, teacher, LLVC and native streaming samples through
   the monitoring output. Complete jobs can install immutable CPU voice revisions.
   Select the new voice on a Voice Changer node and route it to an output bus.

## Downloads, disk space and compatibility

The current GitHub release also provides **PirateWave-Voice-Training-CPU.zip**, containing the pinned CPU tools and shared assets separately from the app. Extract it into one folder, leave the `.part` files intact, and select that folder in Tools/storage for offline CPU installation. CUDA remains a separate in-app download.

Public portable builds and optional bundles are distributed through
[legacy voice downloads](https://github.com/MitchCodes/PatchAudio-Releases/releases).
The source repository remains private. Download all parts for the chosen backend
and shared bundle for offline import; do not extract individual part files.

| Optional bundle | Download bytes | Installed bytes |
|---|---:|---:|
| CPU training tools | 408,081,670 | 1,626,470,496 |
| CUDA training tools | 2,786,567,626 | 5,093,004,744 |
| Shared models and licensed speech | 730,167,404 | 788,966,452 |

Choose one training backend plus shared assets. GitHub downloads use parts no
larger than 1 GiB, with the identical offline parts accepted by the app. The
embedded catalog is trusted with the executable; remote catalogs cannot replace
its pinned hashes. Windows curl performs resumable HTTPS transfers. No drivers
are installed and CUDA never silently falls back to CPU.

The fixed recipe reserves an estimated 6 GiB of additional job growth for latest
and previous checkpoints, atomic-save staging, generated pairs and exports,
plus the greater of 2 GiB or 10% headroom. This is separate from tools, download
caches and existing recordings. Imported copies and raw recording also check
free space before writing.

Archive parts, reconstruction, extraction staging, imported copies and generated
job data all consume space. Job growth is provisionally budgeted at 6 GiB plus
recordings; space checks include the greater of 2 GiB or 10% headroom and run
again during work. The wizard reports exact catalog sizes and labels job growth
as an estimate. Workspace changes apply only to new projects. Installed tools
are immutable and active jobs retain their versions. Explicit cleanup can remove
generated pairs or checkpoints; checkpoint removal can eliminate resumability.
Original recordings and installed voices are preserved.

Custom voices use application-managed native workers and shared verified CPU
libraries. Model revisions contain model data, notices, provenance and validation
results; they do not contain executable code or training checkpoints. Missing
voices retain their project references. Custom DirectML inference is unavailable
until separately validated. Existing global shortcuts, one active stream,
failure silence and deliberate passthrough remain in effect.

## Beta qualification limits

Each generated model must pass strict waveform/state comparison and CPU timing
checks over a 30-second native stream before installation. These checks do not
establish thirty-minute stability, complete microphone-to-output latency, a
gaming-impact result or natural voice quality for that model. CPU qualification
is bound to the local processor and worker/model hashes. Clean-Windows testing
and broader hardware qualification remain outstanding. Training loss is not a
quality score. Recording requirements for better quality remain unmeasured; the
eight-minute Beta training limit is a practical limit, not a quality guarantee.

## Recording tips

Use one consistent speaker, microphone position and vocal character in a quiet,
non-echoing room. Record without music, reverb, voice changing or aggressive
cleanup. Avoid clipping and re-record noisy takes. Include varied vowels,
consonants, questions and natural expression rather than repeating one sentence.
Try: “Please bring the blue jacket before Thursday.” “Did you hear the quiet
footsteps outside?” Keep separate evaluation takes. The in-app scrollable guide
includes clean, clipped and echoing audio examples played only on your selected
monitoring output.

## Measured Beta workflow checks

The fixed recipe completed on CPU and CUDA using three licensed target takes,
separate training/validation/test roles, 250 teacher updates and 1,000 LLVC updates.
Both models passed 30-second native waveform/state comparisons, installation and
a 30-second run through PirateWave's actual live adapter with zero underruns.

| Measurement | CPU-trained model | CUDA-trained model |
|---|---:|---:|
| Native CPU mean inference | 6.58 ms | 6.57 ms |
| Native CPU p99 inference | 7.14 ms | 7.50 ms |
| Chunk duration | 26 ms | 26 ms |
| Live adapter underruns in 30 seconds | 0 | 0 |

These are checks on an i9-10900K; CUDA training used an RTX 4070 SUPER. Resumed
preparation/training runs took 76 minutes on CPU and 7.2 minutes on CUDA; initial
work before the pauses, tool integrity checks and export were additional. These
short-recording fixture measurements do not promise production training times.
CUDA peak PyTorch allocation was 2,630,142,464 bytes, excluding driver/context
overhead. RAM and complete disk peaks are not yet qualified.

CUDA pause also exercised the five-second timeout: PirateWave stopped the worker and
successfully resumed from teacher update 50. Full diagnostics and pinned hashes
are in `VOICE_TRAINING_BETA_VALIDATION.json`. Startup readiness was about 89 ms;
that is not a measured complete microphone-to-output latency.

The measured research history below is retained for transparency. References to
the former availability gate describe earlier builds, not current Beta availability.

## Implementation progress: embedded runtimes and held-out review

### LLVC quality investigation

The listening reviewer ranked 1462 above 1673 and 1272, with artificial/robotic
sound still objectionable. A matched-utterance stage review then identified the
teacher as acceptable and LLVC as the stage that makes the sound worse. This is
not final voice acceptance; the underlying cause is still under investigation.

The first duration experiment holds the no-F0 teacher, target recordings, 100
paired source clips and pretrained initialization fixed, increasing target 1462
from 500 to 1,000 LLVC updates. Training completed on embedded CUDA in 290.86
seconds with 2,091,064,832 bytes peak PyTorch allocated GPU memory. This excludes
other GPU allocations and is not a hardware minimum or production time estimate.
The 5,000/20,000 budgets remain contingent on validation and listening improvement.

The 1,000-update candidate passed 30.004 seconds of strict native waveform/state
comparison and exact diagnostic/normal-protocol parity. Audio-only native mean
was 6.73 ms, p99 10.98 ms per 26 ms chunk. This is not a thirty-minute stability
or integrated latency qualification. All 100 saved training pairs and held-out
original/teacher clips are byte-identical to the 500-update baseline. The duration
review received "About the same" feedback. The unchanged recipe will not advance
to 5,000 updates; numerical parity does not establish quality.

A source audit found a substantial context difference: the original
prototype crops to 10,240 samples (0.64 seconds) before calling the generator.
The pinned upstream dataset supplies 65,536 samples (4.096 seconds) to the
generator and crops only the discriminator's training view to 10,240 samples.
Upstream generator losses use the longer output. The prototype also differs in
batch size (1 versus 9), optimizer settings and scheduler cadence. These are
verified implementation differences, not established causes of the artifacts.
The controlled context experiment retains the teacher, pairs, initialization and
1,000-update budget. Do not copy upstream's
discriminator argument ordering blindly: the current probe deliberately preserves
consistent real/fake labels.

The longer-context experiment passed a two-update calibration, then resumed to
1,000 updates. The resumed invocation took 305.90 seconds, plus 33.84 seconds for
calibration, with 3,413,211,648 bytes peak PyTorch GPU allocation (not total VRAM).
The generator and its losses see 65,536 samples; the discriminator update still
sees 10,240 samples. Of 100 clips, 23 need padding, accounting for 5.32% of the
context windows. This tests context, not the complete upstream recipe/batch size.

The new model passed 30.004 seconds of strict native waveform/state comparison
and exact diagnostic/normal-protocol parity. Native mean was 6.83 ms, p99 10.97 ms
per 26 ms chunk. All 100 training pairs and held-out original/teacher clips match
the short-context 1,000-update baseline byte-for-byte. Listening feedback was
"About the same": the longer context did not establish a perceptual improvement
at this budget. Neither tested change is accepted as a quality fix. Further
duration escalation remains gated. This is not release qualification. Both candidate reports and the
calibration are retained in `dist/voice-training/llvc-context`.

Developer tools now support independent teacher/LLVC budgets, preserve NumPy RNG
state and a previous checkpoint, and avoid advancing the learning-rate scheduler
when stopping between its intervals. A regression check verifies identical model
weights and random state after split versus uninterrupted training. This uses a
new recipe version; it does not silently resume older experimental jobs.

The untouched pinned pretrained checkpoint has now been evaluated on the same
three held-out utterances as all 1462 candidates. It passed 30.004 seconds of
native waveform/state checks, with exact diagnostic/normal-protocol parity.
Native mean was 6.66 ms and p99 10.35 ms per 26 ms chunk. A comparison of model
weights confirms the trained checkpoints differ from initialization (relative
weight L2 differences of 0.82%, 1.16%, and 1.58% for 500 updates, 1,000 short-context,
and 1,000 long-context updates). These differences do not measure voice quality.
`dist/voice-training/pretrained-review/review.html` provides offline and native
samples with identical source speech. The pretrained target identity differs;
listening should compare intelligibility and artifacts, not identity with 1462.
The reviewer reported "Pretrained is clearly cleaner". This points toward
degradation during adaptation; it does not establish which recipe component
causes it, and the different target identities remain a confound. Next diagnostics
should examine paired-audio alignment and adversarial optimization (including
the randomly initialized discriminator) before escalating training duration.
No additional training was run for this diagnostic and no candidate was installed.

### Pair timing and adversarial diagnostics

Read-only checks examined all 100 training pairs and six held-out teacher pairs.
Three-band energy-envelope correlation (5 ms resolution, +/-200 ms search)
reported equal lengths, no clipped teacher samples, median correlation 0.949,
median delay 0 ms and 10th–90th percentile delay 0–0 ms. Individual best lags
were 0 or 5 ms. This rules out an obvious broad offset in this sample, not
phoneme-level timing differences. No training audio was shifted.

The loss audit confirms real teacher audio is labeled real and generated audio
is labeled fake. The initial LLVC checkpoint contains no discriminator weights;
the probe initializes its discriminator independently. Evaluation-mode gradient
checks on three held-out center segments found that adversarial plus feature
matching terms were much smaller than content plus spectral terms. Median GAN
to supervised parameter-gradient norm ratios were 0.23% for the pretrained
generator with a newly seeded discriminator, and 1.82% for the adapted generator
with its trained discriminator. Output-gradient ratios were also small. These
measurements precede clipping and Adam preconditioning, and do not establish
perceptual causation or cover every training step.

The evidence does not support an overpowering discriminator or a broad pair
timing offset. A controlled discriminator warm-up with the generator frozen is
the next candidate experiment; duration/context changes remain unaccepted.
Diagnostic scripts and JSON reports are reproducible under
`dist/voice-training/adaptation-diagnostics`. Training remains unavailable.

### Discriminator warm-up experiment

Target 1462 completed 500 discriminator-only updates followed by 1,000 joint
updates, with the short 10,240-sample context, teacher, data and initialization
held fixed. The generator was frozen in evaluation mode during warm-up, and its
state was verified bit-identical afterward. Joint training started with fresh
optimizers and the pre-warm-up random stream restored; only discriminator weights
carried forward. Tests also verify deterministic interruption/resumption of the
warm-up stage. This is a specified experiment, not an upstream recipe claim.

The run took 389.74 seconds including 103.25 seconds of warm-up and reached
2,091,648,512 bytes peak PyTorch GPU allocation. Native comparison passed all
strict waveform/state checks over 30.004 seconds, with exact diagnostic/normal
protocol parity, mean 6.91 ms and p99 11.01 ms per 26 ms chunk. All 100 training
pairs and held-out original/teacher clips match the baseline byte-for-byte.
The listening page is `dist/voice-training/llvc-warmup/review.html`; the reviewer
reported "About the same". Warm-up did not establish a perceptual improvement
under these conditions and is not selected for the release recipe. No model was installed or enabled, and these short checks do
not replace release qualification.

These changes are developer research tools. The distributed runtime candidates
have not been rebuilt with them, and the application training gate remains closed.

The next implementation pass adds deterministic training/validation/test splits,
reserving three validation and three test source speakers. Target recordings also
have separate held-out clips. Dataset manifests record roles, source hashes and
speaker identities; they become part of checkpoint compatibility.

Both no-F0 and pitch-conditioned RVC teachers can now be exercised. The latter
uses pinned RVC F0 weights and a documented pYIN extractor. `--teacher-only`
stops before LLVC training for listening review. The three-target comparison
harness writes a local review page with target references, original speech,
teacher outputs, monitoring-device selection where the browser supports it, and
exportable listening notes. These short experiments do not establish convergence.

Actual embedded Python 3.12 CPU and CUDA candidate bundles have been assembled.
Both completed teacher and LLVC optimizer updates, and each resulting model
passed a 30-second waveform/state comparison against PirateWave's native worker.
Bundle integrity and imports were checked with system Python/Conda paths removed.
This was on the development machine, **not a clean Windows qualification**.

The original frozen HuBERT content extractor is exported at build time. Its
outputs and input gradients match the original exactly at three tested lengths,
allowing training without importing legacy fairseq in the embedded runtime.
This preserves the content loss rather than replacing or removing it.

Unicode-path testing exposed a LibTorch filename limitation. Training exports now
use opened file streams; a separately built candidate native CPU worker also opens
models through Rust's Windows-aware file API. Its short Unicode-path parity check
is recorded separately. The installed curated worker has not been replaced;
promoting the candidate worker requires full requalification.

See [VOICE_TRAINING_PROGRESS.json](VOICE_TRAINING_PROGRESS.json) for candidate
archive sizes/checksums and measured results. Candidate archives are developer
artifacts under `dist/voice-training`; they are not published or accepted by the
application installer. Sizes exclude shared weights, source speech and job data.
Short training/export checks were run during other development work and are not
release training-time, memory-minimum or latency benchmarks.

**Still unimplemented:** the Rust job service and live-audio pause integration,
full recording/import/review wizard, application runtime installation/cleanup,
and custom voice registry. Accepted listening, a converged recipe, personal trial,
clean-Windows testing and long-run qualification remain release blockers. The
existing application availability gate has not been relaxed.

## In PirateWave

Open **Voices → Create voice…** for availability, recording recommendations and
the scrollable **Voice Creation Guide**. **Settings → Voice Training & Storage**
lets you choose the future workspace and explicitly measure its logical file
sizes and the drive's available space. Scans run on a background thread, skip
links/junctions and never delete files. Changing the preference does not move
existing files. Preferences are stored separately from mixer projects.

No training tools or supporting models download from the application. Start
training is disabled. Download/installed/peak estimates remain labelled unknown
until a recipe and distributable runtime are measured; zero existing workspace
usage must not be confused with a zero-size training installation. Existing live
voice packs are stored separately from this workspace.

## Preparing good recordings

- Use one consistent speaker/character and recordings you have permission to use.
- Choose a quiet room with little echo. Keep microphone position and level steady.
- Record dry speech without music, reverb, voice conversion or aggressive cleanup.
- Avoid clipping. Listen to originals; re-record damaged or noisy sections.
- Include varied words, consonants, vowels, questions and natural expression.
- Keep separate evaluation clips. Test conversion on input speakers not used in
  training; a decreasing loss does not establish voice quality.

**10–20 minutes is a provisional collection target, not a validated LLVC minimum.**
The application guide includes practice prompts and troubleshooting. The current
prototype is a research experiment, not a promise of instant cloning from a short
clip. It learns a teacher voice, makes paired speech, then trains a separate live
LLVC voice. [Upstream LLVC pipeline](https://github.com/KoeAI/LLVC#dataset).

## Measured prototype

See [VOICE_TRAINING_FEASIBILITY.json](VOICE_TRAINING_FEASIBILITY.json) for the historical prototype's pinned
sources, asset hashes, actual runs and remaining gates. Developer scripts are in
`scripts/voice-training`; their environments and large artifacts are excluded from
Git and from the portable PirateWave package.

- Real RVC v1 no-f0 teacher training and LLVC training run on CPU and CUDA, using
  licensed LibriSpeech speech, pretrained initialization and actual optimizers.
- The LLVC architecture and original HuBERT convolutional content objective are
  preserved. No pitch-shifting replacement or dropped content loss is used.
- Atomic checkpoints contain both networks/optimizers, schedulers and random
  generator state. Regression checks exercise restoration and failed saves.
- A trained generator exports to the fixed streaming TorchScript contract and is
  compared against the actual existing Rust worker, including intermediate states.
- This does not establish a good voice, a minimum dataset, convergence, long-run
  performance or a portable dependency installation. Prototype loss values are
  diagnostic measurements only.

DirectML **training** is a different runtime from PirateWave's working ONNX Runtime
DirectML **inference** worker. Its prototype fails on required complex-valued
spectrogram operations; the native failure is contained by a separate supervisor.
The Python 3.9 developer environment also needs a separately documented package
compatibility patch. Neither environment is a published training bundle.

Intel XPU training cannot be qualified on the available NVIDIA-only GPU machine.
AMD ROCm 7.2.1's native Windows documentation lists no ML training support; that
version is not offered as a training backend. This is version-specific, not a
claim about all future AMD runtimes.
[AMD's Windows limitations](https://rocm.docs.amd.com/projects/radeon-ryzen/en/docs-7.2.1/docs/limitations/limitationsrad.html#windows).

## What must pass before training is enabled

1. Validate a reduced, reproducible recipe with multiple permissioned target
   voices, held-out speakers, intelligible speech and acceptable vocal identity.
   Record listening decisions rather than inferring quality from training loss.
2. Qualify a supported embedded Python runtime with entirely prebuilt dependencies,
   notices and reproducible backend-specific bundles on clean Windows. The
   development venvs depend on system Python and **are not portable bundles**.
3. Establish runtime download/installed sizes, memory limits, training estimates
   and peak disk use. Provide resumable verified downloads and offline imports.
4. Implement the full Rust-supervised resumable wizard, raw recording/import and
   review, bounded storage/checkpoint cleanup, stage help and monitoring comparisons.
   Heavy preparation/training/export must pause while PirateWave audio runs.
5. Generalize the voice registry without weakening model/runtime compatibility
   checks. Qualify every new model/backend for strict state parity, latency and
   thirty-minute streaming stability before installing it for live use.

No prototype model replaces the existing curated voice, and no incomplete model
can be activated through the current pack installer. Broader hardware remains
unqualified until tested end-to-end on representative physical devices.
