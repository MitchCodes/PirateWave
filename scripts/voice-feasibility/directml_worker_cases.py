"""Actual hybrid worker: each fixture, identical-state and recurrent checks."""
import argparse
import json
from pathlib import Path
import subprocess
import sys


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--pack', type=Path, required=True)
    p.add_argument('--fixture', type=Path, required=True)
    p.add_argument('--upstream', type=Path, required=True)
    p.add_argument('--checkpoint', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    args = p.parse_args()
    if args.out.exists():
        p.error('Preserve existing reports')
    args.out.mkdir(parents=True)
    meta = json.loads((args.fixture/'fixture.json').read_text())
    reports = []
    for case in meta['cases']:
        for mode in ['recurrent', 'single-step']:
            path = args.out/(case['name']+'-'+mode+'.json')
            command = [sys.executable, str(Path(__file__).with_name('check_worker_states.py')),
                '--pack',str(args.pack),'--fixture',str(args.fixture),'--upstream',str(args.upstream),
                '--checkpoint',str(args.checkpoint),'--worker','PatchVoiceDmlWorker.exe',
                '--case',case['name'],'--out',str(path)]
            if mode == 'single-step':
                command.append('--single-step')
            with path.with_suffix('.log').open('w',encoding='utf-8') as log:
                result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
            assert result.returncode == 0, path
            report = json.loads(path.read_text())
            assert report['pass']
            reports.append(report)
            print(case['name'], mode, 'PASS', flush=True)
    summary = {'schema':1,'pass':len(reports)==28 and all(r['pass'] for r in reports),
               'reports':reports,'worker_sha256':reports[0]['worker_sha256']}
    (args.out/'summary.json').write_text(json.dumps(summary,indent=2),encoding='utf-8')


if __name__ == '__main__':
    main()
