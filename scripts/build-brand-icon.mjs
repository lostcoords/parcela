import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';

const require = createRequire(new URL('../docs/website/package.json', import.meta.url));
const { Resvg } = require('@resvg/resvg-js');
const source = readFileSync(new URL('../assets/brand/fence-buddy-reference.png', import.meta.url));
const width = source.readUInt32BE(16);
const height = source.readUInt32BE(20);
const viewBox = [width * 0.1, height * 0.1, width * 0.8, height * 0.8].join(' ');
const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="${viewBox}"><image width="${width}" height="${height}" href="data:image/png;base64,${source.toString('base64')}"/></svg>`;
writeFileSync(new URL('../assets/brand/logo.png', import.meta.url), new Resvg(svg).render().asPng());
console.log('Built the 512×512 Fence Buddy icon from the generated master, preserving transparency.');
