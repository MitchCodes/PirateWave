# Voice-training build and diagnostic tools

PirateWave 0.2.0-beta.1 now runs the fixed experimental recipe through the Rust-managed
Voice Creation wizard. CPU/CUDA tools are optional embedded Python 3.12 bundles;
the end user needs no system Python or package installation. Current adapted
voices may sound robotic. Read [the application guide](../../docs/VOICE_TRAINING.md)
for the supported workflow and its limitations.

`desktop_worker.py` is the bundled entry point. `package_runtime.py`,
`package_shared.py` and `make_release_catalog.py` create immutable, hashed bundles
and release parts. The compiled catalog authenticates exact bytes; changing a
bundle requires a new catalog/build and workflow validation. Public downloads
use pinned URLs; publishing bundles on another host requires updating the compiled catalog and verifying downloads.

The sections below retain the research history. Earlier statements about a
missing supervisor or disabled UI describe the prototype at that time. The Beta
now supplies supervision, pause/resume and installation. Best-validation
checkpoint selection and full clean-Windows/long-run qualification remain future
work; latest and previous checkpoints are retained by this recipe.

## Controlled duration experiments

`review_stages.py --root dist/voice-training/llvc-review --out dist/voice-training/stage-review`
creates matched-utterance teacher/offline/native comparisons from existing results.
It preserves the native stream's causal timing and records excerpt boundaries and
audio hashes. Signal statistics do not grade perceptual quality.

The reviewer identified LLVC as the stage that makes acceptable teacher speech
sound worse. Duration experiments therefore keep the teacher and paired data fixed.
Use `--teacher-steps` and `--llvc-steps` to control stages independently; `--steps`
remains the fallback for either unspecified budget. Start at 1,000 LLVC updates;
do not advance to 5,000 or 20,000 without validation and listening improvement.

The v3 recipe fixes scheduler advancement at non-interval stops. Old v2 checkpoints
cannot resume as v3 jobs; reusable frozen teachers remain subject to target/data
and optimizer compatibility checks. Checkpoints now include NumPy RNG state and
retain one previous snapshot. This is not yet the full application checkpoint
policy: best-validation selection and the Rust supervisor remain outstanding.

`review_duration.py` compares two candidate directories only after checking their
recipe inputs and exact held-out original/teacher audio hashes. Candidate native
waveform/state validation must pass first. No generated review grants acceptance.

The 500-to-1,000 update comparison received "About the same" feedback, so the
unchanged recipe is not advancing to 5,000. The next experiment uses
`--llvc-context-samples 65536` rather than the default 10240. It feeds 4.096 seconds
to the generator, pads shorter clips, keeps the discriminator update's crop at
0.64 seconds, and computes generator losses on the full context. Real/fake labels,
teacher, paired speech, initialization and optimizer settings remain unchanged.
Context is part of checkpoint compatibility. Run two updates to check memory,
then explicitly resume the same job to 1,000; no automatic budget escalation occurs.
Render with `review_duration.py --comparison context`. This mode requires equal
update budgets and byte-identical paired and held-out teacher speech.

`--discriminator-warmup-steps 500` runs the discriminator-only experiment before
joint LLVC training. It freezes the generator in evaluation mode, uses real
teacher/generated labels consistently, checkpoints the warm-up stage, verifies
the generator state is unchanged, then starts joint training with fresh
optimizers and the pre-warm-up random stream restored. Only discriminator weights
carry forward. The warm-up budget is part of the recipe; it cannot change on
resume. `review_duration.py --comparison warmup` requires equal joint budgets and
contexts, byte-identical pairs, and recorded generator immutability. This remains
an experiment and does not qualify user training.

These scripts test the plan's **first gate**. They do not implement the published
Create Voice wizard, download catalog or self-contained training runtime. Never
zip a venv and describe it as a portable runtime: these developer environments
still depend on the installed Python used to create them.

## Pins and notices

- LLVC: `1627c5d358cf9bb2b92b0ccc513d8b36807c923d`, MIT, Koe AI.
- RVC: `81eed5e8f68b6bed1789f682fe78cdd324495afc`, MIT, RVC contributors.
- LLVC's fairseq fork: `44800430a728c2216fd1cf1e8daa672f50dfacba`, MIT,
  Facebook/Meta and contributors. Its dependency metadata requires Hydra 1.3.2.
- RVC assets: Hugging Face `lj1995/VoiceConversionWebUI` revision
  `e6d0c1a17da07c33557852f9dfa2bd44cc75737d`; model card labels it MIT.
- LLVC initialization/feature model: `KoeAI/llvc_models` revision
  `ebfe8c0fdeb974a7eeb463b3abbc8ce42a0e3851`; model card labels it MIT.
- LibriSpeech dev-clean: CC BY 4.0, Vassil Panayotov and Daniel Povey;
  Panayotov et al., ICASSP 2015. https://www.openslr.org/12 . Only this development
  corpus is used here, not the full research training corpus. It is used for a
  reproducible public-speech experiment, not evidence of user listening approval.

