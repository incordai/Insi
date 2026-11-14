"""Disposable authentication/API/upload target with intentionally vulnerable and fixed modes.

Bind only to loopback. SSRF probes may contact only the test-owned upstreams.
This is a security test target, not an application framework or deployable service.
"""
import http.client
from email.parser import BytesParser
from email import policy
from http.cookies import SimpleCookie
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import secrets
import threading
from urllib.parse import urlsplit
import urllib.request

from boundary_controls import Sessions, password_record, authenticate, store_text_upload


# Capture test state per server instance so parallel tests never share users, files or sessions.
class Fixture(ThreadingHTTPServer):
    # Upstream origins and upload directories are created and owned by the calling test.
    def __init__(self, directory, fixed, public_origin, private_origin):
        super().__init__(("127.0.0.1", 0), API)
        self.fixed = fixed
        self.directory = Path(directory)
        self.now = 0
        self.sessions = Sessions(lambda: self.now, 60)
        self.credentials = {user: password_record(user + "-lab-pass") for user in ("alice", "bob")}
        self.public_origin, self.private_origin = public_origin, private_origin
        self.files = {}
        self.records = {"alice": "alice-record", "bob": "bob-private-canary"}
        self.lock = threading.RLock()


# Fixed SSRF policy pins an exact allowed upstream and connects numerically without proxy or redirect use.
def fetch_fixture(server, url):
    parsed = urlsplit(url)
    origin = f"{parsed.scheme}://{parsed.netloc}"
    if server.fixed:
        if origin != server.public_origin or parsed.username or parsed.password or parsed.fragment:
            raise PermissionError("upstream denied")
        allowed = urlsplit(server.public_origin)
        connection = http.client.HTTPConnection("127.0.0.1", allowed.port, timeout=3)
        try:
            connection.request("GET", (parsed.path or "/") + ("?" + parsed.query if parsed.query else ""))
            response = connection.getresponse()
            if response.status != 200:
                raise PermissionError("redirect or upstream failure")
            body = response.read(65537)
        finally:
            connection.close()
    else:
        # Even the vulnerable positive is confined to the two test-owned origins.
        if origin not in (server.public_origin, server.private_origin):
            raise PermissionError("outside disposable lab")
        with urllib.request.urlopen(url, timeout=3) as response:
            body = response.read(65537)
    if len(body) > 65536:
        raise ValueError("upstream body limit")
    return body.decode()


# The same routes and data are used in both modes; only the security decisions differ.
class API(BaseHTTPRequestHandler):
    # Export a bounded JSON response, never an exception traceback or supplied credential.
    def reply(self, status, body, cookie=None):
        raw = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("X-Content-Type-Options", "nosniff")
        if cookie:
            self.send_header("Set-Cookie", cookie)
        self.end_headers()
        self.wfile.write(raw)

    # Read one bounded JSON request; oversized or malformed bodies never reach application handlers.
    def body(self):
        length = int(self.headers.get("Content-Length", "0"))
        if not 0 < length <= 100000:
            raise ValueError("invalid request length")
        raw = self.rfile.read(length)
        # Parse an actual multipart file part without the removed/deprecated cgi module.
        content_type = self.headers.get("Content-Type", "")
        if self.path == "/upload" and content_type.startswith("multipart/form-data;"):
            message = BytesParser(policy=policy.default).parsebytes(("Content-Type: " + content_type + "\r\nMIME-Version: 1.0\r\n\r\n").encode() + raw)
            parts = list(message.iter_parts())
            if message.defects or len(parts) != 1 or parts[0].is_multipart() or parts[0].get_param("name", header="content-disposition") != "file":
                raise ValueError("exactly one file part required")
            part = parts[0]
            return {"name": part.get_filename(), "type": part.get_content_type(), "content": part.get_payload(decode=True).decode("utf-8")}
        value = json.loads(raw)
        if not isinstance(value, dict):
            raise ValueError("object required")
        return value

    # A cookie is a credential, not an identity or a client-selected authorization role.
    def identity(self):
        cookie = SimpleCookie(self.headers.get("Cookie", ""))
        token = cookie["lab-session"].value if "lab-session" in cookie else ""
        return token, self.server.sessions.user(token)

    # Session checks happen for every protected route, including uploads and downloads.
    def do_GET(self):
        try:
            token, user = self.identity()
            path = urlsplit(self.path).path
            if path == "/me":
                self.reply(200, {"user": user})
            elif path.startswith("/records/"):
                owner = path.rsplit("/", 1)[-1]
                if self.server.fixed and owner != user:
                    raise PermissionError("object denied")
                self.reply(200, {"record": self.server.records[owner]})
            elif path.startswith("/files/"):
                file_id = path.rsplit("/", 1)[-1]
                owner, stored = self.server.files[file_id]
                if self.server.fixed and owner != user:
                    raise PermissionError("file denied")
                # Uploaded bytes are returned as JSON strings, never interpreted as active HTML.
                self.reply(200, {"content": stored.read_text()})
            else:
                self.reply(404, {"error": "not found"})
        except PermissionError:
            self.reply(401 if not self.headers.get("Cookie") else 403, {"error": "access denied"})
        except (ValueError, KeyError):
            self.reply(404, {"error": "not found"})

    # Exercise login, revocation, CSRF, SSRF and file policy through actual HTTP requests.
    def do_POST(self):
        try:
            data = self.body()
            with self.server.lock:
                if self.path == "/login":
                    user = data["user"]
                    if user not in self.server.credentials:
                        raise PermissionError("unknown user")
                    if self.server.fixed:
                        token, csrf = authenticate(self.server.credentials, user, data["password"], self.server.sessions)
                    else:
                        token, csrf = self.server.sessions.login(user)
                    flags = "; HttpOnly; SameSite=Strict" if self.server.fixed else ""
                    self.reply(200, {"csrf": csrf}, f"lab-session={token}; Path=/{flags}")
                    return
                token, user = self.identity()
                if self.server.fixed:
                    self.server.sessions.csrf(token, self.headers.get("X-CSRF-Token", ""), self.headers.get("Origin", ""))
                if self.path == "/logout":
                    if self.server.fixed:
                        self.server.sessions.logout(token)
                    self.reply(200, {"logged_out": True}, "lab-session=; Max-Age=0; Path=/")
                elif self.path.startswith("/records/"):
                    owner = self.path.rsplit("/", 1)[-1]
                    if self.server.fixed and owner != user:
                        raise PermissionError("object denied")
                    self.server.records[owner] = data["record"]
                    self.reply(200, {"updated": True})
                elif self.path == "/fetch":
                    self.reply(200, {"body": fetch_fixture(self.server, data["url"])})
                elif self.path == "/upload":
                    payload = data["content"].encode()
                    if self.server.fixed:
                        stored = store_text_upload(self.server.directory, data["name"], data["type"], payload)
                    else:
                        # The unsafe target demonstrates a real write outside its upload directory.
                        # Confine even this positive control to the test-owned parent directory.
                        stored = (self.server.directory / data["name"]).resolve()
                        stored.relative_to(self.server.directory.parent.resolve())
                        stored.write_bytes(payload)
                    file_id = secrets.token_hex(12)
                    self.server.files[file_id] = (user, stored)
                    self.reply(201, {"id": file_id, "stored_name": stored.name})
                else:
                    self.reply(404, {"error": "not found"})
        except PermissionError:
            self.reply(403, {"error": "access denied"})
        except (ValueError, KeyError, TypeError, OSError):
            self.reply(400, {"error": "invalid request"})

    # Never print credential cookies, request bodies or attacker-supplied URL strings.
    def log_message(self, *_args):
        pass
