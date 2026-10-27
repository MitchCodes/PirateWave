"""Application-managed local training worker; no system Python or pip required."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import time

from assets import sha256
from probe import atomic_json


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--job', type=Path, required=True)
    args = p.parse_args()
    job_file = args.job.resolve()
    job = json.loads(job_file.read_text(encoding='utf-8'))
    root = Path(sys.executable).parent
    folder = job_file.parent
    shared = Path(job['shared'])
    if job['schema'] != 1 or job['backend'] not in ('cpu','cuda'):
        raise ValueError('Unsupported training job')
    import torch
    if job['backend'] == 'cuda' and not torch.cuda.is_available():
        raise RuntimeError('CUDA is unavailable; no CPU fallback was performed')
    def state(stage, **extra):
        atomic_json(folder/'progress.json',dict(schema=1,stage=stage,**extra))
        print(json.dumps(dict(stage=stage,**extra)),flush=True)
    state('preparing')
    import numpy as np
    import soundfile as sf
    import torchaudio
    targets = dict(schema=1,target_train=[],target_validation=[],target_test=[])
    for recording in job['recordings']:
        if recording.get('excluded',False):
            continue
        if (folder/'pause.request').exists():
            state('paused')
            return 0
        source = folder/recording['path']
        if not source.resolve().is_relative_to(folder):
            raise ValueError('Unsafe recording path')
        samples, rate = sf.read(source,dtype='float32',always_2d=True)
        wave = torch.from_numpy(samples.mean(axis=1))
        if rate != 16000:
            wave = torchaudio.functional.resample(wave,rate,16000)
        role = recording['role']
        if role not in ('train','validation','test'):
            raise ValueError('Unknown recording role')
        for start in range(0,len(wave),128000):
            clip=wave[start:start+128000]
            if len(clip)<10240 or float(clip.square().mean())<1e-7:
                continue
            relative=f"prepared/{recording['id']}-{start}.wav"
            dest=folder/relative
            dest.parent.mkdir(exist_ok=True)
            sf.write(dest,clip.numpy(),16000,subtype='PCM_16')
            targets['target_'+role].append(dict(path=relative,sha256=sha256(dest)))
    if any(not targets[k] for k in ('target_train','target_validation','target_test')):
        raise ValueError('Include usable speech recordings in separate training, validation and test roles')
    atomic_json(folder/'targets.json',targets)
    command = [sys.executable,str(Path(__file__).with_name('probe.py')),
        '--rvc',str(root/'sources/rvc'),'--llvc',str(root/'sources/llvc'),
        '--assets',str(shared),'--initial-llvc',str(shared/'llvc.pth'),
        '--content-extractor',str(shared/'content-extractor.pt'),
        '--target-dataset',str(folder/'targets.json'),'--backend',job['backend'],
        '--out',str(folder),'--teacher-steps','250','--llvc-steps','1000',
        '--checkpoint-every','50','--clips','60','--source-clips','100','--clip-seconds','8']
    if (folder/'teacher.checkpoint.pt').exists() or (folder/'llvc.checkpoint.pt').exists():
        command.append('--resume')
    trained = subprocess.run(command)
    if (folder/'pause.request').exists():
        state('paused')
        return 0
    if trained.returncode:
        raise RuntimeError('Training failed. See report.json and worker diagnostics; previous checkpoints were preserved.')
    state('exporting')
    native = folder/'native'
    export = [sys.executable,str(Path(__file__).with_name('export_probe.py')),
        '--llvc',str(root/'sources/llvc'),'--checkpoint',str(folder/'llvc-generator.pt'),
        '--worker',job['native_worker'],'--speech',*[str(folder/f'review/{i}-original.wav') for i in (3,4,5)],
        '--seconds','30','--out',str(native)]
    if subprocess.run(export).returncode:
        raise RuntimeError('Native export or waveform/state validation failed')
    if (folder/'pause.request').exists():
        state('paused')
        return 0
    report = json.loads((native/'report.json').read_text())
    if not report['waveform_and_state_pass'] or not report['timing_pass']:
        raise RuntimeError('Native CPU validation/performance checks failed; voice was not activated')
    state('complete',model_sha256=sha256(native/'llvc-stream.pt'),
          note='Beta: short numerical and CPU timing checks passed. Long-run/hardware acceptance remains unmeasured.')
    return 0


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except Exception as error:
        print(json.dumps(dict(stage='failed',error=str(error))),flush=True)
        raise
