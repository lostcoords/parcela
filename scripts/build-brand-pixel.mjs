import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';

const require = createRequire(new URL('../docs/package.json', import.meta.url));
const sharp = require('sharp');
const source = JSON.parse(readFileSync(new URL('../assets/brand/mascot-pixel.json', import.meta.url), 'utf8'));
const { size, palette, rows, outline } = source;
assert.equal(size, 32);
assert(rows.length === size && rows.every((row) => row.length === size));
assert(outline in palette);
for (const color of Object.values(palette)) {
  assert(color.length === 4 && color.every((value) => Number.isInteger(value) && value >= 0 && value <= 255));
  assert(color[3] === 0 || color[3] === 255);
}
const pixels = [];
for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
  let cell = rows[y][x];
  assert(cell in palette);
  if (palette[cell][3] === 0) {
    const neighbors = [-1, 0, 1].flatMap((dy) => [-1, 0, 1].map((dx) => rows[y + dy]?.[x + dx]));
    if (neighbors.some((neighbor) => neighbor && palette[neighbor][3] === 255)) cell = outline;
  }
  pixels.push(...palette[cell]);
}
const raw = Buffer.from(pixels);
const destination = new URL('../assets/brand/mascot-pixel.png', import.meta.url);
await sharp(raw, { raw: { width: size, height: size, channels: 4 } }).png().toFile(destination.pathname);
const decoded = await sharp(destination.pathname).ensureAlpha().raw().toBuffer();
assert.deepEqual(decoded, raw);
console.log('Rebuilt the 32×32 mascot with a one-pixel outline; exact RGBA pixels verified.');
