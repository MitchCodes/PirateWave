"""Developer feasibility assets. Nothing is downloaded by the desktop app.

Downloads are opt-in, resumable, and verified before becoming usable. The
catalog is intentionally separate from any eventual public training catalog.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import tarfile
import urllib.request

RVC_REV = "81eed5e8f68b6bed1789f682fe78cdd324495afc"
LLVC_REV = "1627c5d358cf9bb2b92b0ccc513d8b36807c923d"
WEIGHTS_REV = "e6d0c1a17da07c33557852f9dfa2bd44cc75737d"
WEIGHTS = "https://huggingface.co/lj1995/VoiceConversionWebUI/resolve/" + WEIGHTS_REV + "/"
ASSETS = {
    "hubert_base_ls960.pt": ("https://huggingface.co/KoeAI/llvc_models/resolve/ebfe8c0fdeb974a7eeb463b3abbc8ce42a0e3851/models/embeddings/hubert_base_ls960.pt",
                             1136468879, "1703cf8d2cdc76f8c046f5f6a9bcd224e0e6caf4744cad1a1f4199c32cac8c8d"),
    "G32k.pth": (WEIGHTS + "pretrained/G32k.pth", 72653893,
                  "81817645cde7ed2e2d83f23ef883f33dda564924b497e84d792743912eca4c23"),
    "D32k.pth": (WEIGHTS + "pretrained/D32k.pth", 109978943,
                  "2ab20645829460fdad0d3c44254f1ab53c32cae50c22a66c926ae5aa30abda6f"),
    "hubert_base/pytorch_model.bin": (WEIGHTS + "hubert_base/pytorch_model.bin", 189206711,
                                     "cc8c20f4b90a520757260197a3ff2505705a7adbd20ad9eeaa4e1a9b38442ef5"),
}
PITCH_ASSETS = {
    "f0G32k.pth": (WEIGHTS + "pretrained/f0G32k.pth", 72795627,
                    "285f524bf48bb692c76ad7bd0bc654c12bd9e5edeb784dddf7f61a789a608574"),
    "f0D32k.pth": (WEIGHTS + "pretrained/f0D32k.pth", 109978943,
                    "294db3087236e2c75260d6179056791c9231245daf5d0485545d9e54c4057c77"),
}


def sha256(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def download(url, path, size, checksum):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists() and path.stat().st_size == size and sha256(path) == checksum:
        return
    part = path.with_suffix(path.suffix + ".part")
    offset = part.stat().st_size if part.exists() else 0
    if offset > size:
        raise ValueError("Partial download exceeds catalog size")
    reserve = max(2 * 1024**3, size // 10)
    if shutil.disk_usage(path.parent).free < size - offset + reserve:
        raise OSError("Insufficient space including training download headroom")
    if offset < size:
        req = urllib.request.Request(url, headers={"Range": f"bytes={offset}-"} if offset else {})
        with urllib.request.urlopen(req, timeout=120) as response:
            if offset and response.status == 206:
                if not response.headers.get("Content-Range", "").startswith(f"bytes {offset}-"):
                    raise ValueError("Unexpected resume range")
            else:
                offset = 0
            with part.open("ab" if offset else "wb") as f:
                for chunk in iter(lambda: response.read(1024 * 1024), b""):
                    f.write(chunk)
                    if f.tell() > size:
                        raise ValueError("Download exceeds catalog size")
                f.flush()
                os.fsync(f.fileno())
    if part.stat().st_size != size or sha256(part) != checksum:
        raise ValueError(f"Checksum/length mismatch: {path.name}")
    os.replace(part, path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--download", action="store_true")
    parser.add_argument("--corpus", action="store_true", help="Also download licensed LibriSpeech dev-clean")
    args = parser.parse_args()
    catalog = json.loads(Path(__file__).with_name("assets.lock.json").read_text())
    selected = dict(ASSETS)
    for row in catalog["small_assets"]:
        selected[row["path"]] = (row["url"], row["bytes"], row["sha256"])
    if args.corpus:
        row = catalog["corpus"]
        selected[row["path"]] = (row["url"], row["bytes"], row["sha256"])
    print(json.dumps({"download_bytes": sum(s[1] for s in selected.values()),
                      "assets": selected, "public_runtime_available": False}), flush=True)
    if not args.download:
        return
    for name, (url, size, checksum) in selected.items():
        print("Verifying/downloading " + name, flush=True)
        download(url, args.out / name, size, checksum)
    if args.corpus:
        with tarfile.open(args.out / catalog["corpus"]["path"], "r:gz") as archive:
            members = archive.getmembers()
            base = args.out.resolve()
            total = sum(m.size for m in members)
            if shutil.disk_usage(base).free < total + max(2 * 1024**3, total // 10):
                raise OSError("Insufficient corpus extraction space")
            for member in members:
                target = (base / member.name).resolve()
                if base not in target.parents or not (member.isfile() or member.isdir()):
                    raise ValueError("Unexpected corpus archive entry")
            archive.extractall(base, members=members)


if __name__ == "__main__":
    main()
