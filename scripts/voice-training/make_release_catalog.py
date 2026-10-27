"""Build the application's pinned catalog and <=1 GiB GitHub release parts."""
import argparse
import hashlib
import json
from pathlib import Path
import zipfile
from assets import sha256


def main():
    p=argparse.ArgumentParser()
    p.add_argument('--folder',type=Path,required=True)
    p.add_argument('--catalog',type=Path,required=True)
    p.add_argument('--tag',default='support-assets-v1')
    args=p.parse_args()
    bundles=[]
    for backend in ('cpu','cuda','shared'):
        archive=args.folder/f'PirateWave-Training-{backend.upper() if backend!="shared" else "Shared"}-beta1.zip'
        metadata=json.loads(archive.with_suffix('.json').read_text())
        if sha256(archive)!=metadata['sha256']:raise ValueError('Archive changed')
        with zipfile.ZipFile(archive) as z:manifest=z.read('runtime-manifest.json')
        parts=[]
        with archive.open('rb') as source:
            index=1
            while True:
                block=source.read(1024*1024)
                if not block:break
                part=args.folder/f'{archive.stem}.part{index:03d}'
                digest=hashlib.sha256();size=0
                with part.open('wb') as target:
                    while block:
                        target.write(block);digest.update(block);size+=len(block)
                        if size>=1024**3:break
                        block=source.read(min(1024*1024,1024**3-size))
                parts.append(dict(name=part.name,bytes=size,sha256=digest.hexdigest(),
                    url=f'https://github.com/MitchCodes/PirateWave/releases/download/{args.tag}/{part.name}'))
                index+=1
        bundles.append(dict(id=f'training-{backend}-{metadata["sha256"][:12]}',backend=backend,
            bytes=metadata['bytes'],installed_bytes=metadata['installed_bytes'],sha256=metadata['sha256'],
            manifest_sha256=hashlib.sha256(manifest).hexdigest(),parts=parts))
    catalog=dict(schema=1,bundles=bundles)
    args.catalog.write_text(json.dumps(catalog,indent=2)+'\n')
    (args.folder/'catalog.json').write_text(json.dumps(catalog,indent=2)+'\n')
    print(json.dumps({b['backend']:dict(download=b['bytes'],installed=b['installed_bytes'],parts=len(b['parts'])) for b in bundles}))


if __name__=='__main__':main()
