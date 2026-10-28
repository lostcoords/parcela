<p align="center">
  <img src="assets/brand/logo.svg" alt="Parcela picket mascot" width="150">
</p>

<h1 align="center">Parcela</h1>

<p align="center">
  <img src="https://img.shields.io/badge/status-experimental-e8b44f" alt="Status: experimental">
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/built_with-Rust-b45d3c?logo=rust&amp;logoColor=white" alt="Built with Rust"></a>
  <img src="https://img.shields.io/badge/platforms-macOS%20%7C%20Linux-57966f" alt="Platforms: macOS and Linux">
  <a href="https://github.com/lostcoords/parcela/actions/workflows/check.yml"><img src="https://github.com/lostcoords/parcela/actions/workflows/check.yml/badge.svg" alt="Rust checks"></a>
</p>

<p align="center">
  Deployable dev environments for your AI agents.
</p>

<p align="center">
  <a href="https://parcela.run/docs/">Documentation</a> ·
  <a href="https://parcela.run/docs/getting-started/">Getting started</a> ·
  <a href="https://parcela.run/docs/faq/">FAQ</a>
</p>

Parcela runs a checkout's web servers, workers, and other services at named HTTPS URLs. Each jj workspace or Git worktree gets its own processes and ports, so several agents can test the app in parallel.

Parcela is experimental and supports macOS and Linux. It supervises native processes and uses [Caddy](https://caddyserver.com) for TLS and routing.

## Start an existing app

```sh
curl -fsSL https://parcela.run/install.sh | sh
cd /path/to/your/project
parcela init
```

Review the generated `parcela.yaml` and its data settings. Then run:

```sh
parcela doctor
parcela up --detach
```

Open the URL in the output, such as `https://shop.localhost:1443`. The first terminal `up` offers CA trust behind the system password prompt. Scripts and agents never change system trust.

`parcela logs` shows process output. `parcela down` stops the environment and retains source and data.

For Homebrew and source builds, see [Get started](docs/site/getting-started.md#install-it-by-hand). Upgrades are in the [FAQ](docs/site/faq.md#upgrade-parcela). The [getting started guide](docs/site/getting-started.md) also has a setup prompt for coding agents.

## Describe the whole stack

```yaml
version: 1
project: shop

services:
  web:
    command: bin/rails server -p $PORT -b 127.0.0.1
    http:
      healthcheck: /up
  jobs:
    command: bin/jobs

hooks:
  setup:
    command: bin/setup
```

Each HTTP service receives a leased port through `PORT`. Routes open after readiness checks pass. Setup runs until it succeeds once in each environment. Your configuration decides how to install dependencies and separate databases, queues, and uploads.

The primary checkout uses `https://shop.localhost:1443`. A `signup` checkout uses `https://signup-shop.localhost:1443`. Additional HTTP services add their configured prefix. Sharing with another device requires `--lan`, `--tailscale` or `--domain` for that run.

## Give an agent its own environment

From the configured project, create and start a named checkout:

```sh
parcela create experiment
parcela -w experiment up --detach
parcela -w experiment down
```

Parcela creates a jj workspace or detached Git worktree and registers its preview name. Creation starts nothing. With [declared data](docs/site/configuration.md#data), `snapshot` and `create --from` reproduce a stopped environment's dataset from explicitly declared files, with consistent SQLite backups. See [worktrees and agents](docs/site/worktrees.md#reproduce-a-test-dataset).

Parcela sends anonymous command telemetry unless disabled. Run `parcela telemetry disable` to turn it off. [Telemetry](docs/site/telemetry.md) lists the fields and controls.

## Documentation

Start with the [documentation index](docs/site/docs.md).

- [Get started](docs/site/getting-started.md) with your app, or [build the tutorial app](docs/site/tutorial.md).
- [Configure a project](docs/site/frameworks.md#services-and-setup) and [run parallel worktrees](docs/site/worktrees.md).
- Look up [CLI commands](docs/site/cli.md) and [configuration fields](docs/site/configuration.md).
- Read [environment boundaries](docs/site/worktrees.md#names-and-isolation) and the [FAQ](docs/site/faq.md).
- [Contribute](CONTRIBUTING.md), inspect the [architecture](docs/architecture.md), or read the [changelog](CHANGELOG.md).

## License

Copyright © 2026 LOST COORDS. Parcela is licensed under the [GNU Affero General Public License, version 3 only](LICENSE). See [COPYRIGHT](COPYRIGHT) for the project notice and third-party terms.
