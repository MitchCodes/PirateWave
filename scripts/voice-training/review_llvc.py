"""Evaluate the preferred no-F0 teacher through LLVC and the native worker."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
from probe import atomic_json


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--root", type=Path, default=Path.cwd())
    p.add_argument("--teachers", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--steps", type=int, default=500)
    args = p.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    rows = []
    for speaker in ("1272", "1462", "1673"):
        output = args.out / speaker
        teacher = args.teachers / f"{speaker}-no-f0" / "teacher.checkpoint.pt"
        command = [sys.executable, str(Path(__file__).with_name("supervise.py")),
                   "--out", str(output), "--timeout", "1800", "--",
                   "--rvc", str(args.root / ".tools/voice-training/RVC"),
                   "--llvc", str(args.root / ".tools/voice-feasibility/LLVC"),
                   "--assets", str(args.root / ".tools/voice-training/assets"),
                   "--initial-llvc", str(args.root / ".tools/voice-feasibility/llvc.pth"),
                   "--backend", "cuda", "--steps", str(args.steps), "--clips", "60",
                   "--source-clips", "100", "--clip-seconds", "8", "--checkpoint-every", "50",
                   "--speaker", speaker, "--teacher-mode", "no-f0", "--teacher-checkpoint", str(teacher),
                   "--content-extractor", str(args.root / ".tools/voice-training/assets/content-extractor.pt")]
        print(f"Training LLVC candidate {speaker}", flush=True)
        completed = subprocess.run(command, stdout=subprocess.DEVNULL)
        if completed.returncode:
            raise RuntimeError("Candidate training failed: " + str(output))
        export = [sys.executable, str(Path(__file__).with_name("export_probe.py")),
                  "--llvc", str(args.root / ".tools/voice-feasibility/LLVC"),
                  "--checkpoint", str(output / "llvc-generator.pt"),
                  "--worker", str(args.root / "dist/voice-pack/llvc-f8312-v1/PatchVoiceWorker.exe"),
                  "--speech", *[str(output / f"review/{i}-original.wav") for i in (3, 4, 5)],
                  "--seconds", "30", "--out", str(output / "native")]
        with (output / "export.log").open("w") as log:
            completed = subprocess.run(export, stdout=log, stderr=subprocess.STDOUT, timeout=300)
        if completed.returncode:
            raise RuntimeError("Native candidate comparison failed: " + str(output))
        validation = json.loads((output / "native/report.json").read_text())
        rows.append(dict(target=speaker, teacher="no-f0", llvc_steps=args.steps,
                         native_waveform_state_pass=validation["waveform_and_state_pass"], accepted=False))
        atomic_json(args.out / "matrix.json", dict(schema=1, rows=rows, accepted=False))
    page = ['<!doctype html><meta charset="utf-8"><title>PirateWave LLVC review</title>',
            '<style>body{font:16px system-ui;max-width:1000px;margin:40px auto;background:#171823;color:white}section{margin:30px 0;padding:20px;background:#232535}td{padding:10px}</style>',
            '<h1>LLVC candidate review</h1><p>Experimental, not installed in PirateWave. Use headphones or a monitoring device; do not route browser audio to your outgoing bus. '
            'The native sample cycles three held-out speakers with silence between clips. Offline samples are diagnostic comparisons, not live acceptance.</p>']
    for row in rows:
        speaker = row["target"]
        page.append(f'<section><h2>Target {speaker}</h2><p>Target reference</p><audio controls preload="none" src="{speaker}/review/target-reference-0.wav"></audio>'
                    f'<p>Actual native-worker streaming output</p><audio controls preload="none" src="{speaker}/native/native-converted.wav"></audio><table><tr><th>Original</th><th>Teacher</th><th>LLVC offline</th></tr>')
        for i in (3, 4, 5):
            page.append('<tr>' + ''.join(f'<td><audio controls preload="none" src="{speaker}/review/{i}-{kind}.wav"></audio></td>'
                                        for kind in ('original', 'teacher', 'llvc-offline')) + '</tr>')
        page.append('</table></section>')
    page.append("<script>document.querySelectorAll('audio').forEach(a=>a.addEventListener('play',()=>document.querySelectorAll('audio').forEach(b=>{if(a!==b)b.pause()})));</script>")
    (args.out / "review.html").write_text('\n'.join(page), encoding="utf-8")
    print(str(args.out / "review.html"))


if __name__ == "__main__":
    main()
