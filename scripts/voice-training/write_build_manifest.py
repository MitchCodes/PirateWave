"""Record release inputs/checksums without copying the private source checkout."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def digest(path):
    result = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda:stream.read(1024*1024), b''):
            result.update(block)
    return result.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--folder', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    inputs = [root/name for name in ('Cargo.toml','Cargo.lock','build.ps1','rust-toolchain.toml',
                                    'src/voice/manifest.json','src/training/catalog.json')]
    inputs += list((root/'src').rglob('*.rs'))
    inputs += list((root/'crates').rglob('*.rs'))
    manifest = dict(schema=1, version='0.2.0-beta.1', platform='windows-x64',
                    rustc=subprocess.check_output(['rustc','--version'], text=True).strip(),
                    cargo=subprocess.check_output(['cargo','--version'], text=True).strip(),
                    private_source_published=False,
                    training_sources_in_optional_tools=True,
                    source_inputs={p.relative_to(root).as_posix():digest(p) for p in sorted(set(inputs))},
                    native_worker_sha256=digest(root/'dist/PirateWave/PirateWaveVoiceWorker.exe'),
                    voice_manifest_sha256=digest(root/'src/voice/manifest.json'),
                    training_catalog_sha256=digest(root/'src/training/catalog.json'),
                    qualification='Experimental Beta; see VOICE_TRAINING_BETA_VALIDATION.json for measured workflows and outstanding acceptance.')
    (args.folder/'BUILD_MANIFEST.json').write_text(json.dumps(manifest, indent=2)+'\n', encoding='utf-8')
    files = sorted(args.folder.glob('*.part*'))
    files += [args.folder/name for name in ('PirateWave-LLVC-Voice-v1-beta1.zip',
        'PirateWave-v0.2.0-beta.1-windows-x64.zip','catalog.json','VOICE_TRAINING_NOTICES.txt',
        'VOICE_TRAINING.md','VOICE_TRAINING_BETA_VALIDATION.json','BUILD_MANIFEST.json',
        'validation-cpu.json','validation-cuda.json')]
    if any(not path.is_file() for path in files):
        raise ValueError('Complete release artifacts before writing checksums')
    (args.folder/'SHA256SUMS.txt').write_text(''.join(f'{digest(path)}  {path.name}\n' for path in files), encoding='ascii')
    print(f'Wrote build manifest and {len(files)} artifact checksums.')


if __name__ == '__main__':
    main()
