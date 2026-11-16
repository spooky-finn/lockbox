#!/usr/bin/env python3
"""Publish origin/main's exact tree to github/main without merge conflicts."""

import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def git(repo, *args, check=True, input=None):
    return subprocess.run(
        ["git", "-C", str(repo), *args],
        text=True, input=input, capture_output=True, check=check,
    )


def output(repo, *args):
    return git(repo, *args).stdout.strip()


def remote_url(repo, remote, push=False):
    args = ["remote", "get-url"]
    if push:
        args.append("--push")
    urls = output(repo, *args, "--all", remote).splitlines()
    if len(urls) != 1:
        raise ValueError(f"Expected exactly one URL for {remote}.")
    url = urls[0]
    # Relative filesystem remotes must keep their meaning in the temporary repo.
    if ":" not in url and not os.path.isabs(url):
        url = str((repo / url).resolve())
    return url


def sync(repo, dry_run):
    source_url = remote_url(repo, "origin")
    target_url = remote_url(repo, "github", push=True)
    if source_url == target_url:
        raise ValueError("origin and github must point to different repositories.")

    # An isolated object database leaves local branches, index, worktree, and
    # remote-tracking refs untouched, even when publication fails.
    with tempfile.TemporaryDirectory(prefix="lockbox-sync-") as directory:
        staging = Path(directory)
        git(staging, "init", "--bare", "--quiet")
        print("Fetching origin/main...", flush=True)
        git(staging, "fetch", "--no-tags", "--", source_url,
            "refs/heads/main:refs/heads/source")
        source = output(staging, "rev-parse", "refs/heads/source")

        advertised = git(staging, "ls-remote", "--exit-code", "--heads",
                         "--", target_url, "refs/heads/main", check=False)
        target = ""
        if advertised.returncode == 0:
            print("Fetching github/main...", flush=True)
            git(staging, "fetch", "--no-tags", "--", target_url,
                "refs/heads/main:refs/heads/target")
            target = output(staging, "rev-parse", "refs/heads/target")
        elif advertised.returncode != 2:
            raise subprocess.CalledProcessError(
                advertised.returncode, advertised.args, stderr=advertised.stderr)

        publish = source
        if target:
            source_tree = output(staging, "rev-parse", f"{source}^{{tree}}")
            target_tree = output(staging, "rev-parse", f"{target}^{{tree}}")
            already_imported = git(staging, "merge-base", "--is-ancestor",
                                   source, target, check=False).returncode == 0
            if source_tree == target_tree and already_imported:
                print("github/main already contains origin/main; nothing to push.")
                return
            fast_forward = git(staging, "merge-base", "--is-ancestor",
                               target, source, check=False).returncode == 0
            if not fast_forward:
                print("Using origin/main's exact files; GitHub-only file changes "
                      "will be replaced. Previous history stays reachable.", flush=True)
                identity = []
                for key in ("user.name", "user.email"):
                    value = git(repo, "config", "--get", key, check=False).stdout.strip()
                    if value:
                        identity.extend(["-c", f"{key}={value}"])
                # No textual merge: the source tree is authoritative. Both parents
                # retain GitHub release history and imported development history.
                publish = git(
                    staging, *identity, "commit-tree", source_tree,
                    "-p", target, "-p", source,
                    input=f"Sync origin/main for GitHub releases\n\nSource: {source}\n",
                ).stdout.strip()
            print("File changes to publish:", flush=True)
            print(output(staging, "diff", "--stat", target, publish) or "(no file changes)")

        print(f"origin/main: {source}\ngithub/main: {target or '(new branch)'}\n"
              f"Publish:     {publish}", flush=True)
        args = ["push"]
        if dry_run:
            args.append("--dry-run")
        # The new commit descends from target. The explicit lease additionally
        # rejects concurrent publication, including another invocation of this tool.
        args.extend([f"--force-with-lease=refs/heads/main:{target}", "--",
                     target_url, f"{publish}:refs/heads/main"])
        result = git(staging, *args)
        print(result.stderr.strip())
        print("Dry run complete; nothing published." if dry_run else "Published github/main.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dry-run", action="store_true",
                        help="fetch and prepare publication without updating GitHub")
    args = parser.parse_args()
    try:
        # Resolve relative to this script, so invocation works from any directory.
        repo = Path(output(Path(__file__).resolve().parent, "rev-parse", "--show-toplevel"))
        sync(repo, args.dry_run)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        detail = error.stderr.strip() if isinstance(error, subprocess.CalledProcessError) and error.stderr else str(error)
        print(f"Sync failed: {detail}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
