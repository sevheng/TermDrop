#!/usr/bin/env node
// Regenerate every app icon asset from the two SVG masters in src-tauri/icons/src.
// Usage: npm run icons
//
// Two masters exist because one drawing cannot do both jobs: the full mark
// (tile > terminal window > droplet > prompt) is correct at 128px and turns to
// mud at 32px, so everything at or below SMALL_MAX is rendered from the
// chrome-free variant instead. `tauri icon` produces the bulk of the set from
// the full master; this script then substitutes the small variant into the
// sizes that need it and rebuilds the .icns / .ico containers around the mix.

import { Resvg } from "@resvg/resvg-js";
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, rmSync, copyFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

const ROOT = resolve(import.meta.dirname, "..");
const ICONS = resolve(ROOT, "src-tauri/icons");
const FULL_SVG = resolve(ICONS, "src/icon.svg");
const SMALL_SVG = resolve(ICONS, "src/icon-small.svg");

// At or below this, use the chrome-free variant.
const SMALL_MAX = 64;

const fullSrc = readFileSync(FULL_SVG, "utf8");
const smallSrc = readFileSync(SMALL_SVG, "utf8");

/** Rasterize the variant appropriate for `size` to a PNG buffer. */
const renderCache = new Map();
function render(size) {
  let png = renderCache.get(size);
  if (!png) {
    const svg = size <= SMALL_MAX ? smallSrc : fullSrc;
    png = new Resvg(svg, { fitTo: { mode: "width", value: size } }).render().asPng();
    renderCache.set(size, png);
  }
  return png;
}

// --- 1. let the Tauri CLI lay down the standard set ------------------------
// This is what replaces the old generate_icons.py: it emits a modern,
// Retina-aware .icns instead of the legacy png2icns output, plus the Windows
// Store logos, all from one source png.
//
// The source is staged outside the icons directory on purpose. The CLI writes
// its own icons/icon.png (at 512, the largest standard hicolor size, which is
// the slot bundle.icon uses it for), so handing it a file at that path would
// mean it silently overwrote its own input.
const stagedSource = resolve(tmpdir(), "termdrop-icon-source.png");
writeFileSync(stagedSource, render(1024));

const npx = process.platform === "win32" ? "npx.cmd" : "npx";
execFileSync(npx, ["tauri", "icon", stagedSource], {
  cwd: ROOT,
  stdio: "inherit",
});
rmSync(stagedSource, { force: true });

// No mobile targets in bundle.targets, so drop what the CLI writes for them.
for (const dir of ["android", "ios"]) {
  rmSync(resolve(ICONS, dir), { recursive: true, force: true });
}

// --- 2. substitute the small variant where the full mark would not read ----
const SMALL_PNGS = {
  "32x32.png": 32,
  "64x64.png": 64,
  "Square30x30Logo.png": 30,
  "Square44x44Logo.png": 44,
};
for (const [name, size] of Object.entries(SMALL_PNGS)) {
  writeFileSync(resolve(ICONS, name), render(size));
}

// --- 3. rebuild icon.icns with one PNG member per macOS slot ---------------
// A pure-PNG icns (icp4/icp5/ic07..ic14) is read by everything since 10.7 and
// lets each slot pick its own variant, which patching the CLI's output would
// not. Layout: 'icns', big-endian total length, then type + big-endian
// member length (header included) + payload, repeated.
const ICNS_MEMBERS = [
  ["icp4", 16],
  ["icp5", 32],
  ["ic11", 32], // 16pt @2x
  ["ic12", 64], // 32pt @2x
  ["ic07", 128],
  ["ic13", 256], // 128pt @2x
  ["ic08", 256],
  ["ic14", 512], // 256pt @2x
  ["ic09", 512],
  ["ic10", 1024], // 512pt @2x
];

const icnsParts = ICNS_MEMBERS.map(([type, size]) => {
  const png = render(size);
  const header = Buffer.alloc(8);
  header.write(type, 0, "ascii");
  header.writeUInt32BE(png.length + 8, 4);
  return Buffer.concat([header, png]);
});
const icnsBody = Buffer.concat(icnsParts);
const icnsHeader = Buffer.alloc(8);
icnsHeader.write("icns", 0, "ascii");
icnsHeader.writeUInt32BE(icnsBody.length + 8, 4);
writeFileSync(resolve(ICONS, "icon.icns"), Buffer.concat([icnsHeader, icnsBody]));

// --- 4. rebuild icon.ico the same way -------------------------------------
// PNG-compressed entries throughout, matching what the Tauri CLI emits.
const ICO_SIZES = [16, 24, 32, 48, 64, 256];
const icoImages = ICO_SIZES.map(render);

const icoDir = Buffer.alloc(6 + ICO_SIZES.length * 16);
icoDir.writeUInt16LE(0, 0); // reserved
icoDir.writeUInt16LE(1, 2); // type: icon
icoDir.writeUInt16LE(ICO_SIZES.length, 4);

let offset = icoDir.length;
ICO_SIZES.forEach((size, i) => {
  const entry = 6 + i * 16;
  icoDir.writeUInt8(size >= 256 ? 0 : size, entry + 0); // 0 means 256
  icoDir.writeUInt8(size >= 256 ? 0 : size, entry + 1);
  icoDir.writeUInt8(0, entry + 2); // palette colours
  icoDir.writeUInt8(0, entry + 3); // reserved
  icoDir.writeUInt16LE(1, entry + 4); // colour planes
  icoDir.writeUInt16LE(32, entry + 6); // bits per pixel
  icoDir.writeUInt32LE(icoImages[i].length, entry + 8);
  icoDir.writeUInt32LE(offset, entry + 12);
  offset += icoImages[i].length;
});
writeFileSync(resolve(ICONS, "icon.ico"), Buffer.concat([icoDir, ...icoImages]));

// --- 5. favicon -----------------------------------------------------------
// The browser tab renders it at 16px, so it gets the small variant.
copyFileSync(SMALL_SVG, resolve(ROOT, "public/icon.svg"));

console.log("Icons regenerated from src-tauri/icons/src/*.svg");
