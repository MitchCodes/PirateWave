"""Bounded-memory check of actual native worker states against original LLVC."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import numpy as np
import torch
from diagnose_states import error
from export_llvc import SOURCE_REV,WEIGHTS_SHA


def main():
    p=argparse.ArgumentParser()
    p.add_argument('--pack',type=Path,required=True)
    p.add_argument('--worker',default='PatchVoiceWorker.exe',choices=['PatchVoiceWorker.exe','PatchVoiceDmlWorker.exe'])
    p.add_argument('--fixture',type=Path,required=True)
    p.add_argument('--upstream',type=Path,required=True)
    p.add_argument('--checkpoint',type=Path,required=True)
    p.add_argument('--case',default='quiet_speech')
    p.add_argument('--seconds',type=float)
    p.add_argument('--out',type=Path,required=True)
    p.add_argument('--no-warm',action='store_true')
    p.add_argument('--reference-threads',type=int,choices=[1,2],default=1)
    p.add_argument('--single-step',action='store_true',help='Use identical worker states for the next reference step')
    args=p.parse_args()
    if args.no_warm and args.worker=='PatchVoiceDmlWorker.exe':
        p.error('The hybrid worker requires its fixed warm-up/reset sequence')
    if args.out.exists():p.error('Use a new report path')
    if hashlib.sha256(args.checkpoint.read_bytes()).hexdigest()!=WEIGHTS_SHA:raise ValueError('Wrong checkpoint')
    if subprocess.check_output(['git','-C',str(args.upstream),'rev-parse','HEAD'],text=True).strip()!=SOURCE_REV:raise ValueError('Wrong source')
    sys.path.insert(0,str(args.upstream.resolve()))
    from model import Net
    torch.set_num_threads(args.reference_threads);torch.set_num_interop_threads(1)
    model=Net(**json.loads((args.upstream/'experiments/llvc/config.json').read_text(encoding='utf-8'))['model_params'])
    model.load_state_dict(torch.load(args.checkpoint,map_location='cpu',weights_only=True)['model']);model.eval()
    meta=json.loads((args.fixture/'fixture.json').read_text(encoding='utf-8'))
    packets=np.fromfile(args.fixture/(args.case+'.input.f32'),dtype='<f4').reshape(-1,*meta['shapes'][0])
    count=len(packets) if args.seconds is None else int(np.ceil(args.seconds*meta['sample_rate']/meta['hop']))
    sizes=[int(np.prod(s)) for s in meta['shapes'][1:]]
    states=[torch.zeros(s) for s in meta['shapes'][1:]]
    pack=args.pack.resolve()
    command=[str(pack/args.worker),str(pack/'llvc-stream.pt'),'--diagnostic-states']
    if args.no_warm:command.append('--no-warm')
    child=subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,
        env=dict(os.environ,KMP_BLOCKTIME='0',OMP_WAIT_POLICY='PASSIVE'),cwd=pack,creationflags=0x08000000 if os.name=='nt' else 0)
    zero={'max':0.0,'rms':0.0,'normalized_rms':0.0,'normalized_max':0.0,'element_scaled_max':0.0,'pass':True}
    maxima={name:dict(zero) for name in meta['output_names']}
    previous=np.zeros(meta['context'],dtype=np.float32)
    window=[];window_count=0;max_window_rms=0.0;max_window_peak=0.0
    passed=True;completed=0;started=time.perf_counter();audio_hash=hashlib.sha256()
    try:
        if child.stdout.read(8)!=b'PATCHVS1':raise ValueError('Worker failed to start')
        with torch.inference_mode():
            for index in range(count):
                packet=packets[index%len(packets)].copy()
                packet.flatten()[:meta['context']]=previous
                previous=packet.flatten()[-meta['context']:].copy()
                expected=model(torch.from_numpy(packet.copy()),*states,pad=not model.lookahead)
                states=list(expected[1:])
                child.stdin.write(packet.tobytes());child.stdin.flush()
                data=child.stdout.read((meta['hop']+sum(sizes))*4)
                if len(data)!=(meta['hop']+sum(sizes))*4:raise ValueError('Truncated diagnostic stream')
                actual=np.frombuffer(data,dtype='<f4');at=0
                audio_hash.update(data[:meta['hop']*4])
                for name,size,reference in zip(meta['output_names'],[meta['hop'],*sizes],expected):
                    value=actual[at:at+size].reshape(reference.shape);at+=size
                    e=error(value,reference.numpy())
                    if name=='converted':e['pass']=e['max']<=1e-4 and e['rms']<=1e-5
                    for key in zero:
                        if key!='pass':maxima[name][key]=max(maxima[name][key],e[key])
                    maxima[name]['pass']=maxima[name]['pass'] and e['pass'];passed=passed and e['pass']
                window.extend((actual[:meta['hop']].astype(np.float64)-expected[0].flatten().numpy()).tolist())
                while len(window)>=meta['sample_rate']:
                    one=np.asarray(window[:meta['sample_rate']]);del window[:meta['sample_rate']]
                    max_window_rms=max(max_window_rms,float(np.sqrt(np.mean(one*one))))
                    max_window_peak=max(max_window_peak,float(np.abs(one).max()));window_count+=1
                if args.single_step:
                    at=meta['hop'];states=[]
                    for shape,size in zip(meta['shapes'][1:],sizes):
                        states.append(torch.from_numpy(actual[at:at+size].copy().reshape(shape)));at+=size
                completed=index+1
                if index%2308==0:print(f'Actual worker checked {completed*meta["hop"]/meta["sample_rate"]:.1f}s, pass={passed}',flush=True)
                if not passed:break
    finally:
        child.kill();child.wait()
    report={'schema':1,'model_sha256':hashlib.sha256((pack/'llvc-stream.pt').read_bytes()).hexdigest(),
        'worker_sha256':hashlib.sha256((pack/args.worker).read_bytes()).hexdigest(),'worker':args.worker,
        'case':args.case,'warmup':not args.no_warm,'reference_threads':args.reference_threads,'single_step':args.single_step,'audio_seconds':completed*meta['hop']/meta['sample_rate'],
        'elapsed_seconds':time.perf_counter()-started,'state_maxima':maxima,'audio_sha256':audio_hash.hexdigest(),
        'one_second_windows':window_count,'max_window_rms':max_window_rms,'max_window_error':max_window_peak,
        'pass':passed and completed==count and max_window_rms<=1e-5 and max_window_peak<=1e-4}
    args.out.write_text(json.dumps(report,indent=2),encoding='utf-8');print(report,flush=True)
    return 0 if report['pass'] else 1


if __name__=='__main__':raise SystemExit(main())
