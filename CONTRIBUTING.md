# Contribute to Parcela

Parcela's Rust CLI supervises local development processes. Caddy handles HTTPS and routing. Read [Architecture](https://parcela.run/docs/architecture/) before changing lifecycle, routing, state, or process ownership.

## Build and check code

Use Rust and Cargo. Live suites also need Caddy on `PATH`.

```sh
cargo build --locked
scripts/check
```

`scripts/check` runs formatting checks, Clippy with warnings denied, Rust tests, and Python tests. Use jj for version control in this repository and preserve unrelated working changes.

Some Rust tests are randomized: the injector, `parcela.yaml` loading and their parsers run against many generated inputs (`src/fuzz.rs`). `PARCELA_FUZZ_ROUNDS` runs more rounds, and a failure prints the `PARCELA_FUZZ_SEED` that replays it. The nightly `Stress` workflow runs every test and live suite several times on Linux and macOS, with far more rounds, to catch flaky tests and races before a push does.

`cargo llvm-cov` measures which code the tests reach. To include the live suites, export `cargo llvm-cov show-env --export-prefix`, then run `scripts/check-live` and `cargo test` before `cargo llvm-cov report`.

## Verify live behavior

Use the isolated harness instead of starting this repository's environment for a test.

```sh
scripts/check-live
scripts/check-live docs-example
scripts/check-live recovery --case owner_crash
```

The `updates` suite runs an installer round-trip with local download fixtures and a real proxy. It verifies that concurrent refreshes replace one proxy, preserve app PIDs and hooks, keep the CA and front door, and leave deliberately stopped or newer proxies alone.

The `dashboard` suite checks the local UI over real Caddy. Set `PARCELA_PLAYWRIGHT_MODULE` to an installed Playwright module path to also run its Chromium interaction checks. `PARCELA_CHROMIUM_EXECUTABLE` can select an existing Chromium binary. The browser runs headless, and its HTTP requests verify Caddy with the fixture's CA without changing system trust. Set `PARCELA_DASHBOARD_ARTIFACTS` to a directory to retain desktop and mobile screenshots.

The `upgrades` suite downloads the two latest releases older than the build (or the tags in `PARCELA_UPGRADE_FROM`), starts environments and the proxy with each, then checks that the build answers their old owners, replaces the old proxy without restarting apps, and stops, restarts and starts them. It needs network access to GitHub.

The `recovery` suite kills, freezes and starves Parcela's own processes: owners, the proxy and Caddy, at chosen and at random moments. `random_crashes` prints its seed; `PARCELA_CHAOS_SEED` replays one and `PARCELA_CHAOS_ROUNDS` runs more rounds.

Each fixture uses a disposable directory, isolated state, an untrusted test CA, and explicit ports from 30000 through 32767. The runner builds once and writes a JSON report per suite. `--output DIR` retains reports at a chosen path. `--caddy PATH` selects Caddy.

`--jobs N` runs suites at once, each in its own process with its own slice of those ports, and splits the `recovery` cases across the jobs, since that suite is most of the run. `scripts/check-live --jobs 8` takes about a minute on a laptop instead of about six. Each recovery case's report records its seconds. The live suites use `target/debug/parcela`, or the one under `CARGO_TARGET_DIR`, so a build into another target directory leaves a running suite alone.

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

The dashboard's `src/proxy/dashboard/index.html`, `logs.html` and `ui.css` are built from `lostcoords-ui`, committed, and compiled into the binary, so running Parcela needs no Ruby. After changing `scripts/dashboard/` or the gem, run `scripts/build-dashboard`; `bin/rails test` fails if they are stale.

If you change runnable examples, also run their live suite. `scripts/check-live docs-example` verifies `examples/docs-demo`. The Claude Code installation block is checked by `tests/test_skill_install.py` through `scripts/check`.

Describe what you ran and any behavior you could not verify. A documentation build does not prove the app works. Do not publish, deploy, or change system configuration as part of verification.
