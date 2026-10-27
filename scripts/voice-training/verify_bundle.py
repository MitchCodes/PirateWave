"""Verify candidate ZIP integrity without extracting or executing its contents.

The caller must obtain the catalog from a trusted source; this is not a signature
verifier and does not grant runtime or recipe qualification.
"""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import zipfile
from assets import sha256


def verify(archive, catalog):
    if archive.stat().st_size != catalog["bytes"] or sha256(archive) != catalog["sha256"]:
        raise ValueError("Archive differs from catalog")
    with zipfile.ZipFile(archive) as bundle:
        names = bundle.namelist()
        if len(set(names)) != len(names):
            raise ValueError("Duplicate archive entries")
        for name in names:
            path = PurePosixPath(name)
            if (path.is_absolute() or ".." in path.parts or "\\" in name or ":" in name
                    or name != path.as_posix() or bundle.getinfo(name).is_dir()):
                raise ValueError("Invalid archive path")
            if (bundle.getinfo(name).external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError("Archive links are not allowed")
        manifest_info = bundle.getinfo("runtime-manifest.json")
        if manifest_info.file_size > 16 * 1024 * 1024:
            raise ValueError("Oversized runtime manifest")
        manifest = json.loads(bundle.read(manifest_info))
        if manifest["schema"] != 1 or manifest["backend"] != catalog["backend"]:
            raise ValueError("Runtime identity mismatch")
        if set(names) != set(manifest["files"]) | {"runtime-manifest.json"}:
            raise ValueError("Missing or unexpected archive files")
        total = manifest_info.file_size
        for name, expected in manifest["files"].items():
            info = bundle.getinfo(name)
            if info.file_size != expected["bytes"]:
                raise ValueError("File size mismatch: " + name)
            digest = hashlib.sha256()
            with bundle.open(name) as stream:
                for block in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(block)
            if digest.hexdigest() != expected["sha256"]:
                raise ValueError("File hash mismatch: " + name)
            total += info.file_size
        if total != catalog["installed_bytes"]:
            raise ValueError("Installed storage count mismatch")
    return {"integrity_pass": True, "bytes": catalog["bytes"], "installed_bytes": total,
            "release_qualified": False}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("archive", type=Path)
    args = p.parse_args()
    print(json.dumps(verify(args.archive, json.loads(args.archive.with_suffix(".json").read_text()))))


if __name__ == "__main__":
    main()
