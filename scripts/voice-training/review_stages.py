"""Compare the same held-out utterance at each existing pipeline stage.

Native excerpts preserve the streaming timeline (including causal delay); they
are not time-warped, normalized, or evidence of perceptual quality.
"""
import argparse
import html
import json
from pathlib import Path

import numpy as np
import soundfile as sf

from assets import sha256
from probe import atomic_json


def build(root, output):
    output.mkdir(parents=True, exist_ok=True)
    rows = []
    page = ['<!doctype html><meta charset="utf-8"><title>PirateWave stage diagnosis</title>',
            '<style>body{font:17px system-ui;max-width:1050px;margin:40px auto;background:#171823;color:#eee}section{padding:20px;background:#242637;margin:24px 0}audio{width:100%}label{display:block;margin:14px 0}textarea{width:95%;min-height:100px}</style>',
            '<h1>Where does the robotic sound begin?</h1><p>Use headphones and select a monitoring output in Windows Volume mixer. Do not send browser audio to your outgoing bus. Nothing autoplays.</p>',
            '<p>Compare the same sentence at each stage. Native excerpts include the existing streaming delay and a half-second tail. No normalization or timing correction is applied. These are existing 250-step teacher / 500-step LLVC candidates, not improved voices.</p>']
    for speaker in ('1462', '1272', '1673'):
        candidate = root / speaker
        report = json.loads((candidate / 'native/report.json').read_text())
        if not report['waveform_and_state_pass']:
            raise ValueError('Native validation failed: ' + speaker)
        native_path = candidate / 'native/native-converted.wav'
        native, rate = sf.read(native_path, dtype='float32')
        if rate != 16000 or native.ndim != 1 or not np.isfinite(native).all():
            raise ValueError('Invalid native audio')
        page.append(f'<section><h2>Target {speaker}</h2><label>Target reference<audio controls preload="none" src="{html.escape((candidate / "review/target-reference-0.wav").resolve().as_uri(), quote=True)}"></audio></label>')
        offset = 0
        for index in (3, 4, 5):
            original = candidate / f'review/{index}-original.wav'
            samples, sample_rate = sf.read(original)
            if sample_rate != rate or samples.ndim != 1:
                raise ValueError('Unexpected source audio format')
            end = offset + len(samples) + 8000
            if end > len(native):
                raise ValueError('Native recording is too short for an unchanged excerpt')
            excerpt = output / f'{speaker}-{index}-native.wav'
            sf.write(excerpt, native[offset:end], rate, subtype='PCM_16')
            paths = [original, candidate / f'review/{index}-teacher.wav',
                     candidate / f'review/{index}-llvc-offline.wav', excerpt]
            page.append(f'<h3>Held-out sentence {index - 2}</h3>')
            files = []
            for label, path in zip(('Original', 'Teacher', 'LLVC offline', 'Native streaming'), paths):
                data, sample_rate = sf.read(path)
                finite = bool(np.isfinite(data).all())
                if not finite:
                    raise ValueError('Non-finite audio: ' + str(path))
                files.append(dict(stage=label, sha256=sha256(path), seconds=len(data)/sample_rate,
                                  peak=float(np.max(np.abs(data))), rms=float(np.sqrt(np.mean(data**2))), finite=finite))
                page.append(f'<label>{label}<audio controls preload="none" src="{html.escape(path.resolve().as_uri(), quote=True)}"></audio></label>')
            rows.append(dict(target=speaker, source_index=index, native_source_sha256=sha256(native_path),
                             native_start_sample=offset, native_end_sample=end, files=files))
            offset = end
        page.append('<label>Notes: Does the teacher already sound robotic? Does LLVC make it worse? Does native sound different from offline?<textarea></textarea></label></section>')
    page.append("<button id='save'>Save listening notes</button><script>document.querySelectorAll('audio').forEach(a=>a.addEventListener('play',()=>document.querySelectorAll('audio').forEach(b=>{if(a!==b)b.pause()})));document.getElementById('save').onclick=()=>{const notes=[...document.querySelectorAll('section')].map(s=>({target:s.querySelector('h2').textContent,notes:s.querySelector('textarea').value}));const u=URL.createObjectURL(new Blob([JSON.stringify({accepted:false,notes},null,2)],{type:'application/json'}));const a=document.createElement('a');a.href=u;a.download='stage-listening-notes.json';a.click();setTimeout(()=>URL.revokeObjectURL(u),1000)};</script>")
    (output / 'review.html').write_text('\n'.join(page), encoding='utf-8')
    atomic_json(output / 'evidence.json', dict(schema=1, release_accepted=False, rows=rows,
                note='Signal statistics and existing native parity cannot identify perceptual artifacts; listening required.'))
    return output / 'review.html'


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--root', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    args = p.parse_args()
    print(build(args.root, args.out))
