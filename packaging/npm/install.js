// postinstall: download this platform's insi binary, check it against the
// digest pinned in checksums.json, and put it in bin/.
//
// Only the one expected file is taken out of the archive, after extracting
// into a fresh folder of its own; nothing in the archive can choose where
// anything lands.

"use strict";

const fs = require("fs");
const https = require("https");
const os = require("os");
const path = require("path");
const { execFileSync } = require("child_process");
const p = require("./platform");

const pkg = require("./package.json");

function fetch(url, redirects = 5) {
  return new Promise((resolve, reject) => {
    https
      .get(url, { headers: { "User-Agent": `insi-npm/${pkg.version}` } }, (res) => {
        if ([301, 302, 303, 307, 308].includes(res.statusCode) && res.headers.location) {
          res.resume();
          if (redirects === 0) return reject(new Error(`too many redirects for ${url}`));
          return resolve(fetch(new URL(res.headers.location, url).toString(), redirects - 1));
        }
        if (res.statusCode !== 200) {
          res.resume();
          return reject(new Error(`${url}: HTTP ${res.statusCode}`));
        }
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () => resolve(Buffer.concat(chunks)));
        res.on("error", reject);
      })
      .on("error", reject);
  });
}

async function archive(baseUrl, name) {
  if (/^https:\/\//.test(baseUrl)) return fetch(`${baseUrl}/${name}`);
  if (/^[a-z]+:\/\//i.test(baseUrl)) throw new Error(`only https or a local folder: ${baseUrl}`);
  return fs.readFileSync(path.join(baseUrl, name));
}

async function install(env = process.env, here = __dirname) {
  const tgt = p.target(process.platform, process.arch);
  if (!tgt) {
    throw new Error(
      `no insi build for ${process.platform}-${process.arch}; build it with \`cargo install insi\``
    );
  }
  const name = p.archiveName(pkg.version, tgt);
  const want = p.expected(p.readChecksums(here), name);
  const bytes = await archive(p.base(pkg.version, env), name);
  const got = p.sha256(bytes);
  if (got !== want) {
    throw new Error(`${name}: SHA-256 ${got} is not the pinned ${want}; nothing was installed`);
  }
  const work = fs.mkdtempSync(path.join(os.tmpdir(), "insi-npm-"));
  try {
    const file = path.join(work, name);
    fs.writeFileSync(file, bytes);
    const out = path.join(work, "x");
    fs.mkdirSync(out);
    execFileSync(p.tarCommand(process.platform, process.env), ["-xzf", file, "-C", out], {
      stdio: "inherit",
    });
    const bin = p.binaryName(process.platform);
    const found = path.join(out, bin);
    if (!fs.existsSync(found) || !fs.lstatSync(found).isFile()) {
      throw new Error(`${name} holds no ${bin}`);
    }
    const dest = path.join(here, "bin", bin);
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    fs.copyFileSync(found, dest);
    fs.chmodSync(dest, 0o755);
    return dest;
  } finally {
    fs.rmSync(work, { recursive: true, force: true });
  }
}

if (require.main === module) {
  install().then(
    (dest) => console.log(`insi installed: ${dest}`),
    (e) => {
      console.error(`insi: ${e.message}`);
      process.exit(1);
    }
  );
}

module.exports = { install };
