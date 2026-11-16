"""Detect version bumps and validate releases before building or creating a tag."""

import json
import os
from pathlib import Path
import re
import subprocess
import tomllib


def emit(**values):
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        for key, value in values.items():
            output.write(f"{key}={value}\n")


def version_tuple(version):
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        raise SystemExit("Version must have the form X.Y.Z, without a v prefix.")
    return tuple(map(int, version.split(".")))


def main():
    if os.environ["GITHUB_REF"] != f"refs/heads/{os.environ['DEFAULT_BRANCH']}":
        raise SystemExit("Run releases from the repository's default branch.")

    manifest = tomllib.loads(Path("Cargo.toml").read_text())
    automatic = os.environ.get("GITHUB_EVENT_NAME") == "push" and not os.environ.get("RELEASE_RECHECK")
    version = manifest["package"]["version"] if automatic else os.environ["RELEASE_VERSION"]
    current = version_tuple(version)
    if automatic:
        before = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())["before"]
        if before != "0" * 40:
            previous = subprocess.run(
                ["git", "show", f"{before}:Cargo.toml"],
                check=True, capture_output=True, text=True,
            )
            old = version_tuple(tomllib.loads(previous.stdout)["package"]["version"])
            if current < old:
                raise SystemExit("Release versions must increase, never decrease.")
            if current == old:
                emit(release="false")
                print("Version unchanged; CI runs without publishing a release.")
                return

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
        if automatic:
            emit(release="false")
            print(f"Tag {tag} already exists; nothing to publish.")
            return
        raise SystemExit(f"Tag {tag} already exists; releases never replace existing tags.")

    emit(version=version, tag=tag, release="true")
    print(f"Validated {tag} at {os.environ['GITHUB_SHA']}.")


if __name__ == "__main__":
    main()
