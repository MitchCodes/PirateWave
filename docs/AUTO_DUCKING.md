# Auto Ducking

Add **Auto Ducking** from Add audio node. Open **Ducking controls** on its Mixer card or Routing context menu. Effects rack selection also opens its controls; route other processor nodes before or after it for additional effects.

## Routing

Connect the trigger signal to **Priority** and the audio to turn down to **Ducked**. Multiple sources can mix into each input, and a source can connect to both. Each wire has its own destination input; select a wire to disconnect only that route.

The three outputs work simultaneously:

- **Mixed** sums Priority and attenuated Ducked audio.
- **Priority** carries the priority audio without ducking.
- **Ducked** carries only the attenuated background audio.

There is no automatic normalization or limiter. Allow headroom when summing; final output buses retain their normal clipping protection. Routing multiple outputs to the same destination adds them, so combining Mixed with Priority duplicates the Priority signal. A downstream Recording node captures the actual selected output, including ducking and lookahead.

## Detection and timing

**Level** mode (default) responds to a rolling 10 ms RMS measurement on the louder Priority channel. The default threshold is -35 dBFS (-80 to 0 dBFS). Activity releases 6 dB below the threshold, avoiding chatter.

**Speech** mode uses the already bundled RNNoise model, independently analyzing each channel in 10 ms frames and using the larger speech probability. Only detection uses the model: audible Priority audio is not denoised or otherwise replaced. The level threshold remains a minimum level guard. **Speech sensitivity** is the required probability, default 60% (10–95%); a higher value requires more confident speech. Activity releases 15 percentage points below that setting, floored at 1%. It is a probabilistic classifier, not a guarantee that music, noise or a particular voice will be classified correctly. No model download is needed.

**Duck amount** defaults to -18 dB (-60 to 0 dB). **Attack** defaults to 10 ms (1–500 ms), **Hold** to 150 ms (0–2,000 ms), and **Release** to 400 ms (10–5,000 ms). Attack and Release use exponential smoothing in dB (approximately 63% of the change per configured time). Activity restarts Hold; Release begins when activity and Hold end. The same attenuation applies to both background channels, preserving stereo balance. Meters show both input peaks, detected activity and current reduction.

Missing or silent Priority audio releases attenuation normally; missing Ducked audio yields silence on that separate output. When either input is disconnected or its mixed block is entirely silent (including muted/unavailable sources), the node skips both level and speech detection. After pending Hold/Release completes, undelayed audio uses direct pass-through copying. Enabled lookahead still delays both outputs and drains buffered background audio correctly; it is never dropped or shortened by this optimization. Detection resumes automatically on a fresh analysis window when both signals return. A connected input carrying nonzero noise still counts as audio and may require detection.

Only signal checks, output routing/metering, configured delay and any remaining gain transition run on the inactive path; no speech inference runs once there is no background signal left to duck. Detection continues while bypassed when both inputs are active. If Speech detection fails, the node reports Level fallback and retries Speech initialization outside rendering.

## Lookahead and bypass

Lookahead defaults to **Off**, adding no audio delay. Enabling it remembers a default **10 ms**, adjustable from 0–50 ms. Both input audio paths are delayed equally, keeping all three outputs aligned. Detection analyzes the undelayed Priority signal, allowing ducking to start before the delayed sound. Speech analysis and Attack still require time; short lookahead does not guarantee fully ducked initial consonants.

Changing the effective lookahead clears the delay history, outputs silence while refilling, and fades in over 5 ms. Unrelated routing edits preserve detector, envelope and delay history. **Bypass ducking** smoothly removes attenuation while retaining configured lookahead and routing. Priority audio remains unchanged except for lookahead and its brief reconfiguration fade.

## Keybinds and Loupedeck

**Ducking keybinds** offers Enable ducking, Disable ducking and Toggle ducking. Shortcuts are scoped by stable node identity and are unregistered when the node is removed.

Loupedeck exposes those actions under Nodes and all parameters under **Auto Ducking parameters**. Knobs use the parameter's displayed step, with configurable sensitivity; presses restore defaults. Detection and lookahead mode are discrete choices. Exact node names must uniquely match in the current project, as with other PirateWave controller assignments. No existing profiles are changed automatically.
