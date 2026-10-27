"""Regression checks for the feasibility worker's integrity and resume boundary."""
import hashlib
import io
import random
import tempfile
import sys
import unittest
from pathlib import Path
from unittest.mock import patch
from types import SimpleNamespace

import assets
import probe
import supervise


class Response(io.BytesIO):
    def __init__(self, content, status=200, headers=None):
        super().__init__(content)
        self.status = status
        self.headers = headers or {}


class DownloadTests(unittest.TestCase):
    def test_resume_and_servers_that_ignore_range_both_produce_verified_file(self):
        payload = b"verified training model bytes"
        for honored in (True, False):
            with self.subTest(honored=honored), tempfile.TemporaryDirectory() as directory:
                destination = Path(directory) / "model.bin"
                destination.with_suffix(".bin.part").write_bytes(payload[:8])
                response = Response(payload[8:] if honored else payload, 206 if honored else 200,
                                    {"Content-Range": f"bytes 8-{len(payload)-1}/{len(payload)}"})
                with patch.object(assets.urllib.request, "urlopen", return_value=response):
                    assets.download("https://example.com/model", destination, len(payload), hashlib.sha256(payload).hexdigest())
                self.assertEqual(destination.read_bytes(), payload)

    def test_corruption_does_not_replace_previous_install(self):
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / "model.bin"
            destination.write_bytes(b"old")
            with patch.object(assets.urllib.request, "urlopen", return_value=Response(b"bad")):
                with self.assertRaises(ValueError):
                    assets.download("https://example.com/model", destination, 3, hashlib.sha256(b"new").hexdigest())
            self.assertEqual(destination.read_bytes(), b"old")

    def test_wrong_resume_offset_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / "model.bin"
            destination.with_suffix(".bin.part").write_bytes(b"ab")
            with patch.object(assets.urllib.request, "urlopen", return_value=Response(b"cd", 206, {"Content-Range": "bytes 0-1/4"})):
                with self.assertRaisesRegex(ValueError, "resume range"):
                    assets.download("https://example.com/model", destination, 4, hashlib.sha256(b"abcd").hexdigest())
            self.assertFalse(destination.exists())


