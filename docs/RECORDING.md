# Recording and playback nodes

Add **Recording** from **Add audio node**. Open its **Recording / Playback** page from its mixer card, its routing context menu, or the Effects rack node picker.

## Routing

A recording node has one mixed input and three independently connectable output types:

- **Passthrough** forwards the live input mix, whether recording is armed or stopped.
- **Preview** carries only slot playback requested by Preview or Preview & send.
- **Send** carries only slot playback requested by Send or Preview & send.

In Routing, click the desired labeled output port on the recording node, then the input port of a processor or output bus. Each output can connect to multiple destinations. Different ports can connect to the same destination and will mix there. Preview and send never enter passthrough automatically. A recording node's slot effects apply only to playback and export; its live passthrough and rolling capture stay dry with respect to that rack. Upstream processors still affect the captured signal. Desktop-loopback preview is allowed because recorded playback is not a live feedback path; live passthrough into the captured device remains protected by the existing feedback check.

Example: Microphone -> Recording input; Passthrough -> voice processor -> stream mix; Preview -> headphones; Send -> virtual cable. Start recording, then capture a recent phrase into a slot and send just that phrase to the cable.

## Arming and slots

**Start recording** arms that node and starts the audio engine if necessary. **Stop recording** stops that node's rolling capture; existing slots remain playable. A node with zero slots allocates no rolling history. Audio engine Stop ends the session, including rolling histories; captured slots remain on disk. Device reconnect recovery keeps the session alive. Hardware reconfiguration preserves compatible active recording histories.

Add up to 64 named slots per node. Each slot has a lookback duration from 0.1 to 600 seconds. **Capture recording** snapshots the most recent processed audio as of the capture request, limited to audio available since that recorder was started. It replaces the slot's current take and resets selection to the full new recording. Capturing does not require stopping recording. Jobs are bounded; an overloaded or overwritten snapshot reports an error instead of silently capturing a different time window.

Drag either waveform boundary or edit **Start** and **End** in seconds to crop non-destructively. **Reset selection** restores the whole take. Changing either selection boundary (by dragging or editing Start/End) or pressing Reset selection stops that slot's preview and send playback, including pending playback loads. **Play preview**, **Send**, **Preview & send**, and exports all use the current selection. **Stop playing recording**, on the recorder page and mixer card, stops all of that node's slot voices on both playback outputs and cancels pending playback loads; the app's Stop all sounds action also stops recording playback. Concurrent playback mixes, with a 32-voice / 256 MiB playback budget.

**Delete recording** clears the slot's take and crop while preserving the slot and its keybinds. **Remove slot** removes its controls and all its shortcuts. In-flight results are versioned so a deleted or replaced slot cannot be resurrected by a late worker reply.

## Slot effects

Open **Slot effects** on the recorder page or mixer card to edit its effects rack. Add, reorder, enable/disable and configure built-in effects just as on a processor node. The rack is shared by all of that recorder's slots. **Bypass slot effects** bypasses the whole rack while retaining each effect's individual enabled state.

Effects run when a slot is previewed, sent, previewed & sent, or saved. Capture never bakes this rack into the take. Each playback starts with fresh effect history; simultaneous slots are independent. Preview & send receives the same processed samples on both ports. Rack edits and toggles update active playback; toggling bypass clears stored effect history. A saved file uses a snapshot of the rack when Save is requested, processed on a file worker.

Playback and export keep the exact selection duration. Delay/reverb tails beyond the selection are trimmed; effects with latency may shift audio within that duration. Crop changes still stop active and deferred playback. Original takes and waveforms remain unchanged.

## Keybinds

**Recording keybinds** opens that node's Start recording, Stop recording, and Stop playing recording actions, plus whole-rack Disable, Enable and Toggle effects. Each rack effect also offers Enable, Disable and Toggle keybinds on this page and from its rack **Keybinds** button. Removing an effect unregisters its shortcuts. Each slot's **Keybinds** button opens only that slot's five actions:

1. Capture recording
2. Preview recording
3. Send recording
4. Preview & send recording
5. Delete recording

Record or type a chord, then **Apply keybinds**. Empty chords are unbound. Registration uses stable node/slot IDs; removing a slot/node releases its keys and stale queued actions are ignored. Clearing a slot's keybinds preserves node-level start/stop bindings and other slots' bindings.

## Saving and storage

**Save WAV / MP3** opens a file dialog. Select the format or use a `.wav` / `.mp3` filename. The selected crop is written by a worker to a temporary sibling file and published only after successful encoding. WAV uses stereo 48 kHz float PCM; MP3 uses the installed Windows Media Foundation encoder and an enumerated stereo 48 kHz output type, preferring 256 kbps. No encoder executable is bundled or downloaded. MP3 support requires the Windows Media Foundation components.

Captured takes are immutable WAV assets in the application's configuration folder under `recordings/`; the project stores their paths, slot settings and selection. Reopening a project loads waveforms and audio asynchronously. Take files detached by deletion/replacement are retained to preserve other saved project versions or Save As copies. User exports are never deleted. Stale worker captures that were never attached to a project are cleaned up. Copy the managed recordings folder as well as the JSON project when transferring projects between machines; there is no embedded-media project archive.

## Shared histories and processing

The engine allocates one history per unique incoming signal, sized for the largest slot among active downstream recorders. Four recorders with twenty slots that all share one microphone use one minute-long microphone history when the largest slot is one minute. Passthrough chains of recording nodes reuse the original histories; mixed inputs are summed when capturing. Different processed signals require separate histories so recordings match the actual graph audio. Switch destinations retain their own gating history, including silence on inactive branches.

Histories store post-processing stereo audio at 48 kHz. They include block rounding and a roughly half-second snapshot reserve, not one buffer per slot. A one-minute shared history uses 23,414,016 bytes in the validation harness. Active histories have a 512 MiB aggregate budget. Growing/shrinking a required history reconstructs that history; capture becomes available as it fills again. Each recording node has its own start epoch so a newly armed recorder does not capture another recorder's older audio.

Recording-only graphs without an opened output stream use absolute software-clock deadlines, so sleep granularity does not cause sample-rate drift. Normal rendering writes preallocated atomic blocks without allocation, disk I/O or waiting for file workers. Workers copy snapshots with generation checks, mix contributors, make waveforms, read/write assets and encode exports. Bounded queues and three file workers keep ordinary export work off the render thread. Waveform metadata is separate from the 256 MiB decoded-clip cache.

Native encoder references: [Microsoft MP3 encoder](https://learn.microsoft.com/en-us/windows/win32/medfound/mp3-audio-encoder), [Media Foundation sink writer](https://learn.microsoft.com/en-us/windows/win32/medfound/using-the-sink-writer).
