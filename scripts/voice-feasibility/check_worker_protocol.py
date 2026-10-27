"""Check the actual optional helper's binary IPC against saved references."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import numpy as np


def main():
    p=argparse.ArgumentParser()
    p.add_argument('--pack',type=Path,required=True)
    p.add_argument('--worker',default='PatchVoiceWorker.exe',choices=['PatchVoiceWorker.exe','PatchVoiceDmlWorker.exe'])
    p.add_argument('--fixture',type=Path,required=True)
    p.add_argument('--out',type=Path,required=True)
    args=p.parse_args()
    if args.out.exists():p.error('Preserve evidence by using a new report path')
    pack=args.pack.resolve()
    meta=json.loads((args.fixture/'fixture.json').read_text(encoding='utf-8'))
    if meta['hop']!=416:raise ValueError('This worker requires the qualified 26ms export')
    env=dict(os.environ,KMP_BLOCKTIME='0',OMP_WAIT_POLICY='PASSIVE')
    cases=[]
    for case in meta['cases']:
        source=(args.fixture/(case['name']+'.input.f32')).read_bytes()
        result=subprocess.run([str(pack/args.worker),str(pack/'llvc-stream.pt')],input=source,
                              stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=env,cwd=pack,
                              creationflags=0x08000000 if os.name=='nt' else 0,timeout=300)
        if result.returncode or result.stdout[:8]!=b'PATCHVC1':raise ValueError('Worker failed: '+result.stderr.decode('utf-8',errors='replace')[:1000])
        actual=np.frombuffer(result.stdout[8:],dtype='<f4').astype(np.float64)
        reference=np.fromfile(args.fixture/(case['name']+'.reference.f32'),dtype='<f4').astype(np.float64)
        if actual.shape!=reference.shape or not np.isfinite(actual).all():raise ValueError('Invalid worker output')
        difference=actual-reference
        peak=float(np.abs(difference).max());rms=float(np.sqrt(np.mean(difference*difference)))
        cases.append({'name':case['name'],'max_error':peak,'rms_error':rms,'pass':peak<=1e-4 and rms<=1e-5})
        print(case['name'],peak,rms,flush=True)
    report={'schema':1,'model_sha256':hashlib.sha256((pack/'llvc-stream.pt').read_bytes()).hexdigest(),
            'worker_sha256':hashlib.sha256((pack/args.worker).read_bytes()).hexdigest(),'worker':args.worker,
            'cases':cases,'pass':all(c['pass'] for c in cases)}
    args.out.write_text(json.dumps(report,indent=2),encoding='utf-8')
    return 0 if report['pass'] else 1


if __name__=='__main__':raise SystemExit(main())
