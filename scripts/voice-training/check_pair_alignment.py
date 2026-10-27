"""Read-only timing diagnostics for pitch-changing training pairs.

Envelope correlation estimates timing, not voice quality or sample alignment.
Do not shift training data automatically from these estimates.
"""
import argparse
import json
from pathlib import Path

import numpy as np
import soundfile as sf
from scipy.signal import stft

from assets import sha256
from probe import atomic_json


def features(audio):
    _, _, spectrum = stft(audio, fs=16000, nperseg=512, noverlap=432,
                          boundary=None, padded=False)
    power = np.abs(spectrum) ** 2
    frequency = np.fft.rfftfreq(512, 1/16000)
    bands = [(80, 400), (400, 1500), (1500, 6000)]
    energy = [np.log1p(1000 * np.sqrt(power[(frequency >= lo) & (frequency < hi)].sum(axis=0)))
              for lo, hi in bands]
    return np.asarray(energy)


def lag_estimate(source, converted, max_lag=40):
    # Positive lag means converted audio occurs later than source audio.
    scores = []
    for lag in range(-max_lag, max_lag + 1):
        start = max(0, -lag)
        end = min(source.shape[1], converted.shape[1] - lag)
        if end - start < 40:
            scores.append(float('-inf'))
            continue
        a, b = source[:, start:end].copy(), converted[:, start+lag:end+lag].copy()
        a -= a.mean(axis=1, keepdims=True)
        b -= b.mean(axis=1, keepdims=True)
        denominator = np.sqrt((a*a).sum(axis=1) * (b*b).sum(axis=1))
        valid = denominator > 1e-12
        scores.append(float(((a*b).sum(axis=1)[valid] / denominator[valid]).mean()) if valid.any() else float('-inf'))
    if not np.isfinite(scores).any():
        return dict(usable=False)
    best = int(np.argmax(scores))
    zero = scores[max_lag]
    return dict(usable=True, converted_delay_ms=(best-max_lag)*5,
                peak_correlation=scores[best], zero_lag_correlation=zero,
                improvement=scores[best]-zero, search_boundary=best in (0, len(scores)-1))


def analyze(original, converted):
    a, rate_a = sf.read(original, dtype='float32')
    b, rate_b = sf.read(converted, dtype='float32')
    if rate_a != 16000 or rate_b != 16000 or a.ndim != 1 or b.ndim != 1:
        raise ValueError('Expected mono 16 kHz pairs')
    if not np.isfinite(a).all() or not np.isfinite(b).all():
        raise ValueError('Non-finite audio')
    af, bf = features(a), features(b)
    windows = []
    for start in range(0, min(af.shape[1], bf.shape[1])-200+1, 200):
        windows.append(dict(start_seconds=start*.005,
                            **lag_estimate(af[:, start:start+200], bf[:, start:start+200])))
    return dict(original_sha256=sha256(original), converted_sha256=sha256(converted),
                original_samples=len(a), converted_samples=len(b),
                original_peak=float(np.abs(a).max()), converted_peak=float(np.abs(b).max()),
                converted_clipping_fraction=float(np.mean(np.abs(b) >= 32767/32768)),
                full_clip=lag_estimate(af, bf), windows=windows)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--candidate', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    args = p.parse_args()
    rows = []
    for path in sorted(args.candidate.glob('pair-*-original.wav')):
        rows.append(dict(name=path.stem, role='training',
                         **analyze(path, path.with_name(path.name.replace('-original', '-converted')))))
    for index in range(6):
        rows.append(dict(name=f'review-{index}', role='held-out',
                         **analyze(args.candidate / f'review/{index}-original.wav',
                                   args.candidate / f'review/{index}-teacher.wav')))
    reliable = [r['full_clip'] for r in rows if r['full_clip']['usable']
                and r['full_clip']['peak_correlation'] >= .7 and not r['full_clip']['search_boundary']]
    result = dict(schema=1, method='Three-band log-energy envelope correlation, 5 ms resolution, +/-200 ms search',
                  interpretation='Heuristic timing estimates, not phoneme alignment or proof of a training defect. No audio modified.',
                  release_qualified=False, rows=rows,
                  summary=dict(pairs=len(rows), equal_lengths=all(r['original_samples']==r['converted_samples'] for r in rows),
                    high_correlation_pairs=len(reliable),
                    median_delay_ms=float(np.median([r['converted_delay_ms'] for r in reliable])) if reliable else None,
                    p10_p90_delay_ms=np.percentile([r['converted_delay_ms'] for r in reliable], [10,90]).tolist() if reliable else None,
                    max_clipping_fraction=max(r['converted_clipping_fraction'] for r in rows)))
    args.out.parent.mkdir(parents=True, exist_ok=True)
    atomic_json(args.out, result)
    print(json.dumps(result['summary']))


if __name__ == '__main__':
    main()
