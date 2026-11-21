// postinstall: download this version's prebuilt fastpilot (see lib/download.js). npm 12 skips
// install scripts unless they are allowed; then bin/fastpilot.js does the same on the first run.
"use strict";
const { ensure } = require("./lib/download");

ensure((line) => console.error(line)).catch((e) => {
  console.error(`fastpilot: ${e.message}`);
  process.exit(1);
});
