"""Repackage only the existing qualified files matching PirateWave's pinned manifest."""
import argparse
import json
from pathlib import Path
import zipfile
from assets import sha256

p=argparse.ArgumentParser()
p.add_argument('--out',type=Path,required=True)
args=p.parse_args()
root=Path(__file__).resolve().parents[2]
manifest_path=root/'src/voice/manifest.json'
manifest=json.loads(manifest_path.read_text())
source=root/'dist/voice-pack/llvc-f8312-v1'
for name,spec in manifest['files'].items():
    path=source/name
    if path.stat().st_size!=spec['bytes'] or sha256(path)!=spec['sha256']:
        raise ValueError('Qualified curated file changed: '+name)
args.out.parent.mkdir(parents=True,exist_ok=True)
with zipfile.ZipFile(args.out,'w',zipfile.ZIP_DEFLATED,compresslevel=1) as archive:
    archive.write(manifest_path,'manifest.json')
    for name in manifest['files']:archive.write(source/name,name)
print(str(args.out))
