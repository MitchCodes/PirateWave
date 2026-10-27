"""Compare actual Rust streaming output with independent SciPy + upstream inference.

Prefixing the upstream reference with 16 zero samples lets its offline shift
discard padding instead of microphone data. This tests the causal live alignment.
"""
import argparse
import ast
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time
import numpy as np
import scipy.signal
import torch
from export_llvc import SOURCE_REV,WEIGHTS_SHA


def main():
    p=argparse.ArgumentParser()
    p.add_argument('--trace',type=Path,required=True)
    p.add_argument('--upstream',type=Path,required=True)
    p.add_argument('--checkpoint',type=Path,required=True)
    p.add_argument('--out',type=Path,required=True)
    args=p.parse_args()
    if args.out.exists():p.error('Use a new report filename')
    if subprocess.check_output(['git','-C',str(args.upstream),'rev-parse','HEAD'],text=True).strip()!=SOURCE_REV:raise ValueError('Wrong source')
    if hashlib.sha256(args.checkpoint.read_bytes()).hexdigest()!=WEIGHTS_SHA:raise ValueError('Wrong checkpoint')
    sys.path.insert(0,str(args.upstream.resolve()))
    from model import Net
    torch.set_num_threads(1);torch.set_num_interop_threads(1)
    model=Net(**json.loads((args.upstream/'experiments/llvc/config.json').read_text(encoding='utf-8'))['model_params'])
    model.load_state_dict(torch.load(args.checkpoint,map_location='cpu',weights_only=True)['model']);model.eval()
    tree=ast.parse((args.upstream/'infer.py').read_text(encoding='utf-8'))
    fn=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='infer_stream')
    scope={'torch':torch,'np':np,'time':time}
    exec(compile(ast.Module(body=[fn],type_ignores=[]),'upstream/infer.py','exec'),scope)
    meta=json.loads(Path(str(args.trace)+'.json').read_text(encoding='utf-8'))
    source=np.fromfile(str(args.trace)+'.input.f32',dtype='<f4')
    actual=np.fromfile(str(args.trace)+'.output.f32',dtype='<f4')[meta['first_ready_sample']:]
    n=np.arange(255,dtype=np.float64)-127
    kernel=(2*6800/48000*np.sinc(2*6800/48000*n)*np.hanning(255)).astype(np.float32)
    # Reproduce coefficient quantization, but filter with an independent float64
    # implementation. Tolerances account for scalar float32 accumulation.
    total=np.float32(0)
    for value in kernel:total=np.float32(total+value)
    kernel=(kernel/total).astype(np.float64)
    mono=scipy.signal.lfilter(kernel,[1.0],source.astype(np.float64))[2::3].astype(np.float32)
    padded=torch.cat((torch.zeros(16),torch.from_numpy(mono)))
    with torch.inference_mode():converted,_,_=scope['infer_stream'](model,padded,2,16000)
    expected=scipy.signal.upfirdn(kernel*3,converted.flatten().numpy().astype(np.float64),up=3)[:len(actual)]
    fade=np.minimum(np.arange(1,len(expected)+1,dtype=np.float64)/240,1)
    expected*=fade
    if len(expected)!=len(actual) or not np.isfinite(actual).all():raise ValueError('Invalid rendered output')
    difference=actual-expected
    peak=float(np.abs(difference).max());rms=float(np.sqrt(np.mean(difference*difference)))
    report={'schema':1,'method':'Actual Rust graph-side stream versus independent SciPy filtering and original infer_stream with causal zero-prefix alignment',
            'samples':len(actual),'first_ready_sample':meta['first_ready_sample'],'max_error':peak,'rms_error':rms,'pass':peak<=1e-4 and rms<=1e-5}
    args.out.write_text(json.dumps(report,indent=2),encoding='utf-8');print(report)
    return 0 if report['pass'] else 1


if __name__=='__main__':raise SystemExit(main())
