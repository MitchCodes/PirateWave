# Application Capture

Application Capture is a Windows audio source that captures one application's process tree, independently of the playback device it uses.

## Use

1. Open the application you want to capture and start its audio.
2. In PirateWave, choose **Add node > Application Capture**, enter a name, and create it.
3. On its Mixer card, click **Refresh apps** and choose the application's window.
4. In Routing, connect the source to an output bus, directly or through effects processors. Assign the bus a playback device and start audio.

The source has normal gain, balance, mute, solo, effects and all 14 source keybind actions. New keybinds are unbound. Its selection and controls are saved with the project.

Selection saves the executable's full path and a window-title hint, rather than a temporary PID. Matching prefers the saved title, then another visible window of the same executable, then a running process without a visible window. **Match exact window title** requires the saved title (case-insensitive); leave it off for applications whose title changes with their content. If several instances share a path, use a stable exact title to distinguish them. Click Refresh apps to update the picker after opening an application.

Missing applications produce silence and a waiting message. The capture worker retries once per second and reconnects when the saved application returns with a new PID. Capture failures do not stop other sources or output buses. Stalled streams also have bounded watchdog recovery; see [Audio recovery](AUDIO_RECOVERY.md). Changing the selected application reconfigures the audio session.

## Scope and requirements

- Includes audio produced by the selected process and its child processes. This is application capture, not individual browser-tab or individual-window audio isolation.
- Requires Windows process-loopback support. Microsoft's sample specifies Windows build 20348 or later; Windows 11 is supported. Unsupported activation is reported in diagnostics. OBS exposes its feature on some earlier Windows 10 builds; PirateWave does not claim those builds have been verified.
- Protected content and applications that Windows cannot capture can remain silent. Apps running with greater privileges may be missing from the picker.
- No OBS installation, injected DLL or virtual audio cable is required for capture. A virtual cable remains useful when sending the resulting mix to another application's microphone input.
- PirateWave's own windows are excluded from selection to avoid capturing its output back into itself. Other feedback routes remain the user's responsibility.

## Implementation

`src/application_capture.rs` resolves each saved selection and shares one MTA COM capture worker per target PID across nodes, with a separate bounded queue for each reader. This avoids Windows rejecting duplicate clients for the same process; removing one node leaves the other readers running. The worker calls `ActivateAudioInterfaceAsync` with `VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK`, an `AUDIOCLIENT_ACTIVATION_PARAMS` blob and `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE`. An agile completion handler owns activation parameters until completion, including cancellation and late callbacks. Capture uses shared-mode 48 kHz stereo float audio, event notifications and bounded queues; silent packets are explicitly zeroed. Shutdown joins the worker and releases the client and handles.

The engine reads the queue through its existing occupancy-corrected `QueueReader`. The graph treats this node as a source, so routing, processors, channel controls and shortcuts use existing code paths. Capture status uses a separate bounded message queue so one unavailable application does not stop all audio.

Microsoft references: [process-loopback sample](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/), [process inclusion modes](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ne-audioclientactivationparams-process_loopback_mode), and [sample implementation](https://github.com/microsoft/Windows-classic-samples/blob/main/Samples/ApplicationLoopback/cpp/LoopbackCapture.cpp). OBS describes the feature in its [Application Audio Capture guide](https://obsproject.com/kb/application-audio-capture-guide).

## Installed OBS inspection

REA 6.4.0 with Ghidra 12.1.4 inspected the installed OBS 32.2.2 plugin at `C:/Program Files/obs-studio/obs-plugins/64bit/win-wasapi.dll`, found adjacent to the supplied OBS installation. SHA-256: `876ef593d3faeb62f8422b9f1dcbd62ae1a94147b9586590e39307cb3fba102d`. This was static analysis; OBS was not launched or attached.

| Address | Observation | REA evidence |
| --- | --- | --- |
| `0x1800080c0` | Registers `wasapi_process_output_capture`; registration gate accepts Windows 10 build 19041+ or Windows major 11+. This gate alone does not establish API support. | `ev_1787b24b0d0ad2b0137d4ee8c5fa375cf81780ee89c0a7319b6ce86c280fed7f` |
| `0x180005730` | Builds window list and title/class/executable matching priorities. | `ev_41d9d69c025d4e1a9cd5b04efca117fdba07159adf1495f782239d2f2078439a` |
| `0x1800033b0` | Resolves selected window with `ms_find_window`, then gets its process ID. | `ev_b3eb5c68fc9d019e6aa99dbb04a6b9eef1b0c5d8585cfbd8ec7c66d94a97b23f` |
| `0x180002ea0` | Activates `VAD\\Process_Loopback` with a 12-byte activation blob: process activation type 1, selected PID and include-tree mode 0. Uses float32 shared audio with loopback/event flags. | `ev_f1a51ee2f85a249875a5ada0358c622230bcf45269f45f5e828dc6221e67d76e` |
| `0x180003e50` | Checks the selected window, handles silent packets and releases capture buffers; exits on device invalidation. | `ev_2663ae8b58bca7000289880a7b9edc4f823ef80eab22579c6919800c77d20d90` |
| `0x1800054d0` | Selection changes signal worker reconfiguration. | `ev_9dd4d4d1c6c7e90147398982654e23a1db5b3ec1396fc72b0851740615ab979f` |

Windows constant names are interpretations of decoded arguments against SDK definitions. Internal addresses are retained because the adjacent PDB was not explicitly loaded. Exact OBS retry timing and matching internals in `obs.dll` were not traced. PirateWave uses independently written Rust Windows API integration, its own selection/lifecycle logic and existing graph; no OBS source or decompiled function bodies were copied. Owned REA sessions were closed. Raw local investigation files are under ignored `dist/application-capture-research/`.

## Verification

Unit tests cover saved selection validation/round-trip, routing and mute, source shortcuts, path matching, title preference and PID changes. A headless UI interaction test creates a node, chooses an application, changes title matching and opens its keybind editor. Native UI smoke coverage includes Mixer, creation, keybinds and Routing.

`PirateWave.exe --application-capture-self-test` performs an opt-in native test using a VB-Audio CABLE playback endpoint as an isolated test sink. It starts two owned fixture applications, each with a child process generating a different tone. It checks process-tree capture, rejection of the other application, routed effects, concurrent readers, live engine input/output, source mute, absent-target silence, restart with a new PID and shutdown. The test does not change Windows defaults or mute state and closes its own fixture processes. A full live run measured **55.5 dB** separation from the unrelated application. Its report is written to `dist/application-capture-self-test.txt`.
