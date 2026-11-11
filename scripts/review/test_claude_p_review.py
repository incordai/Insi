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

    # claude must not start inside the PR checkout, where a PR's own
    # .claude/settings.json hooks, .mcp.json servers and CLAUDE.md would load.
    def test_claude_runs_from_an_empty_folder_with_project_settings_off(self):
        calls = []

        def record(cmd, **kw):
            calls.append((cmd, kw["cwd"], os.listdir(kw["cwd"])))
            return answer('{"findings":[]}')

        with mock.patch.object(adapter.subprocess, "run", side_effect=record):
            adapter.ask("diff")
        cmd, cwd, listing = calls[0]
        self.assertNotEqual(os.path.realpath(cwd), os.path.realpath(os.getcwd()))
        self.assertEqual(listing, [])
        self.assertEqual(cmd[cmd.index("--setting-sources") + 1], "user")
        self.assertEqual(cmd[cmd.index("--tools") + 1], "")
        self.assertIn("--strict-mcp-config", cmd)
        self.assertFalse(os.path.exists(cwd), "the empty folder is removed")

    def test_a_failed_model_call_is_an_error(self):
        with mock.patch.object(adapter.subprocess, "run", return_value=answer("", 1)):
            with self.assertRaises(RuntimeError):
                adapter.ask("diff")


class Exit(unittest.TestCase):
    # A bad base (git fails) and a missing `claude` (OSError) both end in the
    # documented exit 2, not a traceback with exit 1.
    def test_git_or_launch_failures_exit_2(self):
        with mock.patch.object(sys, "argv", ["x", "no-such-base-ref"]), \
                mock.patch.object(sys, "stderr"):
            self.assertEqual(adapter.main(), 2)
        with mock.patch.object(adapter, "git", return_value="diff --git a/f b/f\n+x\n"), \
                mock.patch.object(adapter.subprocess, "run", side_effect=FileNotFoundError("claude")), \
                mock.patch.object(sys, "argv", ["x", "main"]), \
                mock.patch.object(sys, "stderr"):
            self.assertEqual(adapter.main(), 2)


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

    # One file bigger than CHUNK is split at its hunks, not cut: every hunk is
    # sent, and every piece still names the file. The old code dropped the end.
    def test_one_huge_file_is_split_at_hunks_without_dropping_any(self):
        header = "diff --git a/big b/big\n--- a/big\n+++ b/big\n"
        hunks = [f"@@ -{i},3 +{i},3 @@\n" + f"+line {i}\n" * 3000 for i in range(10)]
        pieces = adapter.chunks(header + "".join(hunks))
        self.assertGreater(len(pieces), 1)
        self.assertTrue(all(len(p) <= adapter.CHUNK for p in pieces))
        self.assertTrue(all(p.startswith(header) for p in pieces))
        joined = "".join(pieces)
        for i in range(10):
            self.assertIn(f"@@ -{i},3 +{i},3 @@", joined)

    # A single hunk that alone exceeds CHUNK cannot be split safely, so the
    # review fails instead of reporting a partly read change as complete.
    def test_a_hunk_too_large_to_send_fails_the_review(self):
        diff = "diff --git a/big b/big\n@@ -1 +1 @@\n" + "+x\n" * (adapter.CHUNK // 2)
        with self.assertRaises(RuntimeError):
            adapter.chunks(diff)


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
