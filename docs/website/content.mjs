import { existsSync, mkdirSync, readFileSync, rmSync, watch, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderCard, renderIcon } from './og.mjs';

const project = dirname(fileURLToPath(import.meta.url));
export const repository = resolve(project, '../..');
export const srcDir = join(project, '.generated/src');

export const groups = [
  { label: 'Start here', pages: [
    ['docs', 'Documentation', 'docs/site/docs.md'],
    ['getting-started', 'Get started', 'docs/site/getting-started.md'],
  ] },
  { label: 'Guides', pages: [
    ['frameworks', 'Configure your app', 'docs/site/frameworks.md'],
    ['worktrees', 'Worktrees and agents', 'docs/site/worktrees.md'],
    ['troubleshooting', 'Troubleshooting', 'docs/site/troubleshooting.md'],
  ] },
  { label: 'Reference', pages: [
    ['cli', 'CLI reference', 'docs/site/cli.md'],
    ['configuration', 'Configuration', 'docs/site/configuration.md'],
    ['faq', 'FAQ', 'docs/site/faq.md'],
    ['telemetry', 'Telemetry', 'docs/site/telemetry.md'],
  ] },
  { label: 'More', pages: [
    ['tutorial', 'Example app', 'docs/site/tutorial.md'],
    ['contributing', 'Contribute to Parcela', 'CONTRIBUTING.md'],
  ] },
];
export const pages = [
  ['index', 'Home', 'docs/site/index.md'],
  ...groups.flatMap((group) => group.pages),
  ['operating-skill', 'Operating skill', '.agents/skills/parcela/SKILL.md'],
  ['architecture', 'Architecture', 'docs/architecture.md'],
  ['docs-site', 'Build the documentation site', 'docs/website/README.md'],
];
export const route = (slug) => slug === 'index' ? '/' : slug === 'docs' ? '/docs/' : `/docs/${slug}/`;

export const downloads = {
  'parcela-skill.txt': '.agents/skills/parcela/SKILL.md',
  'logo.png': 'assets/brand/logo.png',
  'third-party-notices.txt': 'docs/website/THIRD_PARTY_NOTICES.txt',
};

const routes = new Map(pages.map(([slug, , source]) => [join(repository, source), route(slug)]));
const raw = new Map(Object.entries(downloads).map(([name, source]) => [join(repository, source), `/assets/${name}`]));

function render(source) {
  const text = readFileSync(join(repository, source), 'utf8');
  if (/<!--\s*@include:|<script/i.test(text)) throw new Error(`Includes and scripts are not allowed: ${source}`);
  const markdown = source.startsWith('docs/site/') ? text : text.replace(/^---\n[\s\S]*?\n---\n/, '');
  return markdown.replace(/\]\(([^)]+)\)/g, (match, reference) => {
    if (/^(?:[a-z]+:|#|\/)/i.test(reference)) return match;
    const [file, suffix = ''] = reference.split(/(?=[?#])/);
    const target = resolve(dirname(join(repository, source)), file);
    const link = suffix === '?raw=1' ? raw.get(target) : routes.get(target)?.concat(suffix);
    if (!link) throw new Error(`Link to an unpublished file: ${reference} from ${source}`);
    return `](${link})`;
  });
}

// A page's title and description for search results and link previews: the frontmatter's
// when it has them, else the first heading (or the sidebar name, when the heading is just
// "Parcela") and the first paragraph that does not lead into a list or code block.
function describe(markdown, name) {
  const front = /^---\n([\s\S]*?)\n---\n/.exec(markdown)?.[1] ?? '';
  const field = (name) => new RegExp(`^${name}: (.+)$`, 'm').exec(front)?.[1].trim();
  const body = markdown.replace(/^---\n[\s\S]*?\n---\n/, '');
  const plain = (text) => text
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/`([^`]*)`/g, '$1')
    .replace(/\*\*?([^*]+)\*\*?/g, '$1')
    .replace(/\s+/g, ' ')
    .trim();
  const heading = plain(/^# (.+)$/m.exec(body)?.[1] ?? '');
  const title = field('title') ?? (heading === 'Parcela' ? name : heading);
  const paragraph = body.split(/\n{2,}/).map((block) => block.trim())
    .find((block) => /^[A-Za-z`*[]/.test(block) && !block.startsWith('```') && !block.endsWith(':'));
  let description = field('description') ?? plain(paragraph ?? '');
  if (description.length > 180) {
    const cut = description.slice(0, 180);
    description = cut.includes('. ') ? cut.slice(0, cut.lastIndexOf('. ') + 1) : `${cut.replace(/\s+\S*$/, '')}…`;
  }
  return { title, description };
}

export const meta = new Map();
export const card = (slug) => `/og/${slug}.png`;

function write(path, bytes) {
  if (existsSync(path) && readFileSync(path).equals(Buffer.from(bytes))) return;
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, bytes);
}

export function prepareContent({ clean = true } = {}) {
  const files = pages.map(([slug, , source]) => [join(srcDir, route(slug).slice(1), 'index.md'), render(source)]);
  if (clean) rmSync(srcDir, { recursive: true, force: true });
  for (const [path, markdown] of files) write(path, markdown);
  for (const [name, source] of Object.entries(downloads)) write(join(srcDir, 'public/assets', name), readFileSync(join(repository, source)));
  if (clean || !existsSync(join(srcDir, 'public/apple-touch-icon.png'))) write(join(srcDir, 'public/apple-touch-icon.png'), renderIcon(180));
  // The installer and the schema live at the site root, next to the pages.
  write(join(srcDir, 'public/install.sh'), readFileSync(join(project, 'install.sh')));
  write(join(srcDir, 'public/schema/v1.json'), readFileSync(join(project, 'schema/v1.json')));
  write(join(srcDir, 'public/_redirects'), '/docs /docs/ 301\n');
  write(join(srcDir, 'public/_headers'), '/install.sh\n  Content-Type: text/x-shellscript; charset=utf-8\n  Cache-Control: public, max-age=300\n');
  for (const [i, [slug, name]] of pages.entries()) {
    const page = describe(files[i][1], name);
    if (meta.get(slug)?.title === page.title && meta.get(slug)?.description === page.description) continue;
    meta.set(slug, page);
    write(join(srcDir, 'public', card(slug)), renderCard(page));
  }
}

export const watchSources = {
  name: 'parcela-sources',
  apply: 'serve',
  configureServer(server) {
    const sources = new Set([...routes.keys(), ...raw.keys()]);
    const watchers = [...new Set([...sources].map(dirname))].map((directory) => watch(directory, (_event, name) => {
      if (!name || !sources.has(join(directory, name.toString()))) return;
      try {
        prepareContent({ clean: false });
      } catch (error) {
        server.config.logger.error(error.message);
      }
    }));
    server.httpServer?.once('close', () => watchers.forEach((watcher) => watcher.close()));
  },
};
