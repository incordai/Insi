# Copyright (c) 2026-2027 Incord Ai.
# https://incord.ai
#
# SPDX-License-Identifier: BUSL-1.1

"""Real HTTP vulnerable/fixed controls with observed records, requests and filesystem effects."""
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import tempfile
import threading
import unittest
import urllib.request
import urllib.error
from api_fixture import Fixture

ROOT = Path(__file__).resolve().parents[2]


# Test-owned upstreams represent public content and a private internal canary on different origins.
class Upstream(BaseHTTPRequestHandler):
    # Count actual requests so a blocked fetch cannot hide an internal request made before rejection.
    def do_GET(self):
        self.server.hits += 1
        if self.path == "/redirect":
            self.send_response(302)
            self.send_header("Location", self.server.redirect)
            self.end_headers()
            return
        self.send_response(200)
        self.end_headers()
        self.wfile.write(self.server.content)

    # Keep target URLs and request data out of logs.
    def log_message(self, *_args):
        pass


# All servers are loopback-only, have bounded client deadlines and join during cleanup.
@contextmanager
def running(server):
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


# Request helpers return actual HTTP statuses, private JSON bodies and session cookies to the test.
def request(origin, path, data=None, cookie=None, csrf=None, site="https://lab.invalid", multipart=False):
    headers = {"Content-Type": "application/json", "Origin": site}
    if cookie:
        headers["Cookie"] = cookie
    if csrf:
        headers["X-CSRF-Token"] = csrf
    # Upload probes use wire-format multipart headers and bytes rather than a JSON-only stand-in.
    if multipart:
        boundary = "insi-lab-boundary"
        headers["Content-Type"] = "multipart/form-data; boundary=" + boundary
        payload = (f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="{data["name"]}"\r\nContent-Type: {data["type"]}\r\n\r\n').encode() + data["content"].encode() + f"\r\n--{boundary}--\r\n".encode()
    else:
        payload = json.dumps(data).encode() if data is not None else None
    req = urllib.request.Request(origin + path, data=payload, headers=headers)
    try:
        response = urllib.request.urlopen(req, timeout=5)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, json.load(response), response.headers.get("Set-Cookie", "").split(";", 1)[0]


# Every test runs the identical request scenario against both deliberately unsafe and fixed modes.
class APIFixtureTests(unittest.TestCase):
    # Own all targets and storage so no test ever contacts or writes to a real deployment.
    @contextmanager
    def fixture(self, fixed):
        scratch = ROOT / ".incord/temp"
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            uploads = Path(directory) / "uploads"
            uploads.mkdir()
            private = ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
            private.hits, private.content = 0, b"internal-private-canary"
            with running(private) as private_origin:
                public = ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
                public.hits, public.content, public.redirect = 0, b"ordinary-public-content", private_origin + "/canary"
                with running(public) as public_origin:
                    app = Fixture(uploads, fixed, public_origin, private_origin)
                    with running(app) as origin:
                        yield app, origin, public, private, Path(directory)

    # A helper logs in through the real endpoint and preserves its issued CSRF/cookie values.
    def login(self, origin, user="alice"):
        status, body, cookie = request(origin, "/login", {"user": user, "password": user + "-lab-pass"})
        self.assertEqual(status, 200)
        return cookie, body["csrf"]

    def test_authentication_revocation_and_expiry_over_http(self):
        for fixed in (False, True):
            with self.subTest(fixed=fixed), self.fixture(fixed) as (app, origin, *_):
                self.assertEqual(request(origin, "/me")[0], 401)
                wrong = request(origin, "/login", {"user": "alice", "password": "wrong"})
                self.assertEqual(wrong[0], 403 if fixed else 200)
                # Observe whether the wrong password actually issued a usable credential.
                if fixed:
                    self.assertFalse(wrong[2])
                else:
                    self.assertEqual(request(origin, "/me", cookie=wrong[2])[1]["user"], "alice")
                cookie, csrf = self.login(origin)
                self.assertEqual(request(origin, "/me", cookie=cookie)[1]["user"], "alice")
                self.assertEqual(request(origin, "/logout", {}, cookie, csrf)[0], 200)
                self.assertEqual(request(origin, "/me", cookie=cookie)[0], 403 if fixed else 200)
                cookie, _ = self.login(origin)
                app.now = 60
                self.assertEqual(request(origin, "/me", cookie=cookie)[0], 403)

    def test_object_authorization_csrf_and_persistent_record_effect(self):
        for fixed in (False, True):
            with self.subTest(fixed=fixed), self.fixture(fixed) as (app, origin, *_):
                alice, csrf = self.login(origin)
                bob, bob_csrf = self.login(origin, "bob")
                status, body, _ = request(origin, "/records/bob", cookie=alice)
                self.assertEqual(status, 403 if fixed else 200)
                self.assertEqual("bob-private-canary" in str(body), not fixed)
                # No proof, another session's proof and another origin all fail in fixed mode.
                for proof, site in [(None, "https://lab.invalid"), (bob_csrf, "https://lab.invalid"), (csrf, "https://evil.invalid")]:
                    self.assertEqual(request(origin, "/records/alice", {"record": "tampered"}, alice, proof, site)[0], 403 if fixed else 200)
                self.assertEqual(app.records["alice"], "alice-record" if fixed else "tampered")
                self.assertEqual(request(origin, "/records/alice", {"record": "legitimate"}, alice, csrf)[0], 200)
                self.assertEqual(app.records["alice"], "legitimate")
                self.assertEqual(request(origin, "/records/bob", {"record": "stolen"}, alice, csrf)[0], 403 if fixed else 200)
                self.assertEqual(app.records["bob"], "bob-private-canary" if fixed else "stolen")

    def test_ssrf_direct_and_redirect_canaries_with_real_transport(self):
        for fixed in (False, True):
            with self.subTest(fixed=fixed), self.fixture(fixed) as (app, origin, public, private, _):
                cookie, csrf = self.login(origin)
                status, body, _ = request(origin, "/fetch", {"url": app.public_origin + "/content"}, cookie, csrf)
                self.assertEqual((status, body["body"]), (200, "ordinary-public-content"))
                for url in [app.private_origin + "/canary", app.public_origin + "/redirect"]:
                    status, body, _ = request(origin, "/fetch", {"url": url}, cookie, csrf)
                    self.assertEqual(status, 403 if fixed else 200)
                    self.assertEqual("internal-private-canary" in str(body), not fixed)
                self.assertEqual(private.hits, 0 if fixed else 2)

    def test_upload_traversal_type_size_download_and_owner_boundaries(self):
        for fixed in (False, True):
            with self.subTest(fixed=fixed), self.fixture(fixed) as (app, origin, public, private, directory):
                alice, csrf = self.login(origin)
                bob, _ = self.login(origin, "bob")
                upload = {"name": "note.txt", "type": "text/plain", "content": "ordinary-text"}
                # Both modes still require authentication; the positive defects are ownership and file policy.
                self.assertEqual(request(origin, "/upload", upload, multipart=True)[0], 403)
                status, body, _ = request(origin, "/upload", upload, alice, csrf, multipart=True)
                self.assertEqual(status, 201)
                self.assertEqual(body["stored_name"] == "note.txt", not fixed)
                self.assertEqual(request(origin, "/files/" + body["id"], cookie=alice)[1]["content"], "ordinary-text")
                self.assertEqual(request(origin, "/files/" + body["id"], cookie=bob)[0], 403 if fixed else 200)
                for name, kind, content in [("../escaped.txt", "text/plain", "escape-canary"), ("payload.php", "text/plain", "not-executed"), ("wrong.txt", "image/png", "fake-type"), ("large.txt", "text/plain", "x" * 65537)]:
                    status, _, _ = request(origin, "/upload", {"name": name, "type": kind, "content": content}, alice, csrf, multipart=True)
                    self.assertEqual(status, 400 if fixed else 201)
                self.assertEqual((directory / "escaped.txt").exists(), not fixed)


if __name__ == "__main__":
    unittest.main()
