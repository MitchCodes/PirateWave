# Automatic main builds

Windows CI builds the PirateWave portable archive, `PirateWave.lplug4`, and `PirateWave.streamDeckPlugin`. Publication is opt-in and runs only when the repository variables and token below are configured. CI artifacts and local builds still work.

## One-time GitHub configuration

In the publishing repository, set Actions variables `PUBLISH_RELEASES=true` and `RELEASES_REPOSITORY=owner/repository`. Add a `RELEASES_TOKEN` secret with Contents write access to that explicit destination. No destination is inferred from the former release repository. Optional model/runtime bundles use pinned download URLs. Before retiring an asset host, publish replacement assets and update the pinned catalogs and hashes.

Only main-branch push events publish. Pull requests, other branches, and manual runs produce downloadable artifacts without publishing. Tokens are never committed or exposed to pull-request publishing steps.

## Release behavior

- Tags include the Cargo application version, workflow run number and source commit prefix: `v0.2.0-beta.5-main.42.abcdef123456`. Main pushes do not require manually pushing version tags or changing the version for every build.
- Releases contain `PirateWave-v<VERSION>-windows-x64.zip`, standalone `PirateWave.lplug4` and `PirateWave.streamDeckPlugin`, and `SHA256SUMS.txt`. The ZIP includes the identical controller packages. Release notes identify the source commit; GitHub's generated source archives belong to the configured destination.
- Publication starts as a draft and becomes public only after all four uploads pass GitHub SHA-256 verification. Failed uploads leave a recoverable draft. Rerunning the same workflow reuses its tag and does not overwrite an already published release.
- Each main-push run has its own concurrency group. A newer push does not cancel an earlier main build. Pull-request builds retain branch-based cancellation.
- Existing optional voice/training bundles remain in `v0.2.0-beta.1`; they are not rebuilt or reuploaded on each application push.

## Clean-runner dependencies and checks

CI installs Rust and .NET 10, then runs `scripts/prepare-ci-dependencies.ps1`. The bootstrap downloads a pinned portable runtime archive and official LogiPluginApi NuGet package, verifies their hard-coded SHA-256 hashes, and extracts only explicitly allowed worker/runtime and SDK files into ignored build directories. It neither installs Logi Plugin Service nor packages SDK host assemblies. The bundled native worker and runtime files match the previously validated beta.1 distribution.

`PIRATEWAVE_LOUPEDECK_SDK_DIR` selects the downloaded SDK for plugin compilation and native SDK callback tests. Normal local builds continue to use the installed SDK unless this environment variable or `-PluginApiDir` is supplied. Update the pinned URLs/hashes deliberately when changing dependencies.

Before publishing, CI runs formatting, Rust tests and Clippy, native VST/32-bit bridge checks, allocation/performance harnesses, Loupedeck SDK/native connection tests, publisher safeguard tests, official plugin package verification, and portable/plugin archive consistency checks. The Actions artifact contains all three deliverables and their checksums even on non-publishing builds.

Local checks without publishing:

```powershell
./scripts/prepare-ci-dependencies.ps1 -DestinationRoot .tools/ci-bootstrap-sandbox
python -m unittest discover -s scripts -p test_publish_main_release.py
python scripts/publish-main-release.py --folder dist --version 0.2.0-beta.5 --commit (git rev-parse HEAD) --run 1 --dry-run
```
