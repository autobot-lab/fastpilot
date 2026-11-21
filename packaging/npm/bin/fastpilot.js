#!/usr/bin/env node
// Runs the prebuilt fastpilot binary with this process's arguments and stdio (the MCP server
// talks over stdin/stdout, so they are passed through untouched). When npm did not run the
// install step (npm 12 skips install scripts unless allowed), the first run downloads and checks
// the binary first; every message about that goes to stderr, never stdout. Later runs make no
// network request.
"use strict";
const { spawnSync } = require("child_process");
const { ensure } = require("../lib/download");

async function main() {
  let bin;
  try {
    bin = await ensure((line) => process.stderr.write(`${line}\n`));
  } catch (e) {
    process.stderr.write(`fastpilot: ${e.message}\n`);
    return 1;
  }
  // Under `npx`, MCP configs written by `fastpilot demo` / `init` should start `npx -y fastpilot mcp`.
  const env = process.env.npm_command === "exec" ? { ...process.env, FASTPILOT_VIA_NPX: "1" } : process.env;
  const r = spawnSync(bin, process.argv.slice(2), { stdio: "inherit", env });
  if (r.error) {
    process.stderr.write(`fastpilot: ${r.error.message} (reinstall the package, or set FASTPILOT_SKIP_DOWNLOAD=1 and put your own build on PATH)\n`);
    return 1;
  }
  return r.status === null ? 1 : r.status;
}

main().then((code) => process.exit(code));
