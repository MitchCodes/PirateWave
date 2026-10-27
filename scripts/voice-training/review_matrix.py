"""Produce reproducible teacher comparisons before accepting an LLVC recipe."""
import argparse
import html
import json
from pathlib import Path
import subprocess
import sys
from probe import atomic_json


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--steps", type=int, default=250)
    parser.add_argument("--render-only", action="store_true")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    rows = []
    for speaker in ("1272", "1462", "1673"):
        for mode in ("no-f0", "f0"):
            output = args.out / f"{speaker}-{mode}"
            if args.render_only:
                report = json.loads((output / "report.json").read_text())
                if "error" in report or report.get("stage") != "teacher_listening_pending":
                    raise ValueError("Teacher comparison is incomplete: " + str(output))
                rows.append(dict(target=speaker, mode=mode, accepted=False))
                continue
            command = [sys.executable, str(Path(__file__).with_name("supervise.py")),
                       "--out", str(output), "--timeout", "1800", "--",
                       "--rvc", str(args.root / ".tools/voice-training/RVC"),
                       "--llvc", str(args.root / ".tools/voice-feasibility/LLVC"),
                       "--assets", str(args.root / ".tools/voice-training/assets"),
                       "--initial-llvc", str(args.root / ".tools/voice-feasibility/llvc.pth"),
                       "--backend", "cuda", "--steps", str(args.steps), "--clips", "60",
                       "--source-clips", "2", "--clip-seconds", "8", "--checkpoint-every", "50",
                       "--speaker", speaker, "--teacher-mode", mode, "--teacher-only"]
            print(f"Evaluating teacher {speaker} / {mode}", flush=True)
            completed = subprocess.run(command, stdout=subprocess.DEVNULL)
            report_path = output / "report.json"
            report = json.loads(report_path.read_text()) if report_path.exists() else {}
            row = dict(target=speaker, mode=mode, exit_code=completed.returncode,
                       elapsed_seconds=report.get("elapsed_seconds"),
                       error=report.get("error"), accepted=False)
            rows.append(row)
            atomic_json(args.out / "matrix.json", dict(schema=1, rows=rows, accepted=False))
            if completed.returncode:
                raise RuntimeError(f"Teacher failed: {output}; see its report")
    body = ['<!doctype html><meta charset="utf-8"><title>PirateWave teacher review</title>',
            '<style>body{font:16px system-ui;background:#171823;color:#eee;max-width:1000px;margin:40px auto}section{padding:20px;background:#232535;margin:20px 0}audio{width:290px}td{padding:8px}a{color:#bbb4ff}</style>',
            f'<h1>PirateWave teacher review — experimental</h1><p>{args.steps} updates are a smoke experiment, not an accepted training duration. '
            'Listen through headphones/your monitoring output. These clips are not sent through PirateWave routes. '
            'Compare intelligibility, target identity, consonants and persistent artifacts. No result is accepted automatically.</p>',
            '<button id="device">Choose monitoring device</button><p id="device-status">Browser default output. Choose headphones; do not choose a virtual outgoing bus.</p>']
    for row in rows:
        prefix = f"{row['target']}-{row['mode']}/review"
        body.append(f"<section><h2>Target {html.escape(row['target'])}: {html.escape(row['mode'])}</h2>")
        body.append(f'<p>Target reference</p><audio controls preload="none" src="{prefix}/target-reference-0.wav"></audio>')
        body.append('<table><tr><th>Held-out speaker</th><th>Original</th><th>Teacher</th></tr>')
        for i in range(6):
            body.append(f'<tr><td>{"Validation" if i < 3 else "Test"} {i + 1}</td>' + ''.join(
                f'<td><audio controls preload="none" src="{prefix}/{i}-{kind}.wav"></audio></td>'
                for kind in ("original", "teacher")) + '</tr>')
        body.append('<p>Review notes: intelligibility, resemblance to target, consonants, metallic/noisy artifacts.</p>'
                    f'<textarea aria-label="Notes for {row["target"]} {row["mode"]}" data-target="{row["target"]}" data-mode="{row["mode"]}" rows="4" style="width:95%"></textarea></section>')
    body.append('''<button id="save">Save listening notes</button>
<script>
document.querySelectorAll('audio').forEach(a=>a.addEventListener('play',()=>document.querySelectorAll('audio').forEach(b=>{if(a!==b)b.pause()})));
document.querySelector('#device').onclick=async()=>{try{
 if(!navigator.mediaDevices?.selectAudioOutput)throw Error('Device selection is unavailable in this browser. Select its output in Windows Volume mixer.');
 const d=await navigator.mediaDevices.selectAudioOutput();
 for(const a of document.querySelectorAll('audio'))await a.setSinkId(d.deviceId);
 document.querySelector('#device-status').textContent='Monitoring through '+d.label;
}catch(e){document.querySelector('#device-status').textContent=e.message}};
document.querySelector('#save').onclick=()=>{
 const reviews=Array.from(document.querySelectorAll('textarea')).map(a=>({target:a.dataset.target,mode:a.dataset.mode,notes:a.value}));
 const url=URL.createObjectURL(new Blob([JSON.stringify({schema:1,accepted:false,reviews},null,2)],{type:'application/json'}));
 const a=document.createElement('a');a.href=url;a.download='piratewave-listening-notes.json';a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);
};
</script>''')
    (args.out / "review.html").write_text('\n'.join(body), encoding="utf-8")
    print(str(args.out / "review.html"))


if __name__ == "__main__":
    main()
