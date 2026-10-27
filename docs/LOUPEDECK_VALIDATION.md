# Loupedeck validation

> Historical validation record. Counts, versions, measurements, and limitations below describe the builds tested at that time; they are not a fresh qualification of every current build. See the [documentation index](README.md) for current setup guides.

## Toggle colors — plugin 1.3.0

Validated on 2026-10-23:

- Added explicit optional boolean `active` state to the control catalog, independent of display labels: mute, solo, listening, rack/effect enablement, ducking, recording arming, audio running, selected switch destinations and extension toggle actions. Mute state is reported independently of VST/rack state and solo. Metadata/catalog construction remains on the control worker; audio rendering is unchanged.
- Stateful command images use green text (`#4ADE80`) for ON, red (`#F87171`) for OFF, an ON/OFF label and a matching four-pixel underline. Errors/offline/missing/ambiguous targets remain neutral. Volume reset/press uses its mute state; ordinary knob rotation keeps neutral numeric feedback. Known older-app state labels provide a backward-compatible fallback; unknown legacy states are neutral.
- Rust catalog tests verify independent flags (including mute on VST instruments), both boolean values, switch activation and absent-field deserialization. C# preflight verifies palette, typed/legacy states, each built-in toggle family, volume press versus rotation, neutral errors/missing/ambiguous targets, and redraw on boolean-only changes without dropdown invalidation.
- The full verified dist build passed formatting, 216 workspace tests (7 opt-in tests ignored), Clippy with warnings denied, release/32-bit bridge builds, extension checks and official Logi pack/verify. The SDK/native-pipe integration also passed explicit boolean metadata and live mute-on feedback, reconnect/no replay and the prior controller regressions.
- Actual SDK-generated icons were rendered and visually inspected at 72x72 and 90x90. Pixel checks verified colored text separately from the underline, exact green/red RGB values and a neutral no-underline unknown state.
- Installed plugin 1.3.0 and started the refreshed PirateWave executable. The running service loaded the plugin and received its catalog. The native renderer-state selection check passed all 12 action types with no idle invalidation storm. No physical CT button/dial inputs were synthesized.
- Final ZIP integrity, embedded binaries/guide/plugin and installed-DLL byte comparisons passed.

Artifacts:

- `dist/PirateWave.lplug4` — SHA-256 `0142f83077ef2afad4264e818496b3f98951b728b5bf00eadbe47cc06d578358`
- `dist/PirateWave-v0.2.0-beta.10-windows-x64.zip` — SHA-256 `e34760cd80d3c3c68787dedc2ace2214c588ac0de36a7ccb985ef73809d87848`

Ignored evidence: `dist/loupedeck-color-dist-build.txt`, `dist/loupedeck-color-core-tests.txt`, `dist/loupedeck-color-native-tests.txt`, `dist/loupedeck-color-render-check.txt`, `dist/loupedeck-color-install.txt`, `dist/loupedeck-color-editor-check.txt`, `dist/loupedeck-feedback-*.png`.

## Dependent selection messages — plugin 1.2.4

Validated on 2026-10-22 against Logi Plugin Service 6.4.1.3246 and renderer 6.4.1.364:

- The 1.2.3 change fixed incoming request matching but still emitted PascalCase refresh/reset names. Targeted inspection of the installed renderer shows that selection messages update `controlState[controlName]` and list responses update `listboxItems[request.controlName]` with exact keys. A refresh for `Target` therefore does not update the visible `target` list. Earlier tests queried lists with fabricated selected values and did not apply selection-change messages.
- All four SDK controls now use shared lowercase identifier constants in declarations, state lookup, selection resets, invalidations and saved-parameter lookup. Incoming case-insensitive support and legacy assignment reading remain; project target names retain exact matching.
- The SDK regression now checks actual emitted event/message keys against the renderer's keys, without normalizing them in the test. It failed before the fix with “Refresh event targets a different renderer cache key” and passed afterward. Official build/pack/verify and the SDK/native transport suite passed.
- Added `scripts/test-loupedeck-editor.py`, an opt-in installed-service check. It selects a node, applies returned reset messages to exact state keys, refreshes the lists named by actual service events, selects a target, applies the next response, and reads the resulting action/parameter list and serialized selector. It never saves bindings or executes audio commands.
- Installed version 1.2.4 and ran that flow for all 12 action types against the saved project: Nodes, effects, recording slots, switches, soundboard, mixer and ducking choices populated; absent VST/extension targets showed their explanatory state. No idle invalidation storm remained. This is a native protocol/renderer-state replay, not a physical CT button/dial test or a visual UI assertion.
- REA summary `ev_6f19c2696016d478a31efdf4a419b52badb046703850f44a0f369efd5462be7c` identifies installed `app.asar` SHA-256 `fbb4cd69a59a7f39b3b16879caa56cc517b7db8703c200d803117067356c69ec`. Its semantic analysis was resource-limited; the relevant reducer/request code in `dist/816.renderer.prod.js` was read directly from the artifact to resolve the exact-key question.
- Refreshed distribution/package integrity, embedded files and installed-DLL equality passed. Native audio binaries are unchanged.

