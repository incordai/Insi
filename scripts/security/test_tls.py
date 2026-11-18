# Copyright (c) 2026-2027 Incord Ai.
# https://incord.ai
#
# SPDX-License-Identifier: BUSL-1.1

"""Real TLS handshake controls: trusted host, unknown issuer and hostname mismatch."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import shutil
import ssl
import subprocess
import tempfile
import threading
import unittest
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[2]


# No remote host or real credentials are used by the TLS fixture.
class Handler(BaseHTTPRequestHandler):
    # A small successful response distinguishes a completed TLS request from an exception.
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"local-canary")

    # Keep ephemeral fixture request data out of persistent logs.
    def log_message(self, *_args):
        pass


# Generate a temporary lab CA/certificate and test actual client verification decisions.
@unittest.skipUnless(shutil.which("openssl"), "OpenSSL is not installed")
class TLSTests(unittest.TestCase):
    def test_trust_and_hostname_are_both_required(self):
        scratch = ROOT / ".incord/temp"
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            cert, key = Path(directory) / "cert.pem", Path(directory) / "key.pem"
            subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1", "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost", "-keyout", str(key), "-out", str(cert)], capture_output=True, timeout=30, check=True)
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.minimum_version = ssl.TLSVersion.TLSv1_2
            context.load_cert_chain(cert, key)
            server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
            server.socket = context.wrap_socket(server.socket, server_side=True)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            url = f"https://localhost:{server.server_port}/"
            try:
                trusted = ssl.create_default_context(cafile=str(cert))
                with urllib.request.urlopen(url, context=trusted, timeout=5) as response:
                    self.assertEqual(response.read(), b"local-canary")
                with self.assertRaises(urllib.error.URLError):
                    urllib.request.urlopen(url, context=ssl.create_default_context(), timeout=5)
                with self.assertRaises(urllib.error.URLError):
                    urllib.request.urlopen(f"https://127.0.0.1:{server.server_port}/", context=trusted, timeout=5)
                # The deliberately insecure client demonstrates why disabling verification is unsafe.
                with urllib.request.urlopen(url, context=ssl._create_unverified_context(), timeout=5) as response:
                    self.assertEqual(response.status, 200)
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=5)


if __name__ == "__main__":
    unittest.main()
