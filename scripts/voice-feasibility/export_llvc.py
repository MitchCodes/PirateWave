"""Offline feasibility gate, not PirateWave's production audio path.

Exports unmodified, pinned LLVC math with explicit state I/O. Python is a
development dependency only. Model/fixture/report binaries stay under dist/.
"""
import argparse
import ast
import hashlib
import json
import importlib.metadata
import pathlib
import subprocess
import sys
import time
import copy

SOURCE_REV = "1627c5d358cf9bb2b92b0ccc513d8b36807c923d"
WEIGHTS_REV = "ebfe8c0fdeb974a7eeb463b3abbc8ce42a0e3851"
WEIGHTS_SHA = "cceb7ab9621f84d62d283725ae3281cacb04f762d6a088fe3e97c1a29d4b8c0e"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--upstream", type=pathlib.Path, required=True)
    parser.add_argument("--checkpoint", type=pathlib.Path, required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    parser.add_argument("--chunk-factor", type=int, choices=[1, 2, 4], default=1)
    parser.add_argument("--stable-layernorm", action="store_true",
                        help="Export centered-variance arithmetic instead of fused LayerNormalization")
    parser.add_argument("--cases", nargs="+", help="Optional diagnostic subset; not full qualification")
    parser.add_argument("--freeze-label", action="store_true",
                        help="Precompute the model's fixed zero-label embedding with upstream PyTorch")
    parser.add_argument("--precise-layernorm", action="store_true",
                        help="Diagnostic CPU-only float64 normalization; float32 model boundary")
    args = parser.parse_args()
    upstream = args.upstream.resolve()
    revision = subprocess.check_output(["git", "-C", str(upstream), "rev-parse", "HEAD"], text=True).strip()
    assert revision == SOURCE_REV, revision
    assert sha(args.checkpoint) == WEIGHTS_SHA, "Checkpoint hash mismatch"
    assert not subprocess.check_output(["git", "-C", str(upstream), "status", "--porcelain", "--untracked-files=no"], text=True).strip(), "Upstream files modified"
    sys.path.insert(0, str(upstream))
    import numpy as np
    import onnx
    import torch
    import torchaudio
    from model import Net
    # Load the exact upstream function without importing unrelated training losses.
    tree = ast.parse((upstream / "infer.py").read_text(encoding="utf-8"))
    function = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == "infer_stream")
    reference_scope = {"torch": torch, "np": np, "time": time}
    exec(compile(ast.Module(body=[function], type_ignores=[]), str(upstream / "infer.py"), "exec"), reference_scope)
    infer_stream = reference_scope["infer_stream"]

    torch.set_num_threads(1)
    torch.set_num_interop_threads(1)
    torch.manual_seed(7)
    config_path = upstream / "experiments/llvc/config.json"
    config = json.loads(config_path.read_text(encoding="utf-8"))
    model = Net(**config["model_params"])
    model.load_state_dict(torch.load(args.checkpoint, map_location="cpu", weights_only=True)["model"])
    model.eval()
    rate = config["data"]["sr"]
    hop = model.L * model.dec_chunk_size * args.chunk_factor
    state = (*model.init_buffers(1, torch.device("cpu")), model.convnet_pre.init_ctx_buf(1, torch.device("cpu")))
    export_model = copy.deepcopy(model)
    if args.freeze_label:
        class ConstantLabel(torch.nn.Module):
            def __init__(self, value):
                super().__init__()
                self.register_buffer("value", value)

            def forward(self, label):
                return self.value

        with torch.inference_mode():
            embedding = model.label_embedding(torch.zeros(1, 1)).clone()
        export_model.label_embedding = ConstantLabel(embedding)

    class Streaming(torch.nn.Module):
        def __init__(self):
            super().__init__()
            self.net = export_model

        def forward(self, audio, enc, dec, out, pre):
            # Functionalize caller-visible state mutation for the ONNX boundary.
            return self.net(audio, enc.clone(), dec.clone(), out.clone(), pre.clone(), pad=not self.net.lookahead)

    outdir = args.out.resolve()
    outdir.mkdir(parents=True, exist_ok=True)
    names = ["audio", "encoder", "decoder", "synthesis", "prenet"]
    output_names = ["converted", "out_encoder", "out_decoder", "out_synthesis", "out_prenet"]
    model_path = outdir / "llvc-stream.onnx"
    if args.stable_layernorm or args.precise_layernorm:
        from torch.onnx.symbolic_helper import parse_args

        @parse_args("v", "is", "v", "v", "f", "b")
        def stable_layernorm(g, x, shape, weight, bias, eps, cudnn):
            if args.precise_layernorm:
                x = g.op("Cast", x, to_i=11)
                weight = g.op("Cast", weight, to_i=11)
                bias = g.op("Cast", bias, to_i=11)
            axes = list(range(-len(shape), 0))
            mean = g.op("ReduceMean", x, axes_i=axes, keepdims_i=1)
            centered = g.op("Sub", x, mean)
            variance = g.op("ReduceMean", g.op("Mul", centered, centered), axes_i=axes, keepdims_i=1)
            dtype = torch.float64 if args.precise_layernorm else torch.float32
            denominator = g.op("Sqrt", g.op("Add", variance, g.op("Constant", value_t=torch.tensor(eps, dtype=dtype))))
            normalized = g.op("Div", centered, denominator)
            result = g.op("Add", g.op("Mul", normalized, weight), bias)
            return g.op("Cast", result, to_i=1) if args.precise_layernorm else result

        torch.onnx.register_custom_op_symbolic("aten::layer_norm", stable_layernorm, 17)
    example = (torch.zeros(1, 1, hop + 2 * model.L), *state)
    with torch.inference_mode():
        torch.onnx.export(Streaming().eval(), example, str(model_path), input_names=names, output_names=output_names, opset_version=17, do_constant_folding=True)
    exported = onnx.load(str(model_path))
    onnx.checker.check_model(exported, full_check=True)
    metadata = {
        "schema": 1, "source_revision": SOURCE_REV, "weights_revision": WEIGHTS_REV,
        "checkpoint_sha256": WEIGHTS_SHA, "config_sha256": sha(config_path),
        "onnx_sha256": sha(model_path), "sample_rate": rate, "hop": hop,
        "context": 2 * model.L, "reference_shift_samples": model.L,
        "input_names": names, "output_names": output_names,
        "shapes": [list(x.shape) for x in example], "cases": [],
        "versions": {name: importlib.metadata.version(name) for name in ["torch", "torchaudio", "onnx", "numpy", "speechbrain", "onnxruntime-directml"]},
        "onnx_nodes": len(exported.graph.node),
        "stable_layernorm": args.stable_layernorm,
        "freeze_label": args.freeze_label,
        "precise_layernorm": args.precise_layernorm,
    }
    cases = []
    for path in sorted((upstream / "test_wavs").glob("*.wav")):
        audio, sr = torchaudio.load(str(path))
        audio = audio.mean(0)
        if sr != rate:
            audio = torchaudio.functional.resample(audio, sr, rate)
        cases.append((path.stem, audio, sha(path)))
    cases += [("silence", torch.zeros(rate * 2), None), ("quiet_speech", cases[0][1] * 0.02, cases[0][2]), ("loud_speech", (cases[1][1] * 3).clamp(-1, 1), cases[1][2])]
    cases.append(("long_mixed_speech", torch.cat([x[1] for x in cases[:10]] * 2), None))
    if args.cases:
        selected = set(args.cases)
        cases = [case for case in cases if case[0] in selected]
        assert {case[0] for case in cases} == selected, "Unknown diagnostic case"
    with torch.inference_mode():
        for name, audio, original_sha in cases:
            original_audio = audio.clone()
            # Reproduce upstream infer_stream's padding, one-L shift and context exactly.
            original_len = len(audio)
            pad = (-len(audio)) % hop
            audio = torch.nn.functional.pad(audio, (0, pad))
            audio = torch.cat((audio[model.L:], torch.zeros(model.L)))
            chunks = list(torch.split(audio, hop))
            packets = [torch.cat((torch.zeros(2 * model.L) if i == 0 else chunks[i - 1][-2 * model.L:], a)) for i, a in enumerate(chunks)]
            states = tuple(x.clone() for x in state)
            output = []
            started = time.perf_counter()
            for packet in packets:
                value = model(packet.reshape(1, 1, -1), *states, pad=not model.lookahead)
                output.append(value[0].flatten().numpy().copy())
                states = value[1:]
            elapsed = time.perf_counter() - started
            reference = np.concatenate(output)
            # Independently check our boundary preparation against the shipped reference.
            upstream_output, _, _ = infer_stream(model, original_audio, args.chunk_factor, rate)
            np.testing.assert_allclose(reference[:original_len], upstream_output.flatten().numpy(), rtol=0, atol=1e-7)
            reference.astype("<f4").tofile(outdir / (name + ".reference.f32"))
            np.concatenate([p.numpy() for p in packets]).astype("<f4").tofile(outdir / (name + ".input.f32"))
            np.concatenate([s.flatten().numpy() for s in states]).astype("<f4").tofile(outdir / (name + ".states.f32"))
            metadata["cases"].append({"name": name, "chunks": len(packets), "original_samples": original_len, "input_sha256": original_sha, "reference_seconds": elapsed})
            print(f"Reference {name}: {len(packets)} chunks, {elapsed:.2f}s", flush=True)
    (outdir / "fixture.json").write_text(json.dumps(metadata, indent=2), encoding="utf-8")
    print(f"Exported {metadata['onnx_nodes']} nodes: {model_path}", flush=True)


if __name__ == "__main__":
    main()
