import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import zipfile
from assets import sha256
from verify_bundle import verify


class BundleTests(unittest.TestCase):
    def build(self, root, extra=False, corrupt=False):
        path = root / "candidate.zip"
        payload = b"runtime"
        manifest = json.dumps(dict(schema=1, backend="cpu", files={"worker.py": dict(
            bytes=len(payload), sha256=hashlib.sha256(payload).hexdigest())})).encode()
        with zipfile.ZipFile(path, "w") as archive:
            archive.writestr("runtime-manifest.json", manifest)
            archive.writestr("worker.py", b"changed" if corrupt else payload)
            if extra:
                archive.writestr("../escape", b"x")
        return path, dict(backend="cpu", bytes=path.stat().st_size, sha256=sha256(path),
                          installed_bytes=len(manifest) + len(payload))

    def test_valid_bundle_and_storage(self):
        with tempfile.TemporaryDirectory() as directory:
            path, catalog = self.build(Path(directory))
            self.assertTrue(verify(path, catalog)["integrity_pass"])
            catalog["installed_bytes"] += 1
            with self.assertRaisesRegex(ValueError, "storage"):
                verify(path, catalog)

    def test_internal_corruption_and_path_escape(self):
        for extra, corrupt in ((True, False), (False, True)):
            with tempfile.TemporaryDirectory() as directory:
                path, catalog = self.build(Path(directory), extra, corrupt)
                with self.assertRaises(ValueError):
                    verify(path, catalog)
