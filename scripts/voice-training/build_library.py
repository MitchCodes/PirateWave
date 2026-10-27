"""Reproduce the optional LibriSpeech voice library; run with the pinned CUDA runtime.

Training and export are explicit. This script never uploads or installs voices.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import zipfile

VOICES = [
    ('iron-captain', 'Iron Captain', '1272'),
    ('sirens-call', "Siren’s Call", '84'),
    ('crows-nest', "Crow’s Nest", '174'),
    ('storm-corsair', 'Storm Corsair', '251'),
    ('ghost-of-the-galleon', 'Ghost of the Galleon', '1462'),
]

def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(1024*1024), b''): h.update(block)
    return h.hexdigest()

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--runtime',type=Path,required=True)
    p.add_argument('--shared',type=Path,required=True)
    p.add_argument('--worker',type=Path,required=True)
    p.add_argument('--worker-runtime',type=Path,required=True)
    p.add_argument('--out',type=Path,required=True)
    p.add_argument('--voice',choices=[v[0] for v in VOICES],required=True)
    p.add_argument('--skip-training',action='store_true')
    a=p.parse_args()
    runtime,shared,out=a.runtime.resolve(),a.shared.resolve(),a.out.resolve()
    if digest(a.worker) != '5e2d7e362963a9e518f8c8e4b12ded48eaf6c10274bcffa9f712362ff07c95f2': raise ValueError('Use the pinned native voice worker shipped with this build')
    slug,name,speaker=next(v for v in VOICES if v[0]==a.voice)
    folder=out/slug;folder.mkdir(parents=True,exist_ok=True)
    source=Path(__file__).resolve().parent
    if not a.skip_training:
        command=[sys.executable,str(source/'probe.py'),'--rvc',str(runtime/'sources/rvc'),'--llvc',str(runtime/'sources/llvc'),'--assets',str(shared),'--initial-llvc',str(shared/'llvc.pth'),'--content-extractor',str(shared/'content-extractor.pt'),'--backend','cuda','--out',str(folder),'--speaker',speaker,'--teacher-steps','250','--llvc-steps','1000','--checkpoint-every','50','--clips','60','--source-clips','100','--clip-seconds','8']
        if (folder/'llvc.checkpoint.pt').exists() or (folder/'teacher.checkpoint.pt').exists():command.append('--resume')
        subprocess.run(command,check=True)
    training=json.loads((folder/'report.json').read_text(encoding='utf-8'))
    if training.get('stage') != 'export_and_native_validation_pending' or training.get('error'):
        raise ValueError('Training did not complete; refusing to export a partial candidate')
    if training['teacher']['optimizer_steps'] != 250 or training['llvc']['optimizer_steps'] != 1000:
        raise ValueError('Training budgets differ from the library recipe')
    env=dict(os.environ,PATH=str(a.worker_runtime.resolve())+os.pathsep+os.environ['PATH'])
    subprocess.run([sys.executable,str(source/'export_probe.py'),'--llvc',str(runtime/'sources/llvc'),'--checkpoint',str(folder/'llvc-generator.pt'),'--worker',str(a.worker.resolve()),'--speech',*[str(folder/f'review/{i}-original.wav') for i in (3,4,5)],'--seconds','30','--out',str(folder/'native')],env=env,check=True)
    report=json.loads((folder/'native/report.json').read_text(encoding='utf-8'))
    if not (report['waveform_and_state_pass'] and report['timing_pass'] and report['normal_protocol_parity']['pass']):raise ValueError('Native voice checks failed; refusing to package')
    model=folder/'native/llvc-stream.pt'
    voice=dict(schema=1,id='local-library-'+slug,name=name,model_sha256=digest(model),cpu_identifier=os.environ.get('PROCESSOR_IDENTIFIER',''),logical_processors=os.environ.get('NUMBER_OF_PROCESSORS',''),worker_sha256=digest(a.worker),report_sha256=digest(folder/'native/report.json'),numerical_seconds=report['seconds'],beta=True)
    metadata=folder/'package';metadata.mkdir(exist_ok=True)
    (metadata/'voice.json').write_text(json.dumps(voice,indent=2),encoding='utf-8')
    provenance=dict(schema=1,training_execution=json.loads((folder/'report.json').read_text(encoding='utf-8'))['execution'],packager_sha256=digest(Path(__file__)),source='https://www.openslr.org/12',license='CC BY 4.0',speaker=speaker,dataset_sha256=digest(folder/'dataset.json'),recipe=json.loads((folder/'report.json').read_text(encoding='utf-8'))['recipe'],limitations='Experimental 250 teacher / 1000 LLVC updates. Native numerical/timing checks do not grade voice quality. Not a pirate performance or endorsement by the reader.')
    attempts=folder/'native/attempts.json'
    if attempts.exists(): provenance['prior_native_validation_attempts']=json.loads(attempts.read_text(encoding='utf-8'))
    (metadata/'provenance.json').write_text(json.dumps(provenance,indent=2),encoding='utf-8')
    (metadata/'NOTICES.txt').write_text(f'{name}: experimental LLVC adaptation of LibriSpeech reader {speaker}.\nLibriSpeech: Vassil Panayotov, Guoguo Chen, Daniel Povey and Sanjeev Khudanpur. https://www.openslr.org/12\nCC BY 4.0: https://creativecommons.org/licenses/by/4.0/ . Audio converted into a trained voice model; original reader does not endorse this product. Reader metadata and corpus license follow.\nLLVC: Koe AI, MIT. RVC: RVC contributors, MIT. No runtime executables included.\n\n'+(shared/'LibriSpeech/LICENSE.TXT').read_text(encoding='utf-8')+'\n'+next(line for line in (shared/'LibriSpeech/SPEAKERS.TXT').read_text(encoding='utf-8').splitlines() if line.split('|')[0].strip()==speaker)+'\n\n'+(shared/'LLVC_LICENSE.txt').read_text(encoding='utf-8')+'\n'+(shared/'RVC_LICENSE.txt').read_text(encoding='utf-8'),encoding='utf-8')
    package=out/f'PirateWave-Voice-{slug}-v1.zip'
    with zipfile.ZipFile(package,'w',zipfile.ZIP_DEFLATED) as z:
        for filename in ('voice.json','provenance.json','NOTICES.txt'):z.write(metadata/filename,filename)
        z.write(model,'llvc-stream.pt');z.write(folder/'native/report.json','validation.json');z.write(folder/'review/3-llvc-offline.wav','preview.wav')
    entry=dict(id=voice['id'],name=voice['name'],description=f'Experimental LibriSpeech voice (reader {speaker}). CPU playback; local compatibility check on first use.',url=f'https://github.com/MitchCodes/PirateWave/releases/download/voices-v1/{package.name}',bytes=package.stat().st_size,sha256=digest(package))
    (folder/'catalog-entry.json').write_text(json.dumps(entry,indent=2),encoding='utf-8')
    print('Verified package:',package,flush=True)

if __name__=='__main__':main()
