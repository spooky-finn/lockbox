"""Exercise publication using local repositories, without network or credentials."""

import importlib.util
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[3] / "scripts" / "sync-github.py"


class SyncGitHubTests(unittest.TestCase):
    def git(self, repo, *args):
        return subprocess.check_output(
            ["git", "-C", str(repo), *args], text=True, stderr=subprocess.PIPE,
        ).strip()

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.dev = self.root / "dev"
        self.origin = self.root / "origin.git"
        self.github = self.root / "github.git"
        for bare in (self.origin, self.github):
            bare.mkdir()
            self.git(bare, "init", "--bare")
        self.dev.mkdir()
        self.git(self.dev, "init", "-b", "main")
        self.git(self.dev, "config", "user.name", "Sync test")
        self.git(self.dev, "config", "user.email", "sync@example.test")
        (self.dev / "scripts").mkdir()
        shutil.copyfile(SCRIPT, self.dev / "scripts" / SCRIPT.name)
        self.commit("base")
        self.git(self.dev, "remote", "add", "origin", str(self.origin))
        self.git(self.dev, "remote", "add", "github", str(self.github))
        self.git(self.dev, "push", "origin", "main")
        self.git(self.dev, "push", "github", "main")
        self.base = self.git(self.dev, "rev-parse", "HEAD")

    def commit(self, text):
        (self.dev / "shared.txt").write_text(text)
        self.git(self.dev, "add", ".")
        self.git(self.dev, "commit", "-m", text)
        return self.git(self.dev, "rev-parse", "HEAD")

    def run_sync(self, *args, success=True):
        result = subprocess.run(
            [sys.executable, str(self.dev / "scripts" / SCRIPT.name), *args],
            cwd=self.root, text=True, capture_output=True,
        )
        if success:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0)
        return result

    def diverge(self):
        self.commit("GitHub-only change")
        (self.dev / "github-only.txt").write_text("release customization")
        self.git(self.dev, "add", ".")
        self.git(self.dev, "commit", "-m", "GitHub-only file")
        github_head = self.git(self.dev, "rev-parse", "HEAD")
        self.git(self.dev, "tag", "v0.1.0")
        self.git(self.dev, "push", "github", "main", "--tags")
        self.git(self.dev, "reset", "--hard", self.base)
        source_head = self.commit("GitLab authoritative change")
        self.git(self.dev, "push", "origin", "main")
        return github_head, source_head

    def test_conflicts_use_source_tree_and_retain_history_and_tags(self):
        old_target, source = self.diverge()
        self.run_sync()
        published = self.git(self.github, "rev-parse", "main")
        self.assertEqual(self.git(self.github, "show", "main:shared.txt"),
                         "GitLab authoritative change")
        self.assertEqual(self.git(self.github, "rev-parse", "main^{tree}"),
                         self.git(self.origin, "rev-parse", "main^{tree}"))
        self.assertEqual(self.git(self.github, "show", "-s", "--format=%P", published),
                         f"{old_target} {source}")
        self.assertEqual(self.git(self.github, "rev-parse", "v0.1.0"), old_target)
        self.assertEqual(self.git(self.origin, "rev-parse", "main"), source)
        self.run_sync()
        self.assertEqual(self.git(self.github, "rev-parse", "main"), published)
        self.commit("next GitLab change")
        self.git(self.dev, "push", "origin", "main")
        self.run_sync()
        self.assertEqual(self.git(self.github, "show", "main:shared.txt"), "next GitLab change")

    def test_fast_forward_publishes_source_commit_directly(self):
        source = self.commit("source update")
        self.git(self.dev, "push", "origin", "main")
        self.run_sync()
        self.assertEqual(self.git(self.github, "rev-parse", "main"), source)

    def test_dry_run_and_dirty_local_worktree(self):
        old_target, source = self.diverge()
        (self.dev / "shared.txt").write_text("uncommitted local work")
        self.git(self.dev, "add", "shared.txt")
        (self.dev / "untracked.txt").write_text("keep")
        before = self.git(self.dev, "status", "--porcelain")
        self.run_sync("--dry-run")
        self.assertEqual(self.git(self.github, "rev-parse", "main"), old_target)
        self.run_sync()
        self.assertEqual(self.git(self.dev, "status", "--porcelain"), before)
        self.assertEqual(self.git(self.dev, "rev-parse", "HEAD"), source)
        self.assertEqual((self.dev / "shared.txt").read_text(), "uncommitted local work")

    def test_missing_destination_branch(self):
        self.git(self.github, "symbolic-ref", "HEAD", "refs/heads/unused")
        self.git(self.github, "update-ref", "-d", "refs/heads/main")
        self.run_sync()
        self.assertEqual(self.git(self.github, "rev-parse", "main"), self.base)

    def test_destination_only_changes_are_replaced_even_if_source_is_ancestor(self):
        old_target = self.commit("GitHub-only change")
        self.git(self.dev, "push", "github", "main")
        self.run_sync()
        self.assertEqual(self.git(self.github, "rev-parse", "main^{tree}"),
                         self.git(self.origin, "rev-parse", "main^{tree}"))
        self.assertEqual(self.git(self.github, "rev-parse", "main^1"), old_target)

    def test_fetches_destination_push_url(self):
        self.diverge()
        self.git(self.dev, "remote", "set-url", "github", str(self.origin))
        self.git(self.dev, "remote", "set-url", "--push", "github", str(self.github))
        self.run_sync()
        self.assertEqual(self.git(self.github, "rev-parse", "main^{tree}"),
                         self.git(self.origin, "rev-parse", "main^{tree}"))

    def test_source_fetch_failure_leaves_destination_untouched(self):
        self.git(self.dev, "remote", "set-url", "origin", str(self.root / "missing.git"))
        self.run_sync(success=False)
        self.assertEqual(self.git(self.github, "rev-parse", "main"), self.base)

    def test_concurrent_destination_change_rejects_publication(self):
        self.diverge()
        spec = importlib.util.spec_from_file_location("sync_github", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        original_git = module.git

        def race(repo, *args, **kwargs):
            if args[0] == "push":
                self.git(self.github, "update-ref", "refs/heads/main", self.base)
            return original_git(repo, *args, **kwargs)

        with patch.object(module, "git", side_effect=race):
            with self.assertRaises(subprocess.CalledProcessError):
                module.sync(self.dev, False)
        self.assertEqual(self.git(self.github, "rev-parse", "main"), self.base)


if __name__ == "__main__":
    unittest.main()
