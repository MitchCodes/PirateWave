"""Publish verified main-build binaries to the public downloads repository.

Uses only RELEASES_TOKEN from Actions; never reads a developer's Git credential.
Published releases are immutable here. A retry can replace incomplete draft assets.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import urllib.error
import urllib.parse
import urllib.request
import zipfile

REPOSITORY = os.environ.get("RELEASES_REPOSITORY", "")


class GitHub:
    def __init__(self, token):
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", REPOSITORY):
            raise ValueError("Set RELEASES_REPOSITORY explicitly before publishing")
        self.headers = {"Authorization": "Bearer " + token, "Accept": "application/vnd.github+json",
                        "User-Agent": "PirateWave-main-release", "X-GitHub-Api-Version": "2022-11-28"}

    def request(self, path, method="GET", body=None, raw=None):
        url = path if path.startswith("https://uploads.github.com/") else "https://api.github.com/repos/" + REPOSITORY + path
        headers = dict(self.headers)
        data = json.dumps(body).encode() if body is not None else raw
        if raw is not None:
            headers["Content-Type"] = "application/octet-stream"
        request = urllib.request.Request(url, data=data, headers=headers, method=method)
        with urllib.request.urlopen(request, timeout=120) as response:
            return json.load(response) if response.status != 204 else None


def release_plan(folder, version, commit, run):
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?", version):
        raise ValueError("Invalid application version")
    if not re.fullmatch(r"[0-9a-f]{40}", commit) or not re.fullmatch(r"[1-9][0-9]*", run):
        raise ValueError("Invalid source commit or workflow run number")
    paths = [folder / f"PirateWave-v{version}-windows-x64.zip", folder / "PirateWave.lplug4", folder / "PirateWave.streamDeckPlugin"]
    for path in paths:
        if not path.is_file() or path.stat().st_size == 0:
            raise ValueError("Missing release asset: " + path.name)
    with zipfile.ZipFile(paths[0]) as archive:
        if archive.testzip() is not None or any(archive.read(p.name) != p.read_bytes() for p in paths[1:]):
            raise ValueError("Portable ZIP integrity or embedded controller plugin mismatch")
    for package in paths[1:]:
        with zipfile.ZipFile(package) as archive:
            if archive.testzip() is not None:
                raise ValueError("Controller package integrity failure: " + package.name)
    checksums = "".join(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n" for path in paths)
    return dict(tag=f"v{version}-main.{run}.{commit[:12]}", name=f"PirateWave {version} — main {commit[:12]}",
                marker=f"Source commit: `{commit}`\nWorkflow run: `{run}`", paths=paths, checksums=checksums)


def publish(api, plan):
    try:
        release = api.request("/releases/tags/" + plan["tag"])
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
        release = None
    names = [p.name for p in plan["paths"]] + ["SHA256SUMS.txt"]
    if release:
        if plan["marker"] not in release.get("body", ""):
            raise ValueError("Existing release belongs to a different source build")
        if not release["draft"]:
            assets = {a["name"] for a in api.request(f'/releases/{release["id"]}/assets?per_page=100') if a.get("state") == "uploaded"}
            if not set(names).issubset(assets):
                raise ValueError("Published release is incomplete; refusing to modify it")
            print("Release already published for this workflow run: " + release["html_url"])
            return
    else:
        release = api.request("/releases", "POST", dict(
            tag_name=plan["tag"], name=plan["name"], draft=True, prerelease=True,
            body="Automated, validated Windows x64 build from main.\n\n" + plan["marker"] +
                 "\n\nDownload the portable ZIP and optional PirateWave.lplug4 or PirateWave.streamDeckPlugin controller plugins. "
                 "Install the plugin explicitly and enable external controllers in PirateWave Settings > General. "
                 "SHA256SUMS.txt lists asset checksums.\n\nOptional voice and training bundles remain in the v0.2.0-beta.1 release."))
    assets = {a["name"]: a for a in api.request(f'/releases/{release["id"]}/assets?per_page=100')}
    upload = release["upload_url"].split("{", 1)[0]
    parsed = urllib.parse.urlsplit(upload)
    if parsed.scheme != "https" or parsed.hostname != "uploads.github.com":
        raise ValueError("Unexpected GitHub upload endpoint")
    payloads = [(p.name, p.read_bytes()) for p in plan["paths"]] + [("SHA256SUMS.txt", plan["checksums"].encode())]
    for name, data in payloads:
        digest = "sha256:" + hashlib.sha256(data).hexdigest()
        old = assets.get(name)
        if old and old.get("state") == "uploaded" and old.get("digest") == digest:
            continue
        if old:
            api.request(f'/releases/assets/{old["id"]}', "DELETE")
        uploaded = api.request(upload + "?" + urllib.parse.urlencode({"name": name}), "POST", raw=data)
        if uploaded.get("digest") != digest or uploaded.get("state") != "uploaded":
            raise ValueError("Release upload digest/state mismatch: " + name)
    release = api.request(f'/releases/{release["id"]}', "PATCH", {"draft": False})
    print("Published " + release["html_url"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--folder", type=Path, default=Path("dist"))
    parser.add_argument("--version", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--run", required=True)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    plan = release_plan(args.folder, args.version, args.commit, args.run)
    (args.folder / "SHA256SUMS.txt").write_text(plan["checksums"], encoding="utf-8")
    if args.dry_run:
        print(json.dumps({"repository": REPOSITORY, "tag": plan["tag"], "assets": [p.name for p in plan["paths"]] + ["SHA256SUMS.txt"]}))
        return
    if os.environ.get("GITHUB_EVENT_NAME") != "push" or os.environ.get("GITHUB_REF") != "refs/heads/main":
        raise RuntimeError("Release publishing is restricted to main-branch push events")
    token = os.environ.get("RELEASES_TOKEN")
    if not token:
        raise RuntimeError("Configure RELEASES_REPOSITORY and RELEASES_TOKEN for the new public repository")
    publish(GitHub(token), plan)


if __name__ == "__main__":
    main()
