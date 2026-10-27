"""Verify the native GPU worker with a restricted PATH and app-local DLLs."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--pack', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    args = p.parse_args()
    if args.out.exists():
        p.error('Preserve existing runtime evidence')
    pack = args.pack.resolve()
    env = {k:v for k,v in os.environ.items() if k.upper() not in ['PATH','PYTHONPATH','LIBTORCH','VIRTUAL_ENV']}
    env.update(PATH=str(Path(os.environ['SystemRoot'])/'System32'),KMP_BLOCKTIME='0',OMP_WAIT_POLICY='PASSIVE')
    child = subprocess.Popen([str(pack/'PatchVoiceDmlWorker.exe'), str(pack/'llvc-stream.pt')],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=pack, env=env, creationflags=0x08000000)
    try:
        assert child.stdout.read(8) == b'PATCHVC1', 'Native GPU worker failed to initialize'
        command = f"(Get-Process -Id {child.pid}).Modules | ForEach-Object {{ [PSCustomObject]@{{name=$_.ModuleName;path=$_.FileName}} }} | ConvertTo-Json -Compress"
        modules = json.loads(subprocess.check_output(['powershell','-NoProfile','-Command',command],text=True))
        by_name = {m['name'].lower():m for m in modules}
        expected = ['torch_cpu.dll','c10.dll','onnxruntime.dll','directml.dll','msvcp140.dll','msvcp140_1.dll','vcruntime140.dll','vcruntime140_1.dll']
        for name in expected:
            assert Path(by_name[name]['path']).resolve().parent == pack, name
        assert not any('python' in name for name in by_name)
        assert 'd3d12.dll' in by_name
        report = {'schema':1,'pass':True,'restricted_path':True,'python_modules':[],
            'app_local_runtime':{name:hashlib.sha256(Path(by_name[name]['path']).read_bytes()).hexdigest() for name in expected},
            'worker_sha256':hashlib.sha256((pack/'PatchVoiceDmlWorker.exe').read_bytes()).hexdigest(),
            'modules':modules}
    finally:
        child.kill()
        child.wait()
    args.out.write_text(json.dumps(report,indent=2),encoding='utf-8')
    print('DirectML app-local native runtime: PASS',flush=True)


if __name__ == '__main__':
    main()
