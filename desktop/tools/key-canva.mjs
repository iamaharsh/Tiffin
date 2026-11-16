// Turns the owner's Canva export of concept C (opaque cream PNG; the Free plan cannot export transparency or SVG)
// into the shipped brand art: `node key-canva.mjs <canva.png> <master-out.png> <lockup-out.png> [<preview-dir>]`
//  - master-out: 1024 icon master (rounded cream tile + tins, leaf and wordmark) for `tauri icon`
//  - lockup-out: transparent lockup (tins, leaf, wordmark) for Logo.svelte, 320 px wide (2x of the About size)
// The small "recipe organizer" line (below ART_BOTTOM) is dropped: it is unreadable under about 256 px.
// The background is keyed by unmixing: each pixel is the background plus one of three flat ink colours at some
// coverage, so edges get partial alpha with the pure ink colour and leave no cream halo.
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { inflateSync, deflateSync, crc32 } from 'node:zlib';

const [src, masterOut, lockupOut, previews] = process.argv.slice(2);
if (!src || !masterOut || !lockupOut) { console.error('Usage: key-canva.mjs <canva.png> <master-out.png> <lockup-out.png> [<preview-dir>]'); process.exit(1); }

const ART_BOTTOM = 760; // the wordmark ends near y=745, the descriptor starts near y=785 (1024 px export)
const INK = [[200, 85, 45], [110, 125, 55], [70, 40, 30]]; // terracotta, olive, cocoa as sampled from the export (cocoa only matters under ART_BOTTOM, kept so it never keys as terracotta)

function decode(b) {
  if (b.toString('ascii', 1, 4) !== 'PNG') throw new Error('Not a PNG file.');
  const w = b.readUInt32BE(16), h = b.readUInt32BE(20);
  if (b[24] !== 8 || b[25] !== 2 || b[28] !== 0) throw new Error('Expected an 8-bit RGB PNG without interlacing.');
  const idat = [];
  for (let o = 8; o < b.length;) { const n = b.readUInt32BE(o); if (b.toString('ascii', o + 4, o + 8) === 'IDAT') idat.push(b.subarray(o + 8, o + 8 + n)); o += 12 + n; }
  const raw = inflateSync(Buffer.concat(idat)), stride = w * 3, px = Buffer.alloc(h * stride);
  for (let y = 0; y < h; y++) {
    const f = raw[y * (stride + 1)], row = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let x = 0; x < stride; x++) {
      const a = x >= 3 ? px[y * stride + x - 3] : 0, u = y ? px[(y - 1) * stride + x] : 0, c = x >= 3 && y ? px[(y - 1) * stride + x - 3] : 0;
      const p = a + u - c, pa = Math.abs(p - a), pb = Math.abs(p - u), pc = Math.abs(p - c);
      const pred = [0, a, u, (a + u) >> 1, pa <= pb && pa <= pc ? a : pb <= pc ? u : c][f];
      px[y * stride + x] = (row[x] + pred) & 255;
    }
  }
  return { w, h, px };
}

function encode(w, h, rgba) { // rgba: Uint8 non-premultiplied
  const raw = Buffer.alloc(h * (w * 4 + 1));
  for (let y = 0; y < h; y++) Buffer.from(rgba.buffer, rgba.byteOffset + y * w * 4, w * 4).copy(raw, y * (w * 4 + 1) + 1);
  const chunk = (t, d) => { const o = Buffer.alloc(12 + d.length); o.writeUInt32BE(d.length); o.write(t, 4, 'ascii'); d.copy(o, 8); o.writeUInt32BE(crc32(o.subarray(4, 8 + d.length)), 8 + d.length); return o; };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 6;
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw, { level: 9 })), chunk('IEND', Buffer.alloc(0))]);
}

// Premultiplied float RGBA images: { w, h, d: Float32Array }.
const blank = (w, h) => ({ w, h, d: new Float32Array(w * h * 4) });
const fromRgba8 = (w, h, a) => { const i = blank(w, h); for (let p = 0; p < w * h; p++) { const al = a[p * 4 + 3] / 255; for (let c = 0; c < 3; c++) i.d[p * 4 + c] = (a[p * 4 + c] / 255) * al; i.d[p * 4 + 3] = al; } return i; };
const toRgba8 = (i) => { const o = new Uint8Array(i.w * i.h * 4); for (let p = 0; p < i.w * i.h; p++) { const al = i.d[p * 4 + 3]; for (let c = 0; c < 3; c++) o[p * 4 + c] = al ? Math.round(Math.min(1, i.d[p * 4 + c] / al) * 255) : 0; o[p * 4 + 3] = Math.round(al * 255); } return o; };

function key(img) {
  const { w, h, px } = img, edge = [];
  for (let i = 0; i < w; i++) for (const y of [0, h - 1]) edge.push(px.subarray((y * w + i) * 3, (y * w + i) * 3 + 3));
  const bg = [0, 1, 2].map((c) => edge.map((e) => e[c]).sort((a, b) => a - b)[edge.length >> 1]);
  const out = new Uint8Array(w * h * 4);
  for (let p = 0; p < w * h; p++) {
    const d = [0, 1, 2].map((c) => px[p * 3 + c] - bg[c]);
    let best = null;
    for (const ink of INK) {
      const v = ink.map((x, c) => x - bg[c]), a = Math.min(1, Math.max(0, (d[0] * v[0] + d[1] * v[1] + d[2] * v[2]) / (v[0] ** 2 + v[1] ** 2 + v[2] ** 2)));
      const r = Math.hypot(...d.map((x, c) => x - a * v[c]));
      if (!best || r < best.r) best = { r, a, ink };
    }
    if (best.a < 0.05) continue; // background noise
    out.set([...best.ink, Math.round(Math.min(1, best.a) * 255)], p * 4);
  }
  return fromRgba8(w, h, out);
}

