# PirateWave downloads

This page is a download-guide template for optional voice tools. For current PirateWave application builds, use the source repository's Releases or successful CI artifacts; see [Getting started](../../README.md#get-started).

Pinned voice/runtime assets are hosted in the [pinned support assets](https://github.com/MitchCodes/PirateWave/releases/tag/support-assets-v1). The support-assets release contains pinned dependencies only; current application downloads are published by the main build workflow. Install the matching optional assets through PirateWave's Voices page or Voice Creation tools controls.

## Voice Creation Beta

Current trained voices can sound robotic or metallic. This Beta exposes the existing training process; it does not claim improved voice quality or complete clean-Windows, long-run custom-model or gaming-impact qualification.

1. Install the matching optional LLVC voice pack from PirateWave's Voices page.
2. Open Voices → Create voice. Choose a workspace with room for tools and checkpoints.
3. Record or import dry speech, using separate training, validation and test takes. This Beta accepts up to eight minutes of training recordings and thirty minutes overall.
4. Explicitly install CPU **or** NVIDIA CUDA training tools, plus the shared models/source-speech bundle. CUDA needs a compatible installed NVIDIA driver. No system Python, pip, compiler or CUDA toolkit is required.
5. Start training, compare the previews on a selected monitoring device, then install the completed CPU voice revision. Heavy work pauses when PirateWave audio starts; resume is explicit.

For offline installation, download every `.partNNN` file for the selected backend and shared bundle into one folder, then choose that folder in Tools/storage. Do not extract individual parts. The app verifies their exact sizes and checksums before execution. `SHA256SUMS.txt` covers published artifacts. The optional shared bundle includes licensed source speech, not your recordings. Your recordings and training stay local.

| Bundle | Download bytes | Installed bytes |
|---|---:|---:|
| CPU tools | 408,081,670 | 1,626,470,496 |
| CUDA tools | 2,786,567,626 | 5,093,004,744 |
| Shared models/source speech | 730,167,404 | 788,966,452 |

Archives, extraction staging, checkpoints and generated data need additional room. PirateWave displays the estimate and requires the greater of 2 GiB or 10% extra headroom. User voices use CPU playback after short native numerical/timing checks; the curated voice's separately qualified DirectML mode does not qualify custom voices.

Read `docs/VOICE_TRAINING.md` inside the portable build for recording advice, storage, troubleshooting and technical limits. Dependency and model notices accompany the artifacts. Use only recordings you have permission to use.
