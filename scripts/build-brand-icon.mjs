import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';

const require = createRequire(new URL('../docs/package.json', import.meta.url));
const { Resvg } = require('@resvg/resvg-js');
const source = readFileSync(new URL('../assets/brand/fence-buddy-reference.png', import.meta.url));
const width = source.readUInt32BE(16);
const height = source.readUInt32BE(20);
const frame = { x: width * 0.1, y: height * 0.1, width: width * 0.8, height: height * 0.8 };
const render = (area, outputWidth, outputHeight) => new Resvg(`<svg xmlns="http://www.w3.org/2000/svg" width="${outputWidth}" height="${outputHeight}" viewBox="${area.x} ${area.y} ${area.width} ${area.height}"><image width="${width}" height="${height}" href="data:image/png;base64,${source.toString('base64')}"/></svg>`).render();
const logo = render(frame, 512, 512);
writeFileSync(new URL('../assets/brand/logo.png', import.meta.url), logo.asPng());

// The badge is cropped to the art, so the CSS padding alone sets the space around it.
const pixels = logo.pixels;
const opaque = (x, y) => pixels[(y * 512 + x) * 4 + 3] > 24;
const columns = Array.from({ length: 512 }, (_, x) => x).filter(x => Array.from({ length: 512 }, (_, y) => y).some(y => opaque(x, y)));
const rows = Array.from({ length: 512 }, (_, y) => y).filter(y => Array.from({ length: 512 }, (_, x) => x).some(x => opaque(x, y)));
const scale = frame.width / 512;
const art = {
  x: frame.x + columns[0] * scale,
  y: frame.y + rows[0] * scale,
  width: (columns.at(-1) + 1 - columns[0]) * scale,
  height: (rows.at(-1) + 1 - rows[0]) * scale,
};
const badgeHeight = Math.round(96 * art.height / art.width);
writeFileSync(new URL('../src/proxy/dashboard/badge.png', import.meta.url), render(art, 96, badgeHeight).asPng());
console.log(`Built the 512×512 Fence Buddy icon and the 96×${badgeHeight} badge from the generated master, preserving transparency.`);
