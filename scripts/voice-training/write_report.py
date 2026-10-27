"""Summarize measured prototype evidence without treating it as qualification."""
import argparse
import json
from pathlib import Path
import platform
import time

from assets import ASSETS, LLVC_REV, RVC_REV, WEIGHTS_REV, sha256


def read(path):
    return json.loads(path.read_text(encoding="utf-8"))


def size(path):
    return sum(p.stat().st_size for p in path.rglob("*") if p.is_file() and not p.is_symlink())


def training(path):
    report = read(path)
    result = {k: v for k, v in report.items() if k not in {"recordings", "recipe", "teacher", "llvc", "traceback"}}
    for stage in ("teacher", "llvc"):
        if stage in report:
            result[stage] = {k: v for k, v in report[stage].items() if k != "losses"}
    result["report"] = path.as_posix()
    result["report_sha256"] = sha256(path)
    result["listening_accepted"] = False
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path("."))
    args = parser.parse_args()
    root = args.root
    evidence = root / "dist/voice-training"
    export_path = evidence / "cuda-representative-export/report.json"
    native = read(export_path)
    rows = native.pop("rows")
    maxes = {}
    for mode in ("native", "single_step"):
        maxes[mode] = {}
        for name in ("audio", "encoder", "decoder", "synthesis", "prenet"):
            values = [row[mode][name] for row in rows]
            maxes[mode][name] = {field: max(v[field] for v in values) for field in ("max", "rms", "normalized_max", "normalized_rms")}
    native.update(report=export_path.as_posix(), report_sha256=sha256(export_path), worst_errors=maxes)
    scripts = root / "scripts/voice-training"
    catalog = read(scripts / "assets.lock.json")
    result = dict(schema=1, generated_utc=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                  status="feasibility_only", public_training_enabled=False, release_qualified=False,
                  development_python=platform.python_version(),
                  sources={"llvc": {"url": "https://github.com/KoeAI/LLVC", "revision": LLVC_REV, "license": "MIT"},
                           "rvc": {"url": "https://github.com/RVC-Project/Retrieval-based-Voice-Conversion-WebUI", "revision": RVC_REV, "license": "MIT"},
                           "fairseq": {"url": "https://github.com/One-sixth/fairseq", "revision": "44800430a728c2216fd1cf1e8daa672f50dfacba", "license": "MIT"},
                           "rvc_weights": {"url": "https://huggingface.co/lj1995/VoiceConversionWebUI", "revision": WEIGHTS_REV, "model_card_license": "MIT"}},
                  pinned_assets={name: {"url": url, "bytes": count, "sha256": sha} for name, (url, count, sha) in ASSETS.items()},
                  corpus=catalog["corpus"], small_assets=catalog["small_assets"],
                  scripts_sha256={p.name: sha256(p) for p in sorted(scripts.iterdir()) if p.suffix in {".py", ".txt", ".json"}},
                  cpu_execution=training(evidence / "cpu-probe/report.json"),
                  cuda_smoke_execution=training(evidence / "cuda-probe/report.json"),
                  cuda_larger_execution=training(evidence / "cuda-representative/report.json"),
                  native_export=native,
                  directml={"qualified": False, "stage": read(evidence / "directml-supervised/report.json")["stage"],
                            "supervisor": read(evidence / "directml-supervised/supervisor.json"),
                            "pytorch": "2.4.1+cpu", "torch_directml": "0.2.5.dev240914",
                            "module_original_sha256": "56bf0c24d0f2988268a6435a3d367e0b8757e6e30f1d36007a56b99d943783c6",
                            "module_patched_sha256": "6e46340bc3ec56344cc52400c34c13ffc32d0a184d68c42c3e411c9c2ceca1e3",
                            "patch": "Explicit developer-only Python 3.9 staticmethod compatibility fix; no native operator changes",
                            "failure": "Native abort: Invalid or unsupported data type ComplexFloat during spectrogram preparation"},
                  intel_xpu={"qualified": False, "reason": "No supported Intel GPU is available on this machine; CUDA wheel is not an XPU training runtime.",
                             "probe": training(evidence / "xpu-probe/report.json")},
                  amd_rocm={"qualified": False, "tested_on_hardware": False, "version_reviewed": "7.2.1",
                            "reason": "Native Windows documentation lists no ML training support; no AMD GPU was available.",
                            "source": "https://rocm.docs.amd.com/projects/radeon-ryzen/en/docs-7.2.1/docs/limitations/limitationsrad.html#windows"},
                  development_storage={"meaning": "End-of-experiment logical file sizes, NOT a published download estimate, peak measurement or minimum requirement.",
                      "cuda_environment_bytes": size(root / ".tools/voice-training/python"),
                      "directml_environment_bytes": size(root / ".tools/voice-training/directml"),
                      "assets_including_archive_and_extraction_bytes": size(root / ".tools/voice-training/assets"),
                      "experiment_artifacts_bytes": size(evidence),
                      "public_runtime_download_bytes": None, "public_runtime_installed_bytes": None, "peak_training_workspace_bytes": None},
                  implemented_in_application=["Gated voice-creation availability page", "Beginner recording/help guide and prompts",
                                              "Persistent workspace preference", "Background category/free-space measurement"],
                  remaining=["Accepted voice quality for multiple permissioned target speakers and held-out input speakers",
                             "Validated recording duration, convergence and reduced-dataset recipe",
                             "Supported embedded interpreter and fully prebuilt portable runtime bundles; current venvs require system Python",
                             "Measured download/installed/peak sizes and time/memory calibration",
                             "Rust job supervisor/IPC and complete resumable recording/import/review/training wizard",
                             "Verified runtime download/offline installation and safe checkpoint/cache cleanup",
                             "Live-audio training pause integration (currently no training is launched by PirateWave)",
                             "General voice registry and model-specific CPU/DirectML qualification",
                             "Thirty-minute state/stability, complete latency, listening and clean-Windows acceptance"],
                  guard="Existing curated voice/runtime/installer remain intact. No prototype model is installed or activated.")
    # Do not embed machine-specific workspace paths from a crash's stderr.
    tail = result["directml"]["supervisor"].pop("stderr_tail", "")
    result["directml"]["supervisor"]["complex_float_abort_recorded"] = "ComplexFloat" in tail
    out = root / "docs/VOICE_TRAINING_FEASIBILITY.json"
    out.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"report": str(out), "bytes": out.stat().st_size, "release_qualified": False}), flush=True)


if __name__ == "__main__":
    main()
