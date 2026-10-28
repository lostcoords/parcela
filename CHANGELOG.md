# Changelog

## Unreleased

## 0.7.2 - 2026-10-28

### Fixed

- `up --tailscale` skips tailnet HTTPS ports that another process already listens on at the machine's tailnet address, such as a host-network proxy on 443. Before, `tailscale serve` retried that bind forever and the shared URL failed TLS.

## 0.7.1 - 2026-10-28

### Fixed

- Unknown configuration fields now suggest checking the spelling and upgrading Parcela if the configuration requires a newer release.
- `up` and `down` can reconcile a run left behind by a reboot when the filesystem device number changed but the ownership lock's inode did not. Recovery requires every recorded process to belong to a previous boot; replaced locks and control sockets remain protected.

## 0.7.0 - 2026-10-26

Create a named test checkout through Parcela, then seed, snapshot and reproduce its declared data without copying a live SQLite file or touching another environment.

### Added

- `parcela create NAME` creates and registers a Git worktree or jj workspace without starting services. `--revision` pins source and `--from` restores a named data snapshot.
- `data.files` and `data.sqlite` explicitly select environment-owned snapshot content. `snapshot` uses consistent SQLite backups and checksums. `restore` fills only a fresh data directory and never overwrites existing data. Seed and external data export/import commands remain project-defined tasks.

### Fixed

- Parcela closes inherited non-stdio descriptors on startup. Previously, parallel launchers could inherit a temporary listener and keep an unrelated proxy port occupied inside Parcela or Caddy.
- Proxy startup waits for transient ownership-lock probes instead of failing when a simultaneous status check briefly holds the lock.
- Watcher-only `doctor` checks a live shared proxy's compatibility, matching startup refusal. HTTP protocol differences remain warnings, as for HTTP startup. `doctor` now honors `--workspace`.
- `restart` validates changed service sets and HTTP-only flags before stopping the predecessor or forgetting setup completion.
- An abandoned jj workspace no longer hides live checkout inventory or blocks creating another workspace. Its missing root is reported and omitted.

## 0.6.0 - 2026-10-25

SDK watchers, workers, and other non-HTTP services can run as an environment without a dummy web server. Existing HTTP configurations work unchanged.

Before starting a watcher-only environment in an existing installation, restart the shared proxy with the new binary using `parcela proxy restart`. This briefly interrupts environment URLs, but does not restart their processes.

### Added

- Watcher-only environments. `parcela up` supervises services without `http`, with hooks, command health checks, logs, restart, and shutdown, without starting the proxy or changing trust. URL commands report that no HTTP URL exists. A running older proxy must be restarted with the new binary before sharing its registry with these sessions.

### Fixed

- `parcela trust` refuses to change system trust without a terminal, before creating state or starting the proxy. `trust --print` still works in scripts.

### Removed

- The warning that a relay installed by 0.4 does not redirect port 80, in `parcela proxy status` and `parcela doctor`. Run `parcela proxy install` again to add the redirect.
- Redirects from the docs pages that the site redesign merged, like `/docs/installation/`.

### Changed

- `install.sh` skips Caddy with `PARCELA_SKIP_CADDY=1`, which replaces `PARCELA_SKIP_BACKENDS`.

## 0.5.2 - 2026-10-20

### Fixed

- A running environment no longer stops when one check of its processes or listeners is slow, as on a loaded machine. Before, one `ps` or `lsof` call over its time limit stopped every service, with `backend control command timed out`. It stops only when checks keep failing for 30 seconds.

## 0.5.1 - 2026-10-20

### Fixed

- A `compose` service's volumes and networks with a fixed `name` are now each environment's own. Before, two environments shared them, so two databases could run on one data directory. `external` volumes and networks stay shared.
- Parcela reads the Compose file with the `env` from `parcela.yaml` that needs no port, like `COMPOSE_ENV_FILES`, as `docker compose up` does. A Compose file that requires such a variable failed before anything started.
- `parcela proxy restart` replaces a running proxy that this Parcela cannot read, such as one from 0.4. It failed because that proxy still held the port.

### Changed

- `parcela rm` removes a `compose` service's containers and network, and lists the volumes it keeps.
- `parcela upgrade` always installs the latest release unless `--nightly` is given, instead of staying on nightly for a nightly or locally built binary. `--stable` is gone.

### Removed

- `parcela url --service NAME`. Use `parcela url NAME`.
- Migrations for registries from before 0.1.0.

