# Architecture

Parcela turns one checkout's configuration into a supervised environment. It owns identity, process lifecycle, and reservations. A dedicated Caddy instance owns TLS and HTTP routing. The project owns commands, preparation, and data policy.

## Ownership

| Component | Responsibility |
| --- | --- |
| CLI | Finds configuration, selects an environment, and renders text or JSON. |
| Registry | Stores projects, environments, sessions, names, and port leases in SQLite. |
| Environment owner | Holds the ownership lock, runs the process graph, records state, and handles lifecycle requests. |
| Shared proxy process | Supervises Caddy and refreshes routes for running environments. |
| Caddy | Terminates TLS and routes requests using private configuration, storage, and an admin socket. |
| Project | Defines commands, dependencies, hooks, and external effects in `parcela.yaml`. |

## Checkout and data ownership

`create` delegates checkout creation to the repository's VCS, then registers its preview identity. It never starts the process graph. `discovery` remains read-only. Source creation and data restoration are separate operations; failures retain any created checkout with an explicit error rather than deleting source.

Snapshot policy is parsed from `data` in `parcela.yaml`. It is not serialized into the supervised graph: the proxy and environment owners need process configuration, not permission to copy data. Snapshot metadata stores the data policy separately. `snapshot` and `restore` take the environment's restart, owner, and task locks and require a stopped session. Data is staged in a private directory and published with an atomic no-replacement rename. SQLite uses its backup API, never a live main-file/WAL copy.

The CLI accepts only standard input, output, and error as inherited descriptors. Before starting threads it closes other inherited copies, so a caller's temporary socket or ownership lock cannot survive in Parcela or its children.

## Startup and readiness

`parcela up` performs these steps:

1. Find and validate the nearest `parcela.yaml`.
2. Register the checkout on first use. A main checkout gets the project URL. A linked jj workspace or Git worktree gets a preview name.
3. Reserve service hostnames and lease backend ports from 20000 through 29999, including each port a `compose` service's containers publish. Port leases use one transaction.
4. Start the shared proxy if HTTP endpoints are configured. Watcher-only environments skip the proxy and trust setup.
5. Build the process graph. Start hook dependencies, setup if due, prestart, and the remaining services in dependency order.
6. Wait for hooks to complete, services to start, and explicit readiness checks to pass. For HTTP services, confirm that listeners belong to the graph and readiness paths return 2xx.
7. Publish the environment's HTTP routes, if any. A detached start returns when its owner reports readiness.

The owner holds `owner.lock`, serves its control socket, and saves `owner.json`. Each command runs under `bash -c`, or `sh -c` without bash, in a separate process group. Each process writes its own log.

A dependency is ready after its readiness check passes, after it starts if it has no check, or after it exits 0 if it is a hook. Setup enters the graph only until it has succeeded. Ordinary services do not restart automatically. Sharing publishers can restart up to 10 times after failure.

## Shutdown and recovery

`down` asks the owner to withdraw routes and stop the graph. Once nothing that depends on a service is running or stopping, the owner runs the service's `stop` command, if it has one, then sends SIGTERM to its process group. After 10 seconds, remaining groups receive SIGKILL. A `stop` command runs even when its service already exited, because it manages what lives outside the process tree, like containers. Each success is saved in `owner.json`, so an owner that adopts a half-stopped run runs only the commands still owed. The run is stopped only when no recorded process is alive and every `stop` command succeeded. A failed one leaves the run interrupted until `down` runs it again.

`restart` validates the replacement configuration before stopping the old run, then starts a new generation. Sharing belongs to that generation. Domain defaults come from `sharing` in the project configuration; CLI and environment values override them. `--no-share` disables all sharing for the new generation. Detached owners receive the resolved sharing flags without reapplying defaults.

If the owner dies, services may survive. The next lifecycle operation acquires the ownership lock and compares recorded PIDs, start times, and boot identity with live processes. Matching processes are adopted. Parcela never signals a bare PID from disk. A small check-to-signal PID reuse race remains possible.

## Proxy and route leases

The background `parcela proxy run` process supervises one Caddy instance. It renews active routes twice per second. Routes expire three seconds after the last renewal, preventing stale routes from remaining active.

If the proxy dies, environment owners attempt to restart it within about two seconds and republish routes. An explicit `proxy stop` leaves a stopped marker that prevents this recovery. App services keep running while the proxy is stopped.

The proxy does not start at login or reboot. The next `up` starts it. By default it listens only on `127.0.0.1:1443`. A busy listener produces an error naming its owner.

## Browser inventory

The shared proxy also owns a read-only dashboard at `parcela-ui.localhost`, on its existing loopback HTTPS listener. That hostname is reserved when adopting service names. Caddy forwards dashboard requests to a private Unix socket; the dashboard adds no public TCP listener and never exposes Caddy's admin API or owner control sockets.

