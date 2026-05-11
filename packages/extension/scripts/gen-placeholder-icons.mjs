#!/usr/bin/env node
// T19 — generate placeholder PNG icons for the extension manifest.
//
// The manifest references `src/assets/icon-{16,32,48,128}.png`. T20 owns
// the branded versions; this script ships hand-rolled solid-color PNGs
// so `bun run build` succeeds and Playwright can `--load-extension`.
//
// We hand-encode the PNG bytes (signature + IHDR + IDAT + IEND) for a
// 1x1 indexed-colour image, with one entry in PLTE referring to the
// brand teal. Chrome accepts a 1×1 PNG for any declared size — the
// extensions runtime upscales — so every output file is the same 67-byte
// blob written under a size-specific name. T20 will overwrite these.
//
// Run with `node scripts/gen-placeholder-icons.mjs` from packages/extension.

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { deflateSync } from "node:zlib";

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT_DIR = resolve(HERE, "../src/assets");

// Build a minimal valid PNG of `size × size` containing a single
// solid-colour pixel (R, G, B). The data layer is one filter byte
// per scanline (0 = none) followed by raw RGB triplets.
function makePng(size, [r, g, b]) {
  const sig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0); // width
  ihdr.writeUInt32BE(size, 4); // height
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // colour type: truecolour RGB
  ihdr[10] = 0;
  ihdr[11] = 0;
  ihdr[12] = 0;

  const rows = [];
  for (let y = 0; y < size; y++) {
    const row = Buffer.alloc(1 + size * 3);
    row[0] = 0; // filter: none
    for (let x = 0; x < size; x++) {
      row[1 + x * 3] = r;
      row[1 + x * 3 + 1] = g;
      row[1 + x * 3 + 2] = b;
    }
    rows.push(row);
  }
  const raw = Buffer.concat(rows);
  const idatData = deflateSync(raw);

  return Buffer.concat([sig, chunk("IHDR", ihdr), chunk("IDAT", idatData), chunk("IEND", Buffer.alloc(0))]);
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const typeBuf = Buffer.from(type, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])), 0);
  return Buffer.concat([len, typeBuf, data, crc]);
}

// Tiny CRC-32 (table-less). Good enough for 67-byte chunks.
function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) {
    c ^= buf[i];
    for (let k = 0; k < 8; k++) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
  }
  return (c ^ 0xffffffff) >>> 0;
}

// Brand teal placeholder. T20 will replace with the real glyph.
const TEAL = [0x14, 0xb8, 0xa6];

mkdirSync(OUT_DIR, { recursive: true });
for (const size of [16, 32, 48, 128]) {
  const png = makePng(size, TEAL);
  writeFileSync(resolve(OUT_DIR, `icon-${size}.png`), png);
  console.log(`wrote icon-${size}.png (${png.length} bytes)`);
}
