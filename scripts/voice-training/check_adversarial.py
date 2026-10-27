"""Read-only output-gradient diagnostics; no optimizer updates or quality scores."""
import argparse
import json
from pathlib import Path
import sys

import numpy as np
import soundfile as sf
import torch

from assets import LLVC_REV, sha256
from probe import atomic_json, verify_source


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--root', type=Path, default=Path.cwd())
    p.add_argument('--out', type=Path, required=True)
    args = p.parse_args()
    root = args.root.resolve()
    upstream = root / '.tools/voice-feasibility/LLVC'
    verify_source(upstream, LLVC_REV)
    sys.path.insert(0, str(upstream))
    from model import Net
    from discriminators import MultiPeriodDiscriminator, generator_loss, feature_loss
    import auraloss
    torch.set_num_threads(2)
    torch.manual_seed(42)
    if not torch.cuda.is_available():
        raise RuntimeError('This diagnostic explicitly requires CUDA')
    device = torch.device('cuda')
    config = json.loads((upstream / 'experiments/llvc/config.json').read_text())
    candidate = root / 'dist/voice-training/llvc-duration/1462-1000'
    initial = root / '.tools/voice-feasibility/llvc.pth'
    extractor_path = root / '.tools/voice-training/assets/content-extractor.pt'
    metadata = json.loads(extractor_path.with_suffix('.json').read_text())
    if sha256(extractor_path) != metadata['model_sha256']:
        raise ValueError('Content extractor hash mismatch')
    with extractor_path.open('rb') as f:
        extractor = torch.jit.load(f, map_location=device).eval()
    for parameter in extractor.parameters():
        parameter.requires_grad_(False)
    spectral = auraloss.freq.MultiResolutionSTFTLoss(fft_sizes=[1024,2048,512], hop_sizes=[120,240,50],
                 win_lengths=[600,1200,240], scale='mel', n_bins=80, sample_rate=16000, device=device)
    trained = torch.load(candidate / 'llvc.checkpoint.pt', map_location='cpu', weights_only=True)
    rows = []
    for generator_name, state in [('pretrained', torch.load(initial, map_location='cpu', weights_only=True)['model']),
                                  ('adapted_1000', trained['model'])]:
        net = Net(**config['model_params']).to(device).eval()
        net.load_state_dict(state)
        for discriminator_name in ('fresh_seed42', 'adapted_1000'):
            torch.manual_seed(42)
            disc = MultiPeriodDiscriminator(periods=config['periods']).to(device).eval()
            if discriminator_name == 'adapted_1000':
                disc.load_state_dict(trained['discriminator'])
            disc.requires_grad_(False)
            for index in (3,4,5):
                original, rate = sf.read(candidate / f'review/{index}-original.wav', dtype='float32')
                target, target_rate = sf.read(candidate / f'review/{index}-teacher.wav', dtype='float32')
                if rate != 16000 or target_rate != rate or len(original) != len(target):
                    raise ValueError('Unexpected pair contract')
                start = max(0, (len(original)-10240)//2)
                x = torch.tensor(original[start:start+10240], device=device)[None,None]
                y = torch.tensor(target[start:start+10240], device=device)[None,None]
                with torch.no_grad():
                    target_features = extractor(y[:,0])
                out = net(x)
                real, fake, real_features, fake_features = disc(y, out)
                losses = dict(adversarial=generator_loss(fake)[0],
                              feature_matching=feature_loss(real_features, fake_features)*2,
                              spectral=spectral(out,y)*10,
                              content=torch.nn.functional.mse_loss(extractor(out[:,0]), target_features)*100000)
                gradients = {name: torch.autograd.grad(loss, out, retain_graph=True)[0].detach().flatten()
                             for name, loss in losses.items()}
                parameters = tuple(net.parameters())
                parameter_gradients = {}
                for name, gradient in gradients.items():
                    values = torch.autograd.grad(out, parameters, grad_outputs=gradient.reshape_as(out),
                                                 retain_graph=True, allow_unused=True)
                    parameter_gradients[name] = torch.cat([(g if g is not None else torch.zeros_like(p)).detach().flatten()
                                                           for p, g in zip(parameters, values)])
                supervised = gradients['spectral'] + gradients['content']
                gan = gradients['adversarial'] + gradients['feature_matching']
                norm = lambda t: float(torch.linalg.vector_norm(t).cpu())
                cosine = float(torch.nn.functional.cosine_similarity(supervised[None], gan[None]).cpu())
                parameter_supervised = parameter_gradients['spectral'] + parameter_gradients['content']
                parameter_gan = parameter_gradients['adversarial'] + parameter_gradients['feature_matching']
                if not all(bool(torch.isfinite(g).all()) for g in [*gradients.values(), *parameter_gradients.values()]):
                    raise ValueError('Non-finite output gradient')
                rows.append(dict(generator=generator_name, discriminator=discriminator_name, source_index=index,
                    segment_start_sample=start, losses={k:float(v.detach().cpu()) for k,v in losses.items()},
                    output_gradient_l2={k:norm(v) for k,v in gradients.items()},
                    parameter_gradient_l2={k:norm(v) for k,v in parameter_gradients.items()},
                    parameter_gan_to_supervised_norm=norm(parameter_gan)/max(norm(parameter_supervised),1e-20),
                    parameter_gan_supervised_cosine=float(torch.nn.functional.cosine_similarity(parameter_supervised[None],parameter_gan[None]).cpu()),
                    gan_to_supervised_norm=norm(gan)/max(norm(supervised),1e-20),
                    gan_supervised_cosine=cosine,
                    real_score_mean=float(torch.cat(real,dim=1).mean().detach().cpu()),
                    generated_score_mean=float(torch.cat(fake,dim=1).mean().detach().cpu())))
            del disc
        del net
    args.out.parent.mkdir(parents=True, exist_ok=True)
    summary = []
    for generator in ('pretrained','adapted_1000'):
        for discriminator in ('fresh_seed42','adapted_1000'):
            selected=[r for r in rows if r['generator']==generator and r['discriminator']==discriminator]
            summary.append(dict(generator=generator, discriminator=discriminator,
                median_output_gan_ratio=float(np.median([r['gan_to_supervised_norm'] for r in selected])),
                median_parameter_gan_ratio=float(np.median([r['parameter_gan_to_supervised_norm'] for r in selected]))))
    atomic_json(args.out, dict(schema=1, release_qualified=False,
       note='Evaluation-mode output and parameter gradients on three held-out center segments, before clipping or Adam preconditioning. Fresh discriminator is independently seeded, not reconstruction of original initialization. Ratios/cosines do not prove perceptual causation.',
       trained_checkpoint_sha256=sha256(candidate / 'llvc.checkpoint.pt'),
       initial_checkpoint_sha256=sha256(initial), summary=summary, rows=rows))
    for generator in ('pretrained','adapted_1000'):
        for discriminator in ('fresh_seed42','adapted_1000'):
            selected=[r for r in rows if r['generator']==generator and r['discriminator']==discriminator]
            print(json.dumps(dict(generator=generator,discriminator=discriminator,
                 median_gan_ratio=float(np.median([r['gan_to_supervised_norm'] for r in selected])),
                 median_parameter_gan_ratio=float(np.median([r['parameter_gan_to_supervised_norm'] for r in selected])),
                 median_cosine=float(np.median([r['gan_supervised_cosine'] for r in selected])))))


if __name__ == '__main__':
    main()
