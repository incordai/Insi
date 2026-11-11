"""Tests for the claude -p review adapter. Run: python3 -m unittest discover -s scripts/review -p 'test_*.py'"""
import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("claude_p_review", os.path.join(HERE, "claude_p_review.py"))
adapter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(adapter)


def answer(text, code=0):
    return subprocess.CompletedProcess([], code, stdout=text, stderr="")


class Ask(unittest.TestCase):
    # A model answer without a proper findings list must raise, never read as
    # a clean review (the old code returned [] for a missing key).
    def test_a_malformed_answer_is_an_error_not_a_clean_review(self):
        for text in ['{}', '{"findings":null}', '{"findings":"none"}', '{"findings":["x"]}', 'no json']:
            with mock.patch.object(adapter.subprocess, "run", return_value=answer(text)):
                with self.assertRaises(RuntimeError, msg=text):
                    adapter.ask("diff")

    def test_a_good_answer_returns_its_findings(self):
        text = 'Here: {"findings":[{"file":"a.rs","line":3,"message":"bad"}]}'
        with mock.patch.object(adapter.subprocess, "run", return_value=answer(text)):
            self.assertEqual(adapter.ask("diff")[0]["file"], "a.rs")

    def test_a_failed_model_call_is_an_error(self):
        with mock.patch.object(adapter.subprocess, "run", return_value=answer("", 1)):
            with self.assertRaises(RuntimeError):
                adapter.ask("diff")


class Chunks(unittest.TestCase):
    # Every file lands in some piece, and no piece breaks the size limit.
    def test_a_large_diff_is_split_by_file_without_losing_any(self):
        files = [f"diff --git a/f{i} b/f{i}\n" + "+x\n" * 20_000 for i in range(5)]
        pieces = adapter.chunks("".join(files))
        self.assertGreater(len(pieces), 1)
        self.assertTrue(all(len(p) <= adapter.CHUNK + 100 for p in pieces))
        joined = "".join(pieces)
        for i in range(5):
            self.assertIn(f"diff --git a/f{i} b/f{i}", joined)


class ThreeDots(unittest.TestCase):
    # When the base branch moved on after the PR branched, the diff sent to the
    # model must hold only the PR's own change, not the reverse of the newer
    # base commit (the two-dot diff did).
    def test_the_diff_ignores_commits_added_to_the_base_after_branching(self):
        with tempfile.TemporaryDirectory() as d:
            def git(*args):
                subprocess.run(
                    ["git", "-C", d, "-c", "user.name=t", "-c", "user.email=t@t", *args],
                    check=True, capture_output=True,
                )

            def write(name, text):
                with open(os.path.join(d, name), "w") as f:
                    f.write(text)

            git("init", "-q", "-b", "main")
            write("a", "1\n")
            git("add", "a")
            git("commit", "-qm", "one")
            git("checkout", "-q", "-b", "pr")
            write("pr_file", "from the pr\n")
            git("add", "pr_file")
            git("commit", "-qm", "pr change")
            git("checkout", "-q", "main")
            write("later_file", "added to main later\n")
            git("add", "later_file")
            git("commit", "-qm", "base moved on")
            git("checkout", "-q", "pr")
            sent = []
            before = os.getcwd()
            os.chdir(d)
            try:
                with mock.patch.object(adapter, "ask", side_effect=lambda piece: sent.append(piece) or []), \
                        mock.patch.object(sys, "argv", ["x", "main"]), \
                        mock.patch("builtins.print"):
                    self.assertEqual(adapter.main(), 0)
            finally:
                os.chdir(before)
            self.assertEqual(len(sent), 1)
            self.assertIn("+from the pr", sent[0])
            self.assertNotIn("later_file", sent[0])


if __name__ == "__main__":
    unittest.main()
