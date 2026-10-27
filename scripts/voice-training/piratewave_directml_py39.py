"""Developer-only Python 3.9 compatibility fix for torch-directml 0.2.5.

Its class definition calls a staticmethod object, which becomes callable only
in Python 3.10. Invoke the underlying function explicitly instead. This does
not alter DirectML operators. Never apply this to PirateWave's live ORT runtime.
"""
import argparse
import importlib.util
from pathlib import Path

from assets import sha256

ORIGINAL = "56bf0c24d0f2988268a6435a3d367e0b8757e6e30f1d36007a56b99d943783c6"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    spec = importlib.util.find_spec("torch_directml")
    if spec is None:
        parser.error("Run using the isolated developer DirectML environment")
    path = Path(spec.origin)
    if sha256(path) != ORIGINAL:
        parser.error("Unexpected upstream module checksum; refusing to modify it")
    source = path.read_bytes()
    if source.count(b"device_id = default_device()") != 2:
        parser.error("Unexpected module layout")
    if args.apply:
        path.write_bytes(source.replace(b"device_id = default_device()", b"device_id = default_device.__func__()"))
    print({"path": str(path), "sha256": sha256(path), "applied": args.apply})


if __name__ == "__main__":
    main()
