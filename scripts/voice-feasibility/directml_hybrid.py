"""Export and qualify a CPU-encoder/DirectML-transformer streaming candidate.

No audio devices or production packs are modified. Original model arithmetic,
state tolerances and weights are retained; GPU work is explicitly identified.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time

import numpy as np
import onnxruntime as ort
import torch

from diagnose_states import error
from export_llvc import SOURCE_REV, WEIGHTS_SHA


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--upstream', type=Path, required=True)
    p.add_argument('--checkpoint', type=Path, required=True)
    p.add_argument('--fixture', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--device-id', type=int, required=True)
    p.add_argument('--seconds', type=float, default=12)
    p.add_argument('--cases', nargs='+', default=['quiet_speech', 'silence', 'loud_speech'])
    args = p.parse_args()
    if args.out.exists():
        p.error('Use a new output directory to preserve evidence')
    assert subprocess.check_output(['git', '-C', str(args.upstream), 'rev-parse', 'HEAD'], text=True).strip() == SOURCE_REV
    assert not subprocess.check_output(['git', '-C', str(args.upstream), 'status', '--porcelain', '--untracked-files=no'], text=True).strip()
    assert hashlib.sha256(args.checkpoint.read_bytes()).hexdigest() == WEIGHTS_SHA
    sys.path.insert(0, str(args.upstream.resolve()))
    from model import Net
    torch.set_num_threads(2)
    torch.set_num_interop_threads(1)
    net = Net(**json.loads((args.upstream/'experiments/llvc/config.json').read_text())['model_params'])
    net.load_state_dict(torch.load(args.checkpoint, map_location='cpu', weights_only=True)['model'])
    net.eval()
    meta = json.loads((args.fixture/'fixture.json').read_text())
    assert meta['hop'] == 416

    class Front(torch.nn.Module):
        def __init__(self):
            super().__init__()
            self.net = net

        def forward(self, audio, enc, pre):
            conv, pre = self.net.convnet_pre(audio, pre.clone())
            x = self.net.in_conv(audio + conv)
            label = self.net.label_embedding(torch.zeros(1, 1))
            e, enc = self.net.mask_gen.encoder(x, enc.clone())
            l = label.unsqueeze(2) * e
            return x, l, self.net.mask_gen.proj_e2d_l(l), self.net.mask_gen.proj_e2d_e(e), enc, pre

    class Decoder(torch.nn.Module):
        def __init__(self):
            super().__init__()
            self.decoder = net.mask_gen.decoder

        def forward(self, target, memory, state):
            return self.decoder(target, memory, state.clone())

    class Finish(torch.nn.Module):
        def __init__(self):
            super().__init__()
            self.net = net

        def forward(self, x, label, mask, synthesis):
            mask = label + self.net.mask_gen.proj_d2e(mask)
            x = torch.cat((synthesis, x * mask), dim=-1)
            return self.net.out_conv(x), x[..., -self.net.out_buf_len:]

    args.out.mkdir(parents=True)
    with torch.inference_mode():
        initial = [torch.zeros(s) for s in meta['shapes']]
        front = torch.jit.trace(Front().eval(), (initial[0], initial[1], initial[4]), check_trace=False)
        front.save(str(args.out/'front.pt'))
        front = torch.jit.load(str(args.out/'front.pt')).eval()
        x, l, target, memory, _, _ = front(initial[0], initial[1], initial[4])
        decoder = Decoder().eval()
        torch.onnx.export(decoder, (target, memory, initial[2]), str(args.out/'decoder.onnx'),
                          input_names=['target', 'memory', 'state'], output_names=['mask', 'out_state'],
                          opset_version=17, do_constant_folding=True)
        mask, _ = decoder(target, memory, initial[2])
        finish = torch.jit.trace(Finish().eval(), (x, l, mask, initial[3]), check_trace=False)
        finish.save(str(args.out/'finish.pt'))
        finish = torch.jit.load(str(args.out/'finish.pt')).eval()
    options = ort.SessionOptions()
    options.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
    options.enable_mem_pattern = False
    options.intra_op_num_threads = 1
    options.inter_op_num_threads = 1
    options.add_session_config_entry('session.intra_op.allow_spinning', '0')
    options.add_session_config_entry('session.inter_op.allow_spinning', '0')
    options.add_session_config_entry('session.disable_cpu_ep_fallback', '1')
    options.enable_profiling = True
    options.profile_file_prefix = str(args.out/'ort-profile')
    session = ort.InferenceSession(str(args.out/'decoder.onnx'), options,
                                  providers=[('DmlExecutionProvider', {'device_id': str(args.device_id)})])
    assert session.get_providers()[0] == 'DmlExecutionProvider'

    def step(packet, states):
        x, l, target, memory, enc, pre = front(packet, states[0], states[3])
        mask, dec = session.run(None, {'target': target.numpy(), 'memory': memory.numpy(), 'state': states[1].numpy()})
        audio, synth = finish(x, l, torch.from_numpy(mask), states[2])
        return audio, enc, torch.from_numpy(dec), synth, pre

    cases = []
    times = []
    with torch.inference_mode():
        warm = [s.clone() for s in initial[1:]]
        for _ in range(24):
            warm = list(step(initial[0], warm)[1:])
        for name in args.cases:
            packets = np.fromfile(args.fixture/(name+'.input.f32'), dtype='<f4').reshape(-1, *meta['shapes'][0])
            count = min(len(packets), int(args.seconds*16000/meta['hop']))
            reference = [s.clone() for s in initial[1:]]
            candidate = [s.clone() for s in initial[1:]]
            rows = []
            for index, packet in enumerate(packets[:count]):
                packet = torch.from_numpy(packet.copy())
                identical = [s.clone() for s in reference]
                expected = net(packet, *reference, pad=not net.lookahead)
                reference = list(expected[1:])
                started = time.perf_counter()
                free = step(packet, candidate)
                elapsed = (time.perf_counter()-started)*1000
                candidate = list(free[1:])
                if index >= 20:
                    times.append(elapsed)
                forced = step(packet, identical)
                row = {'chunk': index}
                for mode, values in [('identical_input', forced), ('free_running', free)]:
                    row[mode] = {n: error(a.numpy(), b.numpy()) for n, a, b in zip(meta['output_names'], values, expected)}
                    e = row[mode]['converted']
                    e['pass'] = e['max'] <= 1e-4 and e['rms'] <= 1e-5
                rows.append(row)
            passed = all(e['pass'] for r in rows for m in ['identical_input','free_running'] for e in r[m].values())
            cases.append({'name': name, 'pass': passed, 'chunks': rows})
            print(name, 'PASS' if passed else 'FAIL', flush=True)
    profile = session.end_profiling()
    events = json.loads(Path(profile).read_text())
    providers = sorted({e.get('args', {}).get('provider') for e in events if e.get('args', {}).get('provider')})
    assert 'DmlExecutionProvider' in providers, providers
    report = {'schema': 1, 'runtime': 'Rust-deployable LibTorch front/finish + ONNX Runtime DirectML decoder candidate',
              'source_revision':SOURCE_REV,'checkpoint_sha256':WEIGHTS_SHA,
              'ort_version': ort.__version__, 'device_id': args.device_id, 'executed_providers': providers,
              'cpu_threads': 2, 'hop_ms': 26, 'mean_ms': float(np.mean(times)), 'p99_ms': float(np.percentile(times, 99)),
              'parity_pass': all(c['pass'] for c in cases), 'cases': cases,
              'models': {n: hashlib.sha256((args.out/n).read_bytes()).hexdigest() for n in ['front.pt','decoder.onnx','finish.pt']}}
    report['timing_pass'] = report['mean_ms'] <= 13 and report['p99_ms'] < 26
    (args.out/'report.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
    print({k: v for k, v in report.items() if k != 'cases'}, flush=True)
    return 0 if report['parity_pass'] and report['timing_pass'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
