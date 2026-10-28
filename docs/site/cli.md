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

Commands print text in a terminal and one JSON document when stdout is piped or `--json` is given. Exceptions are `url` and `logs`, which print plain text, and `task` and `run`, which pass through command output. Errors go to stderr with the prefix `parcela:`. `NO_COLOR` disables terminal colors.

For example, `parcela status --json | jq -r .state` reads the environment state.

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

The default directory is a sibling named `<repository>-<NAME>`. `--directory` chooses a new directory whose parent already exists. Existing directories, workspace names, and registered environment names are refused. Git checkout hooks are disabled. Ignored files are not copied; files already committed to Git or tracked by jj follow that VCS's rules.

`--from` restores a data snapshot before the first start. It does not restore source history, secrets, setup markers, or running processes. JSON includes `checkout` with the path, VCS and source revision, `workspace` with the environment ID, and `state: stopped`. A creation failure retains any checkout already created and names it in the error.

## snapshot

```text
parcela snapshot NAME
```

Saves the selected environment's [declared data](configuration.md#data) under `<state>/snapshots/<project>/<NAME>`. The environment must be stopped with no owner, restart, or task operation holding its locks. Each SQLite database uses SQLite's backup API, including committed WAL contents. Snapshot names are immutable. There is no automatic seeding, retention limit, or snapshot deletion command.

## restore

```text
parcela restore NAME
```

Restores a snapshot into a fresh registered environment whose data directory does not exist. Existing data is never replaced, including an empty data directory. Project and data declarations must match. Files are staged and checksummed before publication, and a failed restore publishes no partial data. Snapshot and restore require macOS or Linux with atomic no-replacement directory rename support.

Restore does not start services or mark setup complete. The next `up` runs the normal setup and prestart hooks. Those commands must preserve restored data. Seed and external database export/import commands remain explicitly configured [tasks](#task).

## up

```text
parcela up [--detach] [--setup] [--name NAME | --primary] [--backend-port N] [--lan] [--lan-ip IP] [--tailscale] [--domain DOMAIN [--domain-port N]]
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
| `--domain DOMAIN` | Also serves each service at `<name>.DOMAIN`, in plain HTTP on loopback for your own HTTPS proxy. Also set by `PARCELA_DOMAIN`. |
| `--no-share` | Disables all sharing for this run, including project defaults and environment variables. Use `restart --no-share` for an already shared run. |
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
parcela restart [--detach] [--setup] [--backend-port N] [--lan] [--lan-ip IP] [--tailscale] [--domain DOMAIN [--domain-port N]]
```

Checks the replacement configuration, then stops and starts the whole environment. New URL reservations happen after the old run stops. Options have the same meanings as for `up`. Sharing flags must be supplied for the new run.

## status

```text
parcela status
```

Shows registered URLs, live HTTP health, process state, and named ports.

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
parcela proxy install
parcela proxy uninstall
```

One background Caddy serves all environments. Owners recover an unexpected proxy exit. An explicit `proxy stop` keeps it stopped until `proxy start` or the next `up`.

| Subcommand | Effect |
| --- | --- |
| `start` | Starts the proxy. `--port` chooses and saves its port. An occupied port causes an error naming its owner. |
| `stop` | Stops routing for every environment. App processes keep running. |
| `restart` | Replaces the proxy, including after an upgrade. Briefly interrupts URLs. |
| `status` | Shows the port, PIDs, version, and log path. Exits 1 if stopped. |
| `install` | On macOS, installs a launchd relay on loopback port 443 after a password prompt. Uses port 80 for redirects when free. Refuses an occupied 443. |
| `uninstall` | Removes the macOS relay. |

The default port is 1443. `PARCELA_HTTPS_PORT` applies when no port is saved. Port 443 omits the port from URLs and requires permission to bind it. The proxy redirects plain HTTP on its listener to HTTPS.

## trust

```text
parcela trust [--print]
```

Adds Caddy's local CA to the system trust store using `caddy trust`. Prints the certificate and undo command before the system password prompt. All projects use the same CA.

`--print` only prints the certificate path and changes no trust settings. See [Use local HTTPS](faq.md#clients-and-certificates).

## upgrade

```text
parcela upgrade [--nightly]
```

For shell-installer installations, downloads and runs the installer. Installs the latest release, or with `--nightly` the newest build of `main`. For Homebrew or Cargo, prints the matching upgrade command without changing the installation. Parcela does not check for updates automatically.

## telemetry

```text
parcela telemetry [status | enable | disable]
```

Shows or changes the saved telemetry preference. Without a subcommand, shows status. Environment overrides and event fields are in [Telemetry](telemetry.md).

## skill

```text
parcela skill
parcela --skill
```

Prints the bundled agent instructions as Markdown without reading project or state. [Worktrees and agents](worktrees.md#set-up-a-coding-agent) covers installation.

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