An inventory worker reads the registry and probes live owners outside the route-renewal loop. Owner probes share a short budget; missed replies appear as unknown. A separate worker serves a cached, allowlisted browser response with a timestamp. It omits saved sessions, owner credentials, commands, and environment variables. Checkout discovery remains read-only. The CLI overview and dashboard share the inventory collector.

The dashboard can embed a ready app in an iframe, preserving the app's origin and response policy. An optional script supplied by the app opens the dashboard in a side panel. Parcela does not transform proxied application HTML or relax its CSP. App framing policies may require direct navigation instead. Dashboard routes exist only on the loopback HTTPS server, never the LAN or domain servers; Tailscale still reaches app backends directly.

## Sharing for one run

LAN sharing records the selected interface address with the run. Caddy adds a server on that address and the HTTPS port for the shared environment's `.local` names. It serves those names only while the interface still holds the address. If the LAN listener cannot load, the proxy retries configuration without it so loopback routes can remain available.

mDNS publishers use `dns-sd` on macOS or `avahi-publish` on Linux. Tailscale sharing chooses free HTTPS ports from `tailscale serve status` and starts one foreground serve process per service. Tailnet requests reach backend ports directly.

Domain sharing records the domain and a loopback port with the run. Caddy adds a plain-HTTP server on `127.0.0.1` at that port for `<name>.<domain>`, trusting forwarded headers from loopback. The person's own proxy terminates TLS in front of it, so Parcela never holds a public certificate or DNS credentials. Like the LAN server, a domain listener that cannot load is dropped so loopback routes remain available.

Publishers belong to the process graph and stop with the run. Foreground Tailscale serve entries disappear when their owning process exits, including after SIGKILL. Parcela never uses Funnel.

## Durable state and sockets

The state directory follows this precedence:

1. `--state-dir`.
2. `PARCELA_STATE_DIR`.
3. `$XDG_STATE_HOME/parcela`.
4. `~/.local/state/parcela`.

The directory has mode 700. It contains the following records:

| Path under state | Contents |
| --- | --- |
| `registry.sqlite3` | Projects, registrations, names, and port leases. |
| `proxy/` | Caddy configuration, frontdoor configuration, logs, certificates, and the CA. |
| `environments/<uuid>/` | Context and owner records, hook markers, logs, data, temporary files, and cache. |

Control and Caddy admin sockets use a hash of the state path under a per-user temporary directory. The base is `parcela` inside macOS's per-user temporary directory or `/tmp/parcela-<uid>` on Linux. Socket placement does not depend on environment variables. This keeps processes in agreement and paths within Unix socket length limits.

`rm` removes registration, not source or runtime files. A configured teardown hook can change external data. Read-only inspection does not register a checkout or start processes.

## Source map

| Module | Responsibility |
| --- | --- |
| `config` | Parses format version 1 and validates manifests. |
| `compose` | Runs a `compose` service: reads the ports its containers publish from `docker compose config` on `up`, writes the environment's Compose override, and forwards a `compose.http` service's `$PORT` to its container, so routing still reaches only listeners the graph holds. |
| `registry`, `ports` | Stores identity and reservations, leases backend ports. |
| `startup`, `runtime` | Registers a checkout and coordinates startup and readiness. |
| `supervisor`, `logs` | Runs the process graph and retains process output. |
| `owner`, `control`, `lifecycle` | Owns environments and handles stop, restart, and recovery. |
| `router`, `proxy`, `frontdoor`, `relay` | Builds Caddy configuration and runs the proxy and optional macOS relay. |
| `share` | Plans LAN, Tailscale and domain sharing and allowed host variables. |
| `process`, `http` | Checks process identity, listener ownership, and readiness. `http` is a client. |
| `discovery` | Reads jj and Git checkout inventory without modifying it. Unresolvable jj roots are reported and omitted without hiding live checkouts. |
| `checkout` | Creates explicit Git worktrees or jj workspaces without starting services or copying data. |
| `snapshots` | Saves declared environment data with SQLite backups and checksums; restores only to absent data directories. |
| `private`, `sockets` | Owns private files, locks, and socket placement. |
| `inventory`, `dashboard` | Collects read-only environment status and serves the local browser dashboard and switcher. |
| `src/cli` | Parses arguments, selects commands, and renders output. |

## Privileged changes

CA trust changes occur only through `parcela trust` or a first terminal `up`, behind the system password prompt. Nonterminal runs never change trust.

`parcela proxy install` is the only system-service installation path. On macOS, launchd owns loopback port 443 and relays connections through `nc` as the user. Parcela code does not run as root. Packet-filter redirection is not used.

For contributor commands and live verification, see [Contribute to Parcela](../CONTRIBUTING.md). For site checks, see [Build the documentation site](website/README.md).
