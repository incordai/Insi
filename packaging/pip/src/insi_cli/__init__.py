"""Run the insi binary for this platform, fetching it once on first use.

A wheel cannot run code when it is installed, so the binary comes on the
first run instead: the release archive for this platform is downloaded,
checked against the SHA-256 pinned in this package's checksums.json (written
by the release workflow), and only the one expected file is taken out of it,
with tarfile's "data" filter refusing any path that leaves the folder.
"""

import hashlib
import json
import os
import platform
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
from pathlib import Path

__version__ = "0.1.0"
REPO = "incordai/insi"

TARGETS = {
    ("Linux", "x86_64"): "x86_64-unknown-linux-gnu",
    ("Linux", "aarch64"): "aarch64-unknown-linux-gnu",
    ("Darwin", "x86_64"): "x86_64-apple-darwin",
    ("Darwin", "arm64"): "aarch64-apple-darwin",
    ("Windows", "AMD64"): "x86_64-pc-windows-msvc",
}


def target(system=None, machine=None):
    """The Rust target for this machine, or None when none is built."""
    return TARGETS.get((system or platform.system(), machine or platform.machine()))


def archive_name(version, tgt):
    return f"insi-v{version}-{tgt}.tar.gz"


def binary_name(system=None):
    return "insi.exe" if (system or platform.system()) == "Windows" else "insi"


def cache_dir(env=os.environ):
    """Where the binary is kept: INSI_CACHE_DIR, else the user's cache."""
    if env.get("INSI_CACHE_DIR"):
        return Path(env["INSI_CACHE_DIR"])
    if platform.system() == "Windows" and env.get("LOCALAPPDATA"):
        return Path(env["LOCALAPPDATA"]) / "insi" / "cache"
    return Path(env.get("XDG_CACHE_HOME") or Path.home() / ".cache") / "insi"


def expected(checksums, name):
    """The pinned digest for an archive; an archive nothing vouches for is
    not run."""
    want = checksums.get(name)
    if not isinstance(want, str) or len(want) != 64 or any(c not in "0123456789abcdef" for c in want):
        raise RuntimeError(f"no pinned SHA-256 for {name} in checksums.json")
    return want


def read_archive(base, name):
    if base.startswith("https://"):
        request = urllib.request.Request(
            f"{base}/{name}", headers={"User-Agent": f"insi-pip/{__version__}"}
        )
        with urllib.request.urlopen(request, timeout=120) as response:
            return response.read()
    if "://" in base:
        raise RuntimeError(f"only https or a local folder: {base}")
    return (Path(base) / name).read_bytes()


def ensure(env=os.environ, checksums=None):
    """The path of this platform's binary, fetched and checked if needed."""
    tgt = target()
    if not tgt:
        raise RuntimeError(
            f"no insi build for {platform.system()}-{platform.machine()}; "
            "build it with `cargo install insi`"
        )
    binary = cache_dir(env) / __version__ / binary_name()
    if binary.is_file():
        return binary
    if not hasattr(tarfile, "data_filter"):
        raise RuntimeError("this Python's tarfile has no safe extraction filter; use Python 3.12+")
    name = archive_name(__version__, tgt)
    if checksums is None:
        checksums = json.loads((Path(__file__).parent / "checksums.json").read_text())
    want = expected(checksums, name)
    base = env.get("INSI_DOWNLOAD_BASE") or f"https://github.com/{REPO}/releases/download/v{__version__}"
    data = read_archive(base, name)
    got = hashlib.sha256(data).hexdigest()
    if got != want:
        raise RuntimeError(f"{name}: SHA-256 {got} is not the pinned {want}; nothing was installed")
    with tempfile.TemporaryDirectory(prefix="insi-pip-") as work:
        archive = Path(work) / name
        archive.write_bytes(data)
        out = Path(work) / "x"
        out.mkdir()
        with tarfile.open(archive, "r:gz") as tar:
            tar.extractall(out, filter="data")
        found = out / binary_name()
        if not found.is_file() or found.is_symlink():
            raise RuntimeError(f"{name} holds no {binary_name()}")
        binary.parent.mkdir(parents=True, exist_ok=True)
        staged = binary.with_suffix(binary.suffix + ".part")
        staged.write_bytes(found.read_bytes())
        staged.chmod(0o755)
        os.replace(staged, binary)
    return binary


def main():
    try:
        binary = ensure()
    except Exception as e:  # what went wrong is all the person needs
        print(f"insi: {e}", file=sys.stderr)
        return 1
    return subprocess.call([str(binary), *sys.argv[1:]])
