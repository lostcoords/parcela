# Build the documentation site

VitePress builds `https://parcela.run` from this repository's Markdown. Use Node 22.19 or newer. Run the commands below from the repository root.

## Find the source

| Source | Purpose |
| --- | --- |
| `docs/site/docs.md` | User documentation index. |
| `docs/site/*.md` | Four primary tasks, reference, and an example app. |
| `docs/architecture.md`, `CONTRIBUTING.md` | Contributor documentation, also published on the site. |
| `.agents/skills/parcela/SKILL.md` | Operating skill embedded by `parcela --skill` and offered as a download. |
| `examples/docs-demo/` | Complete app from the tutorial. |
| `content.mjs` | Page registry, navigation groups, route mapping, downloads, and metadata. |

## Preview locally

Install dependencies:

```sh
npm ci --prefix docs/website
```

Start a loopback preview:

```sh
npm run dev --prefix docs/website -- --port 5173
```

Open `http://127.0.0.1:5173/`. The page registry watches listed source files. Restart the server to refresh search or after changing the registry.

To preview through Parcela instead, run `parcela up --detach` from this repository. Its setup hook installs dependencies. Open the environment URL it prints. This is a development preview, not the isolated verification harness.

## Add or edit a page

1. Choose a reader need using the [writing guidance](../../CONTRIBUTING.md#write-documentation).
2. Edit its Markdown source. For a new page, add a `[slug, title, source]` entry to `groups` in `content.mjs`.
3. Use relative Markdown links to registered pages. The build converts them to public routes. Links to unpublished files fail.
4. Use one H1. Optional YAML frontmatter sets `title` and `description`. Without those, metadata comes from the first heading and suitable paragraph.
5. Run the documentation check.

Includes and script blocks are rejected. Raw HTML and template expressions render as text. The landing page uses `layout: home` frontmatter and theme components.

The root route `/` is the landing page. `/docs/` is the documentation index. Other pages use `/docs/<slug>/`.

## Check the built site

```sh
npm run check --prefix docs/website
```

The command builds into `target/docs`. It verifies source coverage, links and anchors, metadata, social cards, downloads, search, tutorial snippets, and the rendered Claude skill install block. Tool names gain local SVG marks at render time through `brands.mjs`. Code tokens stay unchanged and CSS wraps them within the reading column. It also checks telemetry endpoint validation.

For executable behavior, run:

```sh
scripts/check
scripts/check-live docs-example
```

The Python tests execute the Claude install block in a sandbox. The live suite runs the tutorial app through real Caddy in disposable environments with isolated state and test ports. Neither trusts a CA system-wide.

## Style the components

The site uses shadcn-vue's New York components with Tailwind v4 and Reka UI.
The generated source lives in `src/components/ui`. Parcela owns that source and
its variants. `components.json` configures the generator, and the `@` alias points
to `src` in both Vite and TypeScript.

| Location | Responsibility |
| --- | --- |
| `src/theme.css` | Semantic colors, radii, shadows, glass, motion, and Tailwind utilities. Light and dark themes use the same token names. |
| `src/parcela.css` | Maps VitePress colors to the semantic theme and styles documentation, fonts, and navigation. |
| `src/media/` | The dashboard recording and its poster, and desktop and phone captures of a real Parcela project's homepage, imported as Vite assets. |
| `src/home.css` | Homepage section width, gutters and rhythm, headings, flat windows, and solid actions with an outer glow. |
| `src/components/ui/button/index.ts` | Button variants and sizes, including Parcela's `glass` variant, pill shape, and larger default controls. |
| `.vitepress/theme/*.vue` | Product components composed from shadcn UI and Parcela's illustrations, browser demo, and installation behavior. |

Homepage actions use `Button`. `HeroDemo.vue` reserves a static 16:9 video
frame beneath the hero, aligned with the other sections. It displays
"Video demo coming soon" until the real recording is available.
The recording plan is in `docs/demo-recording-plan.md` at the repository root.
VitePress provides navigation, search, and code-block copying.

The palette uses Lostcoords UI's neutral paper, ink, and accent colors. Dark
surfaces use its 20.5%, 24.2%, and 28.4% neutral OKLCH lightness values, with
91.5% off-white text and primary buttons. Borders, muted text, shadows, and glass
follow the same theme. The values come from
`../lostcoords-ui/app/assets/tailwind/lostcoords/ui/theme.css` relative to the
repository root. The site uses the same self-hosted Manrope font as WariWari,
with JetBrains Mono for code, and keeps the agent illustrations' colors.

The homepage uses a plain background and typography for hierarchy. Below the
hero and video, every section is a `.home-section` with the same width, gutters,
and vertical rhythm. Section headings are one plain sentence-case line, with no eyebrow labels. `SectionHeading.vue` makes each one a link to its own anchor, such as `/#sharing`. Each section
head ends with `section-actions`: `Button` links into the matching documentation. Example
URLs omit the default `:1443` port. The comparison terminals are flat `window` surfaces, and
the setup steps' file and terminal sit on flat surfaces inside their tiles, with no border or shadow. Screens and the dashboard recording sit directly on the page.

The sections, in order:

| Component | Content |
| --- | --- |
| `HomeStack.vue` | Two full-width marquee rows on a secondary band: coding agents scroll right to left, frameworks left to right. Marks come from `brands.mjs`. Hover pauses a row. Duplicates are hidden from assistive technology; reduced motion shows one static, centered set per row. |
| `HomeCompare.vue` | Three agents in three worktrees: `bin/dev` collides on port 3000, `parcela up` gives each its own URL. Each side is a terminal window whose lines type in once the section scrolls into view; reduced motion and no-JavaScript renders show every line. |
| `HomeSteps.vue` | A bento of three steps: describing and starting the app side by side, and the running app on a laptop and phone in a wide tile below. `DeviceFrame.vue` draws both devices, here and in the sharing section. The Markdown body of `docs/site/index.md` is empty. |
| `HomeDashboard.vue` | A recording of the real `parcela ui` dashboard, `src/media/dashboard.mp4`. It autoplays muted and looped only without reduced motion; otherwise it shows its poster and native controls. |
| `HomeSharing.vue` | This machine, with measured pen arrows to a phone on the LAN and a tailnet device. Each screen shows a capture of a real Parcela project, labelled with the generic name `myapp`. |
| `HomeFeatures.vue` | Six capabilities, each linking to its documentation. |
| `HomeCta.vue` | Common questions, then a closing install section with the mascot, install command, and actions. |

Buttons and open content sections have no decorative borders. The hero release announcement
is a glass pill linking to the release notes, with its tag and link in `themeConfig.release`.
The Get started buttons have a solid blue fill and an outer glow.

Homepage rules live in `src/home.css` and are scoped to the home layout.
Documentation code blocks use soft fills without borders or hover shadows.
Tables use spacing without an outer frame or striped rows. Only the homepage header animates from full width into a compact glass navbar
after scrolling. Documentation keeps its full-width header at every scroll position. Section links precede a compact search control on both layouts. NavSections
uses VitePress link and dropdown components in the content-before slot, and
the default navbar menu is hidden. The DOM order matches keyboard navigation. Reduced transparency has a solid header fallback.

The floating navbar requires VitePress's initialized `home` class, so the initial
HTML renders a full-width header before scroll state is available.
The homepage has rotating agent names, a scrolling logo band, typed comparison
terminals, drawn sharing arrows, an autoplaying dashboard recording, and a swaying mascot.
`reveal.js` fades each block in as it scrolls into view, with an IntersectionObserver so
it works without CSS scroll timelines. The typed terminals, sharing arrows, and pen underline
use `useEntrance` from `useHomeMotion.js`, which arms them only while motion is allowed. Reduced motion stops headline rotation, shows every
block and arrow in place, and disables CSS animations. The video placeholder stays static in either mode. Changes to the
preference take effect without reloading.
Documentation navigation keeps its 280ms article fade. Reduced motion disables it.

The site uses VitePress's `html.dark` class rather than Lostcoords UI's
`data-theme="dark"` attribute. Both sites declare a `darkreader-lock` meta tag so
Dark Reader does not recolor the native theme.

Use semantic tokens for site-wide changes. Add a component variant for a repeated
control treatment. Use a call-site `class` for layout or a local adjustment.
Keep product behavior and illustrations in the custom Vue components.

For the later Lostcoords UI comparison, the relevant customization boundaries are:

| Concern | Parcela with shadcn-vue | Lostcoords UI |
| --- | --- | --- |
| Theme | Semantic CSS variables and a VitePress mapping | Paper, ink, and accent CSS variables in the host theme |
| Control appearance | Typed `variant` and `size` props backed by CVA classes in owned source | Helper variant and size maps, with host overrides |
| Local adjustment | `class`, merged by `cn` | `class:` on helpers, merged by `lc_tw` |
| Interaction | Reka primitives composed in Vue | ActionView markup and Stimulus controllers |
| Product behavior | Custom Vue components composed from UI primitives | Host views and controllers composed from helpers |

This records the implementation boundaries. It does not establish whether
Lostcoords UI needs an interface change.

To add another component, run the pinned CLI from `docs/website`:

```sh
npm run ui:add -- accordion
```

Review the generated diff before replacing an existing component. Regeneration
can overwrite Parcela's variants. The checked-in components came from the
New York registry using shadcn-vue 2.9.0. The CLI version is pinned in the command;
the upstream registry can still change.

Tailwind's global Preflight reset is omitted because VitePress supplies the page
reset. The small reset for `data-slot` elements and explicit solid borders on
controls cover the generated components. Utilities are unlayered so VitePress's
unlayered reset cannot override them. Register new theme tokens in `@theme inline`
when a newly added component needs them.

## Check the UI in a browser

Start the review environment with Parcela and open its named HTTPS URL in an
owned browser activity. Use the already trusted local proxy for this preview.
The browser check rejects raw backend HTTP ports. Run `check-ui.js` with the
configured browser's `run-code --filename` command. For the Brave extension:

```sh
~/bin/playwright-cli --activity=parcela-docs run-code --filename "$PWD/docs/website/check-ui.js"
```

The check exercises the homepage and documentation at desktop and phone sizes,
light and dark themes, local search, mobile navigation, FAQ answers, and the
static video placeholder's aspect ratio and alignment. It also checks the
loaded screens on all three devices, headline rotation, sharing arrows, mascot motion, and reduced motion without a reload. It dispatches
DOM events in the owned tab to avoid changing desktop focus. Native keyboard
delivery and pointer hit testing are not covered. It checks code-copy controls
without writing to the clipboard and leaves the owned page on the homepage.

## Publishing pipeline

The `Documentation` workflow runs the site check and deploys `target/docs` when a push to `main` matches its path filters. `wrangler.jsonc` configures the Cloudflare Worker `parcela-site` and its static assets. Deployment uses the `CLOUDFLARE_API_TOKEN` repository secret.

The build copies the installer, schema, and downloads into the output. It also writes Cloudflare headers, the `/docs` redirect, `robots.txt`, `sitemap.xml`, and a 1200 by 630 social card per page. `og.mjs` renders the cards with resvg. The Worker serves static pages and the telemetry endpoint on `parcela.run`.

`DOCS_BASE` defaults to `/`. `DOCS_SITE` defaults to `https://parcela.run` and controls canonical URLs. `DOCS_OUTPUT` selects the output directory. The standard check uses the production base `/`.
