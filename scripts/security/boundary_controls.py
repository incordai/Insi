"""Reference boundaries for adversarial lab controls, not a production framework."""
import hmac
import hashlib
from http.cookies import SimpleCookie
import ipaddress
import secrets
import threading
from decimal import Decimal
from pathlib import Path
from urllib.parse import urlsplit


# Model session rotation, expiry, revocation and session-bound CSRF proof.
class Sessions:
    # Inject time so expiry tests never depend on wall-clock sleeps.
    def __init__(self, clock, lifetime=300):
        self.clock, self.lifetime, self.sessions = clock, lifetime, {}

    # Authentication rotates and revokes an old identifier rather than promoting it.
    def login(self, user, old=None):
        self.sessions.pop(old, None)
        token, csrf = secrets.token_urlsafe(32), secrets.token_urlsafe(32)
        self.sessions[token] = (user, self.clock() + self.lifetime, csrf)
        return token, csrf

    # Expired or unknown credentials fail closed.
    def user(self, token):
        row = self.sessions.get(token)
        if row is None or self.clock() >= row[1]:
            raise PermissionError("invalid session")
        return row[0]

    # Bind mutation proof to both the authenticated session and an exact origin.
    def csrf(self, token, supplied, origin):
        self.user(token)
        if origin != "https://lab.invalid" or not isinstance(supplied, str) or not hmac.compare_digest(self.sessions[token][2], supplied):
            raise PermissionError("invalid CSRF proof")

    # Logout revokes the server-side credential.
    def logout(self, token):
        self.sessions.pop(token, None)


# Use server-side identities and roles for every object/method authorization decision.
def authorize(user, role, owner, operation):
    if not user or operation not in ("read", "update", "delete") or (user != owner and role != "admin"):
        raise PermissionError("object access denied")


# Lab parameters exercise salted password verification, not a production password-cost recommendation.
def password_record(password):
    salt = secrets.token_bytes(16)
    return salt, hashlib.scrypt(password.encode(), salt=salt, n=16384, r=8, p=1)


# Issue a session only after verifying credentials; missing users receive the same public failure.
def authenticate(credentials, user, password, sessions):
    row = credentials.get(user)
    salt = row[0] if row else bytes(16)
    candidate = hashlib.scrypt(password.encode(), salt=salt, n=16384, r=8, p=1)
    if row is None or not hmac.compare_digest(row[1], candidate):
        raise PermissionError("invalid credentials")
    return sessions.login(user)


# Host-only cookies prevent subdomain scope; attributes complement server-side revocation.
def session_cookie(token):
    cookie = SimpleCookie()
    cookie["__Host-insi-lab"] = token
    cookie["__Host-insi-lab"]["secure"] = True
    cookie["__Host-insi-lab"]["httponly"] = True
    cookie["__Host-insi-lab"]["samesite"] = "Strict"
    cookie["__Host-insi-lab"]["path"] = "/"
    return cookie.output(header="").strip()


# Resolve once, validate all answers and pass a pinned numeric destination to the transport.
# The transport preserves TLS hostname checks and does not follow redirects.
def fetch_public(url, resolve, dial):
    parsed = urlsplit(url)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.port not in (None, 443):
        raise ValueError("HTTPS public-origin requests only")
    addresses = list(resolve(parsed.hostname))
    if not addresses:
        raise ValueError("unresolved host")
    for address in addresses:
        ip = ipaddress.ip_address(address)
        mapped = getattr(ip, "ipv4_mapped", None)
        # Multicast and scoped addresses must not bypass the public-unicast requirement.
        if not ip.is_global or ip.is_multicast or ip.is_reserved or "%" in address or (mapped is not None and not mapped.is_global):
            raise ValueError("non-public destination")
    # Preserve the requested query while pinning the network destination and TLS hostname.
    path = (parsed.path or "/") + ("?" + parsed.query if parsed.query else "")
    status, body = dial(addresses[0], parsed.hostname, path)
    if status != 200 or len(body) > 65536:
        raise ValueError("redirect, failed response or response limit")
    return body


# Narrow text-only policy stores randomized names outside any served directory.
def store_text_upload(directory, filename, media_type, payload):
    if not filename or Path(filename).name != filename or "/" in filename or "\\" in filename or "\x00" in filename:
        raise ValueError("invalid upload name")
    if not filename.endswith(".txt") or media_type != "text/plain" or len(payload) > 65536:
        raise ValueError("unsupported upload")
    text = payload.decode("utf-8")
    if any(ord(char) < 32 and char not in "\n\r\t" for char in text):
        raise ValueError("binary payload refused")
    path = directory / (secrets.token_hex(16) + ".txt")
    with path.open("xb") as stream:
        stream.write(payload)
    return path


# A transaction serializes balance changes and idempotency receipts together.
class Ledger:
    # Isolated in-memory accounts contain only lab values.
    def __init__(self):
        self.balances = {"alice": Decimal("100"), "bob": Decimal("0")}
        self.receipts, self.lock = {}, threading.Lock()

    # Reject invalid amounts and conflicting replays before changing account balances.
    def transfer(self, user, recipient, amount, key):
        value = Decimal(amount)
        if not value.is_finite() or value <= 0 or value.as_tuple().exponent < -2 or not key:
            raise ValueError("invalid transfer")
        fingerprint = (recipient, value)
        with self.lock:
            receipt = self.receipts.get((user, key))
            if receipt:
                if receipt != fingerprint:
                    raise ValueError("conflicting replay")
                return
            if user not in self.balances or recipient not in self.balances or self.balances[user] < value:
                raise PermissionError("insufficient funds or unknown account")
            self.balances[user] -= value
            self.balances[recipient] += value
            self.receipts[user, key] = fingerprint
