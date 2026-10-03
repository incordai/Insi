// Which release archive this machine needs, and how it is checked.
//
// The archive for a release is `insi-v<version>-<target>.tar.gz` on the
// GitHub release `v<version>`. Its SHA-256 is pinned in checksums.json, which
// the release workflow writes into this package before it is published: the
// archive must match what was built, not merely what the download server says.

"use strict";

const crypto = require("crypto");
const fs = require("fs");
const path = require("path");

const REPO = "incordai/insi";

/** The Rust target for a Node platform and arch, or null when none is built. */
function target(platform, arch) {
  const table = {
    "linux-x64": "x86_64-unknown-linux-gnu",
    "linux-arm64": "aarch64-unknown-linux-gnu",
    "darwin-x64": "x86_64-apple-darwin",
    "darwin-arm64": "aarch64-apple-darwin",
    "win32-x64": "x86_64-pc-windows-msvc",
  };
  return table[`${platform}-${arch}`] || null;
}

function archiveName(version, tgt) {
  return `insi-v${version}-${tgt}.tar.gz`;
}

function binaryName(platform) {
  return platform === "win32" ? "insi.exe" : "insi";
}

/** Where archives come from: INSI_DOWNLOAD_BASE (a URL or a folder, for a
 * mirror or a test), else the GitHub release. */
function base(version, env) {
  return env.INSI_DOWNLOAD_BASE || `https://github.com/${REPO}/releases/download/v${version}`;
}

function sha256(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

/** The pinned digest for an archive; throws when there is none, since an
 * archive nothing vouches for is not installed. */
function expected(checksums, name) {
  const want = checksums && checksums[name];
  if (typeof want !== "string" || !/^[0-9a-f]{64}$/.test(want)) {
    throw new Error(`no pinned SHA-256 for ${name} in checksums.json`);
  }
  return want;
}

/** The tar to run. On Windows, the system's own bsdtar: a GNU tar earlier
 * on PATH (Git's) reads `C:\...` as a remote `host:path` and fails. */
function tarCommand(platform, env) {
  if (platform !== "win32") return "tar";
  const root = env.SystemRoot || env.SYSTEMROOT || "C:\\Windows";
  return path.join(root, "System32", "tar.exe");
}

function readChecksums(dir) {
  const file = path.join(dir, "checksums.json");
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (e) {
    throw new Error(`cannot read ${file}: ${e.message}`);
  }
}

module.exports = {
  target,
  archiveName,
  binaryName,
  base,
  sha256,
  expected,
  readChecksums,
  tarCommand,
};
