"""Measure the actual Rust DirectML helper, including IPC and GPU transfers."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import numpy as np


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--pack', type=Path, required=True)
    p.add_argument('--fixture', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--chunks', type=int, default=1600)
    args = p.parse_args()
    if args.out.exists():
        p.error('Preserve existing benchmark reports')
    pack = args.pack.resolve()
    packets = np.fromfile(args.fixture/'long_mixed_speech.input.f32', dtype='<f4').reshape(-1,448)
    child = subprocess.Popen([str(pack/'PatchVoiceDmlWorker.exe'), str(pack/'llvc-stream.pt')],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=pack,
        env=dict(os.environ, KMP_BLOCKTIME='0', OMP_WAIT_POLICY='PASSIVE'), creationflags=0x08000000)
    times = []
    try:
        assert child.stdout.read(8) == b'PATCHVC1'
        for i in range(args.chunks+24):
            start = time.perf_counter()
            child.stdin.write(packets[i%len(packets)].tobytes())
            child.stdin.flush()
            data = child.stdout.read(416*4)
            elapsed = (time.perf_counter()-start)*1000
            assert len(data) == 416*4 and np.isfinite(np.frombuffer(data,dtype='<f4')).all()
            if i >= 24:
                times.append(elapsed)
        child.stdin.close()
        child.wait(timeout=10)
        assert child.returncode == 0
        adapter = child.stderr.read().decode('utf-8', errors='replace').strip()
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()
    report = {'schema':1,'backend':'DirectML transformer + CPU encoder/synthesis', 'adapter_log':adapter,
              'mean_ms':float(np.mean(times)), 'p99_ms':float(np.percentile(times,99)), 'max_ms':max(times),
              'chunks':len(times), 'hop_ms':26, 'includes_ipc_and_transfers':True,
              'worker_sha256':hashlib.sha256((pack/'PatchVoiceDmlWorker.exe').read_bytes()).hexdigest()}
    report['pass'] = report['mean_ms'] <= 13 and report['p99_ms'] < 26
    args.out.write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(report, flush=True)
    return 0 if report['pass'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
