"""Summarize measured candidate evidence without enabling release gates."""
import json
from pathlib import Path
from assets import sha256


def main():
    root = Path(__file__).resolve().parents[2]
    evidence = root / "dist/voice-training"
    def read(relative):
        path = evidence / relative
        if not path.exists():
            return {"pending": True}
        result = json.loads(path.read_text(encoding="utf-8"))
        for key in ("rows", "traceback", "recordings", "recipe"):
            result.pop(key, None)
        for stage in ("teacher", "llvc", "warmup"):
            if stage in result:
                result[stage].pop("losses", None)
        if "details" in result:
            result["details"].pop("paths", None)
        return result
    matrix = evidence / "teacher-review/matrix.json"
    report = dict(schema=1, training_enabled=True, release_qualified=False,
                  scope="Local CPU/CUDA Voice Creation Beta; current experimental quality accepted for release by user. Full acceptance remains distinct from Beta availability.",
                  beta_workflows={backend:read(f"beta-validation/{backend}.json") for backend in ("cpu","cuda")},
                  teacher_comparisons=json.loads(matrix.read_text()) if matrix.exists() else {"pending": True},
                  llvc_comparisons=json.loads((evidence / "llvc-review/matrix.json").read_text())
                      if (evidence / "llvc-review/matrix.json").exists() else {"pending": True},
                  listening=json.loads((root / "docs/VOICE_TRAINING_LISTENING.json").read_text()),
                  pretrained_diagnostic=dict(comparison=read("pretrained-review/comparison.json"),
                      native=read("pretrained-review/native/report.json")),
                  adaptation_diagnostics=dict(alignment=read("adaptation-diagnostics/alignment.json"),
                      adversarial=read("adaptation-diagnostics/adversarial.json")),
                  warmup_experiment=dict(comparison=read("llvc-warmup/comparison.json"),
                      training=read("llvc-warmup/1462-warm500/report.json"),
                      native=read("llvc-warmup/1462-warm500/native/report.json")),
                  duration_experiment=dict(comparison=read("llvc-duration/comparison.json"),
                      training=read("llvc-duration/1462-1000/report.json"),
                      native=read("llvc-duration/1462-1000/native/report.json")),
                  context_experiment=dict(comparison=read("llvc-context/comparison.json"),
                      calibration=read("llvc-context/1462-long/calibration.json"),
                      training=read("llvc-context/1462-long/report.json"),
                      native=read("llvc-context/1462-long/native/report.json")),
                  llvc_candidates={speaker: {"training": read(f"llvc-review/{speaker}/report.json"),
                                             "native": read(f"llvc-review/{speaker}/native/report.json")}
                                   for speaker in ("1272", "1462", "1673")},
                  cpu_runtime=read("PirateWave-Training-CPU-candidate.json"),
                  cuda_runtime=read("PirateWave-Training-CUDA-candidate.json"),
                  cpu_isolation=read("embedded-cpu-isolation.json"),
                  cuda_isolation=read("embedded-cuda-isolation.json"),
                  cpu_training=read("embedded-cpu-smoke-fixed/report.json"),
                  cuda_training=read("embedded-cuda-smoke/report.json"),
                  cpu_export=read("embedded-cpu-export/report.json"),
                  cuda_export=read("embedded-cuda-export/report.json"),
                  unicode_path_native=read("Unicode path 試験/native-embedded/report.json"),
                  content_objective=json.loads((root / ".tools/voice-training/assets/content-extractor.json").read_text()),
                  scripts_sha256={p.name: sha256(p) for p in sorted(Path(__file__).parent.iterdir())
                                  if p.suffix in (".py", ".ps1", ".txt", ".in", ".json")},
                  remaining=["Improve adaptation quality and establish measured recording recommendations",
                             "Clean Windows qualification, complete dependency/license/build provenance review",
                             "Custom-model thirty-minute validation, integrated latency and gaming-impact acceptance",
                             "Independent custom-model DirectML qualification"])
    output = root / "docs/VOICE_TRAINING_PROGRESS.json"
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(str(output))


if __name__ == "__main__":
    main()
