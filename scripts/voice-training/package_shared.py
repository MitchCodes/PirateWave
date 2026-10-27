"""Package pinned shared models and licensed source speech without downloading."""
import argparse
import json
from pathlib import Path
import zipfile
from assets import ASSETS,sha256


def main():
    p=argparse.ArgumentParser()
    p.add_argument('--root',type=Path,default=Path.cwd())
    p.add_argument('--out',type=Path,required=True)
    args=p.parse_args()
    assets=args.root/'.tools/voice-training/assets'
    paths={name:assets/name for name in ASSETS if name!='hubert_base_ls960.pt'}
    for name in paths:
        if sha256(paths[name])!=ASSETS[name][2]:raise ValueError('Pinned shared asset changed: '+name)
    for relative in ('hubert_base/config.json','hubert_base/preprocessor_config.json','content-extractor.pt','content-extractor.json'):
        paths[relative]=assets/relative
    for path in (assets/'LibriSpeech').rglob('*'):
        if path.is_file():paths[path.relative_to(assets).as_posix()]=path
    paths['llvc.pth']=args.root/'.tools/voice-feasibility/llvc.pth'
    if sha256(paths['llvc.pth'])!='cceb7ab9621f84d62d283725ae3281cacb04f762d6a088fe3e97c1a29d4b8c0e':raise ValueError('Initial voice changed')
    paths['LLVC_LICENSE.txt']=args.root/'.tools/voice-feasibility/LLVC/LICENSE'
    paths['RVC_LICENSE.txt']=args.root/'.tools/voice-training/RVC/LICENSE'
    files={name:dict(bytes=path.stat().st_size,sha256=sha256(path)) for name,path in sorted(paths.items())}
    manifest=json.dumps(dict(schema=1,backend='shared',files=files,installed_bytes=sum(s['bytes'] for s in files.values())),indent=2).encode()
    args.out.parent.mkdir(parents=True,exist_ok=True)
    with zipfile.ZipFile(args.out,'w',zipfile.ZIP_DEFLATED,compresslevel=1) as archive:
        for name,path in sorted(paths.items()):archive.write(path,name)
        archive.writestr('runtime-manifest.json',manifest)
    args.out.with_suffix('.json').write_text(json.dumps(dict(schema=1,backend='shared',bytes=args.out.stat().st_size,
        sha256=sha256(args.out),installed_bytes=sum(s['bytes'] for s in files.values())+len(manifest)),indent=2))


if __name__=='__main__':main()
