"""Compare the pinned initial LLVC voice with trained voices on identical speech."""
import argparse
import html
import json
from pathlib import Path
import sys

import numpy as np
import soundfile as sf
import torch

from assets import LLVC_REV, sha256
from probe import atomic_json, verify_source


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--root', type=Path, default=Path.cwd())
    args = p.parse_args()
    root = args.root.resolve()
    output = root / 'dist/voice-training/pretrained-review'
    upstream = root / '.tools/voice-feasibility/LLVC'
    initial = root / '.tools/voice-feasibility/llvc.pth'
    expected = 'cceb7ab9621f84d62d283725ae3281cacb04f762d6a088fe3e97c1a29d4b8c0e'
    if sha256(initial) != expected:
        raise ValueError('Initial checkpoint does not match the pinned asset')
    verify_source(upstream, LLVC_REV)
    sys.path.insert(0, str(upstream))
    from model import Net
    torch.set_num_threads(2)
    torch.manual_seed(42)
    config = json.loads((upstream / 'experiments/llvc/config.json').read_text())
    original_state = torch.load(initial, map_location='cpu', weights_only=True)['model']
    model = Net(**config['model_params']).eval()
    model.load_state_dict(original_state)
    candidates = [
        ('Untouched pretrained voice', output),
        ('1462: 500 updates', root / 'dist/voice-training/llvc-review/1462'),
        ('1462: 1,000 updates, short context', root / 'dist/voice-training/llvc-duration/1462-1000'),
        ('1462: 1,000 updates, long context', root / 'dist/voice-training/llvc-context/1462-long')]
    baseline = candidates[2][1]
    (output / 'review').mkdir(parents=True, exist_ok=True)
    sources = []
    with torch.inference_mode():
        for index in (3, 4, 5):
            path = baseline / f'review/{index}-original.wav'
            audio, rate = sf.read(path, dtype='float32')
            if rate != 16000 or audio.ndim != 1 or not np.isfinite(audio).all():
                raise ValueError('Unexpected source audio contract')
            result = model(torch.from_numpy(audio)[None, None])[0, 0].numpy()
            if not np.isfinite(result).all():
                raise ValueError('Non-finite pretrained output')
            sf.write(output / f'review/{index}-llvc-offline.wav', result, rate, subtype='PCM_16')
            sources.append(dict(index=index, sha256=sha256(path)))
    page = ['<!doctype html><meta charset="utf-8"><title>Pretrained LLVC diagnosis</title>',
            '<style>body{font:17px system-ui;max-width:1200px;margin:40px auto;background:#171823;color:#eee}section{padding:20px;background:#242637;margin:24px 0}audio{width:100%}td{padding:12px}table{width:100%}</style>',
            '<h1>Does the starting LLVC model already sound robotic?</h1>',
            '<p>The untouched pretrained model has a different target voice. Compare clarity, consonants and metallic/robotic artifacts—not whether its identity matches target 1462. Each column uses the same source speech. No audio is normalized or enhanced.</p>',
            '<p>Use headphones or a monitoring output. Avoid routing browser audio to an outgoing bus. Nothing autoplays.</p>']
    def player(path):
        if not path.is_file():
            raise FileNotFoundError(path)
        return f'<audio controls preload="none" src="{html.escape(path.resolve().as_uri(), quote=True)}"></audio>'
    rows = []
    worker = None
    for label, folder in candidates:
        native = json.loads((folder / 'native/report.json').read_text())
        if not native['waveform_and_state_pass']:
            raise ValueError('Native parity failed: ' + label)
        # The older 500-step worker differs only in its Windows file-open path.
        if folder != candidates[1][1]:
            worker = worker or native['worker_sha256']
            if worker != native['worker_sha256']:
                raise ValueError('Native worker differs between primary comparisons')
        row = dict(label=label, native_model_sha256=native['model_sha256'],
                   worker_sha256=native['worker_sha256'], native_pass=True)
        if folder != output:
            for source in sources:
                if sha256(folder / f"review/{source['index']}-original.wav") != source['sha256']:
                    raise ValueError('Review sources differ')
            state = torch.load(folder / 'llvc-generator.pt', map_location='cpu', weights_only=True)['model']
            changed, elements, delta_sum, ref_sum = 0, 0, 0.0, 0.0
            for key, before in original_state.items():
                after = state[key]
                if before.is_floating_point():
                    changed += int(torch.count_nonzero(after != before))
                    elements += before.numel()
                    delta_sum += float(((after.double() - before.double()) ** 2).sum())
                    ref_sum += float((before.double() ** 2).sum())
            row.update(changed_floating_elements=changed, floating_elements=elements,
                       relative_weight_l2=(delta_sum / max(ref_sum, 1e-30)) ** .5)
            del state
        rows.append(row)
        page.append(f'<section><h2>{html.escape(label)} — native streaming</h2>' + player(folder / 'native/native-converted.wav') + '</section>')
    for source in sources:
        index = source['index']
        page.append(f'<section><h2>Sentence {index - 2}: original and teacher</h2>' + player(baseline / f'review/{index}-original.wav') + player(baseline / f'review/{index}-teacher.wav'))
        page.append('<table><tr>' + ''.join(f'<th>{html.escape(label)}</th>' for label, _ in candidates) + '</tr><tr>')
        page.append(''.join('<td>' + player(folder / f'review/{index}-llvc-offline.wav') + '</td>' for _, folder in candidates))
        page.append('</tr></table></section>')
    page.append('<script>document.querySelectorAll("audio").forEach(a=>a.addEventListener("play",()=>document.querySelectorAll("audio").forEach(b=>{if(a!==b)b.pause()})));</script>')
    (output / 'review.html').write_text('\n'.join(page), encoding='utf-8')
    atomic_json(output / 'comparison.json', dict(schema=1, initial_checkpoint_sha256=expected,
                sources=sources, models=rows, release_accepted=False, listening_pending=True,
                interpretation='Weight changes confirm training changed the model; they do not establish perceptual improvement. The pretrained target identity differs from 1462.'))


if __name__ == '__main__':
    main()
