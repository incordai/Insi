// The wrapper's checks, offline: a local folder stands in for the release.
// Run: node test.js

"use strict";

const assert = require("assert");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { execFileSync } = require("child_process");
const p = require("./platform");
const { install } = require("./install");

async function main() {
  assert.strictEqual(p.target("linux", "x64"), "x86_64-unknown-linux-gnu");
  assert.strictEqual(p.target("win32", "x64"), "x86_64-pc-windows-msvc");
  assert.strictEqual(p.target("win32", "ia32"), null);
  assert.throws(() => p.expected({}, "a.tar.gz"), /no pinned SHA-256/);
  assert.throws(() => p.expected({ "a.tar.gz": "short" }, "a.tar.gz"), /no pinned/);

  const tgt = p.target(process.platform, process.arch);
  if (!tgt) {
    console.log(`skipped the install test: no target for ${process.platform}-${process.arch}`);
    return;
  }
  const version = require("./package.json").version;
  const name = p.archiveName(version, tgt);
  const bin = p.binaryName(process.platform);

  // A release folder with an archive holding a stand-in binary.
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "insi-npm-test-"));
  const release = path.join(root, "release");
  const stage = path.join(root, "stage");
  fs.mkdirSync(release);
  fs.mkdirSync(stage);
  fs.writeFileSync(path.join(stage, bin), "stand-in binary");
  execFileSync(p.tarCommand(process.platform, process.env), [
    "-czf",
    path.join(release, name),
    "-C",
    stage,
    bin,
  ]);
  const digest = p.sha256(fs.readFileSync(path.join(release, name)));

  // The package as installed: its files, and checksums.json.
  const pkgDir = path.join(root, "pkg");
  fs.mkdirSync(pkgDir);
  const env = { INSI_DOWNLOAD_BASE: release };

  fs.writeFileSync(path.join(pkgDir, "checksums.json"), JSON.stringify({ [name]: "0".repeat(64) }));
  await assert.rejects(install(env, pkgDir), /is not the pinned/, "a wrong digest installs nothing");
  assert.ok(!fs.existsSync(path.join(pkgDir, "bin", bin)));

  fs.writeFileSync(path.join(pkgDir, "checksums.json"), JSON.stringify({ [name]: digest }));
  const dest = await install(env, pkgDir);
  assert.strictEqual(fs.readFileSync(dest, "utf8"), "stand-in binary");

  await assert.rejects(install({ INSI_DOWNLOAD_BASE: "http://example.invalid" }, pkgDir), /only https/);
  fs.rmSync(root, { recursive: true, force: true });
  console.log("npm wrapper: all checks passed");
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
