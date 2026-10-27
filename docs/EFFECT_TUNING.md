# Tuning effects in PirateWave

Open a node's effects rack from Mixer, Routing or Effects. The same controls
appear on each path. Existing effect parameters, keybinds and controller
assignments continue to work.

## Compare and reset

**Save comparison** remembers an effect's current parameters for this session.
Adjust the sliders, then **Swap comparison** to alternate between the saved and
current settings. The effect's Enabled state stays independent so you can also
compare processed and dry audio. Comparison settings are discarded when the
effect is removed; they are not saved in projects.

**Reset effect** restores all parameter defaults. Numeric parameters also have
individual Reset buttons. Positive frequency and time sliders use logarithmic
spacing for finer adjustment at low values; their numeric fields remain editable.
Comparison swaps, resets and tempo-note buttons use the normal parameter-update
and autosave path. Merely opening a guide or monitor does not edit the project.

## Effect-specific guides

Expand **Tuning guide** where available:

| Effects | Aid |
| --- | --- |
| Equalizer, High Pass | Configured frequency response, logarithmic Hz scale and dB gain |
| Compressor, Gate, Limiter, Gain, Automatic Gain Control, Expander, Upward Compressor | Input/output target curves against an unchanged-level reference |
| Delay | Predicted echo arrivals and levels; tempo calculator with quarter, eighth, dotted-eighth and triplet buttons |
| Tremolo | Two modulation cycles and their gain envelope |
| Flanger, Chorus, WahWah | Cycle duration/rate; WahWah frequency sweep range |
| Pitch, Resonator | Frequency multiplier or tuning pitch |
| Stereo, De-esser | Focused listening and live-analysis guidance |

Hover plots to read axis coordinates. Filter curves follow the actual built-in
filter equations. Dynamics curves show settled targets, not instantaneous
detector state; attack and release affect the sound in motion. Delay plots model
an impulse's repeats, not arbitrary recorded audio. The tempo field is a local
calculator: only clicking a note changes the effect's delay time. Notes outside
the delay's supported range are disabled.

Other processors retain their existing controls and can use comparison/reset and
the live monitor. Recording racks get the static tuning tools and comparison
controls, but do not show capture audio as though it were slot-effect playback.

## Live before / after

On a non-recording rack, expand **Live before / after**, then select **Monitor
this effect**. Start audio and route a source through the node. One effect can
be monitored at a time, including installed extension effects. Selecting another
effect transfers monitoring to it.

- Purple is the effect's input; green is its output. These are measured at this
  exact effect in the chain, before the node's gain or output-bus limiting.
- The spectrum uses 2048 samples at 48 kHz with a Hann window. Left and right
  channels are analyzed independently; each bin displays the louder channel.
- RMS displays the louder channel of the latest block. Peak warnings identify
  output at/above 0 dBFS. Level change includes output gain, wet/dry and tails;
  it is not an internal compressor gain-reduction meter.
- Stereo correlation near +1 means similar channels, near zero means low
  correlation, and near -1 indicates possible mono cancellation. Silence does
  not display a misleading correlation value.
- The waveform shows the selected left/right channel's latest block. Delayed
  effects are not time-aligned to their input for this display.
- Disabled/bypassed effects display their unchanged input/output and Bypassed.

Capturing and polling stop when the monitor is collapsed, fully outside the
scroll viewport, deselected, on another page, minimized, or audio stops. A
600 ms renewable lease also expires if UI updates stop. Returning to a visible
selected monitor starts fresh analysis. Pending packets from an earlier
selection are rejected. No extra input devices or audio streams are opened.

## Reusing the controls

`src/ui/audio_widgets.rs` supplies clipped multi-curve plots with linear/log axes
and cursor readouts, plus the shared EQ response calculation used by Tools.
`src/ui/effect_tuning.rs` supplies session comparisons, guide selection and live
monitoring. Add an effect-specific guide there only when it conveys useful,
accurate information; derive defaults and ranges from its factory descriptors.
New rack effects automatically use the existing core tap by stable effect ID.

`Graph::monitor` is an optional, fixed-size pre/post block capture. Rendering
does no analysis, allocation, locking, logging or I/O for the monitor. The
desktop host publishes to a bounded 32-packet queue without waiting; full queues
drop diagnostic frames only. FFT analysis runs on the UI thread at most 20 times
per second while visible. No audio signal is delayed or modified. Do not put
FFT/catalog/serialization work into the graph tap.

## Validation

Tests compare predicted filter responses and dynamics targets to actual DSP,
check exact tap placement, stereo, bypass and absent targets, and check
visibility, collapse, deletion, stale packets and project neutrality. The
`builtin_graph_check` example checks allocations and CPU with monitoring off/on.

`PirateWave.exe --effect-tuning-smoke` generates six native screenshots at wide and
narrow sizes, using synthetic samples and opening no devices. These screenshots
validate presentation, not physical microphone or speaker performance.
