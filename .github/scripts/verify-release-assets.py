"""Reject incomplete uploads and mislabeled browser extension archives."""

import json
from pathlib import Path
import sys
import zipfile


def main():
    directory = Path(sys.argv[1])
    version = sys.argv[2]
    expected = {
        f"lockbox-{version}-aarch64-apple-darwin.zip",
        f"lockbox-{version}-x86_64-apple-darwin.zip",
        f"lockbox-{version}-x86_64-unknown-linux-gnu.tar.gz",
        f"lockbox_{version}_amd64.deb",
        f"lockbox-chrome-{version}.zip",
        f"lockbox-firefox-{version}.zip",
    }
    actual = {path.name for path in directory.iterdir() if path.is_file()}
    if actual != expected:
        raise SystemExit(f"Unexpected release assets. Missing: {expected - actual}; extra: {actual - expected}")
    for path in directory.iterdir():
        if path.stat().st_size == 0:
            raise SystemExit(f"Empty release asset: {path.name}")
        if path.suffix == ".zip":
            with zipfile.ZipFile(path) as archive:
                if archive.testzip() is not None:
                    raise SystemExit(f"Corrupt archive: {path.name}")
                if path.name.startswith(("lockbox-chrome-", "lockbox-firefox-")):
                    manifest = json.loads(archive.read("manifest.json"))
                    if manifest["version"] != version:
                        raise SystemExit(f"Browser version mismatch: {path.name}")
    print(f"Verified all {len(expected)} release assets.")


if __name__ == "__main__":
    main()
