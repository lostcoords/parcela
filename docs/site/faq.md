# FAQ

## What does Parcela change on my machine?

Parcela stores its registry, logs, certificates, and environment data in a private state directory. The default is `~/.local/state/parcela`. Unix sockets live in a per-user temporary directory. [State locations](configuration.md#state-locations) lists the paths and overrides.

`parcela init` writes `parcela.yaml` in your checkout. Your configured commands and hooks can write other files or change external services.

The shared proxy listens on `127.0.0.1:1443` by default. Backends use loopback ports leased from 20000 through 29999. LAN and tailnet access require a sharing flag for that run. System services require `parcela proxy install`.

Parcela sends anonymous command telemetry unless disabled. [Telemetry](telemetry.md) describes its fields and controls.

## What does trusting the certificate authority allow?

Caddy generates a local root certificate so browsers can accept Parcela's HTTPS URLs. Trust is added only by `parcela trust` or a first `up` in a terminal, behind the system password prompt.

The CA has no name constraints. Anyone who obtains its private key can issue certificates your system accepts for any hostname. The key lives under the private state directory at `proxy/data/pki/authorities/local/root.key`. Keep that directory out of shared backups and dotfiles repositories.

Use `parcela trust` to add trust. To undo it, run `caddy untrust --cert "$(parcela trust --print)"` before deleting the state directory. Set `PARCELA_NO_TRUST=1` to suppress automatic terminal trust attempts.

## Why port 1443?

A normal user can bind port 1443 without `sudo`. Parcela refuses occupied ports rather than taking them over. On macOS, `parcela proxy install` installs a password-protected launchd relay on loopback port 443. It also relays port 80 when free. Restart environments to update their URLs. `parcela proxy uninstall` removes it.

On Linux, explicitly allow unprivileged port 443 with `sudo sysctl net.ipv4.ip_unprivileged_port_start=443`, then run `parcela proxy start --port 443`. This system setting affects all local processes.

## Why use .localhost names?

Names such as `signup-shop.localhost` identify an environment while keeping traffic on the local machine. Browsers treat `.localhost` specially, so no project DNS setup is needed. Other clients depend on their resolver. See [Clients and certificates](#clients-and-certificates).

## How is Parcela different from a hostname proxy?

Parcela runs a declared group of processes per checkout. That includes workers and databases alongside web servers, dependencies and readiness checks, setup hooks, environment data directories, and one stop command for the group. Caddy handles the HTTPS proxying.

## When is a container a better fit?

Use containers when you need isolated execution or exact runtime images. Parcela runs native commands as your user, so editors, debuggers, and file watchers use the local filesystem directly.

A `compose` service runs databases, search engines and other containers from your `docker-compose.yml`, at the versions it pins, with their own containers, volumes and loopback ports in every environment. A container that serves HTTP can get its own URL. See [Docker Compose](frameworks.md#docker-compose).

See [Docker Compose](frameworks.md#docker-compose).

## How does it fit with coding agent tools?

An agent manager can create checkouts and run agents. Parcela runs the app inside each checkout. The embedded operating skill tells an agent how to select, inspect, start, and stop its own environment. See [Worktrees and agents](worktrees.md).

## Does each worktree get its own database?

Only when the configuration gives it one. Parcela separates processes, ports, URLs, and logs. A shared `.env` can still point every checkout at one database. [Keep environment data separate](frameworks.md#separate-data) covers SQLite, PostgreSQL, Redis, and uploads.

## Does it work on Windows or WSL?

Windows is unsupported. Parcela uses Unix sockets, process groups, and `flock`. WSL 2 is untested. A Windows browser would also need to reach the WSL listener and trust its CA.

## Is the configuration stable?

`parcela.yaml` requires `version: 1`. Missing or unsupported versions are rejected. Parcela is experimental, so the format may change. Breaking changes belong in the [changelog](https://github.com/lostcoords/parcela/blob/main/CHANGELOG.md).

## Why AGPL?

The project uses AGPL-3.0-only to keep improvements to Parcela available as source, including changes to hosted versions. See [COPYRIGHT](https://github.com/lostcoords/parcela/blob/main/COPYRIGHT) and [LICENSE](https://github.com/lostcoords/parcela/blob/main/LICENSE) for the terms. There is no contributor license agreement.


## Clients and certificates


### Name resolution

Browsers resolve `localhost` and every subdomain of it to loopback without asking DNS. curl 7.85.0 and later do the same. Other programs ask the system resolver:

- **macOS.** The system resolver answers any `*.localhost` with `::1` and `127.0.0.1`, as checked on macOS 26 with Python, Node, Ruby and Go. The proxy listens on 127.0.0.1 only, so a client must fall back from `::1`; common ones do.
- **Go's own resolver** does not resolve `*.localhost`: `lookup foo.localhost: no such host` on Go 1.27 with `GODEBUG=netdns=go`. On Linux, Go hands `*.localhost` to glibc only when `/etc/nsswitch.conf` lists `myhostname` or `resolve` for hosts and the binary has cgo. A `CGO_ENABLED=0` binary always uses its own.
- **Linux.** glibc resolves `*.localhost` only through systemd-resolved or [`nss-myhostname`](https://www.freedesktop.org/software/systemd/man/latest/nss-myhostname.html). Without them, add the names to `/etc/hosts`, or call the backend port.

For calls from the app itself, the backend port in `PARCELA_SERVICE_<NAME>_PORT` avoids the question.

### Certificate bundles

`parcela trust` adds Parcela's certificate authority to the system trust store. Programs with their own CA list do not see it. `parcela trust --print` prints the root certificate's path and changes nothing. It is in the state directory, by default `~/.local/state/parcela/proxy/data/pki/authorities/local/root.crt`, and exists once the proxy has started.

| Client | Variable | Effect |
|---|---|---|
| Node | `NODE_EXTRA_CA_CERTS` | Adds to Node's built-in list. Read at startup. |
| Node 22.19+, 24.6+ | `NODE_USE_SYSTEM_CA=1` | Also trusts the system store, so `parcela trust` is enough. Same as `--use-system-ca`. |
| Python `requests` | `REQUESTS_CA_BUNDLE` | Replaces certifi's bundle. |
| Python httpx | `SSL_CERT_FILE` | Replaces certifi's bundle. |
| Python `ssl`, `urllib` | `SSL_CERT_FILE` | Replaces OpenSSL's default file. |
| Ruby `Net::HTTP` | `SSL_CERT_FILE` | Replaces OpenSSL's default file. |
| Go | `SSL_CERT_FILE` | Replaces the default file on Linux. On macOS Go uses the system store, so `parcela trust` is enough; from Go 1.27, setting the variable there replaces the store too. |
| curl | `--cacert` | `curl --cacert "$(parcela trust --print)" "$(parcela url)"` |

A bundle containing only the local root omits the public roots used by other HTTPS calls, so create a combined bundle first:

```sh
mkdir -p ~/.local/share
cat "$(python3 -m certifi)" "$(parcela trust --print)" > ~/.local/share/dev-ca.pem
export REQUESTS_CA_BUNDLE=~/.local/share/dev-ca.pem
```

Export the variable in the shell that runs `parcela up`, since commands inherit it. To set it in `parcela.yaml` instead, build the path from the runtime directory, which sits in the same state directory:

```yaml
env:
  NODE_EXTRA_CA_CERTS: ${PARCELA_RUNTIME_DIR}/../../proxy/data/pki/authorities/local/root.crt
```

Inside a service, `parcela trust --print` does not see a `PARCELA_STATE_DIR` from your shell, because Parcela removes inherited `PARCELA_` variables. The path above follows any state directory.

The [Node CA options](https://nodejs.org/api/cli.html#node_extra_ca_certsfile) and [Go system certificate pool](https://pkg.go.dev/crypto/x509#SystemCertPool) describe the runtime-specific trust behavior.

## Upgrade Parcela

Read the [changelog](https://github.com/lostcoords/parcela/blob/main/CHANGELOG.md) for release-specific steps. Then use the method that installed Parcela.

| Installation | Command |
| --- | --- |
| Shell installer | `parcela upgrade` |
| Homebrew | `brew upgrade parcela` |
| Cargo | `cargo install --git https://github.com/lostcoords/parcela --locked --force` |

`parcela upgrade` installs the latest release. Use `--nightly` for the newest build of `main`. For Homebrew and Cargo installations, it prints the appropriate command instead of replacing the binary.

The installer refreshes an older running proxy automatically. If you update through Homebrew or Cargo, the next regular Parcela command does that refresh. Application processes keep running; their URLs briefly disconnect. Stopped proxies stay stopped.

Interactive commands check for a stable release at most once a day and offer an update for shell-installer installations. Accepting resumes your original command with the new CLI. Piped commands and `--json` never prompt. Set `PARCELA_NO_UPDATE=1` to disable release checks.

A CLI installed before automatic update checks needs one explicit `parcela upgrade` to acquire this behavior.

## Remove Parcela

Save any data you need before removing state. A configured `hooks.teardown` can delete external data when `rm` runs.

1. Stop all environments.

   ```sh
   parcela down --all-projects
   ```

2. In each checkout whose teardown hook you want to run, run `parcela rm`.
3. If you trusted the certificate authority, remove that trust before deleting its certificate.

   ```sh
   caddy untrust --cert "$(parcela trust --print)"
   ```

4. On macOS, if you installed the port 443 relay, run `parcela proxy uninstall`.
5. Stop the proxy.

   ```sh
   parcela proxy stop
   ```

6. Remove the binary with `brew uninstall parcela`, `cargo uninstall parcela`, or `rm ~/.local/bin/parcela`, matching your installation.
7. If you want to delete all retained environment data, delete the state directory. Its default is `~/.local/state/parcela`. Check any `--state-dir`, `PARCELA_STATE_DIR`, or `XDG_STATE_HOME` override first.
8. Remove the socket directory if it remains. It is `$(getconf DARWIN_USER_TEMP_DIR)parcela` on macOS or `/tmp/parcela-$(id -u)` on Linux.

Remove `~/.local/bin/caddy` only if the installer added it and nothing else uses it. Data your commands created elsewhere remains until you remove it yourself.
