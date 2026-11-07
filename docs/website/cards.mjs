// Draws the social cards after `bin/rails kiln:build`: one 1200x630 PNG per page in og/, from the
// titles and descriptions the build wrote to cards.json, which is then removed.
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { renderCard } from './og.mjs';

const output = process.argv[2] ?? join(import.meta.dirname, '../../target/docs');
const list = join(output, 'cards.json');
const pages = JSON.parse(readFileSync(list, 'utf8'));
mkdirSync(join(output, 'og'), { recursive: true });
for (const { slug, title, description } of pages) writeFileSync(join(output, 'og', `${slug}.png`), renderCard({ title, description }));
rmSync(list);
console.log(`${pages.length} cards in ${join(output, 'og')}`);
