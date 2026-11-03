"""Check release guards without GitHub credentials, network, or native builds."""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile


SCRIPTS = Path(__file__).resolve().parents[1]
ROOT = SCRIPTS.parents[1]


def load_script(filename):
    spec = importlib.util.spec_from_file_location(filename, SCRIPTS / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


metadata = load_script("release-metadata.py")
assets = load_script("verify-release-assets.py")


class ReleaseMetadataTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.output = Path(self.directory.name, "output")
        version = json.loads((ROOT / "ext/manifest.json").read_text())["version"]
        self.environment = {
            "RELEASE_VERSION": version,
            "GITHUB_REF": "refs/heads/main",
            "DEFAULT_BRANCH": "main",
            "GH_REPO": "fixture/lockbox",
            "GITHUB_OUTPUT": str(self.output),
            "GITHUB_SHA": "a" * 40,
        }
        self.tag = f"v{version}"

    def invoke(self, refs=None):
        with patch.dict(os.environ, self.environment), patch.object(metadata.subprocess, "run") as request:
            request.return_value.stdout = json.dumps(refs or [])
            with contextlib.redirect_stdout(io.StringIO()):
                metadata.main()

    def test_valid_request_emits_matching_version_and_tag(self):
        self.invoke()
        self.assertEqual(self.output.read_text(), f"version={self.environment['RELEASE_VERSION']}\ntag={self.tag}\n")

    def test_non_default_branch_is_rejected(self):
        self.environment["GITHUB_REF"] = "refs/heads/feature"
        with self.assertRaisesRegex(SystemExit, "default branch"):
            self.invoke()

    def test_version_mismatch_is_rejected(self):
        self.environment["RELEASE_VERSION"] = "999.999.999"
        with self.assertRaisesRegex(SystemExit, "must match"):
            self.invoke()

    def test_invalid_versions_are_rejected(self):
        for version in ["v0.1.0", "0.1.0-rc.1", "01.1.0", "0.1.0\nother=value", "$(id)"]:
            with self.subTest(version=version), self.assertRaisesRegex(SystemExit, "X.Y.Z"):
                self.environment["RELEASE_VERSION"] = version
                self.invoke()

    def test_existing_exact_tag_is_rejected(self):
        with self.assertRaisesRegex(SystemExit, "already exists"):
            self.invoke([{"ref": f"refs/tags/{self.tag}"}])

    def test_tag_prefix_is_not_an_existing_release(self):
        self.invoke([{"ref": f"refs/tags/{self.tag}0"}])
        self.assertTrue(self.output.exists())

    def test_api_errors_prevent_release(self):
        with patch.dict(os.environ, self.environment), patch.object(
            metadata.subprocess, "run", side_effect=metadata.subprocess.CalledProcessError(1, "gh")
        ), self.assertRaises(metadata.subprocess.CalledProcessError):
            metadata.main()
        self.assertFalse(self.output.exists())


class ReleaseAssetTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        for name in [
            "lockbox-0.1.0-aarch64-apple-darwin.zip",
            "lockbox-0.1.0-x86_64-apple-darwin.zip",
            "lockbox-chrome-0.1.0.zip",
            "lockbox-firefox-0.1.0.zip",
        ]:
            self.write_zip(name, "0.1.0")
        for name in ["lockbox-0.1.0-x86_64-unknown-linux-gnu.tar.gz", "lockbox_0.1.0_amd64.deb"]:
            (self.root / name).write_bytes(b"fixture")

    def write_zip(self, name, version):
        with zipfile.ZipFile(self.root / name, "w") as archive:
            archive.writestr("manifest.json", json.dumps({"version": version}))

    def invoke(self):
        with patch("sys.argv", ["verify-release-assets.py", str(self.root), "0.1.0"]):
            with contextlib.redirect_stdout(io.StringIO()):
                assets.main()

    def test_complete_asset_set_is_accepted(self):
        self.invoke()

    def test_missing_asset_is_rejected(self):
        (self.root / "lockbox_0.1.0_amd64.deb").unlink()
        with self.assertRaisesRegex(SystemExit, "Missing"):
            self.invoke()

    def test_extra_stale_asset_is_rejected(self):
        (self.root / "lockbox-chrome-0.0.1.zip").write_bytes(b"stale")
        with self.assertRaisesRegex(SystemExit, "extra"):
            self.invoke()

    def test_empty_asset_is_rejected(self):
        (self.root / "lockbox_0.1.0_amd64.deb").write_bytes(b"")
        with self.assertRaisesRegex(SystemExit, "Empty"):
            self.invoke()

    def test_browser_manifest_must_match_release(self):
        self.write_zip("lockbox-firefox-0.1.0.zip", "0.0.1")
        with self.assertRaisesRegex(SystemExit, "version mismatch"):
            self.invoke()

    def test_corrupt_zip_is_rejected(self):
        (self.root / "lockbox-chrome-0.1.0.zip").write_bytes(b"not a ZIP")
        with self.assertRaises(zipfile.BadZipFile):
            self.invoke()


if __name__ == "__main__":
    unittest.main()
