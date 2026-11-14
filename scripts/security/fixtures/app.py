"""Disposable SQL-injection control target, only for an isolated lab network."""
import json
import os
import sqlite3
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlsplit


# In-memory data ensures no production data or host files enter a lab test.
class Target(BaseHTTPRequestHandler):
    # Select the vulnerable or parameterized query while keeping all other behavior identical.
    def do_GET(self):
        name = parse_qs(urlsplit(self.path).query).get("name", [""])[0]
        with sqlite3.connect(":memory:") as db:
            db.execute("CREATE TABLE users (name TEXT, note TEXT)")
            db.executemany("INSERT INTO users VALUES (?, ?)", [("public", "hello"), ("private", "lab-canary")])
            if os.environ.get("LAB_MODE") == "fixed":
                rows = db.execute("SELECT note FROM users WHERE name = ?", (name,)).fetchall()
            else:
                # Deliberately vulnerable positive control, confined to disposable test images.
                rows = db.execute("SELECT note FROM users WHERE name = '" + name + "'").fetchall()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(rows).encode())

    # Avoid logging injected request strings into benchmark artifacts.
    def log_message(self, *_args):
        pass


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 8080), Target).serve_forever()
