# Copyright (c) 2026-2027 Incord Ai.
# https://incord.ai
#
# SPDX-License-Identifier: BUSL-1.1

"""Adversarial boundary cases with ordinary accepted controls and explicit unsafe contrasts."""
from concurrent.futures import ThreadPoolExecutor
from decimal import Decimal
from pathlib import Path
import tempfile
import unittest
from boundary_controls import Sessions, authorize, fetch_public, store_text_upload, Ledger, password_record, authenticate, session_cookie

ROOT = Path(__file__).resolve().parents[2]


# These tests exercise concrete lab policies; they do not audit a deployed authentication stack.
class BoundaryTests(unittest.TestCase):
    # Bad credentials must never create a session, and equal passwords must have different salts.
    def test_credentials_and_cookie_scope(self):
        from http.cookies import SimpleCookie
        credentials = {"alice": password_record("lab-only-password")}
        self.assertNotEqual(credentials["alice"], password_record("lab-only-password"))
        sessions = Sessions(lambda: 0)
        for user, password in [("alice", "wrong"), ("missing", "lab-only-password")]:
            with self.assertRaises(PermissionError):
                authenticate(credentials, user, password, sessions)
        self.assertFalse(sessions.sessions)
        token, _ = authenticate(credentials, "alice", "lab-only-password", sessions)
        self.assertEqual(sessions.user(token), "alice")
        cookie = SimpleCookie(session_cookie(token))["__Host-insi-lab"]
        self.assertTrue(cookie["secure"] and cookie["httponly"])
        self.assertEqual(cookie["samesite"], "Strict")
        self.assertEqual(cookie["path"], "/")
        self.assertFalse(cookie["domain"])

    def test_session_fixation_expiry_logout_and_csrf_binding(self):
        now = [0]
        sessions = Sessions(lambda: now[0], 10)
        old, _ = sessions.login("anonymous")
        token, csrf = sessions.login("alice", old)
        self.assertNotEqual(old, token)
        self.assertEqual(sessions.user(token), "alice")
        with self.assertRaises(PermissionError):
            sessions.user(old)
        other, other_csrf = sessions.login("bob")
        sessions.csrf(token, csrf, "https://lab.invalid")
        for proof, origin in [("", "https://lab.invalid"), (other_csrf, "https://lab.invalid"), (csrf, "https://evil.invalid")]:
            with self.assertRaises(PermissionError):
                sessions.csrf(token, proof, origin)
        sessions.logout(other)
        with self.assertRaises(PermissionError):
            sessions.user(other)
        now[0] = 10
        with self.assertRaises(PermissionError):
            sessions.user(token)

    def test_api_object_and_method_authorization(self):
        for method in ("read", "update", "delete"):
            authorize("alice", "user", "alice", method)
            authorize("operator", "admin", "alice", method)
            with self.assertRaises(PermissionError):
                authorize("bob", "user", "alice", method)
        for user, method in [("", "read"), ("alice", "promote-admin")]:
            with self.assertRaises(PermissionError):
                authorize(user, "user", "alice", method)

    def test_ssrf_blocks_private_mixed_and_mapped_addresses_before_dial(self):
        dialed = []

        # The test transport records only the numeric address; it never performs a network request.
        def dial(ip, host, path):
            dialed.append((ip, host, path))
            return 200, b"public-response"

        # Query preservation must not re-resolve or change the pinned network destination.
        self.assertEqual(fetch_public("https://example.org/path?q=value", lambda host: ["93.184.216.34"], dial), b"public-response")
        self.assertEqual(dialed, [("93.184.216.34", "example.org", "/path?q=value")])
        for addresses in [["127.0.0.1"], ["10.0.0.1"], ["169.254.169.254"], ["::1"], ["::ffff:127.0.0.1"], ["224.0.0.1"], ["ff02::1"], ["93.184.216.34", "192.168.1.1"], []]:
            with self.assertRaises(ValueError):
                fetch_public("https://example.org/", lambda host: addresses, dial)
        self.assertEqual(len(dialed), 1)
        for url in ["file:///etc/passwd", "http://example.org", "https://user:pass@example.org", "https://example.org:8080"]:
            with self.assertRaises(ValueError):
                fetch_public(url, lambda host: ["93.184.216.34"], dial)
        with self.assertRaises(ValueError):
            fetch_public("https://example.org", lambda host: ["93.184.216.34"], lambda *args: (302, b"redirect"))

    def test_upload_policy_refuses_paths_binary_and_excess_size(self):
        scratch = ROOT / ".incord/temp"
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            folder = Path(directory)
            path = store_text_upload(folder, "note.txt", "text/plain", b"ordinary text")
            self.assertEqual(path.read_bytes(), b"ordinary text")
            self.assertNotEqual(path.name, "note.txt")
            for name, kind, payload in [("../note.txt", "text/plain", b"x"), ("script.php", "text/plain", b"x"), ("note.txt", "image/png", b"x"), ("note.txt", "text/plain", b"\x00"), ("note.txt", "text/plain", b"x" * 65537)]:
                with self.assertRaises(ValueError):
                    store_text_upload(folder, name, kind, payload)

    def test_business_logic_replay_invalid_amounts_and_concurrent_spend(self):
        ledger = Ledger()
        ledger.transfer("alice", "bob", "10.00", "first")
        ledger.transfer("alice", "bob", "10.00", "first")
        self.assertEqual(ledger.balances["alice"], Decimal("90"))
        for amount in ("-1", "0", "NaN", "Infinity", "0.001"):
            with self.assertRaises(ValueError):
                ledger.transfer("alice", "bob", amount, "invalid")
        with self.assertRaises(ValueError):
            ledger.transfer("alice", "bob", "20", "first")

        # Distinct concurrent requests must not both spend a balance that covers only one.
        def spend(key):
            try:
                ledger.transfer("alice", "bob", "60", key)
                return True
            except PermissionError:
                return False

        with ThreadPoolExecutor(max_workers=2) as pool:
            self.assertEqual(sum(pool.map(spend, ["second", "third"])), 1)
        self.assertEqual(sum(ledger.balances.values()), Decimal("100"))


if __name__ == "__main__":
    unittest.main()
