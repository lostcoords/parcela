# Build the documentation site

`https://parcela.run` is a small Rails app that [Kiln](https://github.com/davafons/kiln) renders to static files, styled with [lostcoords-ui](https://ui.lostcoords.com). The pages are Kiln Markdown views. Use Ruby 4.0 and Node 22.19 or newer. Run the commands below from `docs`.

## Find the source

| Source | Purpose |
| --- | --- |
| `app/views/pages/*.html.md` | The pages: the documentation index, tasks, reference, the example app and the architecture. |
| `CONTRIBUTING.md`, `.agents/skills/parcela/SKILL.md`, this README | Pages rendered from files outside `app/views`, as Kiln content entries. The skill is also embedded by `parcela --skill` and offered as a download. |
| `examples/docs-demo/` | Complete app from the tutorial. |
| `app/models/docs.rb` | Page registry: routes, navigation groups, titles and descriptions. |
| `app/models/brands.rb` | Tool-name marks from `config/brands.json`. |
| `app/views/home/` | The landing page. |
| `lib/downloads.rb` | Files served under `/assets/` as downloads. |

## Preview locally

Install dependencies:

```sh
bundle install
npm ci
```

Run `parcela up --detach` from the repository root and open the URL it prints. Its prestart hook builds the CSS and a second service rebuilds it on change. Pages re-render when their Markdown changes.

In development, [Tensaku](https://tensaku.lostcoords.com) runs on the pages. Alt-click text to edit it in its Markdown or ERB source, or press Alt+Shift+T to leave a note. Notes are files in `docs/tmp/tensaku/notes/`, and `.claude/skills/tensaku` tells a coding agent how to work through them.

Without Parcela, run `bin/rails tailwindcss:build` and then `bin/rails server -p 3000`.

## Add or edit a page

1. Choose a reader need using the [writing guidance](https://parcela.run/docs/contributing/#write-documentation).
2. Edit its view in `app/views/pages/`. For a new page, add `SLUG.html.md` there and a `Page` with its title and description to `GROUPS` in `app/models/docs.rb`.
3. Link pages by route, such as `/docs/cli/#config`, and downloads by their `/assets/` path. The build fails on a broken link or anchor.
4. Use one H1.
5. Run the checks.

A ```` ```diagram ```` fence keeps its columns and gets no copy button. Tool names in prose, such as Rails or Claude Code, gain their mark from `config/brands.json`; code, headings and link text stay as written.

The root route `/` is the landing page. `/docs/` is the documentation index. Other pages use `/docs/<slug>/`.

## Check the site

```sh
bin/rails test
npm test
RAILS_ENV=production bin/rails kiln:build
npm run cards
```

`bin/rails test` builds the site into a temporary directory and checks source coverage, headings, metadata, the sitemap, downloads, search, tutorial snippets, code blocks and the rendered Claude skill install block. The build fails on any broken internal link, anchor or importmap module. `npm test` checks the telemetry endpoint's validation and the card size.

`kiln:build` writes the site to `target/docs` at the repository root. `npm run cards` then draws a 1200 by 630 social card per page into `target/docs/og` with resvg (`og.mjs`), from the titles and descriptions the build wrote to `cards.json`.

For executable behavior, run:

```sh
scripts/check
scripts/check-live docs-example
```

The Python tests execute the Claude install block in a sandbox. The live suite runs the tutorial app through real Caddy in disposable environments with isolated state and test ports. Neither trusts a CA system-wide.

## Style the pages

| Location | Responsibility |
| --- | --- |
| `app/assets/tailwind/application.css` | Tailwind and lostcoords-ui imports, fonts, and the dark palette as lostcoords-ui tokens. |
| `app/assets/stylesheets/site.css` | Font faces, tool names, diagrams and documentation prose. |
| `app/assets/stylesheets/highlight.css` | Rouge's GitHub dark theme for code. |
| `app/views/shared/` | Header, search, sidebar, page outline, pager and footer. |

The site is dark only: `<html data-theme="dark">`, with no theme switch, and a `darkreader-lock` meta tag so Dark Reader leaves it alone. Code blocks use `lc_code_block` for the copy button. Search uses `lc_search` with MiniSearch over `/search.json`: one entry per page and one per h2 or h3 section. The header, favicon, social cards and Apple touch icon (`public/apple-touch-icon.png`, the mascot on `#171717` at 180 px) use Fence Buddy from `assets/brand/logo.png`.

## Publishing pipeline

The `Documentation` workflow runs the checks, builds `target/docs` and draws the cards, then deploys when a push to `main` matches its path filters. `wrangler.jsonc` configures the Cloudflare Worker `parcela-site` and its static assets. Deployment uses the `CLOUDFLARE_API_TOKEN` repository secret.

The build copies the installer and the schema to the site root and the downloads to `/assets/`. It also writes `public/_headers` for the installer, `robots.txt`, `sitemap.xml` and `404.html`. The Worker serves the static pages and the telemetry endpoint (`worker.mjs`) on `parcela.run`.
