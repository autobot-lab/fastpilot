// postinstall: download this version's prebuilt fastpilot for the platform from the GitLab
// package registry, check its SHA-256, and unpack it into ./vendor.
// FASTPILOT_DOWNLOAD_BASE: a mirror; FASTPILOT_SKIP_DOWNLOAD=1: skip (use your own binary).
"use strict";
const crypto = require("crypto");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { execFileSync } = require("child_process");

const { version } = require("./package.json");
const base = process.env.FASTPILOT_DOWNLOAD_BASE || "https://gitlab.com/api/v4/projects/88423537/packages/generic/fastpilot";
const targets = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
};

async function get(url) {
  const res = await fetch(url, { redirect: "follow" });
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  return Buffer.from(await res.arrayBuffer());
}

async function main() {
  if (process.env.FASTPILOT_SKIP_DOWNLOAD === "1") return;
  const target = targets[`${process.platform}-${process.arch}`];
  if (!target) {
    throw new Error(`no prebuilt fastpilot for ${process.platform}-${process.arch}; build it from source: https://gitlab.com/autobot-lab/fastpilot#build`);
  }
  const name = `fastpilot-${version}-${target}`;
  const archive = await get(`${base}/${version}/${name}.tar.gz`);
  const want = (await get(`${base}/${version}/${name}.tar.gz.sha256`)).toString().split(/\s+/)[0];
  const got = crypto.createHash("sha256").update(archive).digest("hex");
  if (want !== got) throw new Error(`checksum mismatch for ${name}.tar.gz (expected ${want}, got ${got})`);
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "fastpilot-"));
  try {
    const file = path.join(tmp, `${name}.tar.gz`);
    fs.writeFileSync(file, archive);
    execFileSync("tar", ["-xzf", file, "-C", tmp]);
    const vendor = path.join(__dirname, "vendor");
    fs.mkdirSync(vendor, { recursive: true });
    fs.copyFileSync(path.join(tmp, name, "fastpilot"), path.join(vendor, "fastpilot"));
    fs.chmodSync(path.join(vendor, "fastpilot"), 0o755);
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
  console.log(`fastpilot ${version} (${target}) installed`);
}

main().catch((e) => {
  console.error(`fastpilot: ${e.message}`);
  process.exit(1);
});
