"""Sensitivity diagnostic: unchanged weights, PyTorch float32 versus float64.

The float64 path is an oracle experiment, not an accepted runtime profile.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys

import numpy as np
import torch

from diagnose_states import error
from export_llvc import SOURCE_REV, WEIGHTS_SHA


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--upstream', type=Path, required=True)
    p.add_argument('--checkpoint', type=Path, required=True)
    p.add_argument('--fixture', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    args = p.parse_args()
    if args.out.exists():
        p.error('Use a new output path to preserve evidence')
    assert subprocess.check_output(['git', '-C', str(args.upstream), 'rev-parse', 'HEAD'], text=True).strip() == SOURCE_REV
    assert hashlib.sha256(args.checkpoint.read_bytes()).hexdigest() == WEIGHTS_SHA
    sys.path.insert(0, str(args.upstream.resolve()))
    from model import Net
    torch.set_num_threads(1)
    torch.set_num_interop_threads(1)
    model = Net(**json.loads((args.upstream / 'experiments/llvc/config.json').read_text(encoding='utf-8'))['model_params'])
    model.load_state_dict(torch.load(args.checkpoint, map_location='cpu', weights_only=True)['model'])
    model.eval()
    oracle = copy.deepcopy(model).double()

    class ConstantLabel(torch.nn.Module):
        def __init__(self, value):
            super().__init__()
            self.register_buffer('value', value)

        def forward(self, label):
            return self.value

    # Net creates its fixed zero label in default float32. Calculate the SAME
    # constant in double here so all learned operations use oracle precision.
    with torch.inference_mode():
        oracle.label_embedding = ConstantLabel(oracle.label_embedding(torch.zeros(1, 1, dtype=torch.float64)))
    meta = json.loads((args.fixture / 'fixture.json').read_text(encoding='utf-8'))
    packets = np.fromfile(args.fixture / 'quiet_speech.input.f32', dtype='<f4').reshape(-1, *meta['shapes'][0])
    states = [torch.zeros(s) for s in meta['shapes'][1:]]
    recurrent = [s.double() for s in states]
    records = []
    with torch.inference_mode():
        for index, packet in enumerate(packets):
            initial = [s.double() for s in states]
            expected = model(torch.from_numpy(packet.copy()), *states, pad=not model.lookahead)
            states = list(expected[1:])
            forced = oracle(torch.from_numpy(packet.copy()).double(), *initial, pad=not oracle.lookahead)
            free = oracle(torch.from_numpy(packet.copy()).double(), *recurrent, pad=not oracle.lookahead)
            recurrent = list(free[1:])
            row = {'chunk': index}
            for mode, values in [('identical_input', forced), ('free_running', free)]:
                row[mode] = {name: error(a.numpy(), b.numpy()) for name, a, b in zip(meta['output_names'], values, expected)}
                e = row[mode]['converted']
                e['pass'] = e['max'] <= 1e-4 and e['rms'] <= 1e-5
            records.append(row)
    report = {'schema': 1, 'diagnostic': 'PyTorch float64 against unchanged float32 reference; not a runtime qualification',
              'checkpoint_sha256': WEIGHTS_SHA, 'source_revision': SOURCE_REV, 'torch_version': torch.__version__,
              'case': 'quiet_speech', 'hop': meta['hop'], 'seconds': len(packets)*meta['hop']/meta['sample_rate'],
              'chunks': records, 'pass': all(e['pass'] for r in records for m in ['identical_input','free_running'] for e in r[m].values())}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2), encoding='utf-8')
    for mode in ['identical_input','free_running']:
        for name in meta['output_names']:
            print(mode, name, 'normalized RMS',max(r[mode][name]['normalized_rms'] for r in records),
                  'normalized peak', max(r[mode][name]['normalized_max'] for r in records),
                  'first failing chunk', next((r['chunk'] for r in records if not r[mode][name]['pass']), None))
    return 0 if report['pass'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