function crop(i, x0, y0, x1, y1) { const o = blank(x1 - x0, y1 - y0); for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) o.d.set(i.d.subarray((y * i.w + x) * 4, (y * i.w + x) * 4 + 4), ((y - y0) * o.w + x - x0) * 4); return o; }

function resize(i, ow, oh) { // area average (downscaling only)
  const o = blank(ow, oh), sx = i.w / ow, sy = i.h / oh;
  for (let y = 0; y < oh; y++) for (let x = 0; x < ow; x++) {
    const acc = [0, 0, 0, 0]; let wt = 0;
    for (let yy = Math.floor(y * sy); yy < Math.ceil((y + 1) * sy); yy++) for (let xx = Math.floor(x * sx); xx < Math.ceil((x + 1) * sx); xx++) {
      const f = Math.min(xx + 1, (x + 1) * sx) - Math.max(xx, x * sx), g = Math.min(yy + 1, (y + 1) * sy) - Math.max(yy, y * sy), k = f * g;
      for (let c = 0; c < 4; c++) acc[c] += i.d[(yy * i.w + xx) * 4 + c] * k; wt += k;
    }
    for (let c = 0; c < 4; c++) o.d[(y * ow + x) * 4 + c] = acc[c] / wt;
  }
  return o;
}

function over(dst, top, ox, oy) {
  for (let y = 0; y < top.h; y++) for (let x = 0; x < top.w; x++) {
    const s = ((y) * top.w + x) * 4, d = ((y + oy) * dst.w + x + ox) * 4, k = 1 - top.d[s + 3];
    for (let c = 0; c < 4; c++) dst.d[d + c] = top.d[s + c] + dst.d[d + c] * k;
  }
}

// Rounded square, `inset` from the canvas edge, filled `fill` with an optional `rim` ring (hex colours), anti-aliased by distance.
function tile(w, h, inset, r, fill, rim, rimW) {
  const o = blank(w, h), f = fill.map((c) => c / 255), m = (rim ?? fill).map((c) => c / 255), hx = (w - 2 * inset) / 2, hy = (h - 2 * inset) / 2;
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    const qx = Math.abs(x + 0.5 - w / 2) - (hx - r), qy = Math.abs(y + 0.5 - h / 2) - (hy - r);
    const d = Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) + Math.min(Math.max(qx, qy), 0) - r;
    const outer = Math.min(1, Math.max(0, 0.5 - d)), inner = rimW ? Math.min(1, Math.max(0, 0.5 - (d + rimW))) : 1;
    for (let c = 0; c < 3; c++) o.d[(y * w + x) * 4 + c] = (f[c] * inner + m[c] * (1 - inner)) * outer;
    o.d[(y * w + x) * 4 + 3] = outer;
  }
  return o;
}

const full = key(decode(readFileSync(src)));
let x0 = full.w, x1 = 0, y0 = full.h, y1 = 0;
for (let y = 0; y < ART_BOTTOM; y++) for (let x = 0; x < full.w; x++) if (full.d[(y * full.w + x) * 4 + 3] > 0.3) { x0 = Math.min(x0, x); x1 = Math.max(x1, x + 1); y0 = Math.min(y0, y); y1 = Math.max(y1, y + 1); }
const art = crop(full, x0, y0, x1, y1);

const lockup = resize(art, 320, Math.round(320 * art.h / art.w));
writeFileSync(lockupOut, encode(lockup.w, lockup.h, toRgba8(lockup)));

// 640 px of art on the 896 px tile leaves the safe-area margin Windows needs at small sizes.
const mark = resize(art, 640, Math.round(640 * art.h / art.w)), master = tile(1024, 1024, 64, 210, [0xF9, 0xE8, 0xCE], [0xE3, 0xC7, 0x9B], 10);
over(master, mark, 192, Math.round((1024 - mark.h) / 2));
writeFileSync(masterOut, encode(1024, 1024, toRgba8(master)));

if (previews) { // lockup on the light page and on the dark page (on a cream tile, as Logo.svelte does), both 2x
  mkdirSync(previews, { recursive: true });
  const pad = 40, W = lockup.w + 2 * pad, H = lockup.h + 2 * pad;
  const light = tile(W, H, 0, 0, [0xFF, 0xF6, 0xE9]); over(light, lockup, pad, pad);
  const dark = tile(W, H, 0, 0, [0x2B, 0x21, 0x18]), t = tile(W - 2 * 16, H - 2 * 16, 0, 28, [0xFF, 0xF6, 0xE9]);
  over(dark, t, 16, 16); over(dark, lockup, pad, pad);
  for (const [n, i] of [['lockup-light', light], ['lockup-dark', dark]]) writeFileSync(`${previews}/${n}.png`, encode(i.w, i.h, toRgba8(i)));
}
console.log(`art ${art.w}x${art.h} from (${x0},${y0}); lockup ${lockup.w}x${lockup.h}`);
