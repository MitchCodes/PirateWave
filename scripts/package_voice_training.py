"""Package pinned CPU tools and shared assets for offline Voice Creation setup."""
import argparse
import hashlib
import json
from pathlib import Path
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent
CATALOG = ROOT / "src/training/catalog.json"
NAME = "PirateWave-Voice-Training-CPU.zip"


def digest_file(path):
    with path.open("rb") as source:
        return digest_stream(source)


def digest_stream(source):
    digest = hashlib.sha256()
    while chunk := source.read(1024 * 1024):
        digest.update(chunk)
    return digest.hexdigest()


def parts(catalog):
    bundles = json.loads(Path(catalog).read_text(encoding="utf-8"))["bundles"]
    selected = [b for b in bundles if b["backend"] in ("cpu", "shared")]
    if sorted(b["backend"] for b in selected) != ["cpu", "shared"]:
        raise ValueError("Expected exactly CPU and shared training bundles")
    result = [part for bundle in selected for part in bundle["parts"]]
    for part in result:
        if Path(part["name"]).name != part["name"] or "\\" in part["name"]:
            raise ValueError("Unsafe training part name")
        if not part["url"].startswith("https://"):
            raise ValueError("Training downloads require HTTPS")
    if len({p["name"] for p in result}) != len(result):
        raise ValueError("Duplicate training part")
    return result


def validate_training_archive(path, catalog=CATALOG):
    expected = parts(catalog)
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)) or set(names) != {p["name"] for p in expected} | {"README.txt"}:
            raise ValueError("Unexpected optional training archive contents")
        archive.read("README.txt")  # Check CRC for the instructions as well.
        for part in expected:
            if archive.getinfo(part["name"]).file_size != part["bytes"]:
                raise ValueError("Training part size mismatch")
            with archive.open(part["name"]) as source:
                if digest_stream(source) != part["sha256"]:
                    raise ValueError("Training part checksum mismatch")


def package(folder, catalog=CATALOG, cache=None):
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=True)
    cache = Path(cache) if cache else ROOT / ".tools/release-training"
    cache.mkdir(parents=True, exist_ok=True)
    expected = parts(catalog)
    if sum(p["bytes"] for p in expected) + 65536 >= 2 ** 31:
        raise ValueError("CPU/shared bundle exceeds GitHub's per-asset limit")
    for part in expected:
        destination = cache / part["name"]
        if destination.is_file() and destination.stat().st_size == part["bytes"] and digest_file(destination) == part["sha256"]:
            continue
        temporary = destination.with_suffix(destination.suffix + ".download")
        try:
            with urllib.request.urlopen(part["url"], timeout=120) as source, temporary.open("wb") as output:
                remaining = part["bytes"]
                while chunk := source.read(min(1024 * 1024, remaining + 1)):
                    remaining -= len(chunk)
                    if remaining < 0:
                        raise ValueError("Training download exceeds pinned size")
                    output.write(chunk)
            if temporary.stat().st_size != part["bytes"] or digest_file(temporary) != part["sha256"]:
                raise ValueError("Training download integrity failure")
            temporary.replace(destination)
        finally:
            temporary.unlink(missing_ok=True)
    destination = folder / NAME
    temporary = destination.with_suffix(".tmp")
    try:
        with zipfile.ZipFile(temporary, "w", compression=zipfile.ZIP_STORED) as archive:
            archive.writestr("README.txt", "PirateWave optional CPU Voice Creation tools and shared assets.\nExtract this ZIP into one folder; do not extract the .part files.\nIn Voices > Create voice > Tools/storage, select this folder for offline tools installation.\nChoose CPU. Both CPU tools and shared assets are included.\nInstall the matching voice pack through Voices first.\nCUDA tools remain a separate optional in-app download.\nPinned catalog part filenames are retained to match the executable's pinned catalog.\nLicenses and speech/model notices are contained in the verified bundles.\n")
            for part in expected:
                archive.write(cache / part["name"], part["name"])
        validate_training_archive(temporary, catalog)
        temporary.replace(destination)
    finally:
        temporary.unlink(missing_ok=True)
    return destination


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--folder", type=Path, default=Path("dist"))
    args = parser.parse_args()
    print(package(args.folder))
