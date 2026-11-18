# Copyright (c) 2026-2027 Incord Ai.
# https://incord.ai
#
# SPDX-License-Identifier: BUSL-1.1

"""A reusable HTTP contract must catch a planted authorization regression in a live server."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import os
import threading
import unittest
from unittest.mock import patch
import http_contract


# The test target has two users and a deliberate switch for a broken ownership check.
class Handler(BaseHTTPRequestHandler):
    # Requests exercise real HTTP status/body decisions instead of a mocked request library.
    def do_GET(self):
        if self.path == "/redirect":
            self.send_response(302)
            self.send_header("Location", "https://example.org/")
            self.end_headers()
            return
        token = self.headers.get("Authorization")
        if token != "Bearer lab-alice":
            status, body = 401, b"authentication required"
        elif self.path == "/private/bob" and not self.server.insecure:
            status, body = 403, b"forbidden"
        else:
            status, body = 200, b"bob-canary" if self.path.endswith("bob") else b"alice-record"
        self.send_response(status)
        self.end_headers()
        self.wfile.write(body)

    # Avoid recording credentials or test request data in persistent logs.
    def log_message(self, *_args):
        pass


# The same contract passes the fixed target and fails the broken target without leaking its token.
class HTTPContractTests(unittest.TestCase):
    def test_real_authorization_regression_and_redirect_refusal(self):
        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        server.insecure = False
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        plan = {"version": 1, "base": f"http://127.0.0.1:{server.server_port}", "cases": [
            {"id": "authenticated-own-object", "path": "/private/alice", "headers_from_env": {"Authorization": "LAB_TOKEN"}, "status": 200, "contains": ["alice-record"]},
            {"id": "unauthenticated-object", "path": "/private/alice", "status": 401},
            {"id": "other-users-object", "path": "/private/bob", "headers_from_env": {"Authorization": "LAB_TOKEN"}, "status": 403, "forbids": ["bob-canary"]},
            {"id": "redirect-not-followed", "path": "/redirect", "status": 302},
        ]}
        try:
            with patch.dict(os.environ, {"LAB_TOKEN": "Bearer lab-alice"}):
                fixed = http_contract.run(plan)
                self.assertTrue(fixed["passed"])
                self.assertNotIn("Bearer lab-alice", str(fixed))
                server.insecure = True
                broken = http_contract.run(plan)
                self.assertFalse(broken["passed"])
                self.assertEqual(broken["cases"][2]["status"], 200)
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)

    # An operator typo must not silently turn a local test into an external scan.
    def test_public_targets_and_cross_origin_paths_are_refused(self):
        for base, path in [("https://example.org", "/"), ("http://localhost", "//example.org/")]:
            with self.assertRaises(ValueError):
                http_contract.run({"version": 1, "base": base, "cases": [{"id": "invalid", "path": path, "status": 200}]})


if __name__ == "__main__":
    unittest.main()
