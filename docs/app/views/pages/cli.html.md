# CLI reference

`parcela [GLOBAL OPTIONS] COMMAND` acts on the current checkout unless another target is selected. With no command, it shows checkout status or the next setup step. `parcela <command> --help` lists its public options.

## Global options

| Option | Meaning |
| --- | --- |
| `-C`, `--path DIR` | Checkout directory. Configuration lookup walks from this directory to its parents. Defaults to the current directory. |
| `-w`, `--workspace ENV` | A registered environment, from any directory: one of its URLs or hostnames, with or without `.localhost` and the port, its name when only one environment has it, or its ID from `parcela ls --json`. Conflicts with `--path`. |
| `--state-dir DIR` | State directory. Precedence is this option, `PARCELA_STATE_DIR`, `$XDG_STATE_HOME/parcela`, then `~/.local/state/parcela`. |
| `--json` | JSON output even in a terminal. |
| `--help`, `-h` | Help. |
| `--version`, `-V` | Version. |

## Output

Commands print text in a terminal and one JSON document when stdout is piped or `--json` is given. Exceptions are `url` and `logs`, which print plain text, and `task` and `run`, which pass through command output. Errors go to stderr with the prefix `parcela:`. `up --detach` shows a progress line on stderr while it waits, in a terminal outside CI.

Colors appear only in a terminal. `CLICOLOR_FORCE=1` or `FORCE_COLOR=1` keeps them when output is piped, for `less -R` or CI logs; `NO_COLOR` turns them off and wins over both. Forcing colors never switches a piped command from JSON to text.

For example, `parcela status --current --json | jq -r .state` reads the environment state.

| Exit code | Meaning |
| --- | --- |
| `0` | Success. |
| `1` | Command failure, failed doctor check, or no proxy for `proxy status`. |
| `2` | Invalid arguments. |
| `124` | A task exceeded `timeout_seconds`. |
| Other | Exit code from the command executed by `task` or `run`. |

## init

```text
parcela init [--name NAME] [--command CMD]
```

Writes a minimal `parcela.yaml` without replacing an existing file. The command is inferred from `package.json`, Rails, or Django, with a Python file server as fallback. `--command` overrides the guess. `--name` sets the project name.

## create

```text
parcela create NAME [--directory DIR] [--revision REV] [--from SNAPSHOT]
```

Creates and registers a preview without starting services or running setup. Git repositories get a detached worktree at `HEAD` by default. jj repositories get a named workspace based on `@`, including the changes jj snapshots from the source checkout. `--revision` chooses another revision. jj takes precedence in colocated repositories; a jj failure never falls back to Git. Other VCS backends are unsupported.

