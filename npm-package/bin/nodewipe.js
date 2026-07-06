#!/usr/bin/env node
// Thin shim: forwards to the native binary downloaded by scripts/download-binary.js.
const path = require("path");
const { spawnSync } = require("child_process");

const binName = process.platform === "win32" ? "nodewipe.exe" : "nodewipe";
const binPath = path.join(__dirname, binName);

const result = spawnSync(binPath, process.argv.slice(2), { stdio: "inherit" });

if (result.error) {
  console.error(`Failed to run nodewipe binary: ${result.error.message}`);
  console.error("Try reinstalling: npm install -g nodewipe");
  process.exit(1);
}

process.exit(result.status ?? 1);
