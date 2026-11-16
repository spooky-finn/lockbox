"""Check release guards without GitHub credentials, network, or native builds."""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
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
notes = load_script("release-notes.py")


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
            "GITHUB_EVENT_NAME": "workflow_dispatch",
            "RELEASE_RECHECK": "",
        }
        self.tag = f"v{version}"

    def invoke(self, refs=None):
        with patch.dict(os.environ, self.environment), patch.object(metadata.subprocess, "run") as request:
            request.return_value.stdout = json.dumps(refs or [])
            with contextlib.redirect_stdout(io.StringIO()):
                metadata.main()

    def test_valid_request_emits_matching_version_and_tag(self):
        self.invoke()
        self.assertEqual(self.output.read_text(), f"version={self.environment['RELEASE_VERSION']}\ntag={self.tag}\nrelease=true\n")

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

    def push(self, before_version, refs=None):
        event = Path(self.directory.name, "event.json")
        event.write_text(json.dumps({"before": "b" * 40}))
        self.environment.update(GITHUB_EVENT_NAME="push", GITHUB_EVENT_PATH=str(event), RELEASE_VERSION="")
        with patch.dict(os.environ, self.environment), patch.object(metadata.subprocess, "run") as request:
            request.side_effect = [
                type("Result", (), {"stdout": f'[package]\nversion = "{before_version}"'})(),
                type("Result", (), {"stdout": json.dumps(refs or [])})(),
            ]
            with contextlib.redirect_stdout(io.StringIO()):
                metadata.main()
            return request.call_count

    def test_unchanged_push_skips_release_without_api_call(self):
        calls = self.push(self.environment["RELEASE_VERSION"])
        self.assertEqual(calls, 1)
        self.assertEqual(self.output.read_text(), "release=false\n")

    def test_version_bump_push_releases(self):
        self.push("0.0.0")
        self.assertIn("release=true\n", self.output.read_text())

    def test_version_decrease_is_rejected(self):
        with self.assertRaisesRegex(SystemExit, "never decrease"):
            self.push("999.999.999")

    def test_existing_tag_on_push_is_skipped(self):
        self.push("0.0.0", [{"ref": f"refs/tags/{self.tag}"}])
        self.assertEqual(self.output.read_text(), "release=false\n")

    def test_first_push_can_release_without_previous_commit(self):
        event = Path(self.directory.name, "event.json")
        event.write_text(json.dumps({"before": "0" * 40}))
        self.environment.update(GITHUB_EVENT_NAME="push", GITHUB_EVENT_PATH=str(event), RELEASE_VERSION="")
        self.invoke()
        self.assertIn("release=true\n", self.output.read_text())

    def test_publication_recheck_still_rejects_existing_tag(self):
        self.environment.update(GITHUB_EVENT_NAME="push", RELEASE_RECHECK="true")
        with self.assertRaisesRegex(SystemExit, "already exists"):
            self.invoke([{"ref": f"refs/tags/{self.tag}"}])


class ReleaseNotesTests(unittest.TestCase):
    def test_real_git_history_excludes_commits_before_previous_version(self):
        with tempfile.TemporaryDirectory() as directory:
            def git(*args):
                return subprocess.run(
                    ["git", *args], cwd=directory, check=True, capture_output=True, text=True,
                ).stdout.strip()

            def commit(subject):
                git("-c", "user.name=Release test", "-c", "user.email=release@example.test",
                    "commit", "--allow-empty", "-m", subject)

            git("init")
            commit("Old release")
            git("tag", "v0.1.0")
            commit("Direct push after release")
            commit("Fix without a pull request")
            git("tag", "vnext")
            with patch.object(notes, "git", side_effect=git):
                previous = notes.previous_tag()
                result = notes.render_changes(previous)
            self.assertEqual(previous, "v0.1.0")
            self.assertNotIn("Old release", result)
            self.assertIn("Direct push after release", result)
            self.assertIn("Fix without a pull request", result)

    def test_selects_only_version_tags(self):
        with patch.object(notes, "git", return_value="vnext\nv2.0.0-rc.1\nv1.2.0\nv1.1.0"):
            self.assertEqual(notes.previous_tag(), "v1.2.0")

    def test_no_tags_means_full_history(self):
        with patch.object(notes, "git", return_value=""):
            self.assertIsNone(notes.previous_tag())

    def test_all_commits_and_markdown_escaping(self):
        with patch.object(notes, "git", return_value="abc\tDirect push\n123\t<script> [link](bad)\n789\tMerge PR") as git:
            result = notes.render_changes("v0.1.0")
            git.assert_called_once_with("log", "--reverse", "--format=%H%x09%s", "v0.1.0..HEAD")
        self.assertIn("Direct push", result)
        self.assertIn("Merge PR", result)
        self.assertNotIn("<script>", result)
        self.assertIn(r"\[link\]", result)

    def test_first_release_lists_full_history(self):
        with patch.object(notes, "git", return_value="abc\tInitial commit") as git:
            result = notes.render_changes(None)
            self.assertEqual(git.call_args.args[-1], "HEAD")
        self.assertIn("First release", result)


class ReleaseAssetTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        for name in [
            "lockbox-0.1.0-aarch64-apple-darwin.zip",
            "lockbox-0.1.0-x86_64-apple-darwin.zip",
            "lockbox-0.1.0-x86_64-pc-windows-msvc.zip",
            "lockbox-chrome-0.1.0.zip",
            "lockbox-firefox-0.1.0.zip",
        ]:
            self.write_zip(name, "0.1.0")
        for name in ["lockbox-0.1.0-x86_64-unknown-linux-gnu.tar.gz", "lockbox_0.1.0_amd64.deb"]:
            (self.root / name).write_bytes(b"fixture")

    def write_zip(self, name, version):
        with zipfile.ZipFile(self.root / name, "w") as archive:
            archive.writestr("manifest.json", json.dumps({"version": version}))
            if name.endswith("-x86_64-pc-windows-msvc.zip"):
                archive.writestr("lockbox.exe", b"fixture")

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

    def test_windows_zip_requires_desktop_executable(self):
        with zipfile.ZipFile(self.root / "lockbox-0.1.0-x86_64-pc-windows-msvc.zip", "w") as archive:
            archive.writestr("README.txt", "missing executable")
        with self.assertRaisesRegex(SystemExit, "lockbox.exe"):
            self.invoke()


if __name__ == "__main__":
    unittest.main()
