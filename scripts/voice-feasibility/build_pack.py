"""Assemble the explicit-install pack; qualification requires recorded evidence.

Run after building piratewave-voice-worker. Native dependencies come from the pinned
development wheel; the resulting pack does not include or invoke Python.
"""
import hashlib
import argparse
import math
import json
import os
from pathlib import Path
import shutil
import subprocess
import wave
import zipfile
import numpy as np

MODEL_SHA='730b547dbafea5e295302cd7d9c5a4bae212251ea9515a07443499645774945c'

def require(condition,message):
    if not condition:raise ValueError(message)

def validate_automated(q):
    require(q['continuous']['pass'] and q['continuous']['audio_seconds']>=1800 and q['continuous']['model_sha256']==MODEL_SHA,'Continuous model qualification is missing')
    require(q['integrated_pipeline']['pass'] and q['integrated_pipeline']['duration_seconds']>=1800 and q['integrated_pipeline']['underruns']==0,'Integrated stability qualification is missing')
    require(q['native_profile']['parity_pass'] and q['native_profile']['timing_pass'] and q['native_profile']['model_sha256']==MODEL_SHA,'Native profile qualification is missing')
    require(q['runtime_libraries']==q['environment']['runtime_libraries'],'Native runtime libraries differ from qualification')
    require(q['native_profile']['cpu_identifier']==q['environment']['cpu_identifier'],'Native profile uses another CPU')
    worker=q['worker_sha256']
    require(q['worker_states']['pass'] and q['worker_states']['audio_seconds']>=1800 and q['worker_states']['model_sha256']==MODEL_SHA and q['worker_states']['worker_sha256']==worker,'Actual worker state qualification is missing or stale')
    require(q['integrated_pipeline']['model_sha256']==MODEL_SHA and q['integrated_pipeline']['worker_sha256']==worker,'Integrated qualification uses another worker')
    require(q['app_local_runtime']['pass'] and q['app_local_runtime']['app_local_crt'] and not q['app_local_runtime']['python_modules'] and q['app_local_runtime']['worker_sha256']==worker,'App-local runtime qualification is missing or stale')
    require(q['worker_cases']['pass'] and len(q['worker_cases']['reports'])==28 and all(r['pass'] and r['worker_sha256']==worker and r['model_sha256']==MODEL_SHA for r in q['worker_cases']['reports']),'Intermediate state case qualification is missing or stale')
    require(q['normal_protocol']['pass'] and q['normal_protocol']['worker_sha256']==worker,'Normal worker protocol has not passed')
    require(q['causal_alignment']['pass'],'Causal streaming alignment has not passed')
    return True

