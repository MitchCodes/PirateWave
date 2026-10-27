"""Real training feasibility probe, NOT a released voice-creation runtime.

Exercises the pinned RVC v1 no-f0 teacher and unchanged LLVC architecture/losses
on licensed speech. Small runs prove execution only, never voice quality.
"""
import argparse
import gc
import hashlib
import json
import os
from pathlib import Path
import random
import shutil
import subprocess
import sys
import time
import traceback

from assets import ASSETS, PITCH_ASSETS, LLVC_REV, RVC_REV, sha256
from dataset import split, manifest


def atomic_json(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    with temporary.open("w", encoding="utf-8") as f:
        json.dump(value, f, indent=2, allow_nan=False)
        f.flush()
        os.fsync(f.fileno())
    os.replace(temporary, path)


def checkpoint(path, value):
    import torch
    temporary = path.with_suffix(".tmp")
    with temporary.open("wb") as f:
        torch.save(value, f)
        f.flush()
        os.fsync(f.fileno())
    if path.exists():
        previous = path.with_suffix(path.suffix + ".previous")
        previous_temporary = previous.with_suffix(previous.suffix + ".tmp")
        shutil.copyfile(path, previous_temporary)
        with previous_temporary.open("r+b") as stream:
            os.fsync(stream.fileno())
        os.replace(previous_temporary, previous)
    os.replace(temporary, path)


def device_for(name):
    import torch
    if name == "directml":
        import torch_directml
        return torch_directml.device()
    device = torch.device(name)
    if name == "cuda" and not torch.cuda.is_available():
        raise RuntimeError("CUDA is unavailable; refusing implicit CPU fallback")
    if name == "xpu" and not (hasattr(torch, "xpu") and torch.xpu.is_available()):
        raise RuntimeError("Intel XPU is unavailable; refusing implicit CPU fallback")
    return device


def synchronize(device):
    import torch
    if device.type == "cuda":
        torch.cuda.synchronize(device)
    elif device.type == "xpu":
        torch.xpu.synchronize(device)
    else:
        # Also forces DirectML completion without assuming CUDA semantics.
        torch.ones(1, device=device).cpu()


def verify_source(path, revision):
    packaged = path / "patch-source.json"
    if packaged.is_file():
        content = json.loads(packaged.read_text(encoding="utf-8"))
        if content.get("revision") != revision or not content.get("files"):
            raise ValueError("Packaged source revision mismatch")
        for relative, expected in content["files"].items():
            candidate = path / relative
            if not candidate.resolve().is_relative_to(path.resolve()) or sha256(candidate) != expected:
                raise ValueError("Packaged source file mismatch: " + relative)
        return
    actual = subprocess.check_output(["git", "-C", str(path), "rev-parse", "HEAD"], text=True).strip()
    dirty = subprocess.check_output(["git", "-C", str(path), "status", "--porcelain", "--untracked-files=no"], text=True)
    if actual != revision or dirty:
        raise ValueError("Training source differs from pinned clean revision: " + str(path))


def check_gradients(model):
    import torch
    gradients = [p.grad for p in model.parameters() if p.grad is not None]
    if not gradients or not all(bool(torch.isfinite(g).all().cpu()) for g in gradients):
        raise RuntimeError("Missing or non-finite training gradients")


def validate_teacher(previous, recipe):
    for key in ("rvc_revision", "speaker", "clip_seconds", "teacher_mode", "optimizer"):
        if previous.get(key) != recipe.get(key):
            raise ValueError("Reusable teacher differs in " + key)
    if previous["recordings"]["target_train"] != recipe["recordings"]["target_train"]:
        raise ValueError("Reusable teacher has different target recordings")


def optimizer_state(net_g, net_d, optim_g, optim_d, schedulers, step, recipe):
    import numpy as np
    import torch
    device = next(net_g.parameters()).device.type
    numpy_rng = np.random.get_state()
    return dict(schema=1, recipe=recipe, step=step, model=net_g.state_dict(),
                discriminator=net_d.state_dict(), optim_g=optim_g.state_dict(),
                optim_d=optim_d.state_dict(), schedulers=[s.state_dict() for s in schedulers],
                python_rng=random.getstate(), torch_rng=torch.get_rng_state(),
                numpy_rng=[numpy_rng[0], numpy_rng[1].tolist(), *numpy_rng[2:]],
                cuda_rng=torch.cuda.get_rng_state_all() if device == "cuda" else [],
                xpu_rng=torch.xpu.get_rng_state_all() if device == "xpu" else [])


def restore(path, net_g, net_d, optim_g, optim_d, schedulers, recipe):
    import numpy as np
    import torch
    state = torch.load(path, map_location="cpu", weights_only=True)
    if state["schema"] != 1 or state["recipe"] != recipe:
        raise ValueError("Checkpoint does not match this recipe/dataset/backend")
    net_g.load_state_dict(state["model"])
    net_d.load_state_dict(state["discriminator"])
    optim_g.load_state_dict(state["optim_g"])
    optim_d.load_state_dict(state["optim_d"])
    for scheduler, saved in zip(schedulers, state["schedulers"]):
        scheduler.load_state_dict(saved)
    random.setstate(state["python_rng"])
    torch.set_rng_state(state["torch_rng"])
    if "numpy_rng" in state:
        numpy_rng = state["numpy_rng"]
        np.random.set_state((numpy_rng[0], np.asarray(numpy_rng[1], dtype=np.uint32), *numpy_rng[2:]))
    if state["cuda_rng"]:
        torch.cuda.set_rng_state_all(state["cuda_rng"])
    if state["xpu_rng"]:
        torch.xpu.set_rng_state_all(state["xpu_rng"])
    return state["step"]


def stage_steps(args, stage):
    """Keep legacy --steps as the default, with independently controlled stages."""
    return getattr(args, stage + "_steps", None) or args.steps


def training_context(original, target, samples):
    """Matched crops; longer-context experiments pad short speech like upstream."""
    import torch.nn.functional as F
    if original.numel() != target.numel():
        raise ValueError("Training pair lengths must match")
    size = min(original.numel(), samples)
    start = random.randint(0, original.numel() - size)
    original, target = original[start:start + size], target[start:start + size]
    if samples > 10240 and size < samples:
        original, target = F.pad(original, (0, samples - size)), F.pad(target, (0, samples - size))
    return original[None, None], target[None, None]


def warm_discriminator(args, net_g, net_d, loss_step, recipe, report):
    """Warm only discriminator weights; joint training starts with fresh optimizers/RNG."""
    import numpy as np
    import torch
    total = args.discriminator_warmup_steps
    if not total:
        return
    report_path = args.out / 'warmup.json'
    if args.resume and (args.out / 'llvc.checkpoint.pt').exists():
        report['warmup'] = json.loads(report_path.read_text())
        return
    before = {key: value.detach().cpu().clone() for key, value in net_g.state_dict().items()}
    python_rng, numpy_rng = random.getstate(), np.random.get_state()
    torch_rng = torch.get_rng_state()
    cuda_rng = torch.cuda.get_rng_state_all() if args.device.type == 'cuda' else None
    modes = net_g.training, net_d.training
    flags = [p.requires_grad for p in net_g.parameters()]
    net_g.eval().requires_grad_(False)
    net_d.train()
    optimizers = [torch.optim.AdamW(n.parameters(), lr=1e-4, betas=(.8,.99), eps=1e-9) for n in (net_g,net_d)]
    schedulers = [torch.optim.lr_scheduler.ExponentialLR(o, gamma=.999875) for o in optimizers]
    path = args.out / 'discriminator-warmup.checkpoint.pt'
    began = time.perf_counter()
    start = restore(path, net_g, net_d, *optimizers, schedulers, recipe) if args.resume and path.exists() else 0
    if start > total:
        raise ValueError('Warm-up checkpoint exceeds requested budget')
    losses = []
    try:
        for step in range(start, total):
            if (args.out / 'pause.request').exists():
                checkpoint(path, optimizer_state(net_g, net_d, *optimizers, schedulers, step, recipe))
                raise InterruptedError('Paused discriminator warm-up')
            losses.append(loss_step(step, optimizers[1]))
            if step % 50 == 0:
                print(json.dumps(dict(stage='discriminator_warmup', step=step+1, total=total, losses=losses[-1])), flush=True)
            if (step+1) % args.checkpoint_every == 0 or step+1 == total:
                checkpoint(path, optimizer_state(net_g, net_d, *optimizers, schedulers, step+1, recipe))
        unchanged = all(torch.equal(before[k], v.detach().cpu()) for k,v in net_g.state_dict().items())
        if not unchanged:
            raise RuntimeError('Generator changed during discriminator-only warm-up')
        report['warmup'] = dict(steps=total, resumed_from=start, generator_unchanged=unchanged,
            elapsed_seconds=time.perf_counter()-began, losses=losses,
            joint_optimizer_policy='Fresh optimizers and restored pre-warm-up RNG; only discriminator weights carry forward',
            checkpoint_sha256=sha256(path))
        atomic_json(report_path, report['warmup'])
    finally:
        random.setstate(python_rng)
        np.random.set_state(numpy_rng)
        torch.set_rng_state(torch_rng)
        if cuda_rng is not None:
            torch.cuda.set_rng_state_all(cuda_rng)
        net_g.train(modes[0])
        net_d.train(modes[1])
        for p, flag in zip(net_g.parameters(), flags):
            p.requires_grad_(flag)


def train_loop(args, stage, net_g, net_d, loss_step, recipe, report):
    import torch
    net_g.train()
    net_d.train()
    optimizers = [torch.optim.AdamW(n.parameters(), lr=1e-4, betas=(0.8, 0.99), eps=1e-9)
                  for n in (net_g, net_d)]
    schedulers = [torch.optim.lr_scheduler.ExponentialLR(o, gamma=0.999875) for o in optimizers]
    state_file = args.out / (stage + ".checkpoint.pt")
    start = restore(state_file, net_g, net_d, *optimizers, schedulers, recipe) if args.resume and state_file.exists() else 0
    total = stage_steps(args, stage)
    if start > total:
        raise ValueError("Checkpoint is beyond the requested stage budget")
    times, losses = [], []
    for step in range(start, total):
        if (args.out / "pause.request").exists():
            checkpoint(state_file, optimizer_state(net_g, net_d, *optimizers, schedulers, step, recipe))
            raise InterruptedError("Paused after atomic checkpoint")
        synchronize(args.device)
        began = time.perf_counter()
        values = loss_step(step, *optimizers)
        synchronize(args.device)
        times.append(time.perf_counter() - began)
        losses.append(values)
        if step % 10 == 0:
            print(json.dumps(dict(stage=stage, step=step + 1, total=total, seconds=times[-1], losses=values)), flush=True)
        # One immutable recipe; user can resume an interrupted development run.
        if (step + 1) % args.checkpoint_every == 0:
            for scheduler in schedulers:
                scheduler.step()
        if (step + 1) % args.checkpoint_every == 0 or step + 1 == total:
            checkpoint(state_file, optimizer_state(net_g, net_d, *optimizers, schedulers, step + 1, recipe))
    report[stage] = dict(optimizer_steps=total, resumed_from=start,
                         mean_step_seconds=sum(times) / len(times) if times else None,
                         max_step_seconds=max(times) if times else None, losses=losses,
                         checkpoint_bytes=state_file.stat().st_size, checkpoint_sha256=sha256(state_file),
                         finite_gradients=True, quality_accepted=False)
    atomic_json(args.out / "report.json", report)
    net_g.eval()


def run(args, report):
    import numpy as np
    import soundfile as sf
    import torch
    import torchaudio
    import torch.nn.functional as F
    verify_source(args.rvc, RVC_REV)
    verify_source(args.llvc, LLVC_REV)
    required_assets = dict(ASSETS)
    if args.teacher_mode == "f0":
        required_assets.update(PITCH_ASSETS)
    if args.content_extractor:
        required_assets.pop('hubert_base_ls960.pt', None)
    for name, (_, size, expected) in required_assets.items():
        if not (args.assets / name).exists() or (args.assets / name).stat().st_size != size or sha256(args.assets / name) != expected:
            raise ValueError("Missing or corrupt pinned training asset: " + name)
    torch.set_num_threads(args.threads)
    torch.set_num_interop_threads(1)
    torch.manual_seed(42)
    np.random.seed(42)
    random.seed(42)
    args.device = device_for(args.backend)
    report.update(torch=torch.__version__, device=str(args.device), threads=args.threads)
    if args.device.type == "cuda":
        report["gpu"] = torch.cuda.get_device_name(args.device)
        report["gpu_free_bytes_before"] = torch.cuda.mem_get_info()[0]
        torch.cuda.reset_peak_memory_stats()
    sys.path.insert(0, str(args.rvc.resolve()))
    from infer.module.models import SynthesizerTrnMs256NSFsid_nono, SynthesizerTrnMs256NSFsid, MultiPeriodDiscriminator
    from infer.hubert import HubertModelWithFinalProj
    from infer.module import commons
    from train import losses as rvc_losses
    from train.mel_processing import spectrogram_torch, mel_spectrogram_torch, spec_to_mel_torch
    # Deliberately use an explicit licensed corpus, not arbitrary user recordings.
    files = sorted((args.assets / "LibriSpeech/dev-clean").rglob("*.flac"))
    splits = split(files, args.speaker, args.clips, args.source_clips or args.clips)
    if getattr(args, 'target_dataset', None):
        imported = json.loads(args.target_dataset.read_text(encoding='utf-8'))
        if imported.get('schema') != 1:
            raise ValueError('Unsupported target dataset')
        for role in ('target_train', 'target_validation', 'target_test'):
            paths = []
            for row in imported[role]:
                path = args.target_dataset.parent / row['path']
                if not path.resolve().is_relative_to(args.target_dataset.parent.resolve()) or sha256(path) != row['sha256']:
                    raise ValueError('Target dataset changed or has an unsafe path')
                paths.append(path)
            if not paths:
                raise ValueError('Separate training, validation and test recordings are required')
            splits[role] = paths
    targets, sources = splits["target_train"], splits["source_train"]
    evaluation = splits["source_validation"] + splits["source_test"]
    if getattr(args, 'target_dataset', None):
        dataset_manifest = dict(schema=1, splits={role:[dict(path=str(p.resolve()), speaker='user' if role.startswith('target') else p.parent.parent.name,
                                      sha256=sha256(p)) for p in paths] for role, paths in splits.items()})
    else:
        dataset_manifest = manifest(splits, args.assets)
    atomic_json(args.out / "dataset.json", dataset_manifest)
    report["dataset_sha256"] = sha256(args.out / "dataset.json")
    report["recordings"] = dataset_manifest["splits"]
    recipe = dict(rvc_revision=RVC_REV, llvc_revision=LLVC_REV, backend=args.backend,
                  speaker=args.speaker, clip_seconds=args.clip_seconds,
                  recordings=report["recordings"], kind="feasibility-fp32-v3-stage-budgets")
    recipe["content_objective_source"] = ASSETS["hubert_base_ls960.pt"][2]
    recipe["content_export_sha256"] = sha256(args.content_extractor) if args.content_extractor else None
    recipe["optimizer"] = dict(kind="AdamW", learning_rate=1e-4, betas=[0.8, 0.99], eps=1e-9,
                               scheduler_gamma=0.999875, scheduler_interval=args.checkpoint_every)
    recipe["teacher_mode"] = args.teacher_mode
    recipe["llvc_context_samples"] = args.llvc_context_samples
    recipe["discriminator_warmup_steps"] = args.discriminator_warmup_steps
    recipe["pitch_extractor"] = "librosa-pyin-50-1100Hz-hop160" if args.teacher_mode == "f0" else None
    report["recipe"] = recipe
    hubert = HubertModelWithFinalProj.from_pretrained(str(args.assets / "hubert_base"), local_files_only=True,
                                                     attn_implementation="eager").to(args.device).eval()
    hubert.requires_grad_(False)
    report["stage"] = "teacher_feature_preparation"
    atomic_json(args.out / "report.json", report)
    examples = []
    for path in targets + sources + evaluation:
        if (args.out / 'pause.request').exists():
            raise InterruptedError('Paused feature preparation')
        samples, rate = sf.read(path, dtype="float32")
        if rate != 16000 or samples.ndim != 1:
            raise ValueError("Unexpected source corpus audio contract")
        original = torch.from_numpy(samples[:int(args.clip_seconds * rate)]).to(args.device)
        with torch.no_grad():
            features = hubert.final_proj(hubert(original[None], output_hidden_states=True).hidden_states[9])
            phone = F.interpolate(features.transpose(1, 2), scale_factor=2, mode="nearest").transpose(1, 2)
            wave = torchaudio.functional.resample(original, 16000, 32000)[None, None]
            spec = spectrogram_torch(wave[:, 0], 1024, 32000, 320, 1024)
            length = min(phone.shape[1], spec.shape[-1])
            pitch = None
            if args.teacher_mode == "f0":
                import librosa
                f0, _, _ = librosa.pyin(original.cpu().numpy(), fmin=50, fmax=1100,
                                       sr=16000, frame_length=1024, hop_length=160)
                f0 = np.nan_to_num(f0[:length], nan=0.0).astype(np.float32)
                f0 = np.pad(f0, (0, max(0, length - len(f0))))
                mel = 1127 * np.log1p(f0 / 700)
                low, high = 1127 * np.log1p(50 / 700), 1127 * np.log1p(1100 / 700)
                coarse = np.rint(np.clip((mel - low) * 254 / (high - low) + 1, 1, 255)).astype(np.int64)
                pitch = (torch.from_numpy(coarse)[None], torch.from_numpy(f0)[None])
            examples.append((original.cpu(), phone[:, :length].cpu(), spec[:, :, :length].cpu(), wave[:, :, :length * 320].cpu(), pitch))
    del hubert
    gc.collect()
    if args.device.type == "cuda":
        torch.cuda.empty_cache()
    config = json.loads((args.rvc / "configs/v1/32k.json").read_text())
    teacher_type = SynthesizerTrnMs256NSFsid if args.teacher_mode == "f0" else SynthesizerTrnMs256NSFsid_nono
    teacher = teacher_type(513, 40, **config["model"], is_half=False, sr=32000).to(args.device)
    discriminator = MultiPeriodDiscriminator(False).to(args.device)
    prefix = "f0" if args.teacher_mode == "f0" else ""
    teacher.load_state_dict(torch.load(args.assets / (prefix + "G32k.pth"), map_location="cpu", weights_only=True)["model"])
    discriminator.load_state_dict(torch.load(args.assets / (prefix + "D32k.pth"), map_location="cpu", weights_only=True)["model"])

    def teacher_step(step, optim_g, optim_d):
        _, phone, spec, wave, pitch = examples[step % len(targets)]
        phone, spec, wave = [v.to(args.device) for v in (phone, spec, wave)]
        lengths = torch.tensor([phone.shape[1]], device=args.device)
        pitch_args = [p.to(args.device) for p in pitch] if pitch is not None else []
        out, ids, _, mask, (_, z_p, m_p, logs_p, _, logs_q) = teacher(phone, lengths, *pitch_args, spec, lengths, torch.zeros(1, dtype=torch.long, device=args.device))
        real = commons.slice_segments(wave, ids * 320, 12800)
        mel = spec_to_mel_torch(spec, 1024, 80, 32000, 0.0, None)
        expected = commons.slice_segments(mel, ids, 40)
        actual = mel_spectrogram_torch(out[:, 0], 1024, 80, 32000, 320, 1024, 0.0, None)
        dr, dg, _, _ = discriminator(real, out.detach())
        d_loss = rvc_losses.discriminator_loss(dr, dg)[0]
        optim_d.zero_grad(set_to_none=True)
        d_loss.backward()
        check_gradients(discriminator)
        optim_d.step()
        dr, dg, fr, fg = discriminator(real, out)
        g_loss = rvc_losses.generator_loss(dg)[0] + rvc_losses.feature_loss(fr, fg) + F.l1_loss(expected, actual) * 45 + rvc_losses.kl_loss(z_p, logs_q, m_p, logs_p, mask)
        optim_g.zero_grad(set_to_none=True)
        g_loss.backward()
        check_gradients(teacher)
        optim_g.step()
        return dict(generator=float(g_loss.detach().cpu()), discriminator=float(d_loss.detach().cpu()))

    report["stage"] = "teacher_training"
    atomic_json(args.out / "report.json", report)
    if args.teacher_checkpoint:
        saved = torch.load(args.teacher_checkpoint, map_location="cpu", weights_only=True)
        validate_teacher(saved["recipe"], recipe)
        teacher.load_state_dict(saved["model"])
        teacher.eval()
        recipe["teacher_checkpoint_sha256"] = sha256(args.teacher_checkpoint)
        report["teacher"] = dict(reused_checkpoint_sha256=recipe["teacher_checkpoint_sha256"],
                                 optimizer_steps=saved["step"], quality_accepted=False)
        del saved
    else:
        train_loop(args, "teacher", teacher, discriminator, teacher_step, recipe, report)
    pairs, review_pairs = [], []
    review_dir = args.out / "review"
    review_dir.mkdir(exist_ok=True)
    with torch.no_grad():
        for index, (original, phone, _, _, pitch) in enumerate(examples[len(targets):]):
            if (args.out / 'pause.request').exists():
                raise InterruptedError('Paused pair preparation')
            phone = phone.to(args.device)
            pitch_args = [p.to(args.device) for p in pitch] if pitch is not None else []
            out = teacher.infer(phone, torch.tensor([phone.shape[1]], device=args.device), *pitch_args, torch.zeros(1, dtype=torch.long, device=args.device))[0]
            converted = torchaudio.functional.resample(out, 32000, 16000)[0, 0].cpu()
            size = min(original.numel(), converted.numel())
            if index < len(sources):
                pairs.append((original[:size], converted[:size]))
                sf.write(args.out / f"pair-{index}-original.wav", original[:size].numpy(), 16000, subtype="PCM_16")
                sf.write(args.out / f"pair-{index}-converted.wav", converted[:size].numpy(), 16000, subtype="PCM_16")
            else:
                review_pairs.append((original[:size], converted[:size]))
                review_index = index - len(sources)
                sf.write(review_dir / f"{review_index}-original.wav", original[:size].numpy(), 16000, subtype="PCM_16")
                sf.write(review_dir / f"{review_index}-teacher.wav", converted[:size].numpy(), 16000, subtype="PCM_16")
    report["pairs"] = dict(count=len(pairs), duration_seconds=sum(x.numel() / 16000 for x, _ in pairs),
                          target_recording_seconds=sum(x[0].numel() / 16000 for x in examples[:len(targets)]),
                          source_speakers=len({p.parent.parent.name for p in sources}), held_out_target_speaker=True)
    for index, path in enumerate(splits["target_test"][:3]):
        samples, rate = sf.read(path, dtype="float32")
        sf.write(review_dir / f"target-reference-{index}.wav", samples[:int(args.clip_seconds * rate)], rate)
    if args.teacher_only:
        report["stage"] = "teacher_listening_pending"
        report["quality_accepted"] = False
        return
    del teacher, discriminator
    gc.collect()
    if args.device.type == "cuda":
        torch.cuda.empty_cache()
    sys.path.insert(0, str(args.llvc.resolve()))
    from model import Net
    from discriminators import MultiPeriodDiscriminator as LlvcDiscriminator
    import auraloss
    config = json.loads((args.llvc / "experiments/llvc/config.json").read_text())
    net = Net(**config["model_params"]).to(args.device)
    if sha256(args.initial_llvc) != "cceb7ab9621f84d62d283725ae3281cacb04f762d6a088fe3e97c1a29d4b8c0e":
        raise ValueError("LLVC initialization is not the pinned pretrained checkpoint")
    net.load_state_dict(torch.load(args.initial_llvc, map_location="cpu", weights_only=True)["model"])
    disc = LlvcDiscriminator(periods=config["periods"]).to(args.device)
    # The original content loss uses ONLY HuBERT's convolutional extractor.
    # Load the original pinned checkpoint through torchaudio's fairseq importer;
    # do not replace this objective with a different embedding or drop it.
    report["stage"] = "llvc_content_extractor_loading"
    atomic_json(args.out / "report.json", report)
    # Importing the complete fairseq model requires a compatible prebuilt wheel.
    # This is intentionally a hard feasibility gate, not an optional loss.
    if args.content_extractor:
        metadata = json.loads(args.content_extractor.with_suffix(".json").read_text())
        if (metadata["model_sha256"] != sha256(args.content_extractor)
                or metadata["source_sha256"] != ASSETS["hubert_base_ls960.pt"][2]
                or not metadata["checks"] or not all(r["passed"] for r in metadata["checks"])):
            raise ValueError("Invalid build-time content extractor export")
        with args.content_extractor.open("rb") as stream:
            extractor = torch.jit.load(stream, map_location=args.device).eval()
        for parameter in extractor.parameters():
            parameter.requires_grad_(False)
    else:
        import fairseq
        model, _, _ = fairseq.checkpoint_utils.load_model_ensemble_and_task([str(args.assets / "hubert_base_ls960.pt")])
        extractor = model[0].feature_extractor.to(args.device).eval().requires_grad_(False)
        del model
    multires = auraloss.freq.MultiResolutionSTFTLoss(fft_sizes=[1024, 2048, 512], hop_sizes=[120, 240, 50],
                win_lengths=[600, 1200, 240], scale="mel", n_bins=80, sample_rate=16000, device=args.device)
    from discriminators import discriminator_loss, generator_loss, feature_loss

    def discriminator_step(step, optimizer):
        original, target = training_context(*pairs[step % len(pairs)], args.llvc_context_samples)
        original, target = original.to(args.device), target.to(args.device)
        with torch.no_grad():
            fake = net(original)
        if fake.shape[-1] > 10240:
            start = random.randint(0, fake.shape[-1] - 10240)
            target, fake = target[:, :, start:start+10240], fake[:, :, start:start+10240]
        real_scores, fake_scores, _, _ = disc(target, fake)
        loss = discriminator_loss(real_scores, fake_scores)[0]
        optimizer.zero_grad(set_to_none=True)
        loss.backward()
        check_gradients(disc)
        optimizer.step()
        return dict(discriminator=float(loss.detach().cpu()))

    warm_discriminator(args, net, disc, discriminator_step, recipe, report)

    def llvc_step(step, optim_g, optim_d):
        original, target = pairs[step % len(pairs)]
        original, target = training_context(original, target, args.llvc_context_samples)
        original, target = original.to(args.device), target.to(args.device)
        out = net(original)
        # Match upstream's long generator context with a short discriminator
        # update. Generator losses below still see the complete contextual output.
        if args.llvc_context_samples > 10240:
            start = random.randint(0, out.shape[-1] - 10240)
            real, fake = target[:, :, start:start + 10240], out.detach()[:, :, start:start + 10240]
        else:
            real, fake = target, out.detach()
        dr, dg, _, _ = disc(real, fake)
        d_loss = discriminator_loss(dr, dg)[0]
        optim_d.zero_grad(set_to_none=True)
        d_loss.backward()
        check_gradients(disc)
        optim_d.step()
        dr, dg, fr, fg = disc(target, out)
        with torch.no_grad():
            expected = extractor(target[:, 0])
        content = F.mse_loss(extractor(out[:, 0]), expected) * 100000
        g_loss = generator_loss(dg)[0] + feature_loss(fr, fg) * 2 + multires(out, target) * 10 + content
        optim_g.zero_grad(set_to_none=True)
        g_loss.backward()
        check_gradients(net)
        torch.nn.utils.clip_grad_norm_(net.parameters(), 1.0)
        optim_g.step()
        return dict(generator=float(g_loss.detach().cpu()), discriminator=float(d_loss.detach().cpu()), content=float(content.detach().cpu()))

    report["stage"] = "llvc_training"
    atomic_json(args.out / "report.json", report)
    train_loop(args, "llvc", net, disc, llvc_step, recipe, report)
    review = []
    with torch.no_grad():
        for index, ((original, _), path) in enumerate(zip(review_pairs, evaluation)):
            converted = net(original[None, None].to(args.device))[0, 0].cpu()
            if not bool(torch.isfinite(converted).all()):
                raise RuntimeError("Non-finite held-out LLVC output")
            sf.write(review_dir / f"{index}-llvc-offline.wav", converted.numpy(), 16000, subtype="PCM_16")
            review.append(dict(index=index, source=str(path.relative_to(args.assets)),
                               split="validation" if index < 3 else "test",
                               intelligibility=None, identity=None, consonants=None, artifacts=None,
                               accepted=False))
    atomic_json(review_dir / "listening.json", dict(schema=1, target=args.speaker,
                dataset_sha256=report["dataset_sha256"], playback="monitor only",
                note="Offline diagnostic output; streaming qualification is separate. Loss is not a quality score.",
                reviews=review, accepted=False))
    checkpoint(args.out / "llvc-generator.pt", {"model": net.cpu().state_dict()})
    report["stage"] = "export_and_native_validation_pending"
    if args.device.type == "cuda":
        report["gpu_peak_allocated_bytes"] = torch.cuda.max_memory_allocated()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rvc", type=Path, required=True)
    parser.add_argument("--llvc", type=Path, required=True)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--initial-llvc", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--backend", choices=["cpu", "cuda", "xpu", "directml"], required=True)
    parser.add_argument("--steps", type=int, default=10)
    parser.add_argument("--teacher-steps", type=int, help="Independent teacher update budget; defaults to --steps")
    parser.add_argument("--llvc-steps", type=int, help="Independent LLVC update budget; defaults to --steps")
    parser.add_argument("--discriminator-warmup-steps", type=int, default=0,
                        help="Experimental discriminator-only updates before joint LLVC training")
    parser.add_argument("--llvc-context-samples", type=int, choices=[10240, 65536], default=10240,
                        help="Generator training context at 16 kHz; 65536 is the upstream-length experiment")
    parser.add_argument("--threads", type=int, default=2)
    parser.add_argument("--checkpoint-every", type=int, default=10)
    parser.add_argument("--clips", type=int, default=4)
    parser.add_argument("--source-clips", type=int, help="Separate count of multi-speaker paired-data clips")
    parser.add_argument("--clip-seconds", type=float, default=2.0)
    parser.add_argument("--speaker", default="1272")
    parser.add_argument('--target-dataset', type=Path, help='Application-owned normalized target dataset manifest')
    parser.add_argument("--teacher-mode", choices=["no-f0", "f0"], default="no-f0")
    parser.add_argument("--teacher-only", action="store_true", help="Stop for teacher listening review before LLVC training")
    parser.add_argument("--content-extractor", type=Path, help="Verified build-time export of original frozen content objective")
    parser.add_argument("--teacher-checkpoint", type=Path, help="Reuse a matching internal teacher for paired-data/LLVC experiments")
    parser.add_argument("--resume", action="store_true")
    args = parser.parse_args()
    if args.discriminator_warmup_steps < 0:
        parser.error('Warm-up steps cannot be negative')
    if any(value is not None and value < 1 for value in (args.teacher_steps, args.llvc_steps)):
        parser.error("Stage update budgets must be positive")
    if min(args.steps, args.threads, args.checkpoint_every, args.clips) < 1 or not 0.64 <= args.clip_seconds <= 10:
        parser.error("Positive counts and clip duration 0.64–10 seconds are required")
    if args.source_clips is not None and args.source_clips < 1:
        parser.error("Source clip count must be positive")
    args.out.mkdir(parents=True, exist_ok=True)
    report = dict(schema=1, backend=args.backend, public_training_enabled=False, release_qualified=False,
                  stage="asset_verification", started_utc=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()))
    report["execution"] = dict(python=sys.version, interpreter_sha256=sha256(Path(sys.executable)),
                               probe_sha256=sha256(Path(__file__)),
                               dataset_code_sha256=sha256(Path(__file__).with_name("dataset.py")),
                               seed=42, teacher_budget=stage_steps(args, "teacher"),
                               llvc_budget=stage_steps(args, "llvc"))
    began = time.perf_counter()
    try:
        run(args, report)
    except Exception as error:
        report.update(error=str(error), error_type=type(error).__name__, traceback=traceback.format_exc())
        print(report["traceback"], file=sys.stderr)
    finally:
        report["elapsed_seconds"] = time.perf_counter() - began
        atomic_json(args.out / "report.json", report)
    print(json.dumps({k: v for k, v in report.items() if k not in {"traceback", "recordings", "recipe", "teacher", "llvc", "warmup"}}), flush=True)
    return 1 if "error" in report else 0


if __name__ == "__main__":
    raise SystemExit(main())
