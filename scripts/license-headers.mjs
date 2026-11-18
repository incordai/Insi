#!/usr/bin/env node
// Copyright (c) 2026-2027 Incord Ai.
// https://incord.ai
//
// SPDX-License-Identifier: BUSL-1.1

// THE LICENSE HEADER ON EVERY CODE FILE (the owner, 2026-11-18): Insi is under the Business Source License 1.1
// (LICENSE), so each of its own code files opens with the copyright and SPDX lines below. The planted-fault fixtures
// of the security lab (`scripts/security/fixtures/`) are test inputs for the scanners and are left as they are. There
// is no vendored or third-party code in this repository; if some is ever added, list its folder in `SKIP`.
//
//   node scripts/license-headers.mjs          add the header where it is missing
//   node scripts/license-headers.mjs --check  list the files without it and exit 1 if any (changes nothing)
//
// A file whose first lines already carry an SPDX line is left as it is. A script's `#!` line stays first. Each file
// keeps its own line endings. The Rust half of the check is `crates/insi/tests/license_headers.rs`, run by
// `cargo test`.

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const LINES = ["Copyright (c) 2026-2027 Incord Ai.", "https://incord.ai", "", "SPDX-License-Identifier: BUSL-1.1"];
const SLASH = new Set([".rs", ".js", ".mjs", ".cjs", ".ts"]);
const HASH = new Set([".py", ".sh"]);
const SKIP = ["scripts/security/fixtures/"];

/** The header in a file's comment style, joined with its line ending. */
function header(ext, eol) {
  const mark = SLASH.has(ext) ? "//" : "#";
  return LINES.map((l) => (l ? `${mark} ${l}` : mark)).join(eol) + eol;
}

function extOf(path) {
  const dot = path.lastIndexOf(".");
  return dot < 0 ? "" : path.slice(dot);
}

/** Whether `text` already carries a license line near its top. */
function hasLicense(text) {
  return text.split(/\r?\n/, 12).some((l) => l.includes("SPDX-License-Identifier"));
}

/** A script's `#!` line, which must stay first. A Rust inner attribute (`#![...]`) is not one. */
function hasShebang(text) {
  return text.startsWith("#!") && !text.startsWith("#![");
}

/** `text` with the header added, after a `#!` line when there is one. */
function withHeader(text, ext) {
  const eol = text.includes("\r\n") ? "\r\n" : "\n";
  const head = header(ext, eol);
  if (hasShebang(text)) {
    const end = text.indexOf("\n") + 1;
    return text.slice(0, end) + head + eol + text.slice(end);
  }
  return head + eol + text;
}

const files = execFileSync("git", ["ls-files"], { cwd: root, encoding: "utf8" })
  .split("\n")
  .filter((f) => f && (SLASH.has(extOf(f)) || HASH.has(extOf(f))) && !SKIP.some((s) => f.startsWith(s)));

const check = process.argv.includes("--check");
const missing = [];
for (const file of files) {
  const path = join(root, file);
  const text = readFileSync(path, "utf8");
  if (hasLicense(text)) continue;
  missing.push(file);
  if (!check) writeFileSync(path, withHeader(text, extOf(file)));
}
if (check) {
  for (const file of missing) console.log(file);
  console.log(`${missing.length} of ${files.length} code files have no license header`);
  process.exit(missing.length ? 1 : 0);
}
console.log(`added the license header to ${missing.length} of ${files.length} code files`);
