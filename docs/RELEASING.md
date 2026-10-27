# GitHub releases

Every successful push to `main` creates a prerelease in **this repository**, using the tested commit. Pull requests, other branches, and manual workflow runs retain Actions artifacts without publishing.

## Separate downloads

| Asset | Contents |
| --- | --- |
| `PirateWave-v<VERSION>-windows-x64.zip` | Full Windows x64 app, native helpers, SDK/example extension, and user guides. No voice-training tools/models/speech data or controller installers. |
| `PirateWave.lplug4` | Optional Loupedeck/Logi plugin. |
| `PirateWave.streamDeckPlugin` | Optional Stream Deck plugin. |
| `PirateWave-Voice-Training-CPU.zip` | Optional verified CPU training tools and shared models/licensed speech, ready for offline import. |

`SHA256SUMS.txt` accompanies the four downloads. GitHub also provides its own source-code archives.

Extract the training ZIP into one folder, leaving the `.part` files intact. Install the matching voice pack through Voices, then select the extracted folder in Voice Creation **Tools/storage** for offline CPU setup. The CPU and shared parts total 1,138,249,074 bytes; the outer ZIP adds a small amount of metadata. Licenses/notices remain inside the verified bundles. CUDA tools remain an optional in-app download: the pinned CUDA tools alone exceed GitHub's 2 GiB per-file limit, so the single training ZIP is explicitly CPU-only.

## Permissions and publication

No personal access token or repository variables are required. The build job has `contents: read`. A separate release job runs only after a successful main-push build and has `contents: write` and `actions: read`; it downloads that run's artifact and uses the built-in `GITHUB_TOKEN` with `GITHUB_REPOSITORY`. Repository/organization policies must permit this job permission.

Tags include the Cargo version, workflow run number, and source commit prefix. Release creation sets `target_commitish` to the tested SHA. Each run has its own tag; a newer main push does not cancel an earlier main build.

Publication starts as a draft and becomes public only after all four downloads and the checksum file pass GitHub upload-digest verification. Failed uploads leave a recoverable draft. Reruns can complete draft uploads but never overwrite a published release. Pull-request and non-main event guards are checked in both the workflow and publisher.

## Dependencies and validation

CI installs Rust, Node.js 24 and .NET 10. `scripts/prepare-ci-dependencies.ps1` downloads pinned worker/runtime and Logi SDK dependencies, verifies SHA-256 hashes, and extracts only allowlisted files. SDK host assemblies are not distributed. `PIRATEWAVE_LOUPEDECK_SDK_DIR` selects the SDK location.

`scripts/package_voice_training.py` obtains the CPU/shared parts from `src/training/catalog.json`, verifies their pinned sizes and hashes, and packages them without unpacking or altering them. Catalog part names are preserved because offline installation validates them against the executable's trusted catalog. The source asset host must remain available until catalogs are deliberately updated.

CI runs workspace checks, native plugin/controller tests, allocation checks, official controller package validation, and release safeguards. The publisher rejects optional installers/training parts embedded in the main ZIP, invalid training bundles, missing assets and oversized uploads. Hashing and large uploads stream data rather than reading the training archive into memory.

Local checks (no publication):

```powershell
python -m unittest discover -s scripts -p test_publish_main_release.py
./build.ps1 -WithLoupedeck -WithStreamDeck
python scripts/package_voice_training.py --folder dist
# Use the version from Cargo.toml:
python scripts/publish-main-release.py --folder dist --version 0.2.0-beta.10 --commit (git rev-parse HEAD) --run 1 --dry-run
```

The training packaging command downloads approximately 1.14 GB if its verified cache is absent. Application builds do not download that data; CI invokes the packager separately.

## Pinned support assets

The versioned `support-assets-v1` release in this repository hosts the native CI runtime, optional voice pack, and CPU/CUDA/shared training parts. These are dependencies, separate from automated main-build application releases. CI and the application use public URLs and pinned checksums; no cross-repository credential is needed. The small native bootstrap ZIP contains only the existing worker, its runtime support DLL, and dependency notices, rather than a historical desktop application. Optional training parts and voice-pack bytes retain their original hashes; the public filenames use PirateWave branding.
