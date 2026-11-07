import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Resvg } from '@resvg/resvg-js';

// Social cards: 1200x630 PNGs with a page's title and description, the mascot in a menu
// tile and the landing page's plot grid behind.
const here = import.meta.dirname;
const fontFiles = ['MPLUSRounded1c-Medium.ttf', 'MPLUSRounded1c-ExtraBold.ttf'].map((name) => join(here, 'og', name));
const font = { fontFiles, loadSystemFonts: false, defaultFontFamily: 'Rounded Mplus 1c Medium' };
// Each weight is its own family in these files.
const family = (weight) => `font-family="Rounded Mplus 1c ${weight >= 700 ? 'ExtraBold' : 'Medium'}"`;
const logo = readFileSync(join(here, '../../assets/brand/logo.png')).toString('base64');
const mascot = (x, y, size) => `<image x="${x}" y="${y}" width="${size}" height="${size}" href="data:image/png;base64,${logo}"/>`;

const width = 1200;
const height = 630;
const escape = (text) => text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

function measure(text, size, weight) {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="4000" height="200"><text x="0" y="150" font-size="${size}" ${family(weight)}>${escape(text)}</text></svg>`;
  return new Resvg(svg, { font }).innerBBox()?.width ?? 0;
}

// Greedy word wrap to `max` lines; the last line gets an ellipsis when text is left over.
function wrap(text, size, weight, room, max) {
  const lines = [];
  let line = '';
  const words = text.split(/\s+/);
  for (const [i, word] of words.entries()) {
    const next = line ? `${line} ${word}` : word;
    if (!line || measure(next, size, weight) <= room) {
      line = next;
      continue;
    }
    lines.push(line);
    line = word;
    if (lines.length === max) {
      let last = lines.pop();
      while (measure(`${last}…`, size, weight) > room) last = last.replace(/\s*\S+$/, '');
      lines.push(i < words.length ? `${last}…` : last);
      return lines;
    }
  }
  lines.push(line);
  return lines;
}

function grid() {
  const size = 150;
  const lines = [];
  for (let x = 0; x <= width; x += size) lines.push(`<line x1="${x}" y1="0" x2="${x}" y2="${height}"/>`);
  for (let y = 15; y <= height; y += size) lines.push(`<line x1="0" y1="${y}" x2="${width}" y2="${y}"/>`);
  return `<pattern id="dots" width="18.75" height="18.75" patternUnits="userSpaceOnUse" x="9.4" y="24.4"><circle cx="0" cy="0" r="1.4" fill="#343434"/></pattern>
  <rect width="${width}" height="${height}" fill="url(#dots)"/>
  <g stroke="#242424" stroke-width="2">${lines.join('')}</g>`;
}

export function renderCard({ title, description }) {
  const titleSize = measure(title, 64, 800) > 1300 ? 54 : 64;
  const titleLines = wrap(title, titleSize, 800, 640, 3);
  const descriptionLines = wrap(description, 28, 500, 640, 3);
  const titleTop = 230;
  const titleStep = titleSize * 1.12;
  const descriptionTop = titleTop + (titleLines.length - 1) * titleStep + 64;
  const text = (lines, x, y, step, size, weight, fill) => lines
    .map((line, i) => `<text x="${x}" y="${y + i * step}" font-size="${size}" ${family(weight)} fill="${fill}">${escape(line)}</text>`)
    .join('');
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
  <rect width="${width}" height="${height}" fill="#171717"/>
  ${grid()}
  <rect x="40" y="40" width="720" height="550" rx="40" fill="#202020" fill-opacity="0.92"/>
  <rect x="810" y="140" width="300" height="320" rx="44" fill="#242424"/>
  ${mascot(842, 178, 236)}
  <rect x="80" y="84" width="20" height="20" rx="6" fill="#34bfed"/>
  <rect x="106" y="84" width="20" height="20" rx="6" fill="#ff5a5f"/>
  <rect x="132" y="84" width="20" height="20" rx="6" fill="#3cc46a"/>
  <text x="168" y="102" font-size="28" ${family(800)} fill="#e3e3e3">Parcela</text>
  ${text(titleLines, 80, titleTop, titleStep, titleSize, 800, '#e3e3e3')}
  ${text(descriptionLines, 80, descriptionTop, 40, 28, 500, '#b3b3b3')}
  <text x="80" y="550" font-size="26" ${family(800)} fill="#65c6e3">parcela.run</text>
</svg>`;
  return new Resvg(svg, { font, fitTo: { mode: 'width', value: width } }).render().asPng();
}

