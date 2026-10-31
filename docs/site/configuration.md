# Configuration

Parcela reads `parcela.yaml` from the root of your checkout. Commands look for it in the current directory, or `--path`, and then in each parent directory. Every field has one spelling and one shape. The format is version 1. Unknown keys are errors except top-level `x-` keys. [Configure your app](frameworks.md#services-and-setup) covers the setup workflow.

## version and project

```yaml
version: 1
project: shop
```

Both are required. `version` is the file format, and this Parcela reads `1`. A file without it, or with another number, is an error. `project` names the app in URLs, `https://shop.localhost:1443`, and groups every checkout of the repository. It is a DNS label: lowercase letters, digits and inner hyphens. `parcela init` writes both.

## sharing

For a project that uses an existing HTTPS proxy, declare its domain once:

```yaml
sharing:
  domain: tower.davafons.cc
  domain_port: 1480
```

`up` and `restart` use these defaults in every checkout. `domain_port` is optional and defaults to 1480; it requires `domain`. The port must be between 1024 and 65535. A project with `sharing.domain` needs an HTTP service.

Command-line flags override `PARCELA_DOMAIN` and `PARCELA_DOMAIN_PORT`, which override these project defaults. Top-level `env` configures app processes, not Parcela's sharing options. `up --no-share` or `restart --no-share` disables all sharing, including LAN/Tailscale flags and environment variables. If an existing run is shared, use `restart --no-share` to stop sharing; `up` does not change a running environment.

Parcela serves domain routes on loopback only. Your outer proxy owns certificates, DNS and network access. Projects may use different domains with the same loopback port; Parcela routes them by hostname. Domain names do not select a Tailscale network. LAN and `tailscale serve` sharing still require their flags or environment variables; they cannot be enabled in this section.

## services

`services` must contain at least one service. HTTP is optional. If any service has `http`, exactly one HTTP service must omit `subdomain`. Every long-running process is a service. Each one is a mapping with a `command`, or a `compose` file:

```yaml
services:
  web:
    command: bin/rails server -p $PORT -b 127.0.0.1
    http:
      healthcheck: /up
  jobs:
    command: bin/jobs
```

| Field | Meaning |
|---|---|
| `command` | A shell command, run with `bash -c` (`sh -c` where there is no bash) from the checkout root. Its output goes to `parcela logs`. Required unless the service has `compose`. |
| `compose` | Runs containers from a Docker Compose file instead of `command`. See [compose](#compose). |
| `http` | Makes the service reachable at an HTTPS URL. See below. |
| `healthcheck` | For services without `http`: a shell command that exits 0 once the service is ready, like `pg_isready`. |
| `depends_on` | Services to start first. Parcela waits until each one is ready: its HTTP healthcheck answers, its `healthcheck` passes, or, without either, it has started. |
| `stop` | A shell command that stops what `command` started outside its own processes, like `docker compose stop`. See [stop](#stop). |
| `working_dir` | A directory relative to the checkout root to run `command` in. |
| `env` | Variables for this service only. Values can use `${NAME}`, like the top-level `env`. |

Service names are DNS labels. `setup` and `prestart` are reserved for hooks. Dependency cycles and unknown dependencies are errors. A service cannot declare both `http` and a shell `healthcheck`.

### Watcher-only environments

An environment without `http` supervises processes without starting Caddy, offering certificate trust, or allocating HTTP ports:

```yaml
version: 1
project: sdk
services:
  build:
    command: npm run watch
```

`up`, `status`, `logs`, `restart`, `down`, hooks, dependencies, tasks, and named `ports` work without HTTP. Readiness waits for hooks to complete and services to start. A service with `healthcheck` must pass that command before the environment is ready.

`url` and `open` fail because there is no HTTP URL. `--backend-port`, `--lan`, `--lan-ip`, `--tailscale`, `--domain`, and literal `run` require an HTTP service. Status JSON has `url: null`; the session's `public_url`, `backend_port`, and `router_token` are null, and its HTTP endpoint set is empty. No `PORT`, `PARCELA_URL`, or `PARCELA_HOSTNAME` is supplied. Explicit named ports still use `PARCELA_PORT_<NAME>`.

A running proxy from an older Parcela must be restarted with the new binary before watcher-only sessions can share its registry. Parcela refuses that mixed-version start rather than changing the shared proxy itself.

### compose

Keep databases and search engines in your `docker-compose.yml`, at the versions it pins, and run the app natively:

```yaml
services:
  deps:
    compose:
      file: docker-compose.yml
      services: [postgres, elasticsearch]
  web:
    command: bin/rails server -p $PORT -b 127.0.0.1
    http:
      healthcheck: /up
    depends_on: [deps]
```

| Field | Meaning |
|---|---|
| `file` | Required. The Compose file, relative to the checkout root. Next to `compose.yaml` or `docker-compose.yml`, its override file, like `docker-compose.override.yml`, applies too, as with plain `docker compose`. |
| `services` | The Compose services to start. Compose also starts what they depend on. |
| `http` | Containers that serve HTTP, each at its own URL. See [Containers with URLs](#containers-with-urls). Their Compose services start too. |

The Compose file stays as it is, fixed ports and names included. On `up`, Parcela:

- names the environment's Compose project `<project>-<environment>`, like `shop-primary` or `shop-signup`, so each environment has its own containers, network and volumes;
- leases a port for every port the started containers publish. A service's first port is named after it, like `PARCELA_PORT_POSTGRES`, and its others add the container port, like `PARCELA_PORT_ELASTICSEARCH_9300`;
- writes an override file to the environment's runtime directory that publishes those ports on `127.0.0.1` and the leased port, and drops `container_name` and the `name` of every volume and network that is not `external`;
- runs `docker compose up` for the services in the foreground, so their output is in `parcela logs`;
- calls the service ready once each listed Compose service runs and, if it has a `healthcheck`, is healthy;
- runs `docker compose stop` as the service's [stop](#stop) command. `down` keeps the volumes.

Every service, hook and task gets `COMPOSE_PROJECT_NAME` and `COMPOSE_FILE`, so a plain `docker compose exec postgres psql` reaches the environment's own containers. Your app reads the leased port from `env`:

```yaml
env:
  DATABASE_URL: postgres://postgres@127.0.0.1:${PARCELA_PORT_POSTGRES}/shop
```

- Only one service can use `compose`. List every Compose service it should start, in `services` or `http`.
- `docker compose up` gets every variable Parcela sets, so the Compose file can use them: `APP_URL: ${PARCELA_URL:-http://localhost:3000}` keeps working without Parcela.
- It needs Docker Compose 2.24.4 or newer. `parcela doctor` checks that the file and services exist.
- Compose reads `.env` next to the Compose file for its own variables. A new worktree does not have it until it is copied, and the containers start before `setup` runs. Point Compose at the main checkout's with `COMPOSE_ENV_FILES: ${PARCELA_PRIMARY_DIR}/.env` in `env` if the containers need it.
- A built image shared by every worktree needs an `image:` name next to `build:`, or Compose builds one per environment.
- `parcela rm` removes the environment's containers and network and lists the volumes it keeps. Delete them with `docker volume rm`.

#### Containers with URLs

A container that serves HTTP, such as a mail catcher, an admin UI or an app that only runs in Docker, gets an HTTPS URL like a native service:

```yaml
services:
  deps:
    compose:
      file: docker-compose.yml
      services: [postgres]
      http:
        mail:
          service: mailpit
          port: 8025
          subdomain: mail
          healthcheck: /
```

Each key under `http` is a service: `mail` is at `https://mail-shop.localhost:1443`, has `PARCELA_SERVICE_MAIL_URL`, waits for its `healthcheck`, and is shared by `up --lan`, `--tailscale` and `--domain` like any other. Its fields are those of [HTTP services](#http-services), plus:

| Field | Meaning |
|---|---|
| `service` | The Compose service. Default: the key. |
| `port` | Required. The container port that serves HTTP. The Compose file does not need to publish it. |

Leave out `subdomain` on a container that serves the environment's own URL, for an app that runs entirely in Docker. Parcela routes only listeners its own processes hold, and Docker holds a container's published ports, so Parcela runs a small forwarder from the service's `$PORT` to the container.

A container reaches a native service at `host.docker.internal` and the service's port, like `http://host.docker.internal:${PARCELA_SERVICE_API_PORT}`. Native services listen on `127.0.0.1`. OrbStack on macOS forwards that address to the Mac's loopback. On Linux, containers cannot reach the host's loopback.

### stop

Parcela stops a service by signaling its processes. That is not enough when `command` only asks another program to run something, such as `docker run` asking Docker for a container: if the client is killed or crashes, the container keeps running. `stop` names the command that stops it:

```yaml
ports: [mail]
services:
  mail:
    command: >-
      exec docker run --rm --name "shop-mail-$PARCELA_ENVIRONMENT_NAME"
      -p "127.0.0.1:$PARCELA_PORT_MAIL:1025" axllent/mailpit
    stop: docker stop "shop-mail-$PARCELA_ENVIRONMENT_NAME"
```

A [compose](#compose) service sets its own.

- It runs whenever the environment stops: on `down`, `restart`, a failed `up` and Ctrl-C. It runs after every service that depends on this one has stopped, and before this service gets SIGTERM.
- It runs even if `command` already exited, and after Parcela itself crashed: the next `up`, `down` or `recover` runs it for a run that did not finish stopping.
- It gets the service's variables and `working_dir`, writes to `parcela logs <service>`, and has 120 seconds.
- If it fails, `down` says so and the environment stays interrupted. Run `parcela down` again, which runs it again. Write it so it can run twice.

Without `stop`, a service gets SIGTERM and, 10 seconds later, SIGKILL.

### HTTP services

A service with `http` gets a free port in `$PORT`, must listen on `127.0.0.1:$PORT`, and gets its own URL:

```yaml
services:
  web:
    command: npm run dev -- --port $PORT --host 127.0.0.1
    http:
      healthcheck: /
  api:
    command: bin/api --port $PORT
    http:
      subdomain: api
      healthcheck: /health
```

| Field | Meaning |
|---|---|
| `healthcheck` | Required. A path. `up` waits until `GET` on it returns 2xx, then opens the URLs. |
| `subdomain` | The prefix for this service's URL: `https://api-shop.localhost:1443`. Leave it out on exactly one HTTP service, which gets the environment's own URL, `https://shop.localhost:1443`. |
| `wildcard` | `true` also sends every subdomain of the service's host to it, for subdomain-per-tenant apps: `https://kobe.shop.localhost:1443`, and `https://kobe.signup-shop.localhost:1443` in the `signup` worktree. `.local` names from `--lan` cover the host only. |

Every environment of the project uses the same prefixes in front of its own name: the `signup` worktree's API is at `https://api-signup-shop.localhost:1443`.

## readiness_timeout_seconds

```yaml
readiness_timeout_seconds: 600
```

How long `up` and `restart` wait for every service to be ready, from 1 to 86400. The default is 60. Time spent in hooks does not count, but waiting for a dependency's `healthcheck` does, so raise it for services that are slow on their first start, such as a container image that is built first.

## hooks

Optional commands for preparation and removal. Each hook has a required `command`. `setup` and `prestart` also accept `depends_on`.

```yaml
hooks:
  setup:
    command: bin/setup
    depends_on: [db]
  prestart:
    command: bin/rails db:migrate
    depends_on: [db]
```

| Hook | Runs |
|---|---|
| `setup` | On the first `up` of an environment. Install packages, create and seed the database, copy secrets. `parcela up --setup` runs it again. |
| `prestart` | On every new start, after `setup`. An `up` that reuses an already-ready environment runs no hooks. Run migrations or anything that must happen before the app starts. |
| `teardown` | On `parcela rm`, after the environment stopped. Drop its database or delete data outside the checkout. It cannot use `depends_on`. For a checkout that is already gone, it runs from `PARCELA_RUNTIME_DIR` with the configuration of the last successful run. |

`setup` and `prestart` take a `command` and an optional `depends_on`. On `up`, Parcela starts the hooks' `depends_on` services, waits until they are ready, runs `setup` if it is due, then `prestart`, then starts every other service. A hook that exits nonzero stops `up` with its exit code and output, and no other service starts. `parcela logs setup` shows a hook's output again, until the next run.

A failed setup is retried on the next start. A successful setup is skipped unless `--setup` is given.

## ports

Extra ports for things that are not HTTP services, such as a database:

```yaml
ports: [db, cache]
```

- Each name arrives as `PARCELA_PORT_<NAME>`, uppercased with hyphens turned into underscores: `PARCELA_PORT_CACHE`.
- An environment keeps the same port across `down`, `up` and `restart` while it is free.
- No two running environments get the same port. Parcela does not route or check these ports.
- A [compose](#compose) service adds one for every port its containers publish.

Ports come from 20000 through 29999.

## env

Variables for every service, hook and task:

```yaml
env:
  DATABASE_URL: postgres://127.0.0.1:${PARCELA_PORT_DB}/app
  API_ORIGIN: ${PARCELA_SERVICE_API_URL}
```

Values can use `${NAME}` to insert any variable Parcela sets. A placeholder that does not resolve is an error. You cannot set `PORT`, `HOST` or names starting with `PARCELA_`, nor `COMPOSE_PROJECT_NAME` or `COMPOSE_FILE` when a service uses `compose`. Commands inherit the launching shell's environment except names starting with `PARCELA_`. Service `env` overrides top-level `env`. Use Parcela-provided variables in placeholders. Arbitrary shell variables such as `${HOME}` are unavailable to interpolation.

## tasks

Commands you run with `parcela task NAME`. They get the environment's variables:

```yaml
tasks:
  test:
    command: bin/rails test
    timeout_seconds: 600
  psql:
    command: psql "$DATABASE_URL"
```

| Field | Meaning |
|---|---|
| `command` | Required. A shell command, run from the checkout root. |
| `timeout_seconds` | Parcela stops the task after this many seconds and exits 124. |

Arguments after `--` are appended. `parcela task test -- test/models` runs `bin/rails test test/models`. `timeout_seconds` must be a positive integer when set.

## Editor support

The file format has a JSON Schema at `https://parcela.run/schema/v1.json`. Editors with the YAML language server, such as VS Code with the Red Hat YAML extension, use it for completion and validation when the file starts with this line, which `parcela init` writes:

```yaml
# yaml-language-server: $schema=https://parcela.run/schema/v1.json
```

## Extension keys

Top-level keys that start with `x-` are ignored, as in Docker Compose. Use them for YAML anchors:

```yaml
x-ruby: &ruby
  working_dir: backend
services:
  web:
    <<: *ruby
    command: bin/rails server -p $PORT
    http:
      healthcheck: /up
```

Any other unknown key is an error.

## Environment variables Parcela sets

Every service, hook and task receives these. Service names are uppercased, with hyphens turned into underscores.

| Variable | Example |
|---|---|
| `PORT` | `29381`, the service's own port. Only HTTP services get one. In tasks it is the port of the service at the environment's URL. |
| `HOST` | `127.0.0.1` |
| `BROWSER` | A shim that runs `parcela browse`, so a process asking for a browser offers the URL to the browsers showing the environment. Set `BROWSER` in `env:` to keep another browser. |
| `PARCELA_URL` | `https://shop.localhost:1443`, the environment's URL, only when HTTP is configured |
| `PARCELA_HOSTNAME` | `shop.localhost`, only when HTTP is configured |
| `PARCELA_SHARE_HOSTS` | `shop.local,mb.tail1234.ts.net,shop.dev.example.com`, the hostnames other devices use after `up --lan`, `--tailscale` or `--domain`. Empty otherwise. Used for dev-server host allow lists. |
| `PARCELA_SERVICE_<NAME>_URL` | `PARCELA_SERVICE_API_URL=https://api-shop.localhost:1443` |
| `PARCELA_SERVICE_<NAME>_HOSTNAME` | `PARCELA_SERVICE_API_HOSTNAME=api-shop.localhost` |
| `PARCELA_SERVICE_<NAME>_PORT` | `PARCELA_SERVICE_API_PORT=26798` |
| `PARCELA_PORT_<NAME>` | `PARCELA_PORT_DB=26953`, one per entry in `ports` |
| `PARCELA_ENVIRONMENT_NAME` | `primary`, or the worktree's name, like `signup` |
| `PARCELA_DATA_DIR` | The environment's private data directory |
| `PARCELA_RUNTIME_DIR` | The directory that holds `data`, `tmp`, `cache` and `log` |
| `PARCELA_PRIMARY_DIR` | The main checkout, where untracked files like `.env` usually live |
| `PARCELA_PROJECT_ID` | `shop` |
| `PARCELA_ENVIRONMENT_ID` | `7cceea8f-0fd3-4cdc-843f-0883f3c0fc5e` |
| `PARCELA_ROLE` | `primary` or `preview` |
| `PARCELA_GENERATION` | `7`, a counter that increases on every start |
| `COMPOSE_PROJECT_NAME`, `COMPOSE_FILE` | `shop-signup`, and the Compose file with the environment's override. Only when a service uses [compose](#compose). |

## State locations

| Path | Contents |
|---|---|
| `~/.local/state/parcela/` | All of Parcela's state. Override it with `--state-dir`, `PARCELA_STATE_DIR` or `XDG_STATE_HOME`. |
| `…/registry.sqlite3` | Projects, environments, names and port leases. |
| `…/environments/<id>/` | One environment's `PARCELA_RUNTIME_DIR`: `data`, `tmp`, `cache`, `log/owner.log`, each process's `log/processes/<name>.log` and Parcela's own records. |
| `…/proxy/` | The proxy's Caddy config, `proxy.log` and the local certificate authority. |
| `$(getconf DARWIN_USER_TEMP_DIR)parcela/` on macOS, `/tmp/parcela-<uid>/` on Linux | Unix sockets for the running environments and the proxy. |

The state directory is private to your user. `rm` keeps an environment's directory. A later registration gets a new directory. Retained files are not automatically reused.

## data

```yaml
data:
  files: [uploads, dictionaries]
  sqlite: [development.sqlite3]
```

`data` declares the content included by `snapshot` and `restore`. It does not change application bindings. Every path is literal, normalized, and relative to `PARCELA_DATA_DIR`. Nothing is included by default. Paths cannot overlap. `files` accepts regular files and directories; `sqlite` accepts SQLite database files and backs them up through SQLite rather than copying their main file or WAL separately.

Absolute paths, traversal, symlinks, hardlinked files, special files, `.env` files, common credential filenames, keys, and SQLite sidecars are refused. Raw SQLite databases under `files` are refused too. This is not a secret scanner: declared databases and uploads can contain private application records. Snapshots stay private to the current user and are not uploaded.

Snapshots contain only declared data and integrity metadata. Source files, environment variables, process state, setup markers, and undeclared test data are excluded. PostgreSQL, Redis, Compose volumes, and other external data need project-defined export and import tasks. Export a consistent artifact to a declared file before snapshotting it.
