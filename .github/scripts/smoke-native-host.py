"""Exercise the shipped native messaging executable without a desktop session."""

import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile


def frame(message):
    payload = json.dumps(message).encode()
    return struct.pack("=I", len(payload)) + payload


def main():
    executable = str(Path(sys.argv[1]).resolve())
    with tempfile.TemporaryDirectory(prefix="lockbox-host-smoke-") as home:
        directory = Path(home, ".lockbox/browser-sites")
        directory.mkdir(parents=True, mode=0o700)
        Path(directory, "fixture.json").write_text('["https://saved.example"]')
        environment = {**os.environ, "HOME": home}
        for origin, matched in [
            ("https://saved.example/login", True),
            ("https://unknown.example", False),
            ("https://saved.example:444", False),
        ]:
            result = subprocess.run(
                [executable], input=frame({"id": 1, "request": {"action": "match", "origin": origin}}),
                capture_output=True, env=environment, timeout=10, check=True,
            )
            expected = {"id": 1, "status": "ok", "data": {"matched": matched}}
            payload = result.stdout[4:]
            if (len(result.stdout) < 4
                    or struct.unpack("=I", result.stdout[:4])[0] != len(payload)
                    or json.loads(payload) != expected or result.stderr):
                raise SystemExit(f"Native host origin check failed: {origin}")
        for payload in [
            struct.pack("=I", 4) + b"nope",
            frame({"id": 1, "request": {"action": "unlock", "path": "/tmp/fictional.lbx", "password": "fixture"}}),
        ]:
            result = subprocess.run(
                [executable], input=payload, capture_output=True, env=environment, timeout=10,
            )
            if result.returncode == 0 or result.stdout or result.stderr:
                raise SystemExit("Native host must reject malformed and legacy requests silently.")
    print("Optimized native host checks passed.")


if __name__ == "__main__":
    main()
