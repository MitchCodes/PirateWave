"""Create a small, reviewable evidence index; retain full reports in dist."""
import argparse
import hashlib
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--reports", type=Path, required=True)
    parser.add_argument("--diagnostics", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = {
        "schema": 1,
        "release_qualified": False,
        "scope": "One pretrained voice. No additional-voice prerequisite.",
        "limits": {"state_normalized_rms": 1e-4, "state_normalized_max": 1e-3,
                   "audio_rms": 1e-5, "audio_max": 1e-4},
        "runs": [],
    }
    for root in (args.reports, args.diagnostics):
        for path in sorted(root.glob("*.json")):
            report = json.loads(path.read_text(encoding="utf-8-sig"))
            entry = {
                "file": str(path).replace("\\", "/"),
                "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            }
            if isinstance(report.get("chunks"), list):
                for key in ("backend", "optimization", "mkldnn", "model_sha256", "seconds",
                            "hop", "case", "pass", "reset_and_clone_pass", "state_shapes"):
                    if key in report:
                        entry[key] = report[key]
                entry["modes"] = {}
                for mode in ("identical_input", "free_running"):
                    entry["modes"][mode] = {}
                    for name in report["chunks"][0][mode]:
                        values = [r[mode][name] for r in report["chunks"]]
                        entry["modes"][mode][name] = {
                            "max_error": max(v["max"] for v in values),
                            "max_chunk_rms_error": max(v["rms"] for v in values),
                            "max_normalized_rms": max(v["normalized_rms"] for v in values),
                            "max_normalized_peak": max(v["normalized_max"] for v in values),
                            "legacy_element_scaled_max": max(v["element_scaled_max"] for v in values),
                            "first_failing_chunk": next((r["chunk"] for r in report["chunks"] if not r[mode][name]["pass"]), None),
                        }
                entry["operator_traces"] = [
                    {"chunk": r["chunk"], "operators": r["operator_trace"]}
                    for r in report["chunks"] if "operator_trace" in r
                ]
            elif "mean_ms" in report:
                entry.update({key: value for key, value in report.items() if key != "cases"})
                entry["all_final_tensor_states_pass"] = all(c["final_tensor_state_pass"] for c in report["cases"])
                entry["max_audio_error"] = max(c["max_audio_error"] for c in report["cases"])
                entry["max_audio_rms"] = max(c["rms_audio_error"] for c in report["cases"])
                entry["failing_final_states"] = [
                    {"case": c["name"], **s}
                    for c in report["cases"] for s in c["final_states"] if not s["tensor_pass"]
                ]
            elif path.name == "summary.json":
                entry["run_environment"] = report
            else:
                continue
            result["runs"].append(entry)
    args.out.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
