"""Build release notes including direct commits, not only merged pull requests."""

import html
from pathlib import Path
import re
import subprocess


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout.strip()


def previous_tag():
    tags = git("tag", "--merged", "HEAD", "--sort=-version:refname", "--list", "v*")
    return next((tag for tag in tags.splitlines() if re.fullmatch(r"v\d+\.\d+\.\d+", tag)), None)


def render_changes(previous):
    revision = f"{previous}..HEAD" if previous else "HEAD"
    commits = git("log", "--reverse", "--format=%H%x09%s", revision)
    lines = ["## Changes", ""]
    if previous:
        lines.extend([f"All commits since `{previous}` (including direct pushes and merges).", ""])
    else:
        lines.extend(["First release: full commit history.", ""])
    for commit in commits.splitlines():
        sha, subject = commit.split("\t", 1)
        # Commit messages are untrusted Markdown, never workflow commands or shell input.
        subject = re.sub(r"([\\`*_{}\[\]()#+.!|>~-])", r"\\\1", html.escape(subject))
        lines.append(f"- {sha} — {subject}")
    return "\n".join(lines)


def main():
    print(Path(".github/release-notes.md").read_text())
    print(render_changes(previous_tag()))


if __name__ == "__main__":
    main()
