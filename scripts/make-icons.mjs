// Renders the Vision mark (ring + amber core on near-black) into icon.png / icon-large.png.
// Dependency-free: a tiny PNG encoder over zlib, with analytic anti-aliasing.
import { deflateSync } from 'node:zlib';
import { writeFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const BG = [0x07, 0x08, 0x0b];
const RING = [0xf5, 0xf1, 0xe8];
const CORE = [0xe9, 0xa2, 0x3b];

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buf) => {
  let c = 0xffffffff;
  for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
const chunk = (type, data) => {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
};

function icon(size) {
  const raw = Buffer.alloc(size * (size * 4 + 1));
  const c = size / 2;
  const radius = size * 0.22; // squircle-ish tile corner
  const ringR = size * 0.3;
  const ringW = size * 0.045;
  const coreR = size * 0.12;
  const cover = (d) => Math.min(1, Math.max(0, 0.5 - d));
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0;
    for (let x = 0; x < size; x++) {
      const px = x + 0.5 - c;
      const py = y + 0.5 - c;
      const qx = Math.abs(px) - (c - radius);
      const qy = Math.abs(py) - (c - radius);
      const tile = Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) + Math.min(Math.max(qx, qy), 0) - radius;
      const r = Math.hypot(px, py);
      const ring = cover(Math.abs(r - ringR) - ringW / 2);
      const core = cover(r - coreR);
      let col = BG.map((v, i) => v + (RING[i] - v) * ring);
      col = col.map((v, i) => v + (CORE[i] - v) * core);
      const o = y * (size * 4 + 1) + 1 + x * 4;
      raw[o] = col[0];
      raw[o + 1] = col[1];
      raw[o + 2] = col[2];
      raw[o + 3] = Math.round(255 * cover(tile));
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

mkdirSync(join(root, 'branding'), { recursive: true });
writeFileSync(join(root, 'branding', 'icon.png'), icon(80));
writeFileSync(join(root, 'branding', 'icon-large.png'), icon(130));
console.log('branding/icon.png, branding/icon-large.png');
