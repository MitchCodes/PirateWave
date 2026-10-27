"""Contain fatal native training-runtime failures outside the desktop app."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

from probe import atomic_json


def run_worker(command, out, timeout):
    out.mkdir(parents=True, exist_ok=True)
    result = {"schema": 1, "public_training_enabled": False, "release_qualified": False}
    stdout, stderr = out / "worker.stdout.txt", out / "worker.stderr.txt"
    with stdout.open("wb") as output, stderr.open("wb") as errors:
        child = subprocess.Popen(command, stdout=output, stderr=errors,
                                 creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
        try:
            child.wait(timeout=timeout)
            result.update(exit_code=child.returncode, timed_out=False)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait()
            result.update(exit_code=child.returncode, timed_out=True)
    result.update(process_completed=result["exit_code"] == 0 and not result["timed_out"],
                  stderr_tail=stderr.read_bytes()[-8192:].decode(errors="replace"))
    atomic_json(out / "supervisor.json", result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=int, default=3600)
    parser.add_argument("--out", type=Path, required=True)
    args, worker_args = parser.parse_known_args()
    if worker_args[:1] == ["--"]:
        worker_args = worker_args[1:]
    result = run_worker([sys.executable, str(Path(__file__).with_name("probe.py")),
                         "--out", str(args.out), *worker_args], args.out, args.timeout)
    print(json.dumps(result), flush=True)
    return 0 if result["process_completed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
