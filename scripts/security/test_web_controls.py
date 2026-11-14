"""Executable positive/fixed controls for common access and input boundaries.

These are deliberately small Python controls, not a whole-application OWASP audit.
"""
import html
from html.parser import HTMLParser
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


# Demonstrate object-level authorization separately from whether a user is authenticated.
def record_for(records, record_id, user, fixed):
    record = records[record_id]
    if fixed and record["owner"] != user:
        raise PermissionError("record belongs to another user")
    return record["data"]


# Resolve filesystem links before deciding whether a requested path remains below the public root.
def read_public(root, name, fixed):
    path = root / name
    if fixed:
        path = path.resolve()
        path.relative_to(root.resolve())
    return path.read_text()


# Observe the actual HTML parser boundary rather than assuming a substring proves script execution.
class Tags(HTMLParser):
    def __init__(self):
        super().__init__()
        self.tags = []

    # Collect element names; entity-escaped text should never create a script element.
    def handle_starttag(self, tag, attrs):
        self.tags.append(tag)


# Every weakness has an insecure positive, a blocked fixed control and an ordinary valid input.
class WebControls(unittest.TestCase):
    def test_idor_requires_ownership_not_only_authentication(self):
        records = {"1": {"owner": "alice", "data": "alice-canary"}, "2": {"owner": "bob", "data": "bob-canary"}}
        self.assertEqual(record_for(records, "2", "alice", False), "bob-canary")
        with self.assertRaises(PermissionError):
            record_for(records, "2", "alice", True)
        self.assertEqual(record_for(records, "1", "alice", True), "alice-canary")

    def test_path_traversal_and_symlink_escape(self):
        scratch = ROOT / ".incord/temp"
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            folder = Path(directory)
            public = folder / "public"
            public.mkdir()
            (folder / "private.txt").write_text("lab-canary")
            (public / "normal.txt").write_text("public")
            self.assertEqual(read_public(public, "../private.txt", False), "lab-canary")
            with self.assertRaises(ValueError):
                read_public(public, "../private.txt", True)
            self.assertEqual(read_public(public, "normal.txt", True), "public")
            try:
                (public / "link.txt").symlink_to(folder / "private.txt")
            except OSError:
                return
            with self.assertRaises(ValueError):
                read_public(public, "link.txt", True)

    def test_html_injection_is_separate_from_plain_text(self):
        payload = "<script>labCanary()</script>"
        insecure = Tags()
        insecure.feed("<p>" + payload + "</p>")
        self.assertIn("script", insecure.tags)
        fixed = Tags()
        fixed.feed("<p>" + html.escape(payload, quote=True) + "</p>")
        self.assertNotIn("script", fixed.tags)
        self.assertEqual(html.escape("ordinary text"), "ordinary text")


if __name__ == "__main__":
    unittest.main()
