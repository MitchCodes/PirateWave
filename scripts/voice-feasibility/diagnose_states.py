"""Compare every streaming state, using identical-input and recurrent probes.

This is development-only qualification. It does not enable a PirateWave backend.
Existing fixture files and reports are never overwritten.
"""
import argparse
import hashlib
import json
import pathlib
import sys
import time

import numpy as np
import onnxruntime as ort
import torch


def error(actual, reference):
    a, b = actual.astype(np.float64), reference.astype(np.float64)
    if a.shape != b.shape or not a.size or not np.isfinite(a).all() or not np.isfinite(b).all():
        raise ValueError("Empty, mismatched, or nonfinite tensors")
    d = np.abs(a - b)
    rms = float(np.sqrt(np.mean(d * d)))
    peak = float(d.max())
    normalized_rms = rms / max(1.0, float(np.sqrt(np.mean(b * b))))
    normalized_peak = peak / max(1.0, float(np.abs(b).max()))
    return {
        "rms": rms, "max": peak,
        "normalized_rms": normalized_rms, "normalized_max": normalized_peak,
        "element_scaled_max": float((d / (1e-4 + 1e-4 * np.abs(b))).max()),
        "pass": normalized_rms <= 1e-4 and normalized_peak <= 1e-3,
    }


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--upstream", type=pathlib.Path, required=True)
    p.add_argument("--checkpoint", type=pathlib.Path, required=True)
    p.add_argument("--fixture", type=pathlib.Path, required=True)
    p.add_argument("--out", type=pathlib.Path, required=True)
    p.add_argument("--backend", choices=["cpu", "directml"], default="cpu")
    p.add_argument("--device-id", type=int, default=0, help="DXGI adapter index for DirectML")
    p.add_argument("--optimization", choices=["off", "basic", "all"], default="all")
    p.add_argument("--mkldnn", choices=["on", "off"], default="on")
    p.add_argument("--seconds", type=float, default=12)
    p.add_argument("--case", default="quiet_speech")
    p.add_argument("--trace-chunk", type=int)
    p.add_argument("--native-repro", type=pathlib.Path,
                   help="Save the first failing identical-state step for the Rust runner")
    args = p.parse_args()
    if args.seconds <= 0:
        p.error("--seconds must be positive")
    sys.path.insert(0, str(args.upstream.resolve()))
    from model import Net
    from export_llvc import SOURCE_REV, WEIGHTS_SHA
    import subprocess
    assert subprocess.check_output(["git", "-C", str(args.upstream), "rev-parse", "HEAD"], text=True).strip() == SOURCE_REV
    assert not subprocess.check_output(["git", "-C", str(args.upstream), "status", "--porcelain", "--untracked-files=no"], text=True).strip()
    assert hashlib.sha256(args.checkpoint.read_bytes()).hexdigest() == WEIGHTS_SHA
    meta = json.loads((args.fixture / "fixture.json").read_text(encoding='utf-8'))
    model_path = args.fixture / "llvc-stream.onnx"
    assert hashlib.sha256(model_path.read_bytes()).hexdigest() == meta["onnx_sha256"]
    torch.set_num_threads(1)
    torch.set_num_interop_threads(1)
    torch.backends.mkldnn.enabled = args.mkldnn == "on"
    model = Net(**json.loads((args.upstream / "experiments/llvc/config.json").read_text(encoding='utf-8'))["model_params"])
    model.load_state_dict(torch.load(args.checkpoint, map_location="cpu", weights_only=True)["model"])
    model.eval()
    trace_names = []
    trace_reference = {}
    trace_handles = []
    trace_enabled = [False]
    model_bytes = None
    if args.trace_chunk is not None:
        import onnx
        graph = onnx.load(str(model_path))
        # Match export paths to upstream module names explicitly, rather than
        # relying on node order. Encoder LayerNormPermuted returns B,C,T.
        modules = dict(model.named_modules())
        for node in graph.graph.node:
            if node.op_type not in ("Conv", "LayerNormalization", "Gemm"):
                continue
            path = node.name.removeprefix("/net/").rsplit("/", 1)[0]
            parts = path.split("/")
            module_name = ".".join(parts)
            for parent in ("in_conv", "label_embedding", "proj_e2d_e", "proj_e2d_l", "proj_d2e", "layers"):
                module_name = module_name.replace(parent + "." + parent + ".", parent + ".")
            module_name = module_name.replace("encoder.dcc_", "encoder.dcc_layers.dcc_")
            module = modules.get(module_name)
            if module is None or isinstance(module, torch.nn.MultiheadAttention):
                continue
            output_name = node.output[0]
            trace_names.append(output_name)
            graph.graph.output.append(onnx.helper.make_tensor_value_info(output_name, onnx.TensorProto.FLOAT, None))

            def hook(_, inputs, output, name=output_name, permuted=module.__class__.__name__ == "LayerNormPermuted"):
                if trace_enabled[0]:
                    a = output.detach().numpy().copy()
                    if permuted:
                        a = a.transpose(0, 2, 1)
                    trace_reference[name] = a
            trace_handles.append(module.register_forward_hook(hook))
        model_bytes = graph.SerializeToString()
    options = ort.SessionOptions()
    options.intra_op_num_threads = 1
    options.inter_op_num_threads = 1
    options.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
    options.add_session_config_entry("session.intra_op.allow_spinning", "0")
    options.add_session_config_entry("session.inter_op.allow_spinning", "0")
    options.graph_optimization_level = {
        "off": ort.GraphOptimizationLevel.ORT_DISABLE_ALL,
        "basic": ort.GraphOptimizationLevel.ORT_ENABLE_BASIC,
        "all": ort.GraphOptimizationLevel.ORT_ENABLE_ALL,
    }[args.optimization]
    providers = ["CPUExecutionProvider"]
    if args.backend == "directml":
        options.enable_mem_pattern = False
        options.add_session_config_entry("session.disable_cpu_ep_fallback", "1")
        providers = [("DmlExecutionProvider", {"device_id": str(args.device_id)})]
    session = ort.InferenceSession(model_bytes or str(model_path), options, providers=providers)
    assert session.get_providers()[0] == ("CPUExecutionProvider" if args.backend == "cpu" else "DmlExecutionProvider")
    assert [value.name for value in session.get_inputs()] == meta["input_names"]
    assert [value.shape for value in session.get_inputs()] == meta["shapes"]
    assert [value.name for value in session.get_outputs()][:5] == meta["output_names"]
    packets = np.fromfile(args.fixture / (args.case + ".input.f32"), dtype="<f4").reshape(-1, *meta["shapes"][0])
    count = min(len(packets), int(args.seconds * meta["sample_rate"] / meta["hop"]))
    if not count:
        p.error("Duration does not include one chunk")
    reference_states = [torch.zeros(s) for s in meta["shapes"][1:]]
    free_states = [s.numpy().copy() for s in reference_states]
    records = []
    reset_and_clone_pass = False
    native_repro_written = False
    started = time.perf_counter()
    with torch.inference_mode():
        for index, packet in enumerate(packets[:count]):
            # Save input states BEFORE the upstream call, which mutates buffers.
            identical_states = [s.numpy().copy() for s in reference_states]
            trace_enabled[0] = index == args.trace_chunk
            expected = model(torch.from_numpy(packet.copy()), *reference_states, pad=not model.lookahead)
            reference_states = list(expected[1:])
            expected_arrays = [s.numpy().copy() for s in expected]
            if index == 0:
                # The exported wrapper clones each buffer. Confirm that doing
                # so has no effect on upstream results, and a reset is exact.
                reset_output = model(
                    torch.from_numpy(packet.copy()),
                    *(torch.from_numpy(s.copy()).clone() for s in identical_states),
                    pad=not model.lookahead,
                )
                reset_and_clone_pass = all(np.array_equal(a, b.numpy()) for a, b in zip(expected_arrays, reset_output))
                assert reset_and_clone_pass
            forced = session.run(None, dict(zip(meta["input_names"], [packet, *identical_states])))
            recurrent = session.run(None, dict(zip(meta["input_names"], [packet, *free_states])))
            free_states = recurrent[1:5]
            row = {"chunk": index, "time_seconds": index * meta["hop"] / meta["sample_rate"]}
            for mode, values in [("identical_input", forced), ("free_running", recurrent)]:
                row[mode] = {
                    name: error(a, b) for name, a, b in zip(meta["output_names"], values, expected_arrays)
                }
                audio_error = row[mode]["converted"]
                audio_error["pass"] = audio_error["max"] <= 1e-4 and audio_error["rms"] <= 1e-5
            records.append(row)
            if args.native_repro and not native_repro_written and any(not e["pass"] for e in row["identical_input"].values()):
                import shutil
                args.native_repro.mkdir(parents=True, exist_ok=True)
                case_name = f"{args.case}-chunk-{index}"
                packet.astype("<f4").tofile(args.native_repro / (case_name + ".input.f32"))
                expected_arrays[0].astype("<f4").tofile(args.native_repro / (case_name + ".reference.f32"))
                np.concatenate([a.flatten() for a in expected_arrays[1:]]).astype("<f4").tofile(args.native_repro / (case_name + ".states.f32"))
                initial_file = case_name + ".initial.f32"
                np.concatenate([a.flatten() for a in identical_states]).astype("<f4").tofile(args.native_repro / initial_file)
                repro_meta = dict(meta)
                repro_meta["cases"] = [{"name": case_name, "chunks": 1, "initial_state_file": initial_file}]
                repro_meta["diagnostic_only"] = "Identical reference-state step; not a throughput benchmark"
                (args.native_repro / "fixture.json").write_text(json.dumps(repro_meta, indent=2), encoding="utf-8")
                shutil.copyfile(model_path, args.native_repro / model_path.name)
                native_repro_written = True
            if trace_enabled[0]:
                row["operator_trace"] = {
                    name: error(a, trace_reference[name])
                    for name, a in zip(trace_names, forced[5:])
                }
    for handle in trace_handles:
        handle.remove()
    report = {
        "schema": 1, "backend": args.backend, "providers": session.get_providers(),
        "directml_device_id": args.device_id if args.backend == "directml" else None,
        "optimization": args.optimization, "mkldnn": args.mkldnn,
        "source_revision": SOURCE_REV, "checkpoint_sha256": WEIGHTS_SHA,
        "runtime_version": ort.__version__, "torch_version": torch.__version__,
        "state_shapes": meta["shapes"][1:], "reset_and_clone_pass": reset_and_clone_pass,
        "fixture_reference_shift_samples": meta["reference_shift_samples"],
        "fixture_context": meta["context"],
        "model_sha256": meta["onnx_sha256"], "case": args.case, "hop": meta["hop"],
        "seconds": count * meta["hop"] / meta["sample_rate"],
        "elapsed_seconds": time.perf_counter() - started, "chunks": records,
        "pass": all(e["pass"] for r in records for mode in ["identical_input", "free_running"] for e in r[mode].values()),
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2), encoding="utf-8")
    for mode in ["identical_input", "free_running"]:
        for name in meta["output_names"]:
            entries = [r[mode][name] for r in records]
            print(mode, name, "max normalized RMS", max(e["normalized_rms"] for e in entries),
                  "max normalized peak", max(e["normalized_max"] for e in entries),
                  "first failing chunk", next((r["chunk"] for r in records if not r[mode][name]["pass"]), None))
    print("PASS" if report["pass"] else "FAIL", flush=True)
    return 0 if report["pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
