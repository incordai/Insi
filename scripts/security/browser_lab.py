# Copyright (c) 2026-2027 Incord Ai.
# https://incord.ai
#
# SPDX-License-Identifier: BUSL-1.1

"""Loopback-only reflected-HTML controls for visible browser verification."""
import argparse
import html
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit


# The unsafe route is a deliberate local positive; fixed output is escaped and constrained by CSP.
class BrowserLab(BaseHTTPRequestHandler):
    hits = {}
    lock = threading.Lock()
    # Never load a third-party script, image or target as part of a canary check.
    def do_GET(self):
        parsed = urlsplit(self.path)
        # A local canary endpoint permits verification without weakening CSP for inspection scripts.
        if parsed.path in ("/hit", "/status"):
            tag = parse_qs(parsed.query).get("tag", [""])[0][:32]
            with self.lock:
                if parsed.path == "/hit" and (tag in self.hits or len(self.hits) < 100):
                    self.hits[tag] = self.hits.get(tag, 0) + 1
                count = self.hits.get(tag, 0)
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({"tag": tag, "hits": count}).encode())
            return
        fixed = parsed.path == "/fixed"
        value = parse_qs(parsed.query).get("value", ["ordinary text"])[0]
        content = html.escape(value, quote=True) if fixed else value
        body = ("<!doctype html><title>Insi local browser control</title><h1>" + ("Fixed" if fixed else "Vulnerable") + " control</h1><div id=content>" + content + "</div>").encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("X-Content-Type-Options", "nosniff")
        if fixed:
            self.send_header("Content-Security-Policy", "default-src 'none'; base-uri 'none'; frame-ancestors 'none'")
        self.end_headers()
        self.wfile.write(body)

    # Payload strings never enter persistent server logs.
    def log_message(self, *_args):
        pass


# Explicit loopback binding prevents the vulnerable positive from exposing a LAN service.
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=0)
    args = parser.parse_args()
    server = ThreadingHTTPServer(("127.0.0.1", args.port), BrowserLab)
    print("http://127.0.0.1:" + str(server.server_port), flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
