# Cookbook

Open **Cookbook** in the sidebar and select **Import workspace** on a recipe. Each import creates and opens a separate, autosaved workspace; name conflicts receive a numbered suffix. Your previous workspace is preserved. If audio is running, the normal workspace-switch flow restarts audio with the imported routing.

Every recipe contains a regular Microphone input, a Soundboard routed to both Stream Mix and Monitor, and exactly those two output buses. The microphone reaches Stream Mix through the selected workflow, without a parallel dry route. Monitor uses the default playback device at −12 dB; Stream Mix starts unassigned. Choose your microphone, virtual cable/output device and headphones after import. Add your own sounds to the Soundboard. No application paths, sound files, recordings or global shortcuts are bundled.

| Recipe | Workflow |
| --- | --- |
| Clear Waters | Microphone → Clean Mic → Stream Mix. High-pass filter, expander, de-esser, parametric EQ, compressor and limiter, in that order. |
| Silent Seas | Clear Waters with RNNoise Noise Suppression after the high-pass filter. Suppression starts at 100% strength; lower it if speech sounds processed. Adds about 20 ms latency. |
| Music Mate | Microphone enters Auto Ducking’s Priority input; Spotify enters Ducked. Mixed goes to Stream Mix. |
| Voice Party | Microphone → Change Voice switch → Voice A or Voice B → Autotune & Reverb → Stream Mix. |
| Replay Deck | Recording Input application capture → Recorder. Preview → Monitor; Send → Stream Mix. Microphone goes directly to Stream Mix. |
| Captain’s Studio | Microphone → Clean Mic → Change Voice → Voice A/B → Autotune & Reverb → Auto Ducking → Stream Mix. Spotify supplies Ducked. Soundboard and the Recording Input/Recorder pair complete the workspace. |

Expand **Setup & tuning** on each card before import for guidance. Spotify and Recording Input start with no application selected: use their application pickers, so the saved target matches your installed executable. Recorder starts stopped, with empty 5, 15 and 30 second capture slots. Start recording before capturing a slot. Live recording-source audio is not sent through a passthrough route.

Voice recipes use actual Voice Changer nodes. Choose and qualify their voice packs before use; unavailable models do not silently pass dry audio. Both initially reference the default pack; choose your preferred pack in each node. Direct switch branches share a single conversion runtime: changing the selected voice reinitializes conversion and may briefly interrupt the voice while it loads. Change Voice selects Voice A initially. You can choose different packs and assign switch keybinds in the usual editors. Autotune uses automatic voice correction in chromatic mode; choose a key and scale for singing. Reverb starts at a restrained 12% wet mix.

The cleaning settings are starting points: 80 Hz high-pass, gentle downward expansion, bounded de-essing, a broad −2 dB cut at 250 Hz, 3:1 compression, and a −1 dB limiter. Tune thresholds to your input level and room. Ducking uses the normal Level defaults, including −18 dB attenuation and no lookahead.

Recipes create ordinary project data; browsing the Cookbook performs no audio processing or device discovery. Imported nodes, effects, routes and slots remain fully editable and work with existing shortcuts and controller integrations. Browsing never marks the current workspace dirty.
