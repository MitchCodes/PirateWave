"""Bounded-memory continuous TorchScript/original-reference qualification.

Checks every state at every hop and waveform error in one-second windows.
It deliberately does not claim device latency, game impact or listening approval.
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
    p=argparse.ArgumentParser()
    p.add_argument('--upstream',type=Path,required=True)
    p.add_argument('--checkpoint',type=Path,required=True)
    p.add_argument('--fixture',type=Path,required=True)
    p.add_argument('--model',type=Path,required=True)
    p.add_argument('--out',type=Path,required=True)
    p.add_argument('--seconds',type=float,default=1800)
    p.add_argument('--threads',type=int,choices=[1,2],default=1)
    args=p.parse_args()
    if args.out.exists() or args.seconds<=0: p.error('Use a new report directory and positive duration')
    assert subprocess.check_output(['git','-C',str(args.upstream),'rev-parse','HEAD'],text=True).strip()==SOURCE_REV
    assert hashlib.sha256(args.checkpoint.read_bytes()).hexdigest()==WEIGHTS_SHA
    sys.path.insert(0,str(args.upstream.resolve()))
    from model import Net
    torch.set_num_threads(args.threads)
    torch.set_num_interop_threads(1)
    model=Net(**json.loads((args.upstream/'experiments/llvc/config.json').read_text(encoding='utf-8'))['model_params'])
    model.load_state_dict(torch.load(args.checkpoint,map_location='cpu',weights_only=True)['model'])
    model.eval()
    native=torch.jit.load(str(args.model)).eval()
    meta=json.loads((args.fixture/'fixture.json').read_text(encoding='utf-8'))
    packets=np.fromfile(args.fixture/'long_mixed_speech.input.f32',dtype='<f4').reshape(-1,*meta['shapes'][0])
    reference_states=[torch.zeros(s) for s in meta['shapes'][1:]]
    candidate_states=[s.clone() for s in reference_states]
    previous=np.zeros(meta['context'],dtype=np.float32)
    count=int(np.ceil(args.seconds*meta['sample_rate']/meta['hop']))
    zero={'max':0.0,'rms':0.0,'normalized_rms':0.0,'normalized_max':0.0,'element_scaled_max':0.0,'pass':True}
    maxima={name:dict(zero) for name in meta['output_names']}
    window=[]
    windows=0
    max_window_rms=0.0
    max_window_peak=0.0
    args.out.mkdir(parents=True)
    started=time.perf_counter()
    passed=True
    completed=0
    with (args.out/'windows.jsonl').open('w',encoding='utf-8') as journal, torch.inference_mode():
        for index in range(count):
            packet=packets[index%len(packets)].copy()
            packet.flatten()[:meta['context']]=previous
            previous=packet.flatten()[-meta['context']:].copy()
            x=torch.from_numpy(packet)
            expected=model(x,*reference_states,pad=not model.lookahead)
            reference_states=list(expected[1:])
            actual=native(x,*candidate_states)
            candidate_states=list(actual[1:])
            for name,a,b in zip(meta['output_names'],actual,expected):
                if not bool(torch.isfinite(a).all() and torch.isfinite(b).all()):
                    raise ValueError('Nonfinite '+name+' at chunk '+str(index))
                e=zero if torch.equal(a,b) else error(a.numpy(),b.numpy())
                if name=='converted':
                    e=dict(e,pass_=e['max']<=1e-4 and e['rms']<=1e-5)
                    e['pass']=e.pop('pass_')
                for key in zero:
                    if key!='pass': maxima[name][key]=max(maxima[name][key],e[key])
                maxima[name]['pass']=maxima[name]['pass'] and e['pass']
                passed=passed and e['pass']
            difference=(actual[0]-expected[0]).flatten().numpy()
            window.extend(difference.tolist())
            while len(window)>=meta['sample_rate']:
                one=np.asarray(window[:meta['sample_rate']],dtype=np.float64)
                del window[:meta['sample_rate']]
                rms=float(np.sqrt(np.mean(one*one)))
                peak=float(np.abs(one).max())
                max_window_rms=max(max_window_rms,rms)
                max_window_peak=max(max_window_peak,peak)
                journal.write(json.dumps({'second':windows,'rms':rms,'max':peak})+'\n')
                windows+=1
            completed=index+1
            if index%2308==0:
                print(f'Checked {completed*meta["hop"]/meta["sample_rate"]:.1f}s, pass={passed}',flush=True)
            if not passed: break
    summary={'schema':1,'runtime':'TorchScript '+torch.__version__,'source_revision':SOURCE_REV,
        'checkpoint_sha256':WEIGHTS_SHA,'model_sha256':hashlib.sha256(args.model.read_bytes()).hexdigest(),
        'threads':args.threads,'hop':meta['hop'],'sample_rate':meta['sample_rate'],
        'audio_seconds':completed*meta['hop']/meta['sample_rate'],'elapsed_seconds':time.perf_counter()-started,
        'continuous_state':True,'state_maxima':maxima,'one_second_windows':windows,
        'max_window_rms':max_window_rms,'max_window_error':max_window_peak,
        'pass':passed and completed==count and max_window_rms<=1e-5 and max_window_peak<=1e-4}
    (args.out/'summary.json').write_text(json.dumps(summary,indent=2),encoding='utf-8')
    print(summary,flush=True)
    return 0 if summary['pass'] else 1


if __name__=='__main__': raise SystemExit(main())
