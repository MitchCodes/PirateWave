import unittest
import json
import tempfile
from pathlib import Path
from dataset import split
from assets import sha256
from probe import verify_source, validate_teacher


class SplitTests(unittest.TestCase):
    def test_teacher_reuse_allows_new_sources_but_rejects_changed_target_or_mode(self):
        import copy
        recipe = dict(rvc_revision="pin", speaker="1", clip_seconds=8, teacher_mode="no-f0",
                      optimizer={"lr": 1e-4}, recordings={"target_train": ["hash"], "source_train": ["a"]})
        changed = copy.deepcopy(recipe)
        changed["recordings"]["source_train"] = ["b"]
        validate_teacher(recipe, changed)
        changed["teacher_mode"] = "f0"
        with self.assertRaises(ValueError):
            validate_teacher(recipe, changed)
        changed = copy.deepcopy(recipe)
        changed["recordings"]["target_train"] = ["different hash"]
        with self.assertRaises(ValueError):
            validate_teacher(recipe, changed)

    def test_split_is_reproducible_and_speaker_disjoint(self):
        files = [Path(f"corpus/{s}/chapter/{i}.flac") for s in range(10) for i in range(12)]
        a = split(files, "0", 20, 100)
        self.assertEqual(a, split(list(reversed(files)), "0", 20, 100))
        roles = [set(p.parent.parent.name for p in a[r]) for r in
                 ("source_train", "source_validation", "source_test")]
        for i, left in enumerate(roles):
            self.assertNotIn("0", left)
            for right in roles[i + 1:]:
                self.assertFalse(left & right)
        targets = [set(a[r]) for r in ("target_train", "target_validation", "target_test")]
        self.assertFalse(targets[0] & (targets[1] | targets[2]))
        self.assertFalse(targets[1] & targets[2])
        self.assertEqual(len(roles[2]), 3)

    def test_small_corpus_cannot_claim_held_out_validation(self):
        with self.assertRaises(ValueError):
            split([Path("corpus/0/chapter/a.flac")], "0", 1, 1)

    def test_packaged_sources_verify_without_git_and_detect_tampering(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            module = root / "model.py"
            module.write_text("original")
            (root / "patch-source.json").write_text(json.dumps({"revision": "pinned", "files": {"model.py": sha256(module)}}))
            verify_source(root, "pinned")
            with self.assertRaises(ValueError):
                verify_source(root, "different")
            module.write_text("modified")
            with self.assertRaises(ValueError):
                verify_source(root, "pinned")

    def test_packaged_source_manifest_rejects_path_escape(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "source"
            root.mkdir()
            external = root.parent / "outside.py"
            external.write_text("outside")
            (root / "patch-source.json").write_text(json.dumps({"revision": "pinned", "files": {"../outside.py": sha256(external)}}))
            with self.assertRaises(ValueError):
                verify_source(root, "pinned")