def validate_acceptance(q,a,gaming,cpu):
    require(a['schema']==1 and a['model_sha256']==MODEL_SHA,'Manual record uses another model')
    require(bool(cpu) and a['cpu_identifier']==cpu and q['environment']['cpu_identifier']==cpu,'Manual record uses another CPU')
    require(gaming['pass'] and not gaming['inconclusive'] and gaming['cpu_identifier']==cpu,'Game comparison has not passed')
    require(a['listening']['accepted'] and a['listening']['notes'].strip(),'Listening acceptance is missing')
    require(a['clean_windows']['accepted'] and a['clean_windows']['notes'].strip(),'Clean Windows acceptance is missing')
    latency=a['latency']
    require(latency['accepted'] and math.isfinite(latency['added_ms']) and math.isfinite(latency['microphone_to_output_ms']) and 0<latency['added_ms']<=100 and latency['microphone_to_output_ms']>=latency['added_ms'] and latency['method'].strip(),'Complete latency acceptance is missing or inconsistent')
    validate_automated(q)
    return True


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--crt-dir',type=Path,default=Path('C:/Program Files/Microsoft Visual Studio/18/Community/VC/Redist/MSVC/14.51.36231/x64/Microsoft.VC145.CRT'),help='Matching licensed MSVC redistributable CRT directory')
    parser.add_argument('--acceptance',type=Path,help='Completed local manual acceptance record; omitted means gated release candidate')
    args=parser.parse_args()
    root=Path(__file__).resolve().parents[2]
    stage=root/'dist/voice-pack/llvc-f8312-v1'
    stage.mkdir(parents=True,exist_ok=True)
    wheel=root/'.tools/voice-feasibility/python/Lib/site-packages'
    sources={'PatchVoiceWorker.exe':root/'target/voice-torch-probe/release/piratewave-voice-worker.exe',
             'llvc-stream.pt':root/'dist/voice-feasibility/torchscript-26-cpu1/llvc-stream.pt',
             'PATCH_LICENSE.txt':root/'LICENSE',
             'model-config.json':root/'.tools/voice-feasibility/LLVC/experiments/llvc/config.json',
             'LLVC_LICENSE.txt':root/'.tools/voice-feasibility/LLVC/LICENSE',
             'PYTORCH_LICENSE.txt':wheel/'torch-2.5.1+cpu.dist-info/LICENSE',
             'PYTORCH_NOTICE.txt':wheel/'torch-2.5.1+cpu.dist-info/NOTICE'}
    for path in (wheel/'torch/lib').glob('*.dll'):
        if path.name!='torch_python.dll': sources[path.name]=path
    for name in ['msvcp140.dll','vcruntime140.dll','vcruntime140_1.dll']: sources[name]=args.crt_dir/name
    for name,path in sources.items(): shutil.copyfile(path,stage/name)
    subprocess.run(['powershell','-NoProfile','-ExecutionPolicy','Bypass','-File',str(root/'scripts/third-party-notices.ps1'),
                    '-CargoExe',str(Path.home()/'.cargo/bin/cargo.exe'),'-ManifestPath',str(root/'tools/voice-worker/Cargo.toml'),
                    '-OutputPath',str(stage/'WORKER_THIRD_PARTY_NOTICES.md')],check=True,cwd=root)
    previews={'preview.wav':'174-50561-0000','preview-quiet.wav':'quiet_speech','preview-loud.wav':'loud_speech','preview-speaker.wav':'1919-142785-0000'}
    for filename,case in previews.items():
        samples=np.fromfile(root/f'dist/voice-feasibility/hop2/{case}.reference.f32',dtype='<f4')
        with wave.open(str(stage/filename),'wb') as audio:
            audio.setparams((1,2,16000,0,'NONE','not compressed'))
            audio.writeframes((np.clip(samples,-1,1)*32767).astype('<i2').tobytes())
    (stage/'MODEL_NOTICES.txt').write_text(
        'LLVC source: https://github.com/KoeAI/LLVC\n'
        'Source revision: 1627c5d358cf9bb2b92b0ccc513d8b36807c923d\n'
        'Weights: https://huggingface.co/KoeAI/llvc_models\n'
        'Weights revision: ebfe8c0fdeb974a7eeb463b3abbc8ce42a0e3851\n'
        'Checkpoint SHA256: cceb7ab9621f84d62d283725ae3281cacb04f762d6a088fe3e97c1a29d4b8c0e\n'
        'Model repository card labels the weights MIT. See LLVC_LICENSE.txt.\n'
        'Previews: converted upstream LLVC test_wavs/174-50561-0000.wav and 1919-142785-0000.wav, plus quiet/loud variants.\n'
        'Original dataset: LibriSpeech ASR corpus, prepared by Vassil Panayotov with Daniel Povey.\n'
        'Source: https://www.openslr.org/12 ; License: CC BY 4.0, https://creativecommons.org/licenses/by/4.0/\n'
        'Reference: Vassil Panayotov, Guoguo Chen, Daniel Povey and Sanjeev Khudanpur, ICASSP 2015.\n'
        'Preview modifications: neural voice conversion, quiet/loud input-gain variants, PCM encoding.\n'
        'Runtime: native LibTorch 2.5.1 CPU, Rust tch 0.18.1. See PYTORCH_LICENSE.txt and PYTORCH_NOTICE.txt.\n'
        'Microsoft C++ runtime 14.51.36247.0: app-local files from Visual Studio redistributable CRT.\n'
        'Microsoft terms/deployment: https://learn.microsoft.com/cpp/windows/redistributing-visual-cpp-files\n'
        'Copyright Microsoft Corporation. All rights reserved.\n'
        'No game assets or custom voice recordings are included.\n',encoding='utf-8')
    contract={'schema':1,'model_sample_rate':16000,'graph_sample_rate':48000,'hop_samples':416,'context_samples':32,
              'input':{'name':'audio','dtype':'float32','shape':[1,1,448]},
              'states':[{'name':name,'dtype':'float32','shape':shape,'initial':'zeros'} for name,shape in [('encoder',[1,512,510]),('decoder',[1,2,13,256]),('synthesis',[1,512,4]),('prenet',[1,1,24])]],
              'output':{'name':'converted','dtype':'float32','shape':[1,1,416]},
              'state_order':['encoder','decoder','synthesis','prenet'],'speaker':'checkpoint default fixed label',
              'alignment':'Causal context prefix; retain microphone samples. Offline reference shift: 16 samples.',
              'native_runtime':'LibTorch 2.5.1 CPU','adapter':'piratewave-voice-worker / PATCHVC1','threads':2,'inter_op_threads':1}
    (stage/'tensor-contract.json').write_text(json.dumps(contract,indent=2),encoding='utf-8')
    files=[*sources,*previews,'tensor-contract.json','MODEL_NOTICES.txt','QUALIFICATION.json','WORKER_THIRD_PARTY_NOTICES.md']
    continuous=root/'dist/voice-feasibility/torchscript-continuous-30m/summary.json'
    pipeline=root/'dist/voice-feasibility/diagnostics/native-pipeline-final-30m.json'
    worker_states=root/'dist/voice-feasibility/diagnostics/actual-worker-states-30m.json'
    qualification={'native_profile':json.loads((root/'dist/voice-feasibility/diagnostics/native-torch-26-cpu2-final.json').read_text(encoding='utf-8')),
                   'continuous':json.loads(continuous.read_text(encoding='utf-8')) if continuous.exists() else {'status':'pending'},
                   'integrated_pipeline':json.loads(pipeline.read_text(encoding='utf-8')) if pipeline.exists() else {'status':'pending'},
                   'worker_states':json.loads(worker_states.read_text(encoding='utf-8')) if worker_states.exists() else {'status':'pending'},
                   'worker_sha256':hashlib.sha256((stage/'PatchVoiceWorker.exe').read_bytes()).hexdigest(),
                   'causal_alignment':json.loads((root/'dist/voice-feasibility/diagnostics/causal-alignment.json').read_text(encoding='utf-8')),
                   'app_local_runtime':json.loads((root/'dist/voice-feasibility/diagnostics/app-local-runtime.json').read_text(encoding='utf-8')),
                   'worker_cases':json.loads((root/'dist/voice-feasibility/diagnostics/worker-cases.json').read_text(encoding='utf-8')),
                   'normal_protocol':json.loads((root/'dist/voice-feasibility/diagnostics/worker-protocol-final.json').read_text(encoding='utf-8')),
                   'environment':json.loads((root/'dist/voice-feasibility/diagnostics/runtime-environment.json').read_text(encoding='utf-8')),
                   'runtime_libraries':{name:hashlib.sha256(path.read_bytes()).hexdigest() for name,path in sources.items() if name.endswith('.dll')},
                   'release_qualified':False,
                   'pending':['Integrated complete latency acceptance','User-operated game/Discord comparison','Manual listening acceptance','Clean Windows verification']}
    try:
        automated_qualified=validate_automated(qualification)
    except (KeyError,ValueError):
        automated_qualified=False
    qualification['automated_qualified']=automated_qualified
    if args.acceptance:
        acceptance=json.loads(args.acceptance.read_text(encoding='utf-8'))
        gaming=json.loads(Path(acceptance['gaming']['report']).read_text(encoding='utf-8'))
        validate_acceptance(qualification,acceptance,gaming,os.environ.get('PROCESSOR_IDENTIFIER',''))
        qualification.update(release_qualified=True,pending=[],manual_acceptance=acceptance,game_comparison=gaming)
    (stage/'QUALIFICATION.json').write_text(json.dumps(qualification,indent=2),encoding='utf-8')
    model_sha=hashlib.sha256((stage/'llvc-stream.pt').read_bytes()).hexdigest()
    require(model_sha==MODEL_SHA,'Model checksum mismatch')
    manifest={'schema':1,'pack':'llvc-f8312-v1','name':'LLVC voice — Beta',
              'model_sha256':model_sha,'cpu_identifier':qualification['environment']['cpu_identifier'],
              'cpu_name':qualification['environment']['processor_name'],'logical_processors':qualification['environment']['logical_processors'],
              'runtime':'LibTorch 2.5.1 CPU / two threads','hop':416,'release_qualified':qualification['release_qualified'],'automated_qualified':automated_qualified,
              'files':{name:{'sha256':hashlib.sha256((stage/name).read_bytes()).hexdigest(),'bytes':(stage/name).stat().st_size} for name in sorted(files)}}
    text=json.dumps(manifest,indent=2,ensure_ascii=False)+'\n'
    (stage/'manifest.json').write_text(text,encoding='utf-8')
    (root/'src/voice/manifest.json').write_text(text,encoding='utf-8')
    archive=root/'dist/PirateWave-LLVC-Voice-v1.zip'
    with zipfile.ZipFile(archive,'w',zipfile.ZIP_DEFLATED,compresslevel=6) as out:
        for name in ['manifest.json',*sorted(files)]:out.write(stage/name,name)
    print('Pack:',archive,'bytes',archive.stat().st_size)
    print('Release qualified:',qualification['release_qualified'])


if __name__=='__main__': main()
