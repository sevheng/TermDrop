#!/usr/bin/env node
// Bump the application version across package.json, Cargo.toml, and tauri.conf.json.
// Usage: node scripts/bump-version.mjs <version>
// Example: node scripts/bump-version.mjs 0.3.1

import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

const ROOT = resolve(import.meta.dirname, "..");
const FILES = {
  packageJson: resolve(ROOT, "package.json"),
  cargoToml: resolve(ROOT, "src-tauri/Cargo.toml"),
  tauriConf: resolve(ROOT, "src-tauri/tauri.conf.json"),
};

const newVersion = process.argv[2];

if (!newVersion || !/^\d+\.\d+\.\d+/.test(newVersion)) {
  console.error("Usage: node scripts/bump-version.mjs <version>");
  console.error("Example: node scripts/bump-version.mjs 0.3.1");
  process.exit(1);
}

async function updateJson(path, key) {
  const content = JSON.parse(await readFile(path, "utf8"));
  const old = content[key];
  content[key] = newVersion;
  await writeFile(path, JSON.stringify(content, null, 2) + "\n");
  console.log(`Updated ${path}: ${old} -> ${newVersion}`);
}

async function updateTomlVersion(path) {
  let content = await readFile(path, "utf8");
  const match = content.match(/^version\s*=\s*"([^"]+)"/m);
  const old = match ? match[1] : "unknown";
  content = content.replace(/^version\s*=\s*"[^"]+"/m, `version = "${newVersion}"`);
  await writeFile(path, content);
  console.log(`Updated ${path}: ${old} -> ${newVersion}`);
}

await updateJson(FILES.packageJson, "version");
await updateTomlVersion(FILES.cargoToml);
await updateJson(FILES.tauriConf, "version");

console.log("\nDone. Remember to rebuild so Cargo.lock picks up the new package version.");