Artifacts:

- `dist/PirateWave.lplug4` — SHA-256 `76ea14f5e2ac728264d3b66dde49fc255971253358e1fc9d6063e5c0fb9a3e49`
- `dist/PirateWave-v0.2.0-beta.10-windows-x64.zip` — SHA-256 `fcde18fe8729baa218908539c4b442ecab866e3cb07f8732066ec80ebb611ded`

Ignored evidence: `dist/loupedeck-selection-regression-before.txt`, `dist/loupedeck-selection-final-tests.txt`, `dist/loupedeck-selection-build.txt`, `dist/loupedeck-selection-install.txt`, `dist/loupedeck-selection-native-check.txt`.

## Normalized control identifiers — plugin 1.2.3

Validated on 2026-10-22 against Logi Plugin Service 6.4.1.3246:

- The real editor's log contained `node/target` requests in lowercase while 96 node controls were available. PirateWave compared callback control names with PascalCase string literals, returning no items for normalized requests. The earlier native-service check supplied PascalCase names and therefore missed this failure; reducing refresh traffic alone did not fix it.
- Dropdown population and dependent-control-change handlers now compare control identifiers using ordinal case-insensitive equality. Project node/effect/slot names retain exact case-sensitive matching.
- Added SDK regression coverage for lowercase and mixed-case node/target/action requests, normalized state keys, lowercase saved action parameters and dependent-selection invalidations. The new test failed before the fix with an empty node list and passed afterward. Preflight and full SDK/native-pipe integration passed.
- Installed version 1.2.3 successfully. Read-only service queries using lowercase identifiers and lowercase state keys returned all three dependent dropdown levels for the current project: 13 eligible Nodes, plus effect, switch, soundboard, recording-slot, mixer and ducking controls. Absent VST/extension targets showed their explanatory state. No profile bindings were modified. These requests exercise the editor backend; no physical CT input or visual UI success is claimed.
- Refreshed and verified the plugin and portable distribution. ZIP integrity, embedded plugin/guide/native-binary comparisons and installed-DLL equality passed. The native audio binaries are unchanged.

Artifacts:

- `dist/PirateWave.lplug4` — SHA-256 `fb71923cdcf970bcd955b35dad2484044b88fe6511b96e4862a793da0852c5f7`
- `dist/PirateWave-v0.2.0-beta.10-windows-x64.zip` — SHA-256 `efbd6b9d1f7de98e58410ef357f1d382eb2ccf352639fca3ed7f3fbfdfc7ea93`

Ignored evidence: `dist/loupedeck-case-regression-before.txt`, `dist/loupedeck-case-final-tests.txt`, `dist/loupedeck-case-install.txt`, `dist/loupedeck-case-native-check.txt`, `dist/loupedeck-case-native-check.py`.

## Editor refresh fix — plugin 1.2.2

Validated on 2026-10-22 against the running Logi Plugin Service 6.4.1.3246 and the saved PirateWave project:

