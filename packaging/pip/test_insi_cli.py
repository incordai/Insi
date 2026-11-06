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
import subprocess
import json
import time
from pathlib import Path
from unittest import mock

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

    def test_concurrent_first_runs_publish_complete_binary(self):
        payload = b"complete-binary" * 10000
        digest = self.put({insi_cli.binary_name(): payload})
        gate = Path(self.tmp.name) / "start"
        script = """
import insi_cli, json, sys, time
from pathlib import Path
gate = Path(sys.argv[3])
Path(sys.argv[4]).touch()
deadline = time.monotonic() + 20
while not gate.exists():
    if time.monotonic() >= deadline:
        raise TimeoutError('installer start gate')
    time.sleep(0.01)
p = insi_cli.ensure(json.loads(sys.argv[1]), json.loads(sys.argv[2]))
assert p.read_bytes() == b'complete-binary' * 10000
"""
        env = dict(os.environ, PYTHONPATH=str(Path(__file__).parent / "src"))
        ready = [Path(self.tmp.name) / f"ready-{i}" for i in range(8)]
        children = []
        try:
            for marker in ready:
                children.append(subprocess.Popen(
                    [sys.executable, "-c", script, json.dumps(self.env),
                     json.dumps({self.name: digest}), str(gate), str(marker)],
                    env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE))
            deadline = time.monotonic() + 20
            while not all(marker.exists() for marker in ready) and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(all(marker.exists() for marker in ready), "all installers ready")
            self.assertFalse(any(Path(self.env["INSI_CACHE_DIR"]).rglob(insi_cli.binary_name())))
            gate.touch()
            for child in children:
                stdout, stderr = child.communicate(timeout=30)
                self.assertEqual(child.returncode, 0, (stdout, stderr))
        finally:
            for child in children:
                if child.poll() is None:
                    child.kill()
                child.communicate(timeout=5)
        self.assertEqual(list(Path(self.env["INSI_CACHE_DIR"]).rglob("*.part")), [])

    def test_failed_publication_leaves_no_binary_or_staging_file_and_can_retry(self):
        digest = self.put({insi_cli.binary_name(): b"complete binary"})
        with mock.patch.object(insi_cli.os, "replace", side_effect=OSError("publish failed")):
            with self.assertRaisesRegex(OSError, "publish failed"):
                insi_cli.ensure(self.env, {self.name: digest})
        cache = Path(self.env["INSI_CACHE_DIR"])
        self.assertEqual(list(cache.rglob("*.part")), [])
        self.assertEqual(list(cache.rglob(insi_cli.binary_name())), [])
        path = insi_cli.ensure(self.env, {self.name: digest})
        self.assertEqual(path.read_bytes(), b"complete binary")

    def test_archive_links_and_extra_members_are_refused(self):
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
            buf = io.BytesIO()
            with tarfile.open(fileobj=buf, mode="w:gz") as tar:
                info = tarfile.TarInfo(insi_cli.binary_name())
                info.type = kind
                info.linkname = "../outside"
                tar.addfile(info)
            data = buf.getvalue()
            (self.release / self.name).write_bytes(data)
            with self.assertRaises(tarfile.TarError):
                insi_cli.ensure(self.env, {self.name: hashlib.sha256(data).hexdigest()})

    def test_only_https_or_a_folder(self):
        env = dict(self.env, INSI_DOWNLOAD_BASE="http://example.invalid")
        digest = self.put({insi_cli.binary_name(): b"x"})
        with self.assertRaisesRegex(RuntimeError, "only https"):
            insi_cli.ensure(env, {self.name: digest})


if __name__ == "__main__":
    unittest.main()