Asset URLs, byte counts and SHA-256 are in `assets.py` and `assets.lock.json`.
The corpus SHA was recorded after checking OpenSLR's published MD5. Its official
ELDA mirror is used because the other checked endpoints had expired TLS certificates;
certificate verification is never disabled.

`requirements-common.lock.txt` pins the measured Python 3.9 developer dependency
set. CUDA uses torch/torchaudio **2.5.1+cu124**. DirectML uses torch/torchaudio
**2.4.1**, torchvision **0.19.1** and torch-directml **0.2.5.dev240914** in a
different environment. None of these dependencies is added to PirateWave's live workers.
The legacy development interpreter must be replaced/qualified for a public runtime.

## Reproducing the development environment

Use separate environments under `.tools/voice-training/`. Do not modify the
existing `.tools/voice-feasibility/python` inference/export environment.

1. Check out each source at its pinned revision under `.tools/voice-training/RVC`,
   `.tools/voice-training/fairseq` and the existing `.tools/voice-feasibility/LLVC`.
2. Create the developer Python 3.9 venv; install torch/torchaudio 2.5.1+cu124
   from the official PyTorch CUDA 12.4 wheel index, then install the common lock.
3. Build the pinned fairseq fork with `READTHEDOCS=1`, `pip wheel --no-deps
   --no-build-isolation`, then install that wheel. This disables unrelated optional
   translation/scoring native extensions; the HuBERT feature extractor is checked
   by actual training, not assumed to work. Clear `READTHEDOCS` afterward.
4. Keep all third-party source LICENSE files. Future distributed bundles must
   include package license/notice files and their exact runtime manifest.
5. Preview asset sizes before explicitly downloading:

```powershell
.tools/voice-training/python/Scripts/python.exe scripts/voice-training/assets.py --out .tools/voice-training/assets --corpus
.tools/voice-training/python/Scripts/python.exe scripts/voice-training/assets.py --out .tools/voice-training/assets --corpus --download
```

The downloader uses partial files, validates resumed ranges, checks sizes/hashes
before publication and checks extraction paths/types and free space. The full
application installer/download manager remains a later gated step.

## Actual training and native export

```powershell
.tools/voice-training/python/Scripts/python.exe scripts/voice-training/supervise.py --out dist/voice-training/example --timeout 3600 -- --rvc .tools/voice-training/RVC --llvc .tools/voice-feasibility/LLVC --assets .tools/voice-training/assets --initial-llvc .tools/voice-feasibility/llvc.pth --backend cuda --steps 250 --clips 100 --source-clips 500 --clip-seconds 10 --checkpoint-every 50
.tools/voice-training/python/Scripts/python.exe scripts/voice-training/export_probe.py --llvc .tools/voice-feasibility/LLVC --checkpoint dist/voice-training/example/llvc-generator.pt --worker dist/voice-pack/llvc-f8312-v1/PatchVoiceWorker.exe --speech .tools/voice-feasibility/LLVC/test_wavs/174-50561-0000.wav .tools/voice-feasibility/LLVC/test_wavs/1919-142785-0000.wav --seconds 30 --out dist/voice-training/example-export
.tools/voice-training/python/Scripts/python.exe -m unittest discover -s scripts/voice-training -p 'test_*.py'
```

Use `--backend cpu` explicitly to test CPU. No implicit device fallback occurs.
Use `--resume` with matching data, backend and recipe to restore checkpoints.
Creating `pause.request` in the output directory checkpoints at the next optimizer
step boundary and stops; remove the request and resume explicitly. This is a
developer protocol, not yet connected to PirateWave's audio start/stop commands.

The supervisor contains even fatal native exits; inspect `supervisor.json`,
`report.json`, `worker.stdout.txt` and `worker.stderr.txt`. Worker success never
sets `release_qualified` or `public_training_enabled`. The exporter checks waveform
and all four states against the original model with the existing tolerances.
Diagnostic timing includes the large state transfers and parallel reference work.
A separate normal-protocol pass measures audio-only IPC timing and checks that
its output matches the diagnostic stream. Neither is complete microphone-to-output
latency or an integrated thirty-minute qualification.

The recipe uses the unchanged LLVC architecture, pretrained initialization,
adversarial/feature/multiresolution mel losses and the original HuBERT convolutional
content loss. The adapter uses explicit single-device operations and freezes the
content extractor's parameters while preserving gradients to audio. Real/fake
inputs are consistently ordered. It is **not an exact reproduction of the upstream
training schedule**: learning rates, short training, data selection, no-f0 teacher
and scheduling are candidate settings, not an accepted production recipe.

Exported models and WAVs stay in `dist/voice-training`. They cannot be installed
through the current single-voice production installer. Long-run validation,
generation-safe live integration, portable runtime packaging, proper per-speaker
evaluation splits and manual voice-quality acceptance remain required.

## DirectML investigation

Keep its environment separate. Install its pinned torch family before the common
lock and fairseq wheel. On Python 3.9, the upstream package fails in class creation
because it calls a non-callable staticmethod object. The explicit developer-only
`piratewave_directml_py39.py --apply` checks the original module SHA and changes only
those two calls to invoke `__func__`. This is recorded as a patched prototype;
it does not modify native operators, ORT or PirateWave's DirectML inference worker.