- Local service queries confirmed that PirateWave's catalog contained 231 controls. The old plugin nevertheless broadcast 541 dropdown invalidations and 75 adjustment notifications during a 1.5-second idle interval. Invalidations were tied to every state packet rather than changes in available targets.
- Split choice refresh from value/state feedback. Identical packets now produce no SDK invalidations or display refreshes; actual renames/additions/removals refresh choices, while values update device feedback. Choosing an Action or changing Sensitivity no longer invalidates that action's own dropdown.
- SDK callbacks now run on one coalescing notification worker, independent of pipe reads. A slow or failed subscriber cannot prevent the receive loop from draining state; notification work remains bounded. Offline/empty categories display actionable messages, and plugin logs include connection failures/catalog counts.
- Regression checks exercise actual SDK dependent-dropdown callbacks and the SDK invalidation event, including Unicode/quoted selectors, repeated identical snapshots, live feedback, renames and offline/empty states. A blocked feedback callback remains blocked longer than the heartbeat timeout while state reads continue. These editor checks run before every plugin package build.
- Official pack/verify and SDK/native-pipe integration passed, including reconnect/no replay. Workspace tests, formatting and Clippy with warnings denied passed. No audio-engine code changed.
- Installed the final package successfully. Read-only queries to the running service exercised all three dropdown levels across all 12 action types. The Nodes action returned 13 eligible nodes; effect, switch, sound, recording-slot, mixer and ducking actions returned their targets/actions. Absent VST/extension targets returned the explanatory empty state. Idle PirateWave invalidation/adjustment events fell to zero in the same 1.5-second interval.
- The distribution archive was refreshed with the new compiled plugin and guide, retaining the unchanged verified native binaries/SDK. ZIP integrity, embedded-file comparisons and installed-DLL equality passed. No controller bindings were saved or modified, and no physical CT inputs were synthesized. Desktop capture was unavailable; verification used the running service's editor request path rather than a visual UI assertion.

Artifacts:

- `dist/PirateWave.lplug4` — SHA-256 `58d32d0aaf627a5fc3f1146427fb042ffa8ce95882055d982e14217e046dd1ef`
- `dist/PirateWave-v0.2.0-beta.10-windows-x64.zip` — SHA-256 `00d2f67169daebcd12643b917cf44181e1321fb43e9dbc29625af0c3bfa17a1d`

Ignored evidence: `dist/loupedeck-dropdown-final-tests.txt`, `dist/loupedeck-dropdown-final-install.txt`, `dist/loupedeck-dropdown-native-check.txt`, `dist/loupedeck-dropdown-native-check.py`, `dist/loupedeck-dropdown-workspace-tests.txt`, `dist/loupedeck-dropdown-clippy.txt`.

## Installation fix — plugin 1.2.1 / PirateWave 0.2.0-beta.10

Validated on 2026-10-22 against the running Logi Plugin Service 6.4.1.3246:

- Reproduced the original package installation failure using the official Logi Plugin Tool: error 1627, “Install plugin method failed”. The service's Sentry records independently reported that error.
- The installed loader's `PluginAssemblyLoader.TryLoadPluginFromFile` requires exactly one constructible `Plugin` and one `ClientApplication` subclass before installation proceeds. PirateWave lacked the latter, despite being a universal plugin. Added an unlinked `PirateWaveApplication`, preserving `HasNoApplication` and the existing pipe control behavior.
- REA artifact evidence `ev_6cfd33bd086036dd2dbdc5b77290fbeee06d8d27d4c87c2bbe9bcd3444d898d7` identifies installed PluginApi 6.4.1.3246 / .NET 10. REA member inspection exceeded its metadata representation limit; focused ILSpy reconstruction of the installed LoupedeckShared loader established the class requirement. Actual failed/successful installs confirm the diagnosis; static analysis alone did not establish runtime success.
- Every package build now checks entry-point discovery/constructibility, universal-plugin flags and offline SDK installation/uninstallation callbacks. Official pack/verify also passed.
- Integration fixtures now use a unique test pipe so the installed controller cannot compete for their single-client endpoint. Production retains the fixed current-user pipe; environment-based test IDs are read only in explicit diagnostic-host mode.
- Full workspace tests, formatting, Clippy with warnings denied and refreshed dist build passed. The real SDK/native-pipe integration suite passed, including reconnect and absence of replay. These callbacks were simulated; no physical CT inputs were synthesized.
- Installed the corrected final package using the official tool. It reported success; the running service logged plugin version 1.2.1 loaded in 56 ms. The installed DLL matches the package bytes. No profile assignments were created or edited.
- ZIP integrity and embedded plugin/executable/guide byte comparisons passed.

Artifacts:

- `dist/PirateWave.lplug4` — SHA-256 `ef9a2b73e6c5363ddb7ff8e74c9efa320034339419f96511f6b48655ca488870`
- `dist/PirateWave-v0.2.0-beta.10-windows-x64.zip` — SHA-256 `9977469fb31461d819f20666a401ce92216f8280fd621cbd4edd0acf98dfc426`

