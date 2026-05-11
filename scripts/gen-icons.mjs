#!/usr/bin/env node
// Generate branded Mnemonik icons (16/32/48/128 px) as PNG files using only
// Node stdlib (zlib + Buffer). No external image deps. Output: writes
// packages/extension/src/assets/icon-{16,32,48,128}.png.
//
// Design: dark navy background (#0A0F1E) with a soft rounded square; a teal
// (#00D4B4) stylised "M" mark — two diagonal strokes forming an M — centred.
// Pixel-art rendering done in a flat RGBA buffer, then encoded as PNG.
//
// Why hand-rolled? sharp / jimp / canvas are not installed in this env;
// installing them would touch package.json (T19 territory). Pure stdlib keeps
// the change surface minimal.

import { writeFileSync } from "node:fs";
import { deflateSync } from "node:zlib";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const OUT_DIR = resolve(__dirname, "../packages/extension/src/assets");

// Brand tokens.
const BG = [0x0a, 0x0f, 0x1e, 0xff]; // #0A0F1E
const FG = [0x00, 0xd4, 0xb4, 0xff]; // #00D4B4

const SIZES = [16, 32, 48, 128];

// ---- minimal PNG encoder ----
function crc32(buf) {
  let c;
  const table = crc32.table || (crc32.table = (() => {
    const t = new Uint32Array(256);
    for (let n = 0; n < 256; n++) {
      let v = n;
      for (let k = 0; k < 8; k++) v = (v & 1) ? (0xedb88320 ^ (v >>> 1)) : (v >>> 1);
      t[n] = v >>> 0;
    }
    return t;
  })());
  let crc = 0xffffffff;
  for (let i = 0; i < buf.length; i++) crc = table[(crc ^ buf[i]) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const typeBuf = Buffer.from(type, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])), 0);
  return Buffer.concat([len, typeBuf, data, crc]);
}

function encodePng(width, height, rgba) {
  // PNG signature.
  const sig = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

  // IHDR.
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr.writeUInt8(8, 8);   // bit depth
  ihdr.writeUInt8(6, 9);   // color type RGBA
  ihdr.writeUInt8(0, 10);  // compression
  ihdr.writeUInt8(0, 11);  // filter
  ihdr.writeUInt8(0, 12);  // interlace

  // IDAT: scanlines with filter byte 0 prepended per row.
  const stride = width * 4;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (stride + 1)] = 0; // filter: None
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, y * stride + stride);
  }
  const compressed = deflateSync(raw);

  return Buffer.concat([
    sig,
    chunk("IHDR", ihdr),
    chunk("IDAT", compressed),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ---- drawing ----
function makeIcon(size) {
  const buf = Buffer.alloc(size * size * 4);

  // Rounded-square radius scales with size; ~16% looks balanced.
  const radius = Math.max(1, Math.round(size * 0.16));

  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const off = (y * size + x) * 4;
      const inside = insideRoundedSquare(x, y, size, radius);
      const c = inside ? BG : [0, 0, 0, 0];
      buf[off] = c[0];
      buf[off + 1] = c[1];
      buf[off + 2] = c[2];
      buf[off + 3] = c[3];
    }
  }

  // Draw the "M" — four diagonal strokes forming the silhouette.
  // Geometry: M occupies a centred box with padding ~22%.
  const pad = Math.round(size * 0.22);
  const top = pad;
  const bot = size - pad;
  const left = pad;
  const right = size - pad;
  const mid = (left + right) / 2;
  const dip = top + (bot - top) * 0.55; // depth of inner V
  // Stroke half-thickness scales with size; floor 1px so 16x16 still renders.
  const half = Math.max(1, Math.round(size * 0.07));

  // Four line segments: left rise, left dip, right dip, right rise.
  const segments = [
    [left, bot, left, top],   // left vertical
    [left, top, mid, dip],    // left diagonal down
    [mid, dip, right, top],   // right diagonal up
    [right, top, right, bot], // right vertical
  ];

  for (const [x1, y1, x2, y2] of segments) {
    drawThickLine(buf, size, x1, y1, x2, y2, half, FG);
  }

  return buf;
}

function insideRoundedSquare(x, y, size, r) {
  // Treat pixel centres as (x+0.5, y+0.5) for nicer rounding.
  const cx = x + 0.5;
  const cy = y + 0.5;
  if (cx < r) {
    if (cy < r) return dist(cx, cy, r, r) <= r;
    if (cy > size - r) return dist(cx, cy, r, size - r) <= r;
  } else if (cx > size - r) {
    if (cy < r) return dist(cx, cy, size - r, r) <= r;
    if (cy > size - r) return dist(cx, cy, size - r, size - r) <= r;
  }
  return cx >= 0 && cx <= size && cy >= 0 && cy <= size;
}

function dist(ax, ay, bx, by) {
  const dx = ax - bx, dy = ay - by;
  return Math.sqrt(dx * dx + dy * dy);
}

function drawThickLine(buf, size, x1, y1, x2, y2, halfThick, color) {
  // Brute force: for each pixel in the bounding box, compute distance to the
  // line segment; if <= halfThick, paint. Slow but fine for 128x128.
  const minX = Math.max(0, Math.floor(Math.min(x1, x2) - halfThick - 1));
  const maxX = Math.min(size - 1, Math.ceil(Math.max(x1, x2) + halfThick + 1));
  const minY = Math.max(0, Math.floor(Math.min(y1, y2) - halfThick - 1));
  const maxY = Math.min(size - 1, Math.ceil(Math.max(y1, y2) + halfThick + 1));
  for (let y = minY; y <= maxY; y++) {
    for (let x = minX; x <= maxX; x++) {
      const d = pointSegmentDistance(x + 0.5, y + 0.5, x1, y1, x2, y2);
      if (d <= halfThick) {
        const off = (y * size + x) * 4;
        // Only paint if the underlying pixel is inside the rounded square
        // (i.e. opaque background). Skip transparent corners so the mark
        // doesn't bleed past the rounded edge.
        if (buf[off + 3] === 0) continue;
        buf[off] = color[0];
        buf[off + 1] = color[1];
        buf[off + 2] = color[2];
        buf[off + 3] = color[3];
      }
    }
  }
}

function pointSegmentDistance(px, py, ax, ay, bx, by) {
  const abx = bx - ax, aby = by - ay;
  const apx = px - ax, apy = py - ay;
  const ab2 = abx * abx + aby * aby;
  const t = ab2 === 0 ? 0 : Math.max(0, Math.min(1, (apx * abx + apy * aby) / ab2));
  const cx = ax + t * abx, cy = ay + t * aby;
  return dist(px, py, cx, cy);
}

// ---- main ----
for (const size of SIZES) {
  const rgba = makeIcon(size);
  const png = encodePng(size, size, rgba);
  const path = `${OUT_DIR}/icon-${size}.png`;
  writeFileSync(path, png);
  console.log(`wrote ${path} (${png.length} bytes)`);
}
