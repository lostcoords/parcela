# Contribute to Parcela

Parcela's Rust CLI supervises local development processes. Caddy handles HTTPS and routing. Read [Architecture](https://parcela.run/docs/architecture/) before changing lifecycle, routing, state, or process ownership.

## Build and check code

Use Rust and Cargo. Live suites also need Caddy on `PATH`.

```sh
cargo build --locked
scripts/check
```

`scripts/check` runs formatting checks, Clippy with warnings denied, Rust tests, and Python tests. Use jj for version control in this repository and preserve unrelated working changes.

## Verify live behavior

Use the isolated harness instead of starting this repository's environment for a test.

```sh
scripts/check-live
scripts/check-live docs-example
scripts/check-live recovery --case owner_crash
```

The `updates` suite runs an installer round-trip with local download fixtures and a real proxy. It verifies that concurrent refreshes replace one proxy, preserve app PIDs and hooks, keep the CA and front door, and leave deliberately stopped or newer proxies alone.

The `dashboard` suite checks the local UI over real Caddy. Set `PARCELA_PLAYWRIGHT_MODULE` to an installed Playwright module path to also run its Chromium interaction checks. `PARCELA_CHROMIUM_EXECUTABLE` can select an existing Chromium binary. The browser runs headless, and its HTTP requests verify Caddy with the fixture's CA without changing system trust. Set `PARCELA_DASHBOARD_ARTIFACTS` to a directory to retain desktop and mobile screenshots.

Each fixture uses a disposable directory, isolated state, an untrusted test CA, and explicit ports from 30000 through 32767. The runner builds once and writes a JSON report per suite. `--output DIR` retains reports at a chosen path. `--caddy PATH` selects Caddy.

Do not change system trust or take over occupied listeners. Tests with a terminal must set `PARCELA_NO_TRUST=1`. Live sharing tests need disposable environments and must verify that shutdown removes mDNS names and Tailscale serve entries. Follow [AGENTS.md](https://github.com/lostcoords/parcela/blob/main/AGENTS.md) for the full project constraints.

## Write documentation

The [documentation index](https://parcela.run/docs/) is the user entry point. Keep each page focused on one reader need, following [Diátaxis](https://diataxis.fr/):

- Tutorials produce a working result with expected observations.
- How-to guides solve a named task.
- Reference pages describe commands, fields, defaults, and limits.
- Explanations describe concepts and design choices.

Use direct instructions and the names in the CLI and configuration. Put a fact in one canonical page and link to it elsewhere. Retain historical release notes rather than rewriting them as current instructions.

Pages are Kiln Markdown views in `docs/app/views/pages/`. To add or move a page, update `docs/app/models/docs.rb`. Keep existing public routes when possible. Link pages by route, such as `/docs/cli/#config`. The [site guide](https://parcela.run/docs/docs-site/) covers the build and checks.

## Verify documentation changes

```sh
cd docs
bundle install && npm ci
bin/rails test && npm test
RAILS_ENV=production bin/rails kiln:build
```

The dashboard's `src/dashboard/index.html`, `logs.html` and `ui.css` are built from `lostcoords-ui`, committed, and compiled into the binary, so running Parcela needs no Ruby. After changing `scripts/dashboard/` or the gem, run `scripts/build-dashboard`; `bin/rails test` fails if they are stale.

If you change runnable examples, also run their live suite. `scripts/check-live docs-example` verifies `examples/docs-demo`. The Claude Code installation block is checked by `tests/test_skill_install.py` through `scripts/check`.

Describe what you ran and any behavior you could not verify. A documentation build does not prove the app works. Do not publish, deploy, or change system configuration as part of verification.
