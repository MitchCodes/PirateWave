import io
import json
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
from package_voice_training import package, validate_training_archive

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
            if isinstance(raw, Path):
                raw = raw.read_bytes()
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

        cache = self.folder / "cache"
        cache.mkdir()
        self.cache = cache
        bundles = []
        for backend in ["cpu", "shared"]:
            data = backend.encode()
            name = backend + ".part001"
            (cache / name).write_bytes(data)
            bundles.append(dict(backend=backend, parts=[dict(name=name, bytes=len(data), sha256=hashlib.sha256(data).hexdigest(), url="https://example.invalid/" + name)]))
        self.catalog = self.folder / "catalog.json"
        self.catalog.write_text(json.dumps(dict(bundles=bundles)))
        self.training = package(self.folder, self.catalog, cache)
        validator = patch.object(publisher, "validate_training_archive", lambda path: validate_training_archive(path, self.catalog))
        validator.start()
        self.addCleanup(validator.stop)
        self.plan = publisher.release_plan(self.folder, "0.2.0-beta.5", "a" * 40, "42")

    def test_new_release_publishes_only_after_all_verified_uploads(self):
        api = FakeGitHub()
        publisher.publish(api, self.plan)
        self.assertEqual(set(api.assets), {self.app.name, self.plugin.name, self.streamdeck.name, self.training.name, "SHA256SUMS.txt"})
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
        with self.assertRaises(zipfile.BadZipFile):
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
            with self.assertRaisesRegex(RuntimeError, "GITHUB_TOKEN"):
                publisher.main()

    def test_streamdeck_asset_must_be_present_and_valid(self):
        self.streamdeck.write_bytes(b"different Stream Deck plugin")
        with self.assertRaises(zipfile.BadZipFile):
            publisher.release_plan(self.folder, "0.2.0-beta.5", "a" * 40, "42")
        self.streamdeck.unlink()
        with self.assertRaisesRegex(ValueError, "Missing release asset"):
            publisher.release_plan(self.folder, "0.2.0-beta.5", "a" * 40, "42")

    def test_training_bundle_is_required_and_corruption_rejected(self):
        with zipfile.ZipFile(self.training, "a") as archive:
            archive.writestr("unexpected.bin", b"bad")
        with self.assertRaisesRegex(ValueError, "Unexpected optional"):
            publisher.release_plan(self.folder, "0.2.0-beta.5", "a" * 40, "42")
        self.training.unlink()
        with self.assertRaisesRegex(ValueError, "Missing release asset"):
            publisher.release_plan(self.folder, "0.2.0-beta.5", "a" * 40, "42")

    def test_optional_packages_cannot_be_embedded_in_main_download(self):
        with zipfile.ZipFile(self.app, "a") as archive:
            archive.writestr(self.plugin.name, self.plugin.read_bytes())
        with self.assertRaisesRegex(ValueError, "Optional download embedded"):
            publisher.release_plan(self.folder, "0.2.0-beta.5", "a" * 40, "42")

    def test_publication_targets_tested_commit_and_has_four_downloads(self):
        api = FakeGitHub()
        publisher.publish(api, self.plan)
        self.assertEqual(api.release["target_commitish"], "a" * 40)
        self.assertEqual(len(self.plan["paths"]), 4)
        self.assertEqual(len(self.plan["checksums"].splitlines()), 4)
        self.assertIn("CPU tools plus shared assets", api.release["body"])

    def test_download_overrun_is_rejected_and_temporary_file_removed(self):
        (self.cache / "cpu.part001").write_bytes(b"bad")
        with patch("package_voice_training.urllib.request.urlopen", return_value=io.BytesIO(b"toolong")):
            with self.assertRaisesRegex(ValueError, "exceeds pinned size"):
                package(self.folder, self.catalog, self.cache)
        self.assertFalse((self.cache / "cpu.part001.download").exists())

    def test_training_part_content_must_match_pinned_hash(self):
        with zipfile.ZipFile(self.training, "w") as archive:
            archive.writestr("README.txt", "instructions")
            archive.writestr("cpu.part001", b"bad")
            archive.writestr("shared.part001", b"shared")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            validate_training_archive(self.training, self.catalog)

    def test_release_api_uses_current_repository_and_streams_file_upload(self):
        with patch.dict(os.environ, {"GITHUB_REPOSITORY": "example/current", "RELEASES_REPOSITORY": "example/old"}):
            api = publisher.GitHub("fixture-token")
        response = unittest.mock.MagicMock()
        response.__enter__.return_value.status = 200
        response.__enter__.return_value.read.return_value = b"{}"
        def receive(request, timeout):
            self.assertEqual(request.full_url, "https://api.github.com/repos/example/current/releases")
            self.assertEqual(request.get_header("Content-length"), str(self.training.stat().st_size))
            self.assertEqual(request.data.read(2), b"PK")
            return response
        with patch.object(publisher.urllib.request, "urlopen", side_effect=receive):
            api.request("/releases", "POST", raw=self.training)

    def test_non_main_push_cannot_publish_even_with_a_token(self):
        argv = ["publisher", "--folder", str(self.folder), "--version", "0.2.0-beta.5", "--commit", "a" * 40, "--run", "42"]
        with patch("sys.argv", argv), patch.dict(os.environ, {"GITHUB_EVENT_NAME": "push", "GITHUB_REF": "refs/heads/feature", "GITHUB_TOKEN": "fixture"}, clear=True):
            with self.assertRaisesRegex(RuntimeError, "restricted"):
                publisher.main()

    def test_training_packager_enforces_github_asset_size_before_download(self):
        catalog = json.loads(self.catalog.read_text())
        catalog["bundles"][0]["parts"][0]["bytes"] = 2 ** 31
        self.catalog.write_text(json.dumps(catalog))
        with patch("package_voice_training.urllib.request.urlopen") as download:
            with self.assertRaisesRegex(ValueError, "per-asset limit"):
                package(self.folder, self.catalog, self.cache)
            download.assert_not_called()


if __name__ == "__main__":
    unittest.main()