class CheckpointTests(unittest.TestCase):
    def test_interrupted_warmup_resumes_to_same_discriminator(self):
        import torch
        import numpy as np
        def run(folder, resume, interrupt):
            torch.manual_seed(4)
            g, d = torch.nn.Linear(2,2), torch.nn.Linear(2,1)
            args = SimpleNamespace(discriminator_warmup_steps=6, resume=resume, out=folder,
                                   device=torch.device('cpu'), checkpoint_every=2)
            random.seed(17)
            np.random.seed(17)
            torch.manual_seed(17)
            def step(index, optimizer):
                with torch.no_grad():
                    fake = g(torch.rand(1,2)*random.random()*np.random.rand())
                loss = d(fake).square().sum()
                optimizer.zero_grad()
                loss.backward()
                optimizer.step()
                if interrupt and index == 2:
                    (folder / 'pause.request').touch()
                return {'loss':float(loss.detach())}
            probe.warm_discriminator(args,g,d,step,{'recipe':'same'}, {})
            return {k:v.clone() for k,v in d.state_dict().items()}
        with tempfile.TemporaryDirectory() as directory:
            full, interrupted = Path(directory)/'full', Path(directory)/'interrupted'
            full.mkdir()
            interrupted.mkdir()
            expected = run(full,False,False)
            with self.assertRaises(InterruptedError):
                run(interrupted,False,True)
            (interrupted/'pause.request').unlink()
            actual = run(interrupted,True,False)
            for k in expected:
                self.assertTrue(torch.equal(actual[k],expected[k]))

    def test_discriminator_warmup_freezes_generator_and_restores_joint_random_stream(self):
        import numpy as np
        import torch
        with tempfile.TemporaryDirectory() as directory:
            g, d = torch.nn.Linear(2,2), torch.nn.Linear(2,1)
            before_g = {k:v.clone() for k,v in g.state_dict().items()}
            before_d = d.weight.detach().clone()
            args = SimpleNamespace(discriminator_warmup_steps=3, resume=False, out=Path(directory),
                                   device=torch.device('cpu'), checkpoint_every=2)
            random.seed(17)
            np.random.seed(17)
            torch.manual_seed(17)
            expected = random.random(), np.random.rand(), torch.rand(2)
            random.seed(17)
            np.random.seed(17)
            torch.manual_seed(17)
            def step(index, optimizer):
                with torch.no_grad():
                    fake = g(torch.rand(1,2)*random.random()*np.random.rand())
                loss = d(fake).square().sum()
                optimizer.zero_grad()
                loss.backward()
                optimizer.step()
                return {'loss':float(loss.detach())}
            report = {}
            probe.warm_discriminator(args,g,d,step,{'recipe':'warm-test'},report)
            self.assertTrue(report['warmup']['generator_unchanged'])
            for k,v in g.state_dict().items():
                self.assertTrue(torch.equal(v,before_g[k]))
            self.assertFalse(torch.equal(d.weight,before_d))
            self.assertEqual(random.random(),expected[0])
            self.assertEqual(np.random.rand(),expected[1])
            self.assertTrue(torch.equal(torch.rand(2),expected[2]))
            self.assertTrue(all(p.requires_grad for p in g.parameters()))

    def test_long_context_preserves_pair_alignment_and_pads_short_clips(self):
        import torch
        source = torch.arange(20000, dtype=torch.float32)
        x, y = probe.training_context(source, source + 7, 65536)
        self.assertEqual(tuple(x.shape), (1, 1, 65536))
        self.assertTrue(torch.equal(x[0, 0, :20000], source))
        self.assertTrue(torch.equal(y[0, 0, :20000], source + 7))
        self.assertEqual(torch.count_nonzero(x[0, 0, 20000:]).item(), 0)
        self.assertEqual(torch.count_nonzero(y[0, 0, 20000:]).item(), 0)
        x, y = probe.training_context(torch.arange(80000), torch.arange(80000) + 7, 65536)
        self.assertTrue(torch.equal(y - x, torch.full_like(x, 7)))
        self.assertEqual(x.numel(), 65536)
        with self.assertRaises(ValueError):
            probe.training_context(torch.zeros(10), torch.zeros(9), 65536)

    def test_stage_budgets_are_independent_with_legacy_fallback(self):
        args = SimpleNamespace(steps=10, teacher_steps=1000, llvc_steps=5000)
        self.assertEqual(probe.stage_steps(args, 'teacher'), 1000)
        self.assertEqual(probe.stage_steps(args, 'llvc'), 5000)
        self.assertEqual(probe.stage_steps(SimpleNamespace(steps=7), 'llvc'), 7)

    def test_stopping_between_scheduler_boundaries_matches_uninterrupted_training(self):
        import torch
        def run(folder, budget, resume):
            torch.manual_seed(42)
            random.seed(42)
            g, d = torch.nn.Linear(2, 2), torch.nn.Linear(2, 2)
            def loss_step(step, og, od):
                for model, optimizer in ((g, og), (d, od)):
                    optimizer.zero_grad()
                    model(torch.rand(1, 2) * random.random()).square().sum().backward()
                    optimizer.step()
                return {'step': step}
            args = SimpleNamespace(steps=budget, checkpoint_every=2, out=folder,
                                   device=torch.device('cpu'), resume=resume)
            probe.train_loop(args, 'llvc', g, d, loss_step, {'recipe': 'fixed'}, {})
            return torch.load(folder / 'llvc.checkpoint.pt', weights_only=True)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            full, resumed = root / 'full', root / 'resumed'
            full.mkdir()
            resumed.mkdir()
            expected = run(full, 6, False)
            run(resumed, 3, False)
            actual = run(resumed, 6, True)
            for group in ('model', 'discriminator'):
                for key in expected[group]:
                    self.assertTrue(torch.equal(expected[group][key], actual[group][key]))
            self.assertEqual(expected['schedulers'], actual['schedulers'])
            self.assertEqual(expected['python_rng'], actual['python_rng'])
            self.assertTrue(torch.equal(expected['torch_rng'], actual['torch_rng']))

    def test_native_abort_is_contained_and_reported_by_the_supervisor(self):
        with tempfile.TemporaryDirectory() as directory:
            result = supervise.run_worker([sys.executable, "-c", "import os; os._exit(7)"], Path(directory), 10)
            self.assertFalse(result["process_completed"])
            self.assertEqual(result["exit_code"], 7)
            self.assertTrue((Path(directory) / "supervisor.json").is_file())

    def test_resume_restores_model_optimizer_scheduler_and_random_stream(self):
        import numpy as np
        import torch
        torch.manual_seed(17)
        random.seed(17)
        g, d = torch.nn.Linear(2, 2), torch.nn.Linear(2, 2)
        optimizers = [torch.optim.AdamW(n.parameters()) for n in (g, d)]
        schedulers = [torch.optim.lr_scheduler.ExponentialLR(o, 0.9) for o in optimizers]
        for net, optimizer, scheduler in zip((g, d), optimizers, schedulers):
            net(torch.ones(1, 2)).sum().backward()
            optimizer.step()
            scheduler.step()
        recipe = {"corpus_sha256": "fixed", "backend": "cpu"}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "checkpoint.pt"
            probe.checkpoint(path, probe.optimizer_state(g, d, *optimizers, schedulers, 3, recipe))
            expected_g = {key: value.clone() for key, value in g.state_dict().items()}
            expected_random, expected_tensor = random.random(), torch.rand(3)
            expected_numpy = np.random.rand(3)
            with torch.no_grad():
                g.weight.add_(100)
            optimizers[0].param_groups[0]["lr"] = 0.5
            self.assertEqual(probe.restore(path, g, d, *optimizers, schedulers, recipe), 3)
            self.assertEqual(random.random(), expected_random)
            self.assertTrue(torch.equal(torch.rand(3), expected_tensor))
            self.assertTrue(np.array_equal(np.random.rand(3), expected_numpy))
            for key, value in g.state_dict().items():
                self.assertTrue(torch.equal(value, expected_g[key]))
            self.assertAlmostEqual(optimizers[0].param_groups[0]["lr"], 0.0009)
            self.assertEqual(schedulers[0].last_epoch, 1)
            self.assertTrue(optimizers[0].state)
            with self.assertRaisesRegex(ValueError, "recipe/dataset/backend"):
                probe.restore(path, g, d, *optimizers, schedulers, {"backend": "cuda"})

    def test_failed_atomic_save_keeps_the_previous_checkpoint(self):
        import torch
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "checkpoint.pt"
            probe.checkpoint(path, {"step": 5})
            probe.checkpoint(path, {"step": 6})
            self.assertEqual(torch.load(path.with_suffix('.pt.previous'), weights_only=True)['step'], 5)
            with patch.object(torch, "save", side_effect=OSError("disk full")):
                with self.assertRaises(OSError):
                    probe.checkpoint(path, {"step": 7})
            self.assertEqual(torch.load(path, weights_only=True)["step"], 6)
            self.assertEqual(torch.load(path.with_suffix('.pt.previous'), weights_only=True)['step'], 5)


if __name__ == "__main__":
    unittest.main()
