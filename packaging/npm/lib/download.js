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

// Bounded (Security review of #121, Low 4): a hung download would look like a dead MCP server.
const LIMIT = 150 * 1024 * 1024;
const seconds = () => Number(process.env.FASTPILOT_DOWNLOAD_TIMEOUT) || 120;

/// Plain http only for this computer (a local mirror); anything else must be https.
function checkBase(base) {
  let u;
  try {
    u = new URL(base);
  } catch (_) {
    throw new Error(`FASTPILOT_DOWNLOAD_BASE is not a URL: ${base}`);
  }
  const local = ["localhost", "127.0.0.1", "[::1]"].includes(u.hostname);
  if (u.protocol !== "https:" && !(u.protocol === "http:" && local)) {
    throw new Error(`FASTPILOT_DOWNLOAD_BASE must use https (plain http only for localhost): ${base}`);
  }
}

// A connection cut on the way (ECONNRESET, ETIMEDOUT, "fetch failed", a body cut short) is tried
// again, TRIES times in all, after a short pause. Some IPv6 paths (NAT64) reset Node's download
// from GitHub about half the time while IPv4 works (v0.6.0, the user's Mac), so after the first
// cut every later connection of this process looks up IPv4 addresses first. Not tried again: an
// HTTP error, a file over LIMIT, or the overall timeout (a hung download stays bounded).
const TRIES = 3;
const pause = (n) => new Promise((r) => setTimeout(r, 1000 * n));

/// A network error, marked to be tried again, with its plain reason ("ECONNRESET", …).
function cut(e) {
  const c = e && e.cause;
  return Object.assign(new Error(e.message), { cut: (c && (c.code || c.message)) || e.code || e.message });
}

async function get(url, log) {
  for (let n = 1; ; n++) {
    try {
      return await getOnce(url);
    } catch (e) {
      if (!e.cut) throw e;
      if (n >= TRIES) {
        throw new Error(`could not download ${url} (${e.cut}, ${TRIES} tries). Check the network connection and try again: npm install -g fastpilot, or run fastpilot again`);
      }
      require("dns").setDefaultResultOrder("ipv4first");
      log(`fastpilot: the download was cut (${e.cut}); trying again over IPv4 (${n + 1} of ${TRIES})…`);
      await pause(n);
    }
  }
}

async function getOnce(url) {
  let res;
  try {
    res = await fetch(url, { redirect: "follow", signal: AbortSignal.timeout(seconds() * 1000) });
  } catch (e) {
    if (e.name === "TimeoutError" || e.name === "AbortError") throw new Error(`${url}: no answer within ${seconds()} s; check the network and run fastpilot again`);
    throw cut(e);
  }
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  if (Number(res.headers.get("content-length")) > LIMIT) throw new Error(`${url}: larger than ${LIMIT / 1048576} MB; refused`);
  const chunks = [];
  let size = 0;
  try {
    for await (const c of res.body) {
      size += c.length;
      if (size > LIMIT) throw new Error(`${url}: larger than ${LIMIT / 1048576} MB; refused`);
      chunks.push(c);
    }
  } catch (e) {
    if (e.name === "TimeoutError" || e.name === "AbortError") throw new Error(`${url}: no answer within ${seconds()} s; check the network and run fastpilot again`);
    if (size > LIMIT) throw e;
    throw cut(e);
  }
  return Buffer.concat(chunks);
}

/// The words for a folder fastpilot may not write to (a global npm prefix owned by root).
function readOnly(e) {
  return new Error(
    `cannot write ${vendor} (${e.code}). npm did not run fastpilot's install step, and this folder is read-only for you.\n` +
      "Fix it one of these ways:\n" +
      "  - allow fastpilot's install step, then install again:\n" +
      "      npm config set allow-scripts=fastpilot --location=user\n" +
      "      npm install -g fastpilot\n" +
      `  - or install without npm: curl -fsSL https://raw.githubusercontent.com/autobot-lab/fastpilot/v${version}/scripts/install.sh | sh`
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
  checkBase(base);
  const name = `fastpilot-${version}-${target}`;
  log(`fastpilot: downloading fastpilot ${version} for ${target} (once)…`);
  const archive = await get(`${base}/v${version}/${name}.tar.gz`, log);
  const want = (await get(`${base}/v${version}/${name}.tar.gz.sha256`, log)).toString().split(/\s+/)[0];
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
