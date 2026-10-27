"""Build a relocatable candidate bundle; never labels a runtime qualified.

Input must be an official embedded interpreter with preinstalled wheel contents.
This build-machine tool uses git; the resulting worker does not require git/pip.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import zipfile
from assets import LLVC_REV, RVC_REV, sha256
from probe import verify_source


def vendor(source, destination, revision):
    verify_source(source, revision)
    files = subprocess.check_output(["git", "-C", str(source), "ls-files", "-z"]).decode().split("\0")
    hashes = {}
    for name in files:
        if not name:
            continue
        original = source / name
        if not original.is_file() or original.is_symlink():
            continue
        copied = destination / name
        copied.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(original, copied)
        hashes[name] = sha256(copied)
    (destination / "patch-source.json").write_text(json.dumps(dict(revision=revision, files=hashes), indent=2))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--runtime", type=Path, required=True)
    p.add_argument("--root", type=Path, default=Path.cwd())
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--backend", choices=["cpu", "cuda"], required=True)
    args = p.parse_args()
    root = args.runtime.resolve()
    if not (root / "python312._pth").is_file() or (root / "pyvenv.cfg").exists():
        raise ValueError("Expected an embedded Python 3.12 installation, not a venv")
    worker = root / "worker"
    worker.mkdir(exist_ok=True)
    for name in ("probe.py", "assets.py", "dataset.py", "supervise.py", "export_probe.py", "desktop_worker.py", "assets.lock.json"):
        shutil.copy2(Path(__file__).with_name(name), worker / name)
    vendor(args.root / ".tools/voice-training/RVC", root / "sources/rvc", RVC_REV)
    vendor(args.root / ".tools/voice-feasibility/LLVC", root / "sources/llvc", LLVC_REV)
    (root / "python312._pth").write_text("python312.zip\n.\nLib/site-packages\nworker\nimport site\n", encoding="ascii")
    distributions = json.loads(subprocess.check_output([str(root / "python.exe"), "-c",
        "import importlib.metadata as m,json;print(json.dumps([(d.metadata['Name'],d.version) for d in m.distributions()]))"], text=True))
    names = [name.lower().replace('_', '-') for name, _ in distributions]
    if len(set(names)) != len(names):
        raise ValueError("Duplicate package metadata; rebuild a consistent runtime before packaging")
    (root / "requirements-resolved.txt").write_text("\n".join(sorted(f"{n}=={v}" for n, v in distributions)) + "\n")
    files = {}
    for path in sorted(root.rglob("*")):
        if (path.is_file() and "__pycache__" not in path.parts and path.name != "runtime-manifest.json"
                and not path.relative_to(root).as_posix().startswith('Lib/site-packages/bin/')):
            files[path.relative_to(root).as_posix()] = dict(bytes=path.stat().st_size, sha256=sha256(path))
    report = dict(schema=1, backend=args.backend, python="3.12.10", qualified=False,
                  installed_bytes=sum(f["bytes"] for f in files.values()), files=files,
                  pending=["Clean Windows qualification", "Accepted recipe", "License inventory review", "Release signing"])
    (root / "runtime-manifest.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(args.out, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=1) as archive:
        for relative in [*files, "runtime-manifest.json"]:
            archive.write(root / relative, relative)
    catalog = dict(schema=1, backend=args.backend, qualified=False,
                   bytes=args.out.stat().st_size, sha256=sha256(args.out),
                   installed_bytes=report["installed_bytes"] + (root / "runtime-manifest.json").stat().st_size)
    args.out.with_suffix(".json").write_text(json.dumps(catalog, indent=2), encoding="utf-8")
    print(json.dumps(catalog), flush=True)


if __name__ == "__main__":
    main()
