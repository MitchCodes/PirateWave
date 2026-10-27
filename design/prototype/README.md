# Original interface prototype

This HTML/CSS/JavaScript design preceded the native Rust application. It is preserved as a design reference and does not connect to the Rust audio engine.

Open `index.html` in a browser, or serve the repository with `python -m http.server 5173` and visit `http://localhost:5173/design/prototype/`.

The four views demonstrate mixer controls, effect parameters, sound-pad shortcuts, and draggable routing. Audio is synthesized preview tones; device names and meters are illustrative. Shortcuts only work while the preview is focused. The `previews/` folder contains archived native UI screenshots for visual reference. The prototype uses the shared icon in `assets/branding`.
