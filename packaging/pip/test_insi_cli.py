"""The wrapper's checks, offline: a local folder stands in for the release.

Run: python test_insi_cli.py   (standard library only)
"""

import hashlib
import io
import os
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "src"))
import insi_cli  # noqa: E402


def archive(entries):
    """A .tar.gz holding `entries` (name -> bytes)."""
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        for name, data in entries.items():
            info = tarfile.TarInfo(name)
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
    return buf.getvalue()


class Wrapper(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.release = root / "release"
        self.release.mkdir()
        self.env = {"INSI_DOWNLOAD_BASE": str(self.release), "INSI_CACHE_DIR": str(root / "cache")}
        self.tgt = insi_cli.target()
        if not self.tgt:
            self.skipTest("no target for this machine")
        self.name = insi_cli.archive_name(insi_cli.__version__, self.tgt)

    def tearDown(self):
        self.tmp.cleanup()

    def put(self, entries):
        data = archive(entries)
        (self.release / self.name).write_bytes(data)
        return hashlib.sha256(data).hexdigest()

    def test_targets(self):
        self.assertEqual(insi_cli.target("Linux", "x86_64"), "x86_64-unknown-linux-gnu")
        self.assertEqual(insi_cli.target("Windows", "AMD64"), "x86_64-pc-windows-msvc")
        self.assertIsNone(insi_cli.target("Windows", "x86"))

    def test_a_pinned_archive_is_installed_once(self):
        digest = self.put({insi_cli.binary_name(): b"stand-in binary"})
        path = insi_cli.ensure(self.env, {self.name: digest})
        self.assertEqual(path.read_bytes(), b"stand-in binary")
        (self.release / self.name).unlink()
        self.assertEqual(insi_cli.ensure(self.env, {self.name: digest}), path, "cached")

    def test_a_wrong_digest_installs_nothing(self):
        self.put({insi_cli.binary_name(): b"x"})
        with self.assertRaisesRegex(RuntimeError, "is not the pinned"):
            insi_cli.ensure(self.env, {self.name: "0" * 64})
        with self.assertRaisesRegex(RuntimeError, "no pinned SHA-256"):
            insi_cli.ensure(self.env, {})
        self.assertFalse(any(Path(self.env["INSI_CACHE_DIR"]).rglob(insi_cli.binary_name())))

    def test_an_archive_cannot_write_outside_its_folder(self):
        digest = self.put({"../escape": b"x", insi_cli.binary_name(): b"y"})
        with self.assertRaises(tarfile.TarError):
            insi_cli.ensure(self.env, {self.name: digest})
        self.assertFalse((Path(self.tmp.name) / "escape").exists())

    def test_only_https_or_a_folder(self):
        env = dict(self.env, INSI_DOWNLOAD_BASE="http://example.invalid")
        digest = self.put({insi_cli.binary_name(): b"x"})
        with self.assertRaisesRegex(RuntimeError, "only https"):
            insi_cli.ensure(env, {self.name: digest})


if __name__ == "__main__":
    unittest.main()
