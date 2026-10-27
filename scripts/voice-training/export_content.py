"""Build-time export of the original frozen HuBERT content objective.

Keeps fairseq out of the shipped runtime; validates outputs AND input gradients.
Requires the pinned fairseq build environment, never runs on end users' machines.
"""
import argparse
import json
from pathlib import Path
import torch
import fairseq
from assets import ASSETS, sha256


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkpoint", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if sha256(args.checkpoint) != ASSETS["hubert_base_ls960.pt"][2]:
        raise ValueError("Content objective checkpoint differs from pinned original")
    torch.set_num_threads(2)
    torch.manual_seed(42)
    models, _, _ = fairseq.checkpoint_utils.load_model_ensemble_and_task([str(args.checkpoint)])
    reference = models[0].feature_extractor.eval().requires_grad_(False)
    traced = torch.jit.trace(reference, torch.randn(1, 10240))
    rows = []
    for size in (10240, 16000, 32000):
        a = torch.randn(1, size, requires_grad=True)
        b = a.detach().clone().requires_grad_(True)
        x, y = reference(a), traced(b)
        x.square().mean().backward()
        y.square().mean().backward()
        output_error = float((x - y).abs().max())
        gradient_error = float((a.grad - b.grad).abs().max())
        passed = output_error <= 1e-6 and gradient_error <= 1e-7
        rows.append(dict(samples=size, output_max=output_error, gradient_max=gradient_error, passed=passed))
    if not all(r["passed"] for r in rows):
        raise RuntimeError("Content extractor export changed the training objective")
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("wb") as stream:
        torch.jit.save(traced, stream)
    report = dict(schema=1, source_sha256=sha256(args.checkpoint), model_sha256=sha256(args.out),
                  torch=torch.__version__, checks=rows, qualified_for_release=False)
    args.out.with_suffix(".json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report))


if __name__ == "__main__":
    main()
