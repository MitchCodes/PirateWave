This is the `vst` 0.4.0 crate, retained under its original license. PirateWave
adds editor idle and keyboard dispatch, a host callback for plugin-requested window sizes,
and an editor handle that can be transferred to the editor thread. Editor availability
is checked against the plugin's VST2 flags. The host
keeps the plugin instance and its module alive until the editor thread closes.
Chunk reads validate plugin-reported sizes and pointers before copying data.
The control-thread state handle also reads the selected VST2 program.
These changes support the experimental vendor editor host.
