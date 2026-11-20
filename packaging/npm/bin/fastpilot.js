#!/usr/bin/env node
// Runs the prebuilt binary that install.js unpacked, with this process's arguments and stdio
// (the MCP server talks over stdin/stdout, so they are passed through untouched).
"use strict";
const path = require("path");
const { spawnSync } = require("child_process");

const bin = path.join(__dirname, "..", "vendor", "fastpilot");
// Under `npx`, MCP configs written by `fastpilot demo` / `init` should start `npx -y fastpilot mcp`.
const env = process.env.npm_command === "exec" ? { ...process.env, FASTPILOT_VIA_NPX: "1" } : process.env;
const r = spawnSync(bin, process.argv.slice(2), { stdio: "inherit", env });
if (r.error) {
  console.error(`fastpilot: ${r.error.message} (reinstall the package, or set FASTPILOT_SKIP_DOWNLOAD=1 and put your own build on PATH)`);
  process.exit(1);
}
process.exit(r.status === null ? 1 : r.status);
