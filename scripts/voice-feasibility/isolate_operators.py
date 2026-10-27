"""Compare real failing-step operators from identical inputs and a float64 oracle."""
import argparse
import copy
import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys

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
    p.add_argument('--chunk', type=int, default=39)
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
    meta = json.loads((args.fixture / 'fixture.json').read_text(encoding='utf-8'))
    packets = np.fromfile(args.fixture / 'quiet_speech.input.f32', dtype='<f4').reshape(-1, *meta['shapes'][0])
    assert 0 <= args.chunk < len(packets)
    captured = []
    active = [False]
    handles = []
    for name, module in model.named_modules():
        if not isinstance(module, (torch.nn.Conv1d, torch.nn.LayerNorm)):
            continue
        def hook(mod, inputs, output, name=name):
            if active[0]:
                captured.append((name, copy.deepcopy(mod), inputs[0].clone(), output.clone()))
        handles.append(module.register_forward_hook(hook))
    states = [torch.zeros(s) for s in meta['shapes'][1:]]
    with torch.inference_mode():
        for index, packet in enumerate(packets[:args.chunk + 1]):
            active[0] = index == args.chunk
            values = model(torch.from_numpy(packet.copy()), *states, pad=not model.lookahead)
            states = values[1:]
    for handle in handles:
        handle.remove()
    options = ort.SessionOptions()
    options.intra_op_num_threads = 1
    options.inter_op_num_threads = 1
    options.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
    options.graph_optimization_level = ort.GraphOptimizationLevel.ORT_DISABLE_ALL
    options.add_session_config_entry('session.intra_op.allow_spinning', '0')
    results = []
    with torch.inference_mode():
        for name, module, x, expected in captured:
            # Captured module hooks were copied with deepcopy; remove them.
            module._forward_hooks.clear()
            stream = io.BytesIO()
            torch.onnx.export(module, x, stream, input_names=['x'], output_names=['y'], opset_version=17)
            session = ort.InferenceSession(stream.getvalue(), options, providers=['CPUExecutionProvider'])
            actual = session.run(None, {'x': x.numpy()})[0]
            oracle = copy.deepcopy(module).double()(x.double()).numpy()
            expected_array = expected.numpy()
            repeated = module(x).numpy()
            row = {
                'name': name, 'shape': list(x.shape),
                'repeat_bit_exact': bool(np.array_equal(repeated, expected_array)),
                'onnx_vs_torch': error(actual, expected_array),
                'torch_vs_float64': error(expected_array, oracle),
                'onnx_vs_float64': error(actual, oracle),
            }
            if isinstance(module, torch.nn.LayerNorm):
                z = x.transpose(1, 2) if module.__class__.__name__ == 'LayerNormPermuted' else x
                variants = {}
                for precision in ('float', 'double'):
                    variance, mean = torch.var_mean(z.to(getattr(torch, precision)), dim=-1, unbiased=False, keepdim=True)
                    rstd = torch.rsqrt(variance + module.eps).float()
                    mean = mean.float()
                    scale = rstd * module.weight
                    bias = module.bias - mean * scale
                    for expression, y in [('centered', (z-mean)*rstd*module.weight+module.bias),
                                          ('affine', z*scale+bias),
                                          ('fused_affine', torch.addcmul(bias,z,scale))]:
                        if module.__class__.__name__ == 'LayerNormPermuted':
                            y = y.transpose(1, 2)
                        variants[precision+'_'+expression] = error(y.numpy(),expected_array)
                row['normalization_variants'] = variants
            results.append(row)
            print(name, 'ORT/PT peak', row['onnx_vs_torch']['max'],
                  'PT/oracle peak', row['torch_vs_float64']['max'],
                  'ORT/oracle peak', row['onnx_vs_float64']['max'], flush=True)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps({'chunk': args.chunk, 'source_revision': SOURCE_REV,
        'checkpoint_sha256': WEIGHTS_SHA, 'operators': results}, indent=2), encoding='utf-8')


if __name__ == '__main__':
    main()
