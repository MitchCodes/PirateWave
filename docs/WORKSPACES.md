# Workspaces

Use the workspace name at the top left to switch workspaces, create a new one,
or rename the selected workspace. The initial workspace is **Workspace 1**.
Existing studio.json data is copied into it on first startup; the original file
is retained. The last selected workspace is restored on the next launch.

Names are trimmed and must be nonempty and unique (case-insensitive). New
workspaces receive the next available Workspace number. Import creates a separate
managed workspace; conflicting names receive suffixes such as `Studio 2` and
`Studio 3`. The imported JSON is never used as the autosave destination.

Changes autosave. Switching first saves pending changes; a save/load failure
leaves the current workspace in place. Switching during playback is supported:
PirateWave replaces the audio session and starts the selected workspace automatically.
Device reopening can briefly interrupt audio. Playing sounds, rolling recording
history, deferred slot playback, and controller generations are not carried over.
Captured recordings remain referenced by their existing asset paths.

**Export workspace...** writes the current configuration to JSON without changing
its managed location. **Import workspace...** validates a JSON project and adds
it as a new workspace, then selects it. Exports reference media and plugins; they
are not self-contained media bundles. Global application settings and controller
assignments remain shared across workspaces.

Managed files live beside the application settings in `workspaces/`: `index.json`
contains names and the active ID; each numeric-ID JSON contains a project. Names
are never used as filenames. Saves use the existing atomic JSON replacement path.

Use **Clone current workspace** to duplicate the current configuration, including
pending edits, into a new selected workspace. Names use the full current name
plus the first available suffix starting at 2 (for example, `Studio 2`).

**Delete current workspace** asks for confirmation. Cancel keeps the workspace.
Confirm removes its managed configuration and selects another workspace. If it
was the last workspace, PirateWave creates a fresh replacement. Audio restarts in the
replacement if it was running. Shared recordings, sound files, and plugin files
are retained so other workspaces continue to work.
