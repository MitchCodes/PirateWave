"""Export a locally trained generator and compare the actual Rust worker.

This produces a development artifact, never an installed/qualified voice pack.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time

import numpy as np
import torch
import soundfile as sf

from assets import LLVC_REV, sha256
from probe import atomic_json, verify_source


SHAPES = [(1, 512, 510), (1, 2, 13, 256), (1, 512, 4), (1, 1, 24)]


def comparison(actual, expected, waveform=False):
    a = np.asarray(actual, dtype=np.float64)
    b = np.asarray(expected, dtype=np.float64)
    finite = bool(np.isfinite(a).all() and np.isfinite(b).all())
    if not finite:
        return {"pass": False, "finite": False}
    delta = a - b
    peak = float(np.max(np.abs(delta)))
    rms = float(np.sqrt(np.mean(delta * delta)))
    ref_peak = max(1.0, float(np.max(np.abs(b))))
    ref_rms = max(1.0, float(np.sqrt(np.mean(b * b))))
    return dict(finite=True, max=peak, rms=rms, normalized_max=peak / ref_peak, normalized_rms=rms / ref_rms,
                **{"pass": peak <= 1e-4 and rms <= 1e-5 if waveform else peak / ref_peak <= 1e-3 and rms / ref_rms <= 1e-4})


def read_exact(pipe, length):
    result = bytearray()
    while len(result) < length:
        value = pipe.read(length - len(result))
        if not value:
            raise EOFError("Native inference worker exited before completing a tensor")
        result.extend(value)
    return bytes(result)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--llvc", type=Path, required=True)
    parser.add_argument("--checkpoint", type=Path, required=True)
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--speech", type=Path, nargs="+", required=True)
    parser.add_argument("--seconds", type=float, default=30)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if not 1 <= args.seconds <= 1800:
        parser.error("Use 1–1800 seconds")
    verify_source(args.llvc, LLVC_REV)
    torch.set_num_threads(2)
    torch.set_num_interop_threads(1)
    sys.path.insert(0, str(args.llvc.resolve()))
    from model import Net
    config = json.loads((args.llvc / "experiments/llvc/config.json").read_text())
    net = Net(**config["model_params"])
    net.load_state_dict(torch.load(args.checkpoint, map_location="cpu", weights_only=True)["model"])
    net.eval()

    class Streaming(torch.nn.Module):
        def __init__(self):
            super().__init__()
            self.net = net

        def forward(self, audio, encoder, decoder, synthesis, prenet):
            return self.net(audio, encoder.clone(), decoder.clone(), synthesis.clone(), prenet.clone(), pad=not self.net.lookahead)

    args.out.mkdir(parents=True, exist_ok=True)
    states = [torch.zeros(shape) for shape in SHAPES]
    model_path = args.out / "llvc-stream.pt"
    speech = []
    for path in args.speech:
        audio, rate = sf.read(path, dtype="float32")
        if rate != 16000 or audio.ndim != 1 or not np.isfinite(audio).all():
            raise ValueError("Expected finite mono 16 kHz speech")
        speech.extend([audio, np.zeros(8000, dtype=np.float32)])
    phrase = np.concatenate(speech)
    count = int(np.ceil(args.seconds * 16000 / 416))
    audio = np.resize(phrase, count * 416)
    prefix = np.zeros(32, dtype=np.float32)
    rows, converted, times = [], [], []
    process = None
    with torch.inference_mode():
        exported = torch.jit.trace(Streaming().eval(), (torch.zeros(1, 1, 448), *states), check_trace=False)
        # LibTorch's narrow Windows path APIs cannot open every Unicode path.
        # Let Python open the file with Windows Unicode semantics instead.
        with model_path.open("wb") as stream:
            torch.jit.save(exported, stream)
        with model_path.open("rb") as stream:
            exported = torch.jit.load(stream).eval()
        try:
            process = subprocess.Popen([str(args.worker.resolve()), str(model_path.resolve()), "--diagnostic-states"],
                                       stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                       creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
            if read_exact(process.stdout, 8) != b"PATCHVS1":
                raise ValueError("Wrong native diagnostic protocol")
            for index in range(count):
                hop = audio[index * 416:(index + 1) * 416]
                packet = np.concatenate([prefix, hop]).astype("<f4")
                prefix = hop[-32:]
                x = torch.from_numpy(packet.copy()).reshape(1, 1, 448)
                identical = [v.clone() for v in states]
                expected = net(x, *states, pad=not net.lookahead)
                states = [v.clone() for v in expected[1:]]
                single = exported(x, *identical)
                started = time.perf_counter()
                process.stdin.write(packet.tobytes())
                process.stdin.flush()
                sizes = [416, *[int(np.prod(shape)) for shape in SHAPES]]
                values = np.frombuffer(read_exact(process.stdout, sum(sizes) * 4), dtype="<f4")
                times.append((time.perf_counter() - started) * 1000)
                offset = 0
                row = {"chunk": index, "native": {}, "single_step": {}}
                for i, (name, size) in enumerate(zip(["audio", "encoder", "decoder", "synthesis", "prenet"], sizes)):
                    actual = values[offset:offset + size]
                    offset += size
                    row["native"][name] = comparison(actual, expected[i].numpy().reshape(-1), i == 0)
                    row["single_step"][name] = comparison(single[i].numpy(), expected[i].numpy(), i == 0)
                    if i == 0:
                        converted.append(actual.copy())
                rows.append(row)
                if index % 100 == 0:
                    print(json.dumps({"chunk": index, "total": count}), flush=True)
            process.stdin.close()
            if process.wait(timeout=20):
                raise RuntimeError(process.stderr.read().decode(errors="replace"))
        finally:
            if process is not None and process.poll() is None:
                process.kill()
                process.wait()
    samples = np.concatenate(converted)
    sf.write(args.out / "native-converted.wav", samples, 16000, subtype="PCM_16")
    timings = times[20:] or times
    # Normal protocol timing excludes the diagnostic megabyte of state transfers
    # and the parallel reference-model work. Keep the two measurements distinct.
    normal_times, normal_audio = [], []
    process = subprocess.Popen([str(args.worker.resolve()), str(model_path.resolve())],
                              stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
    try:
        if read_exact(process.stdout, 8) != b"PATCHVC1":
            raise ValueError("Wrong normal inference protocol")
        prefix = np.zeros(32, dtype=np.float32)
        for index in range(count):
            hop = audio[index * 416:(index + 1) * 416]
            packet = np.concatenate([prefix, hop]).astype("<f4")
            prefix = hop[-32:]
            started = time.perf_counter()
            process.stdin.write(packet.tobytes())
            process.stdin.flush()
            normal_audio.append(np.frombuffer(read_exact(process.stdout, 416 * 4), dtype="<f4").copy())
            normal_times.append((time.perf_counter() - started) * 1000)
        process.stdin.close()
        if process.wait(timeout=20):
            raise RuntimeError(process.stderr.read().decode(errors="replace"))
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
    normal_timings = normal_times[20:] or normal_times
    report = dict(schema=1, model_sha256=sha256(model_path), checkpoint_sha256=sha256(args.checkpoint),
                  worker_sha256=sha256(args.worker), llvc_revision=LLVC_REV, seconds=count * 416 / 16000,
                  waveform_and_state_pass=all(v["pass"] for row in rows for mode in ["native", "single_step"] for v in row[mode].values()),
                  diagnostic_mean_ms=float(np.mean(timings)), diagnostic_p99_ms=float(np.percentile(timings, 99)),
                  normal_protocol_parity=comparison(np.concatenate(normal_audio), samples, True),
                  native_mean_ms=float(np.mean(normal_timings)), native_p99_ms=float(np.percentile(normal_timings, 99)),
                  timing_pass=float(np.mean(normal_timings)) <= 13 and float(np.percentile(normal_timings, 99)) < 26,
                  public_training_enabled=False, release_qualified=False, rows=rows,
                  pending=["Representative training/quality evaluation", "Thirty-minute streaming qualification",
                           "Integrated latency/stability qualification", "Clean Windows bundled-runtime qualification"])
    atomic_json(args.out / "report.json", report)
    print(json.dumps({k: v for k, v in report.items() if k != "rows"}), flush=True)
    return 0 if report["waveform_and_state_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
