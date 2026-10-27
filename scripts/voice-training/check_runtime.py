"""Verify a candidate runtime's integrity and isolation on this machine.

This is not a substitute for clean-Windows or training recipe qualification.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
from assets import sha256


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    root = args.runtime.resolve()
    manifest = json.loads((root / "runtime-manifest.json").read_text())
    for relative, spec in manifest["files"].items():
        path = (root / relative).resolve()
        if not path.is_relative_to(root) or path.stat().st_size != spec["bytes"] or sha256(path) != spec["sha256"]:
            raise ValueError("Runtime integrity check failed: " + relative)
    env = {key: value for key, value in os.environ.items()
           if key.upper() not in ("PATH", "PYTHONHOME", "PYTHONPATH", "VIRTUAL_ENV", "CONDA_PREFIX")}
    env["PATH"] = str(Path(os.environ.get("SystemRoot", "C:/Windows")) / "System32")
    command = """import json,sys,torch,torchaudio,librosa,transformers
from pathlib import Path
root=Path(sys.executable).resolve().parent
assert all(Path(p).resolve().is_relative_to(root) for p in sys.path)
assert sys.prefix == sys.base_prefix
print('PIRATEWAVE_RUNTIME_RESULT='+json.dumps(dict(python=sys.version,torch=torch.__version__,cuda=torch.cuda.is_available(),paths=sys.path)))
"""
    result = subprocess.run([str(root / "python.exe"), "-c", command], env=env, cwd=root,
                            capture_output=True, text=True, timeout=120)
    if result.returncode:
        raise RuntimeError(result.stderr[-4096:])
    line = next(line for line in result.stdout.splitlines() if line.startswith("PIRATEWAVE_RUNTIME_RESULT="))
    report = dict(schema=1, integrity_pass=True, isolated_imports_pass=True,
                  clean_windows_qualified=False, release_qualified=False,
                  runtime_manifest_sha256=sha256(root / "runtime-manifest.json"),
                  details=json.loads(line.split("=", 1)[1]))
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report), flush=True)


if __name__ == "__main__":
    main()