Ignored evidence: `dist/loupedeck-install-fix-dist.txt`, `dist/loupedeck-install-fix-integration.txt`, `dist/loupedeck-install-fix-native.txt`, `dist/logi-loader.cs`.

## Original qualification — PirateWave 0.2.0-beta.5

The following records the earlier validation before native service installation was exercised.

Validated locally on Windows on 2026-10-22 with installed Logi Plugin Service / PluginApi 6.4.1.3246 and .NET SDK 10.0.402.

- `./build.ps1 -Verify -WithLoupedeck`: format check, workspace tests, Clippy with warnings denied, release executable, 32-bit plugin bridge and portable archive passed. 167 tests passed; seven environment-dependent opt-in tests were ignored by the standard suite.
- Controller core tests resolve all catalog button actions without keyboard bindings and exercise name portability, Unicode/separators, renamed/missing/duplicate nodes, slots, effects and sounds, removed switch routes, parameter clamping/defaults, sensitivity rejection, discrete quantization and VST identity replacement.
- UI controller tests invoke the shared dispatcher for every recording-slot action, recorder-wide stop and mixer adjustments; inspect real engine command types; reject stale project generations and invalid/empty recording operations.
- The C# integration executable references the installed SDK. It constructs all nine concrete SDK action types, checks their configured selector controls, and invokes protected SDK button/dial/reset handlers with simulated parameters. Those callbacks exercise a real native named-pipe fixture, including mixer, built-in choice and VST parameter adjustments and reset.
- Native client/transport checks passed for both startup orders, version rejection, oversized frame rejection, live state updates, capture dispatch to an empty named slot, name reuse after generation changes, bounded queue overflow, adjacent-only coalescing, disconnect detection, offline rejection, reconnect and absence of command replay. The fixture never opens audio hardware or loads/saves user projects. Its VST metadata is synthetic; actual plugin hosting is checked separately below.
- `./scripts/test-plugins.ps1`: native VST2/VST3 scanning, real DSP routing, live parameter changes, instrument note controls and bypass passed on x64 and through the 32-bit bridge.
- Native UI smoke rendered all 67 views. The new General settings control was visually inspected; stderr was empty.
- Recording performance harness: four recorders / twenty 60-second slots share one 23,414,016-byte history, with zero rendering allocations and measured 0.268% of the realtime CPU budget on this machine. Rack/voice effect and pitch-correction harnesses also reported zero processing allocations. These synthetic figures are not a latency guarantee or a hardware stress test with a physical Loupedeck connected.
- Official Logi Plugin Tool 6.4.2.3599 packed and verified `PirateWave.lplug4`. Its contents are only `bin/PirateWavePlugin.dll`, metadata YAML, the generated PirateWave icon and package hash. No SDK/host assemblies or user profiles are included.
- Final portable ZIP integrity passed; its three executables match the portable directory, PirateWave.exe matches the compiled release, and its plugin and guide match the standalone deliverables.

The plugin has not been automatically installed into the running Logi service or assigned to a user profile. Runtime validation used the installed SDK callbacks and native transport fixture, not a plugin loaded into the user's service profile. Physical-device button/knob behavior remains a separate qualification step after explicit installation.

## Artifacts

- `dist/PirateWave.lplug4` — SHA-256 `c3bcf16567c2e9f95b58fbc068984fa0f664800ffb3f095ef938787ef565373c`
- `dist/PirateWave-v0.2.0-beta.5-windows-x64.zip` — SHA-256 `4d3d72dc9702c90e4fec27498444b35b41d18216a1dae35b763880f95fef6eaa`

Logs and screenshots are in the ignored `dist/` folder: `loupedeck-final-build.txt`, `loupedeck-package-check.txt`, `loupedeck-native-release-tests.txt`, `loupedeck-vst-tests.txt`, `loupedeck-*-performance.txt`, `ui-smoke-result.txt`, and `native-settings-general.png`.

Release automation follow-up: the clean-runner bootstrap and official NuGet SDK 6.4.2.3567 were validated locally, including native SDK callbacks without a host installation directory. The portable distribution was rebuilt with that CI SDK and both release asset checksums above were refreshed. `actionlint`, six publisher safeguard tests and a publication dry run passed. Actual cross-repository publication requires the source repository's `RELEASES_TOKEN` Actions secret; no release was published by local validation.
