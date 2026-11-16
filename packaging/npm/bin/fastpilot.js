#!/usr/bin/env node
// Runs the prebuilt binary that install.js unpacked, with this process's arguments and stdio
// (the MCP server talks over stdin/stdout, so they are passed through untouched).
"use strict";
const path = require("path");
const { spawnSync } = require("child_process");

const bin = path.join(__dirname, "..", "vendor", "fastpilot");
const r = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
if (r.error) {
  console.error(`fastpilot: ${r.error.message} (reinstall the package, or set FASTPILOT_SKIP_DOWNLOAD=1 and put your own build on PATH)`);
  process.exit(1);
}
process.exit(r.status === null ? 1 : r.status);