## 0.5.0 - 2026-10-20

Databases and other containers can come from the project's own `docker-compose.yml`, with their own containers, volumes, ports and URLs in every environment, next to native apps. `parcela.yaml` files from 0.4 work unchanged.

### Added

- `services.<name>.compose` runs containers from the project's own Docker Compose file next to native apps: `compose: {file: docker-compose.yml, services: [postgres]}`. Each environment gets its own Compose project, so its own containers, network and volumes, and a leased loopback port for every port the containers publish, as `PARCELA_PORT_<SERVICE>`. The Compose file needs no changes: an override in the environment's runtime directory replaces its host ports and drops `container_name`. Hooks and tasks get `COMPOSE_PROJECT_NAME` and `COMPOSE_FILE`, and the Compose file's default override file applies as with plain `docker compose`. `compose.http` gives a container that serves HTTP its own HTTPS URL, readiness check and sharing, or the environment's own URL for an app that runs entirely in Docker. `parcela doctor` checks the file and services. See [compose](https://parcela.run/docs/configuration/#compose).
- `services.<name>.stop`: a command that stops what a service started outside its own processes, like `docker compose stop` for a `docker compose up` service. It runs on every stop, after the service's dependents and before its SIGTERM, even if the service already exited or Parcela crashed. A failed one leaves the environment interrupted until `down` runs it again. A `compose` service sets it for you.
- `parcela completion <shell>` prints shell completions. `parcela open [SERVICE]` opens a URL in the browser, and `url [SERVICE]` prints one service's URL.
- `-w`/`--workspace` takes an environment's name, URL, hostname or ID. `-C` is short for `--path`.
- `readiness_timeout_seconds` in `parcela.yaml` sets how long `up` and `restart` wait for the services, instead of 60 seconds.
- `parcela upgrade` reruns `install.sh` for an install.sh install, staying on nightly or stable unless `--nightly` or `--stable` says otherwise. For Homebrew and `cargo` installs it prints the command to run instead.

### Fixed

- `up --detach` no longer gives up after the readiness timeout while a long `setup` or `prestart` hook is still running; it waits for the owner, which does not count hook time.

### Changed

- `status` and `ls` answer in about 80 ms instead of 400–800 ms: the supervisor answers requests while it waits, and `status` gets the process list in one reply.
- Errors name the next step for a missing `parcela.yaml`, an unstarted checkout, an unknown environment and a renamed project. `proxy status` and `doctor` flag a 443 relay installed without port 80.
- The proxy records a protocol number, so releases that share it take over its proxy quietly.
- Documentation centers on getting started, app configuration, parallel worktrees, and troubleshooting. Older guide URLs redirect to retained sections. Tool names have icons and accent colors, and code examples wrap within the page.
- Parcela sends anonymous usage data: the command, flag names, exit code, a duration range, version, OS, install method and a random install ID, never paths, names, URLs or values. Nothing is sent before a one-time notice. `parcela telemetry disable`, `DO_NOT_TRACK=1` or `PARCELA_TELEMETRY_DISABLED=1` turns it off, and `PARCELA_TELEMETRY_DEBUG=1` prints the event instead. See https://parcela.run/docs/telemetry/.
- The repository moved to `github.com/lostcoords/parcela` and the Homebrew tap to `lostcoords/tap`: `brew install lostcoords/tap/parcela`. The old addresses redirect.
- The website moved to `https://parcela.run`: the landing page at the root, the docs under `/docs/`, and `install.sh` and the schema at `https://parcela.run/install.sh` and `https://parcela.run/schema/v1.json`. `parcela init`, `parcela upgrade` and the schema's `$id` use the new addresses. `parcela.davafons.com` no longer serves the site.
- The proxy redirects plain HTTP on its port to HTTPS. `parcela proxy install` also relays port 80 when it is free, so `http://name.localhost` opens the environment. Run `parcela proxy install` again to add it to an existing install, then `parcela proxy restart`.
- `up` and `proxy status` suggest `parcela trust` only while the certificate authority is not trusted. `proxy status --json` reports it as `trusted` instead of the always-false `browser_trust_verified`.
- Rerunning `install.sh` over an earlier version says so and how to finish the upgrade. The FAQ shows how to upgrade each kind of install.

## 0.4.0 - 2026-10-19

Parcela runs each environment's processes itself, so Process Compose is no longer needed; only Caddy is. `parcela.yaml` does not change. Before upgrading, run `parcela down --all-projects` and `parcela proxy stop`: environments and a proxy started by 0.3 or earlier cannot be stopped, taken over or reached by 0.4.

### Added

- `up --lan` and `up --tailscale` (and `restart`) also serve one environment to other devices: at `.local` names on the LAN through mDNS, or on the tailnet through `tailscale serve`. `PARCELA_SHARE_HOSTS` lists the shared hostnames for a dev server's host check.
- Colors on a terminal: help, errors, and a color per process in `parcela logs`. `NO_COLOR` turns them off.
- `hooks.teardown` runs on `parcela rm`, for example to drop the environment's database. A failed teardown keeps the environment registered.
- `install.sh` also installs Caddy into `~/.local/bin` when no supported version is on `PATH`, checking the download against its release checksums. It checks Parcela's own download the same way, on macOS too, prints how to add `~/.local/bin` to `PATH` instead of editing shell profiles, and lists the files it wrote. `--help` lists its options.
- `parcela logs --follow` streams new lines from every process, or one.
- `-d` is short for `--detach`.
- `parcela logs` works after `down` too, until the next run: each process writes to `log/processes/<name>.log` in the runtime directory, cut to `<name>.log.1` past 16 MB.
- Nightly builds of `main`: `install.sh --nightly` installs the newest one. Builds other than a release tag report the commit in `--version`, like `0.3.0+20261019.4fed620`.
- The first `up` in a terminal trusts Parcela's certificate authority when it is not trusted yet, behind the system password prompt, once per authority. Never without a terminal; `PARCELA_NO_TRUST=1` turns it off.
- `parcela proxy install` on macOS makes URLs portless (`https://hello.localhost`): launchd owns 127.0.0.1:443 and relays each connection to the proxy as you. `uninstall` removes it, and refuses a user name that could change the service it installs.
- `http.wildcard: true` sends every subdomain of a service's host to it, for subdomain-per-tenant apps.
- `--lan-ip` picks the LAN address for `--lan`. Both refuse a public address. `PARCELA_LAN=1` and `PARCELA_TAILSCALE=1` work like the flags.
- A JSON Schema for `parcela.yaml` at `https://parcela.davafons.com/schema/v1.json`, generated from the parser. `parcela init` adds the line editors use to find it.

### Changed

- Parcela supervises the processes: each runs with `bash -c` (or `sh -c`) in its own process group, starts once its dependencies are ready, and stops in reverse dependency order, SIGTERM then SIGKILL after 10 seconds. Until the environment is first ready, a failed hook, an exited HTTP service or a process that exits nonzero stops `up` at once and names it, instead of waiting for the readiness timeout. Foreground `up` prefixes each line with its process's name.
- If an owner dies, the next `up`, `down` or `recover` adopts its processes by their recorded birth identity, and `down` stops ones that outlived it instead of refusing.
- On macOS, sockets live in the per-user temporary directory instead of `/tmp/parcela-<uid>`, so another user cannot block Parcela by creating that directory first.
- Removed: the hidden `--process-compose` option, `parcela doctor`'s Process Compose check, `child_pid` in `status --json`, and `log/process-compose.log`. A detached owner's `owner.log` holds process output only until the run is first ready.
- `status --json` process states are exactly `pending`, `running`, `stopping`, `restarting`, `completed`, `exited` and `skipped`, instead of Process Compose's names.
- The proxy listens on one HTTPS port, and URLs leave it out when it is 443. The forwarded "standard ports" mode and its port-80 listener are gone. `--port` accepts any port the system lets a normal user listen on.
- If the proxy dies while environments run, they start it again within a few seconds. `parcela proxy stop` still stops it for good.
- `restart` picks up added, removed and renamed services; `down` and `up` are no longer needed for that.
- `up` on a ready environment prints its URLs and exits 0, like `docker compose up -d`.
- Clearer errors: a readiness timeout says what each service answered, a busy `--backend-port` names its holder, and an unknown service or task lists the valid names.
- `--help` lists the global options in their own section. `status --all` is gone; use `ls`.

### Fixed

- Readiness checks stop once a service is ready, so they no longer fill its log.
- `--backend-port` also refuses a port another program holds on `0.0.0.0` or `[::]`, naming the holder before anything starts.
- Long branch and workspace names are cut to fit a hostname, ending in a short hash, instead of failing `up`.
- An `up` interrupted or killed before it launched anything no longer leaves the environment stuck as starting: Ctrl-C during startup stops it cleanly, and the next `up` or `down` releases its ports.

## 0.3.0 - 2026-10-18

`parcela.yaml` has a new format, version 1. Files written for 0.2 or earlier need rewriting, and `parcela doctor` links to the new format. The [configuration reference](docs/site/configuration.md) shows the new format.

### Added

- `hooks.setup` runs once in each new environment and `hooks.prestart` on every `up`, after the services they `depends_on` and before every other service. A failed hook stops `up`. `up --setup` and `restart --setup` run setup again.
- Services take `depends_on`, `working_dir`, `env` and, without `http`, a `healthcheck` command. Dependencies wait for readiness.
- `PARCELA_DATA_DIR`, `PARCELA_ENVIRONMENT_NAME` and `PARCELA_PRIMARY_DIR`.
- Top-level `x-` keys are ignored, for YAML anchors.
- `examples/rails-postgres`, and a tutorial app with a SQLite database per environment.

### Changed

- One format: `version` and `project` are required, and every service is a mapping under `services` with a `command`. HTTP services declare `http: {healthcheck, subdomain}`; the one without `subdomain` gets the environment's URL.
- Tasks take a shell `command` instead of `argv`. Arguments after `--` are appended.
- `parcela logs` without a service prints every service, worker and hook, each line prefixed with its name.

### Removed

- The top-level short form (`web: bin/dev`), `workers`, `default_service`, `environment` (now `env`), `inherit_environment`, `shutdown_timeout_seconds` and the `process_compose` pass-through.

### Fixed

- `parcela doctor` names the right Process Compose tap (`f1bonacc1/tap/process-compose`) and links each backend's install page.

## 0.2.0 - 2026-10-18

A smaller CLI and a faster proxy.

### Changed

- `parcela status` lists each service and worker process; `parcela services` is removed.
- `parcela ls` also shows this project's jj workspaces and Git worktrees that were never started, with the URL they would get; the `workspace` commands are removed.
- `parcela doctor` reports the same configuration error as `up`; `parcela config check` is removed.
- `parcela up` takes back control of services left running after Parcela's supervising process died. `recover` and `context` are hidden.
- Foreground `up` prints `✓ <name> is running at <url>` and no longer shows Process Compose debug lines.
- `--human` is removed; text is the default on a terminal.

### Fixed

- Two simultaneous first `parcela up` runs no longer fail with "unsupported registry schema 0".
- The proxy refreshes routes in about 85 ms with five running environments, down from about 1.2 s, so routes no longer come close to their 3-second expiry under load.

## 0.1.0 - 2026-10-17

First release. Run every checkout of a project as its own local environment, at its own HTTPS URL, in parallel.

### Added

- `parcela init` writes a one-line `parcela.yaml`, guessing the dev server command from `package.json`, Rails or Django.
- Short-form manifests: `web: <command>`, a `services:` map of commands, `workers:` for unrouted processes and `ports:` for named non-HTTP ports. The full schema still works.
- `parcela up` registers the checkout itself. The main checkout gets the project URL; linked jj workspaces and Git worktrees get their workspace or directory name, or `--name`.
- A background HTTPS proxy on the no-sudo port 1443, started by the first `up`. `parcela proxy start|stop|restart|status` manage it, and `--port` saves another port.
- `parcela trust` installs Parcela's local CA after saying what it changes; `--print` only prints the certificate path.
- Named ports (`ports: [db, redis]`) export `PARCELA_PORT_<NAME>`, distinct per environment and kept across restarts.
- `parcela ls`, `down --all`, `down --all-projects`, `rm` and `rm --prune`.
- `parcela doctor` checks backends, configuration, state and the proxy, and names the next step.
- Aligned text on a terminal and JSON when piped or with `--json`.
- Prebuilt binaries for macOS and Linux with a shell installer.

### Fixed

- Environments whose supervisor and services all died (for example after `kill -9`) no longer block `up` and `down`.
- Services listening on `localhost`, `[::1]` or a wildcard address are accepted when they belong to the environment.
- Unix sockets live in `/tmp/parcela-<uid>`, so long home or state paths work.
- Backend ports come from 20000–29999 instead of the OS ephemeral range, so other programs cannot take them before a service binds.

### Removed

- Workspace transfer, checkpoints and SQLite streaming. Manifests with a `transfer:` key are rejected.
- `--test-profile` and the raw proxy listener flags; a saved port profile replaces them.
