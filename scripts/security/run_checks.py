"""Select an installed scrypt-capable Python for scheduled security tests.

Watchman/Inmem can have a different PATH from the interactive terminal. Never
replace scrypt with a weaker algorithm or silently skip authentication controls.
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys


# Probe installed interpreters only; configuration is explicit and no software is downloaded.
def choose_python(root):
    override = os.environ.get("INSI_PYTHON")
    candidates = [override] if override else [
        str(root / ".venv" / "bin" / "python"),
        str(root / ".venv" / "Scripts" / "python.exe"),
        str(root / ".incord" / "temp" / "security-venv" / "bin" / "python"),
        sys.executable, shutil.which("python3"),
        "/opt/homebrew/bin/python3", "/usr/local/bin/python3",
    ]
    seen = set()
    for candidate in candidates:
        if not candidate or candidate in seen or not Path(candidate).is_file():
            continue
        seen.add(candidate)
        try:
            result = subprocess.run([candidate, "-c", "import hashlib,sys; sys.exit(0 if hasattr(hashlib, 'scrypt') else 1)"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10, check=False)
            if result.returncode == 0:
                return candidate
        except (OSError, subprocess.TimeoutExpired):
            continue
    raise RuntimeError("Incomplete: no installed Python provides hashlib.scrypt; install an OpenSSL-enabled Python or set INSI_PYTHON to its executable")


# Return the actual unittest exit so failed controls remain failing scheduled checks.
def main():
    root = Path(__file__).resolve().parents[2]
    try:
        python = choose_python(root)
    except RuntimeError as error:
        print(str(error), file=sys.stderr)
        return 2
    print("Security check interpreter: " + python, flush=True)
    return subprocess.call([python, "-m", "unittest", "discover", "-s", "scripts/security", "-p", "test_*.py"], cwd=root)


if __name__ == "__main__":
    raise SystemExit(main())
