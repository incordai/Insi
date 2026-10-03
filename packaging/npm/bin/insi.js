#!/usr/bin/env node
// Runs the insi binary installed beside this file, with the same arguments,
// streams and exit code.

"use strict";

const path = require("path");
const { spawnSync } = require("child_process");

const bin = path.join(__dirname, process.platform === "win32" ? "insi.exe" : "insi");
const run = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
if (run.error) {
  console.error(`insi: ${run.error.message} (reinstall the package to download the binary again)`);
  process.exit(1);
}
process.exit(run.status === null ? 1 : run.status);