The default directory is a sibling named `<repository>-<NAME>`, or `<repository>-<NAME>` in [`worktrees.directory`](#config), which is created if missing. `--directory` chooses a new directory whose parent already exists. Existing directories, workspace names, and registered environment names are refused. Git checkout hooks are disabled. Ignored files are not copied; files already committed to Git or tracked by jj follow that VCS's rules.

`--from` restores a data snapshot before the first start. It does not restore source history, secrets, setup markers, or running processes. JSON includes `checkout` with the path, VCS and source revision, `workspace` with the environment ID, and `state: stopped`. A creation failure retains any checkout already created and names it in the error.

## snapshot

```text
parcela snapshot NAME
```

Saves the selected environment's [declared data](/docs/configuration/#data) under `<state>/snapshots/<project>/<NAME>`. The environment must be stopped with no owner, restart, or task operation holding its locks. Each SQLite database uses SQLite's backup API, including committed WAL contents. Snapshot names are immutable. There is no automatic seeding, retention limit, or snapshot deletion command.

## restore

```text
parcela restore NAME
```

Restores a snapshot into a fresh registered environment whose data directory does not exist. Existing data is never replaced, including an empty data directory. Project and data declarations must match. Files are staged and checksummed before publication, and a failed restore publishes no partial data. Snapshot and restore require macOS or Linux with atomic no-replacement directory rename support.

Restore does not start services or mark setup complete. The next `up` runs the normal setup and prestart hooks. Those commands must preserve restored data. Seed and external database export/import commands remain explicitly configured [tasks](#task).

## up

```text
parcela up [--detach] [--setup] [--name NAME | --primary] [--backend-port N] [--lan] [--lan-ip IP] [--tailscale] [--domain DOMAIN ... [--domain-port N]]
```

Registers the checkout on first use and starts the configured services. If HTTP services are configured, it starts the proxy as needed and opens routes after readiness. Without HTTP services, it supervises processes without a proxy or URL. A ready environment is reused. If the owner process died but services survived, `up` adopts the recorded processes instead of starting duplicates.

| Option | Effect |
| --- | --- |
| `--detach`, `-d` | Returns after readiness and leaves services running. Without it, process output streams in the foreground and Ctrl-C stops the run. |
| `--setup` | Runs `hooks.setup` again. Otherwise setup runs only until its first success per environment. |
| `--name NAME` | Chooses a preview name on first registration. Conflicts with `--primary` and `--workspace`. |
| `--primary` | Gives a checkout the project's main URL if available. Conflicts with `--workspace`. |
| `--backend-port N` | Sets the default HTTP service's backend port. Minimum 1024. Other ports are leased automatically. |
| `--lan` | Shares this run at `.local` names on the LAN. Also set by `PARCELA_LAN=1`. |
| `--lan-ip IP` | Chooses the LAN IPv4 address. |
| `--tailscale` | Shares this run through `tailscale serve`. Also set by `PARCELA_TAILSCALE=1`. Never uses Funnel. |
| `--domain DOMAIN` | Also serves each service at `<name>.DOMAIN`, in plain HTTP on loopback for your own HTTPS proxy. Repeat it for more domains; they share `--domain-port`. Also set by `PARCELA_DOMAIN`, comma-separated. |
| `--no-share` | Disables all sharing for this run, including host defaults and environment variables. Use `restart --no-share` for an already shared run. |
| `--domain-port N` | The loopback port for `--domain`, 1480 by default. Also set by `PARCELA_DOMAIN_PORT`. |

`hooks.prestart` runs on every new start after setup. A hook failure, an HTTP service exit, or a nonzero process exit before first readiness stops startup. A non-HTTP process may finish with code 0.

After readiness, ordinary services are not restarted automatically. Exits are reported. If an HTTP service stops listening for longer than the readiness timeout, `readiness_timeout_seconds` in `parcela.yaml` (60 seconds by default), the environment stops. Sharing publishers have a separate retry policy.

For an HTTP environment, the first `up` in a terminal may offer system CA trust. `PARCELA_NO_TRUST=1` suppresses it. Nonterminal runs never change trust.

## down

```text
parcela down [--all | --all-projects]
parcela stop [--all | --all-projects]
```

Stops the selected environment in reverse dependency order. Once nothing that depends on a service is running, its `stop` command runs, if it has one, then its process group receives SIGTERM, then SIGKILL after 10 seconds if needed. If a `stop` command fails, `down` says so; run it again. Repeated stops are harmless. Source, data, and registered names remain.

`--all` stops this project's environments. `--all-projects` stops environments across projects. Both conflict with `--workspace` and with each other.

## restart

```text
parcela restart [--detach] [--setup] [--backend-port N] [--lan] [--lan-ip IP] [--tailscale] [--domain DOMAIN ... [--domain-port N]]
```

Checks the replacement configuration, then stops and starts the whole environment. New URL reservations happen after the old run stops. Options have the same meanings as for `up`. Sharing flags must be supplied for the new run.

## status

```text
parcela status
parcela status --current
parcela --workspace ENV status
```

With no selection, shows an overview of registered environments across projects, including their state, URL, and source path. The current checkout has a `*` marker and its project appears first. Inside a project, unregistered sibling worktrees also appear as `not started`. The overview works outside a project and never registers or starts environments.

`--current` shows the current checkout's registered URLs, live HTTP health, process state, and named ports. `--workspace ENV` selects that detailed view from any directory. Bare `parcela` keeps the detailed checkout view.

Overview JSON uses the same envelope as `ls`: `{runtime_verified, workspaces, unregistered}`. Overview rows have a `current` marker and a `urls` map. Registered rows also include `processes`. Scripts that previously read `.state` from plain `status --json` must use `status --current --json` or select `--workspace ENV`.

| Environment state | Meaning |
| --- | --- |
| `ready` | The configured processes have started, hooks have completed, and explicit readiness checks pass. |
| `starting` | Startup is in progress. |
| `unhealthy` | A live health check fails. |
| `stopping` | Shutdown is in progress. |
| `stopped` | The environment is stopped. |
| `unknown` | The supervisor cannot be contacted reliably. |

Process states are `pending`, `running`, `stopping`, `restarting`, `completed`, `exited`, and `skipped`. `completed` means exit code 0. `skipped` means a dependency stopped before the process could start. The JSON response includes a `diagnostic` field for failures.

## ls

```text
parcela ls
```

Lists registered environments across projects. Inside a project, it also lists unregistered sibling jj workspaces and Git worktrees as `not started`. Listing creates no registrations.

## ui

```text
parcela ui
```

Prints the local dashboard URL, `https://parcela-ui.localhost:1443` with the default proxy port. JSON output is `{url}`. The proxy must already be running; this command starts nothing and opens no browser. After upgrading, the next regular command refreshes an older running proxy automatically. The hostname `parcela-ui.localhost` is reserved for the dashboard.

The dashboard groups registered environments by project, discovers unregistered worktrees of known projects, and shows service URLs and process states. Search filters the list. Status refreshes in the background; a failed connection or an old timestamp marks the display as stale. The dashboard only shows status and navigation.

**Open directly** opens a ready service at its own URL. **In workspace** keeps the switcher beside the app in an iframe. Relative app links remain on the app's origin. Apps that set `X-Frame-Options` or a restrictive `frame-ancestors` policy may refuse embedding. Login, cookies, and top-level navigation can also behave differently inside a frame. Use **Open directly** in those cases.

To add Parcela's debug menu to your own app in development, include its script from the app's own host:

<code>&lt;script defer src="/_parcela/widget.js"&gt;&lt;/script&gt;</code>

Every `.localhost` app host answers `/_parcela/` for its own environment, so the script loads from the app's own origin. LAN and domain names pass `/_parcela/` to the app like any other path, so the menu does not appear there. A `parcela` badge sits in the page's top-right corner, with a count of new items your app opened. It opens a panel with three tabs:

- **Opened** lists what the environment's processes asked a browser to open; see [browse](#browse). Remove items one by one or with Clear all. Removal is kept per browser.
- **Environments**, on an app page, shows only that page's environment, with Restart, Stop and Logs. **Manage all** opens the same menu on `parcela-ui.localhost`, which lists every project and worktree Parcela knows, stopped and unregistered ones included, and gives registered ones Open, Start, Stop, Restart and Logs. Run `parcela up` in a checkout once to register it. Actions ignore `PARCELA_LAN`, `PARCELA_TAILSCALE` and `PARCELA_DOMAIN`, but [host `sharing`](#config) applies. Actions run `parcela up --detach`, `down` or `restart --detach` in the background; the row shows progress and the last error line.
- **Logs** tails each process of the chosen environment and follows new output. It keeps the program's own terminal colors and highlights timestamps, HTTP methods and statuses, durations, and error and warning lines. Filter narrows to matching lines; Wrap toggles line wrapping, on by default on narrow screens.

The expand button grows the panel to nearly the whole window, and the open button shows the same menu as a full page at `/_parcela/` in a new tab.

Escape, the close button or a click outside closes the panel. Actions accept only same-origin requests that carry `X-Parcela-Action`. A script on an app page can see, start, stop and read the logs of that page's own environment only; other environments need the dashboard's own name.

## browse

```text
parcela browse TARGET...
```

Offers http(s) URLs, `file://` URLs or absolute file paths to the browsers showing this environment. Processes get it as `BROWSER`, through a shim at `bin/browser` in the runtime directory, so a mail preview, an OAuth helper or a `rails` command that opens a browser reaches the person instead of the machine running the server. It only runs inside an environment and starts nothing.

Each `.localhost` page running the debug menu script shows a toast with an **Open** link for requests made from 15 seconds before the page loaded onward, once per tab. A URL on the environment's own host becomes a path, so it opens on the name the browser is using. A file opens at `/_parcela/files/ID/NAME`; page assets beside it (HTML, CSS, images, icons and fonts, but never dotfiles) resolve too, so relative links work. Opened files are served with a sandbox policy and never run scripts on the app's origin. Parcela keeps the last 20 requests per environment.

What the environment opened, such as mail previews with sign-in links, is offered only on `.localhost` names. LAN, domain and Tailscale names do not serve `/_parcela/`.

## logs

```text
parcela logs [SERVICE] [--tail N] [--follow]
```

Prints output from the latest run's services, workers, and hooks. Without a name, lines carry process-name prefixes. `--tail` defaults to 100 lines per process and accepts 1 through 10000. `--follow`, or `-f`, streams until interrupted or the environment stops.

Logs remain after shutdown until the next start. Files are under `log/processes/<name>.log` in the runtime directory, with one rotated `<name>.log.1` after 16 MB. Detached owner output is in `log/owner.log`.

## url

```text
parcela url [SERVICE]
```

Prints the default service's URL or the named HTTP service's URL. Registered URLs remain available while stopped. Environments without HTTP services have no URL, so `url` and `open` return an error.

## open

```text
parcela open [SERVICE]
```

Opens the default service's URL, or the named service's, in the browser set in `$BROWSER`, or the system's default browser. Refuses a stopped environment.

## doctor

```text
parcela doctor
```

Checks Caddy, `lsof`, `ps`, configuration, the state directory, and the proxy without changing them. Each failure includes a fix. Exit code 1 means at least one check failed. Watcher-only projects skip Caddy and `lsof`, but check compatibility with any live shared proxy. HTTP protocol differences are warnings, as for `up`; watcher-only starts refuse an incompatible proxy. Browser trust and application readiness are not checked.

JSON has the shape `{ready, checks: [{name, ok, detail, fix}], notes}`.

## rm

```text
parcela rm [--prune]
```

Forgets a stopped environment and releases names and ports. Refuses a running environment. A configured `hooks.teardown` runs first. Then, for a `compose` service, Parcela removes the environment's containers and network and lists the volumes it keeps. If either fails, registration remains. Parcela itself retains source and the runtime directory.

`--prune` applies to stopped environments whose checkout directory no longer exists. It conflicts with `--workspace`.

## task

```text
parcela task NAME [-- ARGS...]
```

Runs a configured task with the environment's variables. The environment must be registered, through `create` or `up`, and may be stopped. Arguments after `--` are appended to the task command. Output and exit status pass through, except a timeout exits 124.

## run

```text
parcela run [--name NAME | --primary] [--backend-port N] -- COMMAND...
```

Runs a foreground command as the project's only HTTP service. Requires a `parcela.yaml` with a single service. The command must listen on `127.0.0.1:$PORT`.

Arguments execute without a shell. Shell expansion requires an explicit shell, for example:

```sh
parcela run -- sh -c 'exec python3 -m http.server "$PORT" --bind 127.0.0.1'
```

## proxy

```text
parcela proxy start [--port N]
parcela proxy stop
parcela proxy restart [--port N]
parcela proxy status
```

One background Caddy serves all environments. Owners recover an unexpected proxy exit. An explicit `proxy stop` keeps it stopped until `proxy start` or the next `up`.

| Subcommand | Effect |
| --- | --- |
| `start` | Starts the proxy. `--port` chooses and saves its port. An occupied port causes an error naming its owner. |
| `stop` | Stops routing for every environment. App processes keep running. |
| `restart` | Replaces the proxy, including after an upgrade. Briefly interrupts URLs. |
| `status` | Shows the port, PIDs, version, and log path. Exits 1 if stopped. |
The default port is 1443. `PARCELA_HTTPS_PORT` applies when no port is saved. Port 443 omits the port from URLs and requires permission to bind it; [`relay`](#relay) is the usual way to get URLs without a port. The proxy redirects plain HTTP on its listener to HTTPS.

## relay

```text
parcela relay [status | install | uninstall]
```

Serves URLs without a port. The relay is not the proxy: it is a system service that forwards loopback port 443, and port 80 when free, to the proxy's port, as your user. Without a subcommand, shows whether it is installed.

| Subcommand | Effect |
| --- | --- |
| `status` | Shows whether the relay is installed, whether it takes port 80, and the port it forwards to, as installed. JSON adds `proxy_port`, this state directory's proxy port. |
| `install` | Installs the relay after a password prompt. Refuses an occupied 443. On macOS it is a launchd daemon running `nc`; on Linux, systemd socket units running `systemd-socket-proxyd`. |
| `uninstall` | Removes the relay. |

Restart running environments after either change so their URLs update. `proxy install` and `proxy uninstall` still work as aliases.

## autostart

```text
parcela autostart [status | enable | disable]
```

Starts again, at login, the environments that were running when the machine went down. Environments stopped with `down` stay stopped, as do checkouts that no longer exist. Off by default. Without a subcommand, shows whether it is on and how many environments it would start.

| Subcommand | Effect |
| --- | --- |
| `status` | Shows whether autostart is on. |
| `enable` | Installs a per-user service that runs once at login: a LaunchAgent on macOS, a systemd user unit on Linux. No password prompt. It keeps the current `PATH` and `PARCELA_CONFIG`. |
| `disable` | Removes the service. |

Each environment starts with `parcela up --detach` in its checkout, shared as its last run was: the same domains, Tailscale, and LAN on whatever address the machine has now. A run that shared nothing starts with `--no-share`. Output goes to `autostart.log` in the state directory. On a Linux machine nobody logs in to, run `loginctl enable-linger` so user services start at boot.

## trust

```text
parcela trust [--print | --export | --remove]
```

Adds Caddy's local CA to the system trust store using `caddy trust`. Prints the certificate and undo command before the system password prompt. All projects use the same CA.

| Flag | Effect |
| --- | --- |
| `--print` | Prints the certificate path. Changes nothing. |
| `--export` | Prints the certificate itself, to install on a phone or another computer that opens `.local` URLs: `parcela trust --export > parcela-ca.crt`. Changes nothing. |
| `--remove` | Removes the CA from the system trust store with `caddy untrust`, behind the system password prompt. | See [Use local HTTPS](/docs/faq/#clients-and-certificates).

## upgrade

```text
parcela upgrade [--nightly]
```

For shell-installer installations, downloads and runs the installer. Installs the latest release, or with `--nightly` the newest build of `main`. A successful installation refreshes an older running proxy on its existing port. Application processes, data, and setup hooks keep their current run. URLs briefly disconnect while owners republish their routes. A deliberately stopped proxy stays stopped, and an older CLI never replaces a newer proxy.

Normal interactive commands check for a stable release at most once a day. Shell-installer installations offer **Update now? [Y/n]**. Accepting installs the release, refreshes the running proxy, and resumes the original command with the new CLI. Declining continues the command and suppresses another prompt for a day. Offline release checks do not block the command.

Piped commands, `--json`, background owners, shutdown commands, and diagnostic proxy commands never check for updates or prompt. `PARCELA_NO_UPDATE=1` or `parcela config set updates false` disables automatic release checks. It does not disable local proxy maintenance after an update.

For Homebrew or Cargo installations, Parcela shows an update notice and `upgrade` prints the matching package-manager command without replacing the installation. After that update, the next regular CLI command refreshes an older running proxy. Builds outside known installation paths skip automatic release checks.

## config

```text
parcela config [list | get KEY | set KEY VALUE | add KEY VALUE | remove KEY VALUE | unset KEY | path]
```

Shows or changes this machine's settings. Without a subcommand, lists them. `get` exits 1 when the key is unset. `add` and `remove` change one item of a list; `set` replaces a list with comma-separated values.

| Key | Value |
| --- | --- |
| `sharing.domains` | Shares every run under each domain in the list, like `--domain`. `--domain` replaces the list for a run. Removing the last one also unsets `sharing.domain_port`. |
| `sharing.domain_port` | The loopback port for `sharing.domains`, 1480 by default. |
| `sharing.lan` | `true` shares every run at `.local` names, like `--lan`. |
| `sharing.lan_ip` | The LAN address when LAN sharing is on, like `--lan-ip`. It does not turn LAN sharing on. |
| `sharing.tailscale` | `true` shares every run through `tailscale serve`, like `--tailscale`. Never uses Funnel. |
| `telemetry` | `false` turns off anonymous usage data, like `parcela telemetry disable`. |
| `updates` | `false` turns off release checks, like `PARCELA_NO_UPDATE=1`. |
| `worktrees.directory` | Where `create` puts new checkouts, as `<directory>/<repository>-<NAME>`. An absolute path or one under `~/`. |
| `ports.backend` | The backend port range, like `30000-30999`. Default `20000-29999`. |
| `proxy.port` | The proxy's HTTPS port, 1443 by default. Setting it moves a running proxy; restart environments to update their URLs. Remove the relay first if it is installed. |
| `proxy.caddy` | The Caddy binary: a name on `PATH` or an absolute path. Default `caddy`. |
| `trust` | `false` stops `up` from offering to trust the CA, for example on a headless machine. `parcela trust` still works. |

`sharing` applies to `up`, `restart` and the dashboard's actions, in projects with an HTTP service. Flags and `PARCELA_LAN`, `PARCELA_TAILSCALE`, `PARCELA_DOMAIN` and `PARCELA_DOMAIN_PORT` add to it or override it. `--no-share` turns all of it off for one run. [Share with another device](/docs/worktrees/#share-with-another-device) covers each kind.

The file is `$XDG_CONFIG_HOME/parcela/config.yaml`, or `~/.config/parcela/config.yaml`. `PARCELA_CONFIG` names another file. It is YAML, and `set` rewrites it without comments:

```yaml
sharing:
  domains: ["tower.example.com", "tower.example.org"]
  tailscale: true
telemetry: false
worktrees:
  directory: "~/worktrees"
```

## telemetry

```text
parcela telemetry [status | enable | disable]
```

Shows or changes the saved telemetry preference. Without a subcommand, shows status. Environment overrides and event fields are in [Telemetry](/docs/telemetry/).

## skill

```text
parcela skill
parcela --skill
```

Prints the bundled agent instructions as Markdown without reading project or state. [Worktrees and agents](/docs/worktrees/#set-up-a-coding-agent) covers installation.

## completion

```text
parcela completion bash|zsh|fish|elvish|powershell
```

Prints a completion script for commands and flags. Load it from your shell's startup file:

```sh
source <(parcela completion zsh)          # ~/.zshrc
source <(parcela completion bash)         # ~/.bashrc
parcela completion fish | source          # ~/.config/fish/config.fish
```
