"""Deterministic, speaker-disjoint research splits; no audio is modified."""
import hashlib
import random
from pathlib import Path


def split(files, target, target_count, source_count, seed=42):
    files = sorted(Path(p) for p in files)
    groups = {}
    for path in files:
        groups.setdefault(path.parent.parent.name, []).append(path)
    if target not in groups or len(groups[target]) < 6 or len(groups) < 8:
        raise ValueError("Need six target clips and seven other speakers for independent splits")
    rng = random.Random(seed)
    target_files = list(groups[target])
    rng.shuffle(target_files)
    reserve = max(1, len(target_files) // 10)
    speakers = sorted(set(groups) - {target})
    rng.shuffle(speakers)
    validation, test = speakers[:3], speakers[3:6]
    sources = [p for speaker in speakers[6:] for p in groups[speaker]]
    rng.shuffle(sources)
    result = {
        "target_train": target_files[2 * reserve:][:target_count],
        "target_validation": target_files[:reserve],
        "target_test": target_files[reserve:2 * reserve],
        "source_train": sources[:source_count],
        "source_validation": [groups[s][0] for s in validation],
        "source_test": [groups[s][0] for s in test],
    }
    if len(result["target_train"]) < 2 or len(result["source_train"]) < 2:
        raise ValueError("Need at least two target and source training clips")
    return result


def manifest(splits, root):
    result = {}
    for role, paths in splits.items():
        result[role] = [{"path": p.relative_to(root).as_posix(),
                         "speaker": p.parent.parent.name,
                         "sha256": hashlib.sha256(p.read_bytes()).hexdigest()} for p in paths]
    return {"schema": 1, "splits": result}
