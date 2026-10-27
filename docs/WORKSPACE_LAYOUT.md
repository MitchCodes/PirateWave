# Workspace layout

## Mixer

**Inputs & sources** stays on the left. Microphones, desktop/application captures, Soundboard and VST instruments retain their existing faders, meters, pan, mute, solo and device controls. Scroll sideways for more source strips and vertically when the available height is short.

**Output buses** stays on the right and scrolls vertically on its own. It remains accessible when more sources are added or the window narrows. A long project can contain more strips/buses than fit at once; both areas stay visible while their contents scroll.

**Processors & utility nodes** starts collapsed below these areas. Its disclosure button shows the total node count. Expand it for compact processor, VST effect, Voice Changer, switch, recording and Auto Ducking cards. Filters are All, Processors (including VST effects and Voice Changer), Switches, Recorders and Auto Ducking. Cards wrap into columns and the section scrolls vertically.

Drag the divider above the cards to resize this section. It starts at 240 logical pixels and uses at most 40% of the current body height, retaining space for the upper panes. The preferred height, expanded state and filter are saved in application settings independently of projects. Adding a utility node expands this section and makes the new node available on return to Mixer.

Compact cards retain their quick controls, Edit/controls links and keybind buttons. Right-click a card or its type heading to remove its node. Detailed effect chains and longer help are available in the corresponding editor.

## Routing

Click an output port, then the appropriate input port to connect. The canvas includes all saved node positions and their full dimensions, including negative positions; narrowing or shortening the window changes the viewport rather than moving nodes. Horizontal and vertical scrollbars provide access to the rest of the canvas.

Drag a node toward an edge to grow the canvas and auto-scroll in that direction. Wires and port hit targets follow the same canvas coordinates. Only dragging changes saved positions; scrolling, finding a node, resetting the view and resizing do not change audio routes or node locations.

Use **Find node** to search by name or type and jump to that node. **Reset view** returns to the canvas origin without rearranging anything. Select a wire and use **Disconnect selected** to remove that particular port connection; right-click nodes for controls or removal. Existing cycle/feedback safeguards still apply.

The grid is painted only within the visible viewport so a large amount of empty canvas does not produce a correspondingly large grid drawing workload.

## Node editors

Effects and dedicated node editors share a node picker grouped by type, with a name/type search, compact status and keybind/back actions. Names in the picker are shortened when necessary; hover for the full name. Selection still uses stable node identities.

- Effect racks keep parameters expanded, with concise toolbars, individual effect cards and expandable About/Signal path information.
- Recording racks separate playback/export controls from explanations of dry capture and live passthrough.
- Auto Ducking groups Activity, Detection, Attenuation & timing, Lookahead and Help.
- Voice Changer groups Voice & status, Conversion controls, Actions and Runtime details. Current warnings remain visible.
- VST nodes group Plugin & status, instrument Audition controls, Plugin parameters and Additional PirateWave effects. Long parameter lists keep their own scrollbar.
- Switches group their destination selector and routing summary.

Opening an editor or expanding help does not start/reconfigure audio, change the project or materialize missing default parameter values. Parameter edits, bypass, device changes and other audio controls retain their normal live-update and autosave behavior. The existing audio engine, controller protocol and shortcut actions are unchanged.

## Mixer card order

Drag the node-type label at the top of a card (such as AUDIO INPUT) onto another card in
that section. Drop on the left/right half to place before/after in Inputs &
Sources and the utility grid; use the top/bottom half for Output Buses. The target
card is highlighted. Cards cannot move between sections. Utility filters show
the same saved order, including when returning to All.

Order autosaves per workspace in application UI settings and is restored after
restart. New nodes append in their section. Reordering does not modify project
node order, routes, DSP state, or issue audio update/restart commands. JSON
workspace exports contain audio configuration; Mixer order stays in UI settings.

## Themes

Settings > General > Appearance > Theme changes the palette immediately and
saves it in application settings. Dark preserves the original appearance and is
the default for existing settings. Other choices are Light, Bumblebee (yellow
and black), Mint Green, Halloween, Christmas, Ocean, Rose, Lavender, Ember, and
Midnight. The choice applies to all workspaces, controls, dialogs, routing, plots,
and recording waveforms. It does not change audio or workspace configuration.
