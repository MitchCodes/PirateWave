"""Add a separately qualified hybrid DirectML runtime to the existing CPU pack.

Does not modify CPU models, runtime or qualification evidence. Without all GPU
reports this stages a disabled development candidate only.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import zipfile
from export_llvc import SOURCE_REV, WEIGHTS_SHA


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def validate_reports(worker, state, protocol, pipeline, cases, benchmark):
    for report in [state, protocol, pipeline, cases, benchmark]:
        require(report['pass'] and report['worker_sha256'] == worker, 'GPU evidence failed or uses another worker')
    require(state['audio_seconds'] >= 1800 and state['one_second_windows'] >= 1800, 'GPU continuous state test is incomplete')
    require(pipeline['duration_seconds'] >= 1800 and pipeline['underruns'] == 0 and 0 < pipeline['first_ready_ms'] <= 100,
            'GPU continuous audio pipeline has not passed')
    require(pipeline['backend'] == 'DirectMl', 'GPU pipeline report uses another backend')
    require(len(protocol['cases']) == 14 and len({r['name'] for r in protocol['cases']}) == 14
            and all(r['pass'] for r in protocol['cases']), 'GPU normal protocol cases are incomplete')
    require(len(cases['reports']) == 28 and len({(r['case'], r['single_step']) for r in cases['reports']}) == 28
            and all(r['pass'] and r['worker_sha256'] == worker for r in cases['reports']), 'GPU intermediate-state cases are incomplete')
    require(benchmark['mean_ms'] <= 13 and benchmark['p99_ms'] < 26 and benchmark['chunks'] >= 1600,
            'GPU performance requirement failed')


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--candidate', type=Path, required=True)
    p.add_argument('--qualify', action='store_true')
    p.add_argument('--crt-dir',type=Path,default=Path('C:/Program Files/Microsoft Visual Studio/18/Community/VC/Redist/MSVC/14.51.36231/x64/Microsoft.VC145.CRT'))
    args = p.parse_args()
    root = Path(__file__).resolve().parents[2]
    upstream = root/'.tools/voice-feasibility/LLVC'
    require(subprocess.check_output(['git','-C',str(upstream),'rev-parse','HEAD'],text=True).strip()==SOURCE_REV, 'Wrong LLVC source revision')
    require(not subprocess.check_output(['git','-C',str(upstream),'status','--porcelain','--untracked-files=no'],text=True).strip(), 'LLVC tracked sources have changed')
    require(digest(root/'.tools/voice-feasibility/llvc.pth')==WEIGHTS_SHA, 'Wrong LLVC checkpoint')
    stage = root/'dist/voice-pack/llvc-f8312-v1'
    wheel = root/'.tools/voice-feasibility/python/Lib/site-packages/onnxruntime'
    names = {'PatchVoiceDmlWorker.exe': root/'target/voice-torch-probe/release/piratewave-voice-dml-worker.exe',
             'front.pt': args.candidate/'front.pt', 'decoder.onnx': args.candidate/'decoder.onnx', 'finish.pt': args.candidate/'finish.pt',
             'onnxruntime.dll': wheel/'capi/onnxruntime.dll', 'DirectML.dll': wheel/'capi/DirectML.dll',
             'onnxruntime_providers_shared.dll': wheel/'capi/onnxruntime_providers_shared.dll',
             'ONNX_RUNTIME_LICENSE.txt': wheel/'LICENSE', 'ONNX_RUNTIME_NOTICES.txt': wheel/'ThirdPartyNotices.txt'}
    names['msvcp140_1.dll'] = args.crt_dir/'msvcp140_1.dll'
    # The probe loaded this same runtime from System32 before app-local staging.
    require(digest(names['msvcp140_1.dll']) == '206c931bf90fdad8816de3b5e2ef80b2bcaa9406c89ecc05fe6fddffe251e982', 'GPU C++ runtime differs from validation')
    report = json.loads((args.candidate/'report.json').read_text())
    require(report['parity_pass'] and report['timing_pass'], 'Hybrid export has not passed')
    for name, expected in report['models'].items():
        require(digest(names[name]) == expected, 'Hybrid model differs from validation: '+name)
    for name, path in names.items():
        shutil.copyfile(path, stage/name)
    notices = stage/'DIRECTML_WORKER_THIRD_PARTY_NOTICES.md'
    subprocess.run(['powershell','-NoProfile','-ExecutionPolicy','Bypass','-File',str(root/'scripts/third-party-notices.ps1'),
        '-CargoExe',str(Path.home()/'.cargo/bin/cargo.exe'),'-ManifestPath',str(root/'tools/voice-dml-worker/Cargo.toml'),
        '-OutputPath',str(notices)],check=True,cwd=root)
    q = {'schema': 1, 'available': False, 'backend': 'DirectML transformer + native LibTorch CPU encoder/synthesis',
         'source_revision':SOURCE_REV,'checkpoint_sha256':WEIGHTS_SHA,
         'adapter': 'NVIDIA GeForce RTX 4070 SUPER', 'driver_version': '32.0.16.1656',
         'candidate': report, 'worker_sha256': digest(stage/'PatchVoiceDmlWorker.exe'),
         'files': {name: digest(stage/name) for name in names},
         'manual_pending': ['Listening acceptance', 'Hardware latency', 'Game/Discord impact', 'Clean Windows']}
    q['reference_model_note'] = 'model_sha256 in waveform/state reports identifies the unchanged full reference model; inference uses the three hybrid model hashes in candidate.models.'
    q['live_run_conditions'] = 'Exactly one GPU conversion stream, with MMCSS render/supervisor registration, after all other GPU validators finished.'
    q['competing_stream_run'] = {'result':'Recovered with a second independent GPU converter running: maximum inference/IPC wait 43.6 ms, maximum render gap 3.8 ms. No passing stability claim.',
        'stdout_sha256':digest(root/'dist/voice-feasibility/diagnostics/directml-pipeline-competing-stream.stdout'),
        'stderr_sha256':digest(root/'dist/voice-feasibility/diagnostics/directml-pipeline-competing-stream.stderr')}
    q['unscheduled_run'] = {'result':'Recovered after 807 seconds before MMCSS registration; no passing stability claim.',
        'stdout_sha256':digest(root/'dist/voice-feasibility/diagnostics/directml-pipeline-unscheduled.stdout'),
        'stderr_sha256':digest(root/'dist/voice-feasibility/diagnostics/directml-pipeline-unscheduled.stderr')}
    q['contended_run'] = {'result':'Recovered after 215 seconds with concurrent independent validation sessions and model restarts; not a passing stability or gaming test.',
        'stdout_sha256':digest(root/'dist/voice-feasibility/diagnostics/directml-pipeline-contended.stdout'),
        'stderr_sha256':digest(root/'dist/voice-feasibility/diagnostics/directml-pipeline-contended.stderr')}
    if args.qualify:
        diagnostics = root/'dist/voice-feasibility/diagnostics'
        state = json.loads((diagnostics/'directml-worker-states-30m.json').read_text())
        protocol = json.loads((diagnostics/'directml-worker-protocol.json').read_text())
        pipeline = json.loads((diagnostics/'directml-pipeline-30m.json').read_text())
        cases = json.loads((diagnostics/'directml-worker-cases/summary.json').read_text())
        benchmark = json.loads((diagnostics/'directml-worker-benchmark.json').read_text())
        environment = json.loads((diagnostics/'directml-app-local-runtime.json').read_text())
        scheduling = json.loads((diagnostics/'directml-audio-scheduling.json').read_text(encoding='utf-8-sig'))
        require(sum(t['current_priority'] >= 23 for t in scheduling['threads']) >= 2, 'Audio scheduler registration was not observed')
        validate_reports(q['worker_sha256'], state, protocol, pipeline, cases, benchmark)
        require(environment['pass'] and environment['restricted_path'] and not environment['python_modules']
                and environment['worker_sha256'] == q['worker_sha256'], 'GPU app-local runtime has not passed')
        for name, sha in environment['app_local_runtime'].items():
            actual = next(p for p in stage.iterdir() if p.name.lower() == name.lower())
            require(digest(actual) == sha, 'GPU app-local runtime changed: '+name)
        q.update(available=True, worker_states=state, normal_protocol=protocol, integrated_pipeline=pipeline,
                 worker_cases=cases, native_benchmark=benchmark, app_local_runtime=environment, audio_scheduling=scheduling)
    (stage/'DIRECTML_QUALIFICATION.json').write_text(json.dumps(q, indent=2), encoding='utf-8')
    (stage/'DIRECTML_NOTICES.txt').write_text(
        'Hybrid DirectML voice runtime: GPU transformer decoder; CPU encoder and synthesis.\n'
        'Original LLVC weights and CPU math are unchanged. See MODEL_NOTICES.txt and LLVC_LICENSE.txt.\n'
        'ONNX Runtime DirectML 1.19.2: https://github.com/microsoft/onnxruntime/tree/v1.19.2\n'
        'DirectML is supplied with the matching ONNX Runtime distribution. See ONNX_RUNTIME_LICENSE.txt and ONNX_RUNTIME_NOTICES.txt.\n'
        'Rust dependencies: tch 0.18.1, ort/ort-sys 2.0.0-rc.8, windows 0.54.\n', encoding='utf-8')
    contract = {'schema':1,'rate':16000,'hop':416,'context':32,'dtype':'float32','batch':1,
        'front':{'file':'front.pt','device':'CPU','inputs':['audio','encoder','prenet'],
                 'outputs':['latent','label_latent','target','memory','out_encoder','out_prenet']},
        'decoder':{'file':'decoder.onnx','device':'DirectML','inputs':{'target':[1,256,26],'memory':[1,256,26],'state':[1,2,13,256]},
                   'outputs':{'mask':[1,256,26],'out_state':[1,2,13,256]}},
        'finish':{'file':'finish.pt','device':'CPU','inputs':['latent','label_latent','mask','synthesis'],
                  'outputs':['converted','out_synthesis']},
        'streaming_contract':'tensor-contract.json; same four persistent states, same PATCHVC1/PATCHVS1 IPC, reset all states after warm-up'}
    (stage/'DIRECTML_CONTRACT.json').write_text(json.dumps(contract,indent=2),encoding='utf-8')
    manifest = json.loads((stage/'manifest.json').read_text(encoding='utf-8'))
    require(manifest['automated_qualified'], 'The base CPU encoder/synthesis profile is not qualified')
    for name in [*names, 'DIRECTML_QUALIFICATION.json', 'DIRECTML_NOTICES.txt', 'DIRECTML_CONTRACT.json', 'DIRECTML_WORKER_THIRD_PARTY_NOTICES.md']:
        manifest['files'][name] = {'sha256': digest(stage/name), 'bytes': (stage/name).stat().st_size}
    manifest['directml_qualified'] = q['available']
    manifest['runtime'] = 'LibTorch 2.5.1 CPU / two threads; optional ONNX Runtime 1.19.2 DirectML transformer'
    text = json.dumps(manifest, indent=2, ensure_ascii=False)+'\n'
    (stage/'manifest.json').write_text(text, encoding='utf-8')
    (root/'src/voice/manifest.json').write_text(text, encoding='utf-8')
    if args.qualify:
        with zipfile.ZipFile(root/'dist/PirateWave-LLVC-Voice-v1.zip', 'w', zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
            for name in ['manifest.json', *sorted(manifest['files'])]:
                archive.write(stage/name, name)
    print('Staged DirectML pack; qualified:', q['available'], flush=True)


if __name__ == '__main__':
    main()
