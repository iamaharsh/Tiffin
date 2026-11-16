// Writes a Windows .ico from PNG files: `node build-ico.mjs <out.ico> <png...>`, or `--check <ico>` to list the layers.
// It exists because the 16/24/32 px layers come from a simplified mark (icon-small.svg) while the larger
// layers come from the full mark, and `tauri icon` uses one master for every size.
import { readFileSync, writeFileSync } from 'node:fs';

const fail = (msg) => { console.error(msg); process.exit(1); };
const SIG = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const [first, ...rest] = process.argv.slice(2);

if (first === '--check') {
  if (rest.length !== 1) fail('Usage: build-ico.mjs --check <file.ico>');
  const b = readFileSync(rest[0]);
  if (b.length < 6 || b.readUInt16LE(0) !== 0 || b.readUInt16LE(2) !== 1) fail('Not a valid ICO file.');
  const n = b.readUInt16LE(4);
  if (n === 0 || b.length < 6 + n * 16) fail('Not a valid ICO file.');
  const sizes = [];
  for (let i = 0; i < n; i++) {
    const e = 6 + i * 16;
    const off = b.readUInt32LE(e + 12), len = b.readUInt32LE(e + 8);
    if (off + len > b.length) fail('Not a valid ICO file.');
    sizes.push(b[e] || 256);
  }
  console.log(sizes.join(' '));
} else {
  if (!first || rest.length === 0) fail('Usage: build-ico.mjs <out.ico> <png> [<png> ...]');
  const images = rest.map((p) => {
    const b = readFileSync(p);
    if (b.length < 24 || !b.subarray(0, 8).equals(SIG) || b.toString('ascii', 12, 16) !== 'IHDR') fail(`${p} is not a PNG file.`);
    const w = b.readUInt32BE(16), h = b.readUInt32BE(20);
    if (w !== h || w < 1 || w > 256) fail(`${p} must be square and at most 256 px (is ${w}x${h}).`);
    return { p, w, b };
  }).sort((a, c) => a.w - c.w);
  for (let i = 1; i < images.length; i++) if (images[i].w === images[i - 1].w) fail(`Two images are ${images[i].w} px; each size may appear once.`);
  const head = Buffer.alloc(6 + images.length * 16);
  head.writeUInt16LE(1, 2);
  head.writeUInt16LE(images.length, 4);
  let off = head.length;
  images.forEach(({ w, b }, i) => {
    const e = 6 + i * 16;
    head[e] = head[e + 1] = w === 256 ? 0 : w;
    head.writeUInt16LE(1, e + 4);
    head.writeUInt16LE(32, e + 6);
    head.writeUInt32LE(b.length, e + 8);
    head.writeUInt32LE(off, e + 12);
    off += b.length;
  });
  writeFileSync(first, Buffer.concat([head, ...images.map((i) => i.b)]));
}
