"""Run the insi binary for this platform, fetching it once on first use.

A wheel cannot run code when it is installed, so the binary comes on the
first run instead: the release archive for this platform is downloaded,
checked against the SHA-256 pinned in this package's checksums.json (written
by the release workflow), and only the one expected file is taken out of it,
without extracting archive paths to disk.
"""

import errno
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


# The error numbers a filesystem gives when it cannot make hard links (some
# network mounts, FAT/exFAT, certain container overlays). Only these send
# publication to the exclusive-create path below; any other failure (disk
# full, I/O error, missing folder) is a real failure and is raised.
_NO_HARD_LINKS = frozenset(
    {errno.EPERM, errno.ENOSYS, errno.ENOTSUP, errno.EOPNOTSUPP}
)


# Publish `payload` at `path` on a filesystem that cannot hard-link, still
# refusing to overwrite: O_CREAT|O_EXCL fails with FileExistsError if the
# destination exists, so a winner another process may be running is never
# touched. The file is created without execute permission and made
# executable only after every byte is written, and it is removed if anything
# fails, so a half-written file is never left looking like a finished binary.
def _create_exclusive(path, payload):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_BINARY", 0), 0o600)
    try:
        try:
            view = memoryview(payload)
            while view:
                view = view[os.write(fd, view):]
        finally:
            os.close(fd)
        os.chmod(path, 0o755)
    except BaseException:
        Path(path).unlink(missing_ok=True)
        raise


def ensure(env=os.environ, checksums=None):
    """The path of this platform's binary, fetched and checked if needed."""
    tgt = target()
    if not tgt:
        raise RuntimeError(
            f"no insi build for {platform.system()}-{platform.machine()}; "
            "build it with `cargo install insi`"
        )
    binary = cache_dir(env) / __version__ / tgt / binary_name()
    if binary.is_file():
        return binary
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
        with tarfile.open(archive, "r:gz") as tar:
            members = tar.getmembers()
            # Release archives contain exactly one regular binary. Never
            # extract paths, symlinks, hard links, devices or extra members.
            if len(members) != 1 or members[0].name != binary_name() or not members[0].isfile():
                raise tarfile.TarError(f"{name} must hold only a regular {binary_name()}")
            source = tar.extractfile(members[0])
            if source is None:
                raise tarfile.TarError(f"{name} holds no binary")
            with source:
                payload = source.read()
        binary.parent.mkdir(parents=True, exist_ok=True)
        # Each installer owns its staging file; publish only complete bytes.
        fd, staged_name = tempfile.mkstemp(prefix=".insi-", suffix=".part", dir=binary.parent)
        staged = Path(staged_name)
        try:
            with os.fdopen(fd, "wb") as output:
                output.write(payload)
            staged.chmod(0o755)
            try:
                # Publication must atomically refuse an existing destination:
                # a check followed by replace can overwrite a winner that a
                # caller is already reading or executing. Windows rename never
                # overwrites; Unix rename does, so use a hard link there.
                if os.name == "nt":
                    os.rename(staged, binary)
                else:
                    # A filesystem without hard links cannot take the link;
                    # fall back to an exclusive create, which refuses to
                    # overwrite in the same way. Other errors are raised.
                    try:
                        os.link(staged, binary)
                    except FileExistsError:
                        raise
                    except OSError as e:
                        if e.errno not in _NO_HARD_LINKS:
                            raise
                        _create_exclusive(binary, payload)
            except FileExistsError:
                if not binary.is_file():
                    raise
        finally:
            staged.unlink(missing_ok=True)
    return binary


def main():
    try:
        binary = ensure()
    except Exception as e:  # what went wrong is all the person needs
        print(f"insi: {e}", file=sys.stderr)
        return 1
    return subprocess.call([str(binary), *sys.argv[1:]])
