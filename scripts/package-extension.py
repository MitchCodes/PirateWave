"""Build a deterministic .piratewaveext from a validated unpacked package.
No third-party Python modules. Native validation runs in PirateWave's child scanner.
"""
import argparse
import json
import pathlib
import subprocess
import tempfile
import zipfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("folder", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    parser.add_argument("--piratewave", dest="piratewave", type=pathlib.Path, required=True)
    args = parser.parse_args()
    folder = args.folder.resolve(strict=True)
    output = args.output.resolve()
    if output.is_relative_to(folder):
        parser.error("Write the package outside the input folder")
    manifest = json.loads((folder / "manifest.json").read_text(encoding="utf-8"))
    with tempfile.TemporaryDirectory(prefix="piratewave-extension-verify-") as temp:
        report = pathlib.Path(temp) / "manifest.json"
        subprocess.run([str(args.piratewave.resolve(strict=True)), "--scan-extension", str(folder), str(report)], check=True, timeout=15)
        scanned = json.loads(report.read_text(encoding="utf-8"))
        assert (scanned["id"], scanned["version"]) == (manifest["id"], manifest["version"])
    files = sorted(p for p in folder.rglob("*") if p.is_file())
    if len(files) > 128 or sum(p.stat().st_size for p in files) > 128 * 1024 * 1024:
        parser.error("Package exceeds file/size limits")
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_suffix(".piratewaveext.tmp")
    with zipfile.ZipFile(temporary, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in files:
            if path.is_symlink() or not path.resolve().is_relative_to(folder):
                parser.error("Package links are forbidden")
            info = zipfile.ZipInfo(path.relative_to(folder).as_posix(), date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            archive.writestr(info, path.read_bytes())
    if temporary.stat().st_size > 64 * 1024 * 1024:
        temporary.unlink()
        parser.error("Compressed package exceeds 64 MiB")
    temporary.replace(output)
    print(f"Verified extension package: {output}")


if __name__ == "__main__":
    main()
