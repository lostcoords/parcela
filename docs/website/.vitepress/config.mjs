import { defineConfig } from 'vitepress';
import tailwindcss from '@tailwindcss/vite';
import { slug } from 'github-slugger';
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join, resolve } from 'node:path';
import { brandNames } from '../brands.mjs';
import { card, groups, meta, prepareContent, route, srcDir, watchSources } from '../content.mjs';

const project = fileURLToPath(new URL('..', import.meta.url));
const base = process.env.DOCS_BASE ?? '/';
const site = process.env.DOCS_SITE ?? 'https://parcela.run';

prepareContent();
writeFileSync(join(srcDir, 'public/robots.txt'), `User-agent: *\nAllow: /\n\nSitemap: ${site}${base}sitemap.xml\n`);

const slugOf = (relativePath) => relativePath === 'docs/index.md' ? 'docs' : relativePath.replace(/^docs\//, '').replace(/\/?index\.md$/, '') || 'index';

export default defineConfig({
  srcDir,
  // `parcela up --lan` or `--tailscale` lists the names other devices use, for Vite's host check.
  vite: {
    plugins: [watchSources, tailwindcss()],
    resolve: { alias: { '@': resolve(project, 'src') } },
    server: { allowedHosts: (process.env.PARCELA_SHARE_HOSTS ?? '').split(',').filter(Boolean) },
  },
  base,
  outDir: resolve(project, process.env.DOCS_OUTPUT ?? '../../target/docs'),
  cacheDir: resolve(project, '.generated/cache'),
  cleanUrls: true,
  lang: 'en',
  title: 'Parcela',
  description: 'Isolated dev environments for your AI agents.',
  appearance: 'force-dark',
  sitemap: { hostname: site },
  head: [
    ['meta', { name: 'darkreader-lock' }],
    ['link', { rel: 'icon', type: 'image/png', href: `${base}assets/logo.png?v=fence-buddy` }],
    ['link', { rel: 'apple-touch-icon', href: `${base}apple-touch-icon.png` }],
    ['meta', { name: 'theme-color', content: '#171717' }],
  ],
  // Each page's title and description come from its source (see content.mjs).
  transformPageData(pageData) {
    const page = meta.get(slugOf(pageData.relativePath));
    if (!page) return;
    pageData.description = page.description;
    if (pageData.frontmatter.layout === 'home') return;
    pageData.title = page.title;
    if (/\bParcela\b/.test(page.title)) pageData.titleTemplate = false;
  },
  transformHead({ pageData, title, description }) {
    if (pageData.relativePath === '404.md') return [['meta', { name: 'robots', content: 'noindex' }]];
    const slug = slugOf(pageData.relativePath);
    const url = `${site}${base}${pageData.relativePath.replace(/index\.md$/, '')}`;
    const image = `${site}${base}${card(slug).slice(1)}`;
    const alt = `Parcela: ${meta.get(slug)?.title ?? title}`;
    return [
      ['link', { rel: 'canonical', href: url }],
      ['meta', { property: 'og:type', content: slug === 'index' ? 'website' : 'article' }],
      ['meta', { property: 'og:site_name', content: 'Parcela' }],
      ['meta', { property: 'og:url', content: url }],
      ['meta', { property: 'og:title', content: title }],
      ['meta', { property: 'og:description', content: description }],
      ['meta', { property: 'og:image', content: image }],
      ['meta', { property: 'og:image:type', content: 'image/png' }],
      ['meta', { property: 'og:image:width', content: '1200' }],
      ['meta', { property: 'og:image:height', content: '630' }],
      ['meta', { property: 'og:image:alt', content: alt }],
      ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
      ['meta', { name: 'twitter:title', content: title }],
      ['meta', { name: 'twitter:description', content: description }],
      ['meta', { name: 'twitter:image', content: image }],
      ['meta', { name: 'twitter:image:alt', content: alt }],
    ];
  },
  transformHtml(html, _id, { page, content }) {
    if (page !== '404.md') return html;
    return html.replace('<div id="app"></div>', `<div id="app">${content}</div>`);
  },
  markdown: {
    theme: 'github-dark',
    html: false,
    attrs: { disable: true },
    anchor: { slugify: slug },
    config(markdown) {
      markdown.block.ruler.disable('snippet');
      brandNames(markdown);
      // ```diagram fences are plain text that keeps its columns; see `.diagram` in parcela.css.
      const fence = markdown.renderer.rules.fence;
      markdown.renderer.rules.fence = (tokens, index, ...rest) => {
        const token = tokens[index];
        if (token.info.trim() !== 'diagram') return fence(tokens, index, ...rest);
        token.info = 'text';
        return `<div class="diagram">${fence(tokens, index, ...rest)}</div>`;
      };
      const render = markdown.renderer.render.bind(markdown.renderer);
      markdown.renderer.render = (...args) => `<div v-pre>\n${render(...args)}\n</div>`;
    },
  },
  themeConfig: {
    logo: '/assets/logo.png',
    release: {
      tag: 'v0.7.6',
      link: 'https://github.com/lostcoords/parcela/releases/tag/v0.7.6',
    },
    nav: [
      { text: 'Docs', link: route('docs') },
      { text: 'Get started', link: route('getting-started') },
      { text: 'Guides', items: groups.find(({ label }) => label === 'Guides').pages.map(([slug, text]) => ({ text, link: route(slug) })) },
      { text: 'Reference', items: groups.find(({ label }) => label === 'Reference').pages.map(([slug, text]) => ({ text, link: route(slug) })) },
    ],
    outline: [2, 3],
    search: { provider: 'local' },
    sidebar: groups.map(({ label, pages }) => ({
      text: label,
      items: pages.map(([slug, text]) => ({ text, link: route(slug) })),
    })),
    socialLinks: [{ icon: 'github', link: 'https://github.com/lostcoords/parcela' }],
  },
});
