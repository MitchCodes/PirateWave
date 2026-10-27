"""Render a controlled LLVC-duration comparison without granting acceptance."""
import argparse
import html
import json
from pathlib import Path

from assets import sha256
from probe import atomic_json


def build(baseline, candidate, output, comparison='duration'):
    old = json.loads((baseline / 'report.json').read_text())
    new = json.loads((candidate / 'report.json').read_text())
    old_steps = old['llvc']['optimizer_steps']
    new_steps = new['llvc']['optimizer_steps']
    speaker = html.escape(str(old['recipe']['speaker']))
    old_context = old['recipe'].get('llvc_context_samples', 10240)
    new_context = new['recipe'].get('llvc_context_samples', 10240)
    old_warmup = old['recipe'].get('discriminator_warmup_steps', 0)
    new_warmup = new['recipe'].get('discriminator_warmup_steps', 0)
    if comparison != 'warmup' and old_warmup != new_warmup:
        raise ValueError('Comparison changed discriminator warm-up')
    if comparison == 'duration':
        if old_context != new_context:
            raise ValueError('Duration comparison changed generator context')
        labels = [f'{old_steps:,} updates', f'{new_steps:,} updates']
        explanation = 'Teacher, recordings, paired-data count and pretrained initialization are held fixed.'
    elif comparison == 'context':
        if old_steps != new_steps or old_context == new_context:
            raise ValueError('Context comparison requires equal update budgets and different contexts')
        labels = [f'{old_context/16000:g}-second context', f'{new_context/16000:g}-second context']
        explanation = ('Both candidates use the same update budget, teacher, paired speech and initialization. '
                       'The longer-context candidate feeds 4.096 seconds to the generator and uses full-context generator losses; '
                       'the discriminator update still sees a 0.64-second crop. Short recordings are zero padded.')
    elif comparison == 'warmup':
        if old_steps != new_steps or old_context != new_context or old_warmup == new_warmup:
            raise ValueError('Warm-up comparison requires equal joint budgets/context and different warm-up budgets')
        if not new.get('warmup', {}).get('generator_unchanged'):
            raise ValueError('Missing generator immutability evidence')
        labels = [f'{old_warmup} warm-up updates', f'{new_warmup} warm-up updates']
        explanation = ('Both candidates use the same 1,000 generator updates, teacher, paired speech, context and initialization. '
                       'Warm-up trains only discriminator weights while the generator is frozen. Joint training starts with fresh optimizers '
                       'and the pre-warm-up random state restored. This is an experiment, not an accepted recipe.')
    else:
        raise ValueError('Unknown comparison')
    for key in ('recordings', 'speaker', 'teacher_mode', 'clip_seconds', 'optimizer',
                'content_export_sha256', 'backend', 'teacher_checkpoint_sha256'):
        if old['recipe'].get(key) != new['recipe'].get(key):
            raise ValueError('Duration comparison changed ' + key)
    pair_count = old['pairs']['count']
    if pair_count < 1 or pair_count != new['pairs']['count']:
        raise ValueError('Paired dataset size changed')
    for index in range(pair_count):
        for kind in ('original', 'converted'):
            relative = f'pair-{index}-{kind}.wav'
            if sha256(baseline / relative) != sha256(candidate / relative):
                raise ValueError('Paired training audio changed: ' + relative)
    native = json.loads((candidate / 'native/report.json').read_text())
    if not native['waveform_and_state_pass']:
        raise ValueError('Candidate failed native parity')
    output.mkdir(parents=True, exist_ok=True)
    page = [f'<!doctype html><meta charset="utf-8"><title>LLVC {comparison} comparison</title>',
            '<style>body{font:17px system-ui;max-width:1100px;margin:40px auto;background:#171823;color:white}td{padding:14px}audio{width:100%}table{width:100%}</style>',
            f'<h1>Target {speaker}: LLVC {comparison} comparison</h1><p>Use a monitoring output or headphones; avoid routing browser audio to your outgoing bus. No autoplay. These are experimental voices, not installed packs.</p>',
            f'<p>{explanation} Compare words, voice identity, consonants and robotic artifacts. A passing numerical comparison does not establish better quality.</p>']
    def player(path):
        if not path.is_file():
            raise FileNotFoundError(path)
        return f'<audio controls preload="none" src="{html.escape(path.resolve().as_uri(), quote=True)}"></audio>'
    page.append('<h2>Target reference</h2>' + player(baseline / 'review/target-reference-0.wav'))
    page.append(f'<h2>Native streaming: same three held-out sentences</h2><table><tr><th>{labels[0]}</th><th>{labels[1]}</th></tr><tr>')
    for path in (baseline, candidate):
        page.append('<td>' + player(path / 'native/native-converted.wav') + '</td>')
    page.append(f'</tr></table><h2>Stage comparisons</h2><table><tr><th>Original</th><th>Teacher</th><th>{labels[0]}</th><th>{labels[1]}</th></tr>')
    for index in (3, 4, 5):
        for suffix in ('original', 'teacher'):
            if sha256(baseline / f'review/{index}-{suffix}.wav') != sha256(candidate / f'review/{index}-{suffix}.wav'):
                raise ValueError('Held-out audio changed: ' + suffix)
        paths = [baseline / f'review/{index}-original.wav', baseline / f'review/{index}-teacher.wav',
                 baseline / f'review/{index}-llvc-offline.wav', candidate / f'review/{index}-llvc-offline.wav']
        page.append('<tr>' + ''.join('<td>' + player(p) + '</td>' for p in paths) + '</tr>')
    page.append('</table><script>document.querySelectorAll("audio").forEach(a=>a.addEventListener("play",()=>document.querySelectorAll("audio").forEach(b=>{if(a!==b)b.pause()})));</script>')
    (output / 'review.html').write_text('\n'.join(page), encoding='utf-8')
    atomic_json(output / 'comparison.json', dict(schema=1, accepted=False,
                comparison=comparison, baseline_context_samples=old_context, candidate_context_samples=new_context,
                baseline_warmup_steps=old_warmup, candidate_warmup_steps=new_warmup,
                baseline_report_sha256=sha256(baseline / 'report.json'),
                candidate_report_sha256=sha256(candidate / 'report.json'),
                baseline_steps=old['llvc']['optimizer_steps'], candidate_steps=new['llvc']['optimizer_steps'],
                byte_identical_training_pairs=pair_count,
                native_pass=native['waveform_and_state_pass'], listening_pending=True))


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--baseline', type=Path, required=True)
    p.add_argument('--candidate', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--comparison', choices=['duration', 'context', 'warmup'], default='duration')
    args = p.parse_args()
    build(args.baseline, args.candidate, args.out, args.comparison)