After this import fix, required STFT processing aborts natively with
`Invalid or unsupported data type ComplexFloat`. Run through `supervise.py` so
the failure is reported. DirectML training stays disabled. CPU spectral islands or
another runtime would require separate gradient/parity/performance investigation;
do not silently substitute them and mark this profile qualified.

## Embedded-runtime and listening-review candidates

`dataset.py` reserves three validation and three test source speakers, plus
separate target clips. `probe.py` now writes `dataset.json`; changed data splits
intentionally invalidate old checkpoints. `--teacher-mode f0` uses the separately
pinned `PITCH_ASSETS` and pYIN (50–1100 Hz, 160-sample hop). Download those assets
explicitly with the existing checksum-verifying `download` function before use.
`--teacher-only` stops at teacher listening review instead of training LLVC.

Run `review_matrix.py --out dist/voice-training/teacher-review` to compare both
teacher variants for targets 1272, 1462 and 1673. It runs six supervised jobs
sequentially and stops on errors. `--render-only` regenerates the review page from
completed reports without retraining. Browser previews use a monitoring output;
no PirateWave routes or audio-engine controls are invoked. Listening notes remain local
and never set a release-qualified flag.

`export_content.py` exports the original frozen fairseq content extractor and
checks both output and input-gradient parity at three lengths. Pass its result
using `--content-extractor`. The exported objective keeps gradients to audio;
it is not an inference-only substitute for the loss.

`build_runtime.ps1` is a **build-machine** tool for an empty destination. It uses
the official embedded Python 3.12.10 archive (pinned SHA-256), entirely prebuilt
wheels from `requirements-embedded-{cpu,cuda}.lock.txt`, and separately pinned
upstream sources. No pip or compiler is invoked by the resulting runtime.
`package_runtime.py` vendors source with per-file hashes and license files,
rejects duplicate dependency metadata, creates a file manifest and ZIP catalog,
and always marks the result unqualified. `check_runtime.py` verifies every file
and imports dependencies with system Python/Conda paths removed. This does not
simulate a clean Windows machine or qualify third-party redistribution notices.

The initial input `requirements-embedded.in` is exploratory; use the resolved
backend locks for rebuilding. Runtime file hashes and archive checksums pin the
actual candidate artifacts. Complete wheel provenance/license review and clean
Windows acceptance are still required before publishing them.

Two-step embedded CPU/CUDA smoke outputs and 30-second native comparisons live in
`dist/voice-training/embedded-*`. These checks prove execution, not an accepted
voice recipe or a production performance benchmark. `write_progress.py` summarizes
this evidence in `docs/VOICE_TRAINING_PROGRESS.json`; the original feasibility
report remains a historical snapshot rather than being rewritten as new evidence.

After teacher review, `review_llvc.py --teachers dist/voice-training/teacher-review
--out dist/voice-training/llvc-review` reuses the no-F0 teacher for each of the three
targets, generates 100 source pairs, trains 500 LLVC updates and compares the actual
native worker for 30 seconds. It refuses changed target data, teacher mode or
optimizer settings. Its review page includes actual native streaming output.
This remains a candidate experiment, not a validated minimum recipe. The user's
first preference for no-F0 is recorded in `docs/VOICE_TRAINING_LISTENING.json` and
does not confer final acceptance on any model.

`verify_bundle.py` verifies a candidate archive against its catalog, rejects
unexpected paths/entries/links, checks each file and reconciles installed bytes.
It performs no extraction or execution and does not authenticate the catalog's
origin. Production catalog trust and application installation remain unimplemented.

## Reproduce the optional voice library

`build_library.py` trains one named voice with the pinned CUDA tools and shared LibriSpeech corpus, exports its streaming model, checks native waveform/state parity and timing, then writes a model-only ZIP and a catalog entry. Pass `--runtime`, `--shared`, `--worker`, `--worker-runtime`, `--out`, and `--voice` (see `--help`). Run with the training runtime’s Python. The worker runtime is the installed, verified CPU LLVC library folder. Existing matching checkpoints resume; `--skip-training` is for exporting an already completed run.

Each voice uses a distinct reader, up to 60 target clips, 100 source clips, 8-second maximum clips, 250 teacher updates and 1,000 LLVC updates. The deterministic split holds target recordings and source speakers apart for validation/test. This small recipe is experimental, not the original paper’s full-scale training. See [upstream training requirements](https://github.com/KoeAI/LLVC#dataset). Retain the dataset manifest and full build reports when publishing.

The build tool never uploads or installs voices. After reviewing candidate results, publish verified ZIPs under `voices-v1`, and compile their exact sizes and SHA-256 hashes into `src/voice/library.json`. Never overwrite a published ZIP at a pinned URL; release a new version instead. Include LibriSpeech CC BY 4.0 attribution, reader metadata, modification notice and model licenses. Loss values and finite-output tests do not substitute for listening review.
