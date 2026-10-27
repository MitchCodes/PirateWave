# Workflow release validation

Validation for PirateWave **0.2.0-beta.11**, performed on Windows on 27 October 2026. Controller packages are Loupedeck **1.5.0** and Stream Deck **1.2.0**.

## Automated coverage

- Workspace tests cover scene persistence and legacy defaults, exact-name controller resolution, atomic target validation, manual ownership, interrupted switch crossfades, sample-identical unchanged recalls and retained effect tails.
- Every shipped built-in effect is exercised during a scene transition under the counting allocator. Auto Ducking detection-mode/lookahead transitions have their own allocation assertion; pristine speech-detector replacements are prepared outside rendering.
- Recording tests cover shared raw/processed histories, common partially filled ranges, wraparound, retained-snapshot budget accounting, the 512 MiB limit, packet saturation, aligned 32-track files, markers, cropped immutable exports, incomplete-file recovery and WAV/RF64 headers. RF64 size boundaries are simulated rather than written as multi-gigabyte files.
- A real engine/file-worker UI integration test runs an automation sequence that starts live recording, captures history while recording, inserts a marker, stops/finalizes, and reports completion. Raw and processed tracks are checked against common frame boundaries.
- Deterministic scheduler tests cover holds/hysteresis, arming without synthetic crossings, ordered acknowledgments/delays, all/any conditions, operation failure, overlap/skipped triggers, causal cycles/depth, generation rejection, disarm and saturation without replay.
- Native Windows pipe tests exercise both controller protocols, simultaneous clients, generation precision, restart/reconnect and no replay. Simulated SDK callbacks and selector tests are separate from physical device testing. Official tools validate both installable packages.
- Native screenshot fixtures cover Scenes, session tracks, synchronized take review, Automation, and narrow Mixer/Automation views. They do not start audio or modify user settings.

## Measured audio performance

Release-mode counting-allocator checks reported **zero render allocations** for the new workflow workload and existing graph, recorder and ducking examples.

| Synthetic workload | Measured realtime CPU budget |
| --- | ---: |
| 32 raw/processed histories plus scene transition | 0.592% |
| Existing routing graph | 0.093% |
| Existing built-in rack | 0.144% |
| Existing Level / Speech ducking graphs | 0.333% / 0.783% |
| Existing shared recording histories | 0.108% |

These are local microbenchmarks, not latency guarantees. Disabled workflow taps delegate to the existing graph path. Foreign plugin internals are outside PirateWave's allocator guarantees. No steady-state lookahead was added by these features.

## Sustained recording stress

The explicit ignored stress test ran **60 seconds with 32 tracks**, active scene transitions and armed automation, using real temporary WAV writers and synthetic audio:

- 737,280,000 audio bytes written.
- 4,488 acknowledged automation steps.
- Peak transfer queue: 3 of 863 packets.
- No lost frames; all tracks ended on the same boundary.
- Test completed in 68.27 seconds including finalization and cleaned its temporary media.

This checks a sustained synthetic workload. It does not replace a real-device overnight endurance test, a slow/removable-drive qualification, or testing physical disk-full behavior. Error paths use deterministic failures/saturation tests.

## Checks still requiring hardware

Physical Loupedeck CT, Stream Deck and Stream Deck+ controls have not been qualified for this release. Native pipe transport and simulated SDK callbacks passed. Real monitoring-device selection, unplug/replug during a long recording, and third-party VST/voice-conversion combinations should be exercised on the intended setup before an important session. Existing plugin/bridge regression checks remain part of the workspace suite; this release does not claim every third-party plugin is tested.

Run the commands in [Workflow development](WORKFLOW_DEVELOPMENT.md) to reproduce the checks. See the [user guide](WORKFLOWS.md) for setup and interrupted-take recovery.
