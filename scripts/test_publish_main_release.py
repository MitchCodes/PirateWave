import hashlib
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import urllib.error
import urllib.parse
import zipfile

spec = importlib.util.spec_from_file_location("publisher", Path(__file__).with_name("publish-main-release.py"))
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)


class FakeGitHub:
    def __init__(self):
        self.release = None
        self.assets = {}
        self.calls = []
        self.corrupt_upload = False

    def request(self, path, method="GET", body=None, raw=None):
        self.calls.append((path, method))
        if path.startswith("/releases/tags/"):
            if self.release is None:
                raise urllib.error.HTTPError(path, 404, "Not Found", {}, None)
            return self.release.copy()
        if path == "/releases" and method == "POST":
            self.release = dict(body, id=1, html_url="https://github.com/example/release", upload_url="https://uploads.github.com/repos/example/releases/1/assets{?name,label}")
            return self.release.copy()
        if path.endswith("/assets?per_page=100"):
            return list(self.assets.values())
        if method == "DELETE":
            key = next(name for name, asset in self.assets.items() if str(asset["id"]) == path.split("/")[-1])
            del self.assets[key]
            return None
        if path.startswith("https://uploads.github.com/"):
            name = urllib.parse.parse_qs(urllib.parse.urlsplit(path).query)["name"][0]
            asset = dict(id=len(self.calls), name=name, state="uploaded", digest="sha256:" + hashlib.sha256(raw).hexdigest())
            if self.corrupt_upload:
                asset["digest"] = "invalid"
            self.assets[name] = asset
            return asset
        if method == "PATCH":
            self.release.update(body)
            return self.release.copy()
        raise AssertionError((path, method))


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.folder = Path(self.temp.name)
        self.plugin = self.folder / "PirateWave.lplug4"
        with zipfile.ZipFile(self.plugin, "w") as archive:
            archive.writestr("bin/PirateWavePlugin.dll", b"fixture")
        self.streamdeck = self.folder / "PirateWave.streamDeckPlugin"
        with zipfile.ZipFile(self.streamdeck, "w") as archive:
            archive.writestr("manifest.json", b"fixture")
        self.app = self.folder / "PirateWave-v0.2.0-beta.5-windows-x64.zip"
        with zipfile.ZipFile(self.app, "w") as archive:
            archive.writestr("PirateWave.exe", b"fixture")
            archive.writestr("PirateWave.lplug4", self.plugin.read_bytes())
            archive.writestr("PirateWave.streamDeckPlugin", self.streamdeck.read_bytes())
        self.plan = publisher.release_plan(self.folder, "0.2.0-beta.5", "a" * 40, "42")

    def test_new_release_publishes_only_after_all_verified_uploads(self):
        api = FakeGitHub()
        publisher.publish(api, self.plan)
        self.assertEqual(set(api.assets), {self.app.name, self.plugin.name, self.streamdeck.name, "SHA256SUMS.txt"})
        self.assertFalse(api.release["draft"])
        self.assertTrue(api.release["prerelease"])
        self.assertEqual(api.calls[-1][1], "PATCH")

    def test_published_retry_never_replaces_assets(self):
        api = FakeGitHub()
        publisher.publish(api, self.plan)
        api.calls.clear()
        publisher.publish(api, self.plan)
        self.assertTrue(all(method == "GET" for _, method in api.calls))

    def test_partial_draft_recovers_and_replaces_only_draft_assets(self):
        api = FakeGitHub()
        api.corrupt_upload = True
        with self.assertRaisesRegex(ValueError, "digest/state"):
            publisher.publish(api, self.plan)
        self.assertTrue(api.release["draft"])
        api.corrupt_upload = False
        publisher.publish(api, self.plan)
        self.assertFalse(api.release["draft"])
        self.assertTrue(any(method == "DELETE" for _, method in api.calls))

    def test_existing_release_for_other_source_is_rejected(self):
        api = FakeGitHub()
        publisher.publish(api, self.plan)
        api.release["body"] = "Unrelated release"
        with self.assertRaisesRegex(ValueError, "different source"):
            publisher.publish(api, self.plan)

    def test_missing_or_mismatched_plugin_and_invalid_metadata_fail(self):
        with self.assertRaisesRegex(ValueError, "Invalid source"):
            publisher.release_plan(self.folder, "0.2.0-beta.5", "main", "42")
        self.plugin.write_bytes(b"different plugin")
        with self.assertRaisesRegex(ValueError, "plugin mismatch"):
            publisher.release_plan(self.folder, "0.2.0-beta.5", "a" * 40, "42")
        self.plugin.unlink()
        with self.assertRaisesRegex(ValueError, "Missing release asset"):
            publisher.release_plan(self.folder, "0.2.0-beta.5", "a" * 40, "42")

    def test_branch_guard_and_missing_secret_prevent_network_writes(self):
        argv = ["publisher", "--folder", str(self.folder), "--version", "0.2.0-beta.5", "--commit", "a" * 40, "--run", "42"]
        with patch("sys.argv", argv), patch.dict(os.environ, {"GITHUB_EVENT_NAME": "pull_request", "GITHUB_REF": "refs/heads/main"}, clear=True):
            with self.assertRaisesRegex(RuntimeError, "restricted"):
                publisher.main()

        with patch("sys.argv", argv), patch.dict(os.environ, {"GITHUB_EVENT_NAME": "push", "GITHUB_REF": "refs/heads/main"}, clear=True):
            with self.assertRaisesRegex(RuntimeError, "RELEASES_TOKEN"):
                publisher.main()

    def test_streamdeck_asset_must_be_present_and_identical_inside_portable_zip(self):
        self.streamdeck.write_bytes(b"different Stream Deck plugin")
        with self.assertRaisesRegex(ValueError, "plugin mismatch"):
            publisher.release_plan(self.folder, "0.2.0-beta.5", "a" * 40, "42")
        self.streamdeck.unlink()
        with self.assertRaisesRegex(ValueError, "Missing release asset"):
            publisher.release_plan(self.folder, "0.2.0-beta.5", "a" * 40, "42")


if __name__ == "__main__":
    unittest.main()
