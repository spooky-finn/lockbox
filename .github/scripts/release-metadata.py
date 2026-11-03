"""Validate a manual release before spending time on builds or creating a tag."""

import json
import os
from pathlib import Path
import re
import subprocess
import tomllib


def main():
    version = os.environ["RELEASE_VERSION"]
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        raise SystemExit("Version must have the form X.Y.Z, without a v prefix.")
    if os.environ["GITHUB_REF"] != f"refs/heads/{os.environ['DEFAULT_BRANCH']}":
        raise SystemExit("Run releases from the repository's default branch.")

    manifest = tomllib.loads(Path("Cargo.toml").read_text())
    lockfile = tomllib.loads(Path("Cargo.lock").read_text())
    extension = json.loads(Path("ext/manifest.json").read_text())
    locked_version = next(
        package["version"]
        for package in lockfile["package"]
        if package["name"] == "lockbox" and "source" not in package
    )
    if {manifest["package"]["version"], locked_version, extension["version"]} != {version}:
        raise SystemExit("Version must match Cargo.toml, Cargo.lock, and ext/manifest.json.")

    tag = f"v{version}"
    result = subprocess.run(
        ["gh", "api", f"repos/{os.environ['GH_REPO']}/git/matching-refs/tags/{tag}"],
        check=True,
        capture_output=True,
        text=True,
    )
    if any(ref["ref"] == f"refs/tags/{tag}" for ref in json.loads(result.stdout)):
        raise SystemExit(f"Tag {tag} already exists; releases never replace existing tags.")

    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        output.write(f"version={version}\ntag={tag}\n")
    print(f"Validated {tag} at {os.environ['GITHUB_SHA']}.")


if __name__ == "__main__":
    main()
