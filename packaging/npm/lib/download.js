// The prebuilt fastpilot for this platform, from the GitHub release of this package's version:
// downloaded, checked against its SHA-256 and unpacked into ../vendor. Used by install.js
// (postinstall) and by bin/fastpilot.js on the first run when the install scripts did not run
// (npm 12 skips them unless they are allowed).
//
// - Every message goes to stderr: stdout belongs to the program (the MCP server talks on it).
// - Two first runs at once are safe: each writes its own temp file in vendor/ and renames it into
//   place (atomic); the second one finds the binary already there and uses it.
// - Later runs make no network request: the binary is there.
// FASTPILOT_DOWNLOAD_BASE: a mirror; FASTPILOT_SKIP_DOWNLOAD=1: never download (use your own build).
"use strict";
const crypto = require("crypto");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { execFileSync } = require("child_process");

const { version } = require("../package.json");
const root = path.join(__dirname, "..");
const vendor = path.join(root, "vendor");
const bin = path.join(vendor, "fastpilot");
const stamp = path.join(vendor, "VERSION");
const targets = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
};

/// The binary is there, for this package's version (a binary without a VERSION file came from
/// an older install script of this same package folder, so it is this version too).
function ready() {
  if (!fs.existsSync(bin)) return false;
  try {
    return fs.readFileSync(stamp, "utf8").trim() === version;
  } catch (_) {
    return true;
  }
}

async function get(url) {
  const res = await fetch(url, { redirect: "follow" });
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  return Buffer.from(await res.arrayBuffer());
}

/// The words for a folder fastpilot may not write to (a global npm prefix owned by root).
function readOnly(e) {
  return new Error(
    `cannot write ${vendor} (${e.code}). npm did not run fastpilot's install step, and this folder is read-only for you.\n` +
      "Fix it one of these ways:\n" +
      "  - allow fastpilot's install step, then install again:\n" +
      "      npm config set allow-scripts=fastpilot --location=user\n" +
      "      npm install -g fastpilot\n" +
      "  - or install without npm: curl -fsSL https://raw.githubusercontent.com/autobot-lab/fastpilot/main/scripts/install.sh | sh"
  );
}

function writable(fn) {
  try {
    return fn();
  } catch (e) {
    if (["EACCES", "EPERM", "EROFS"].includes(e.code)) throw readOnly(e);
    throw e;
  }
}

/// Download, verify and place the binary unless it is there; `log` gets the lines for people.
async function ensure(log) {
  if (ready()) return bin;
  if (process.env.FASTPILOT_SKIP_DOWNLOAD === "1") {
    throw new Error("the fastpilot program is not here and FASTPILOT_SKIP_DOWNLOAD=1 is set: put your own build on PATH, or unset it");
  }
  const target = targets[`${process.platform}-${process.arch}`];
  if (!target) {
    throw new Error(`no prebuilt fastpilot for ${process.platform}-${process.arch}; build it from source: https://github.com/autobot-lab/fastpilot#build`);
  }
  writable(() => fs.mkdirSync(vendor, { recursive: true }));
  const base = process.env.FASTPILOT_DOWNLOAD_BASE || "https://github.com/autobot-lab/fastpilot/releases/download";
  const name = `fastpilot-${version}-${target}`;
  log(`fastpilot: downloading fastpilot ${version} for ${target} (once)…`);
  const archive = await get(`${base}/v${version}/${name}.tar.gz`);
  const want = (await get(`${base}/v${version}/${name}.tar.gz.sha256`)).toString().split(/\s+/)[0];
  const got = crypto.createHash("sha256").update(archive).digest("hex");
  if (want !== got) throw new Error(`checksum mismatch for ${name}.tar.gz (expected ${want}, got ${got}); nothing was installed`);
  if (ready()) return bin; // another first run finished meanwhile
  const unpack = fs.mkdtempSync(path.join(os.tmpdir(), "fastpilot-"));
  const part = path.join(vendor, `.fastpilot-${process.pid}-${crypto.randomBytes(4).toString("hex")}.part`);
  const partStamp = `${part}.version`;
  try {
    const file = path.join(unpack, `${name}.tar.gz`);
    fs.writeFileSync(file, archive);
    execFileSync("tar", ["-xzf", file, "-C", unpack]);
    writable(() => {
      fs.copyFileSync(path.join(unpack, name, "fastpilot"), part);
      fs.chmodSync(part, 0o755);
      fs.writeFileSync(partStamp, `${version}\n`);
      fs.renameSync(part, bin);
      fs.renameSync(partStamp, stamp);
    });
  } finally {
    fs.rmSync(unpack, { recursive: true, force: true });
    fs.rmSync(part, { force: true });
    fs.rmSync(partStamp, { force: true });
  }
  log(`fastpilot: fastpilot ${version} (${target}) installed`);
  return bin;
}

module.exports = { ensure, bin, vendor, version };
