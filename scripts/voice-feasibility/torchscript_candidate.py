"""Test whether native PyTorch export preserves the original streaming math.

Development probe only. A passing case is not integrated release approval.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time

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
    p.add_argument('--threads', type=int, choices=[1,2], default=1)
    p.add_argument('--case', default='quiet_speech')
    args = p.parse_args()
    if args.out.exists():
        p.error('Use a new output directory')
    assert subprocess.check_output(['git','-C',str(args.upstream),'rev-parse','HEAD'],text=True).strip()==SOURCE_REV
    assert hashlib.sha256(args.checkpoint.read_bytes()).hexdigest()==WEIGHTS_SHA
    sys.path.insert(0,str(args.upstream.resolve()))
    from model import Net
    torch.set_num_threads(args.threads)
    torch.set_num_interop_threads(1)
    model=Net(**json.loads((args.upstream/'experiments/llvc/config.json').read_text(encoding='utf-8'))['model_params'])
    model.load_state_dict(torch.load(args.checkpoint,map_location='cpu',weights_only=True)['model'])
    model.eval()
    meta=json.loads((args.fixture/'fixture.json').read_text(encoding='utf-8'))

    class Streaming(torch.nn.Module):
        def __init__(self):
            super().__init__()
            self.net=model

        def forward(self,a,e,d,s,p):
            return self.net(a,e.clone(),d.clone(),s.clone(),p.clone(),pad=not self.net.lookahead)

    initial=[torch.zeros(s) for s in meta['shapes']]
    args.out.mkdir(parents=True)
    with torch.inference_mode():
        traced=torch.jit.trace(Streaming().eval(),tuple(initial),check_trace=False)
        # Save/reload: test what the native runtime will actually receive.
        traced.save(str(args.out/'llvc-stream.pt'))
        traced=torch.jit.load(str(args.out/'llvc-stream.pt')).eval()
        reference_states=[s.clone() for s in initial[1:]]
        native_states=[s.clone() for s in initial[1:]]
        packets=np.fromfile(args.fixture/(args.case+'.input.f32'),dtype='<f4').reshape(-1,*meta['shapes'][0])
        rows=[]
        times=[]
        for index,packet in enumerate(packets):
            x=torch.from_numpy(packet.copy())
            identical=[s.clone() for s in reference_states]
            expected=model(x,*reference_states,pad=not model.lookahead)
            reference_states=list(expected[1:])
            started=time.perf_counter()
            free=traced(x,*native_states)
            native_states=list(free[1:])
            if index>=20:
                times.append((time.perf_counter()-started)*1000)
            forced=traced(x,*identical)
            row={'chunk':index}
            for mode,values in [('identical_input',forced),('free_running',free)]:
                row[mode]={name:error(a.numpy(),b.numpy()) for name,a,b in zip(meta['output_names'],values,expected)}
                e=row[mode]['converted']
                e['pass']=e['max']<=1e-4 and e['rms']<=1e-5
            rows.append(row)
    hop_ms=meta['hop']/16
    report={'schema':1,'runtime':'TorchScript '+torch.__version__,'threads':args.threads,'case':args.case,
            'hop_ms':hop_ms,'model_sha256':hashlib.sha256((args.out/'llvc-stream.pt').read_bytes()).hexdigest(),
            'mean_ms':float(np.mean(times)),'p99_ms':float(np.percentile(times,99)),
            'parity_pass':all(e['pass'] for r in rows for m in ['identical_input','free_running'] for e in r[m].values()),
            'chunks':rows}
    report['timing_pass']=report['mean_ms']<=hop_ms/2 and report['p99_ms']<hop_ms
    (args.out/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
    print({k:v for k,v in report.items() if k!='chunks'},flush=True)
    return 0 if report['parity_pass'] and report['timing_pass'] else 1


if __name__=='__main__':
    raise SystemExit(main())
