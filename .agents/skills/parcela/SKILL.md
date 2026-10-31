---
name: parcela
description: "Use Parcela to create, snapshot, restore, inspect, start, stop, restart or recover named local development environments, retrieve service URLs and logs, share one with other devices on request, and run explicitly configured project tasks. Use for Parcela operations, not terminal layout, VCS history or implicit application setup."
---

# Parcela

Parcela runs the services in one checkout's `parcela.yaml` and gives each HTTP service its own HTTPS URL. Caddy serves the URLs. Parcela runs the processes. The project's `parcela.yaml` owns commands, data and anything with external effects.

This skill does not grant permission to start services, process queues, run migrations, change trust or share an environment beyond this machine. Follow the user's authorization and the project's instructions. Do not print credentials, `.env` contents or full process environments.

## Pick the target

Run `parcela --help` or `parcela <command> --help` for syntax. Output is JSON when stdout is not a terminal; pass `--json` whenever you parse it. `url` and `logs` always print plain text.

Set `CHECKOUT` to the absolute path of the checkout the user means. Never infer it from a focused terminal or a branch name.

```sh
parcela --json ls
parcela --json --path "$CHECKOUT" doctor
parcela --path "$CHECKOUT" status --current
```

- `ls` returns `.workspaces`, each with `.workspace.id`, `.workspace.source`, `.workspace.role` and live status. Inside a project it also returns `.unregistered`: worktrees never started, with the URL `up` would give them. Listing registers nothing.
- `status` returns the overview envelope, with `.workspaces` and `.unregistered`. `status --current` or `--workspace ENV status` returns one environment's `.workspace`, `.url`, `.state` and `.processes`.
- `ui` prints the local dashboard URL when the proxy is running. It starts nothing and opens no browser. The dashboard offers status and navigation on loopback.
- Processes get `BROWSER` as `parcela browse`. A `.localhost` page that loads `/_parcela/widget.js` from the app's own host gets a debug menu: what the app opened, every environment, start/stop/restart of registered ones, and process logs. Shared names never serve it. Adding that script tag to a project is a project change; propose it.
- `doctor` returns `{ready, checks: [{name, ok, detail, fix}], notes}` and exits 1 until ready. Report failed checks and their `fix`. Do not apply a fix that installs tools or changes trust without authorization.

From another directory, target an environment by its ID: `parcela --workspace "$ENVIRONMENT_ID" status`. `--workspace` also takes the environment's URL or hostname, such as `signup-shop`; prefer the ID in scripts. `--path` and `--workspace` conflict. Keep any `--state-dir` on every call.

If the checkout has no `parcela.yaml` and the user asks to configure it, `parcela init` writes a minimal one with a guessed command. Review the guess with the user. `init` never overwrites a file.

## Names and data

The main checkout gets the project URL. A linked jj workspace or Git worktree gets its own name on its first `up`; `--name NAME` picks one, and `--primary` gives the project URL to a checkout only when the user intends that. Names stay after the first `up`.

A separate URL does not separate databases, uploads, secrets or queues. Read the project's `parcela.yaml` and instructions before the first `up`, and refuse to start until its data bindings suit the request. https://parcela.run/docs/frameworks/ shows per-environment SQLite, PostgreSQL, Redis and uploads, and host checks for common frameworks; propose such a change rather than make it unasked.

`up` runs `hooks.setup` on an environment's first start and `hooks.prestart` on every start. Tell the user what they will do, such as installing packages or migrating. `up --setup` reruns setup; use it only when asked. Do not run preparation commands that `parcela.yaml` does not declare.

## Create a disposable test environment

Only when checkout creation is authorized:

```sh
parcela --json --path "$CHECKOUT" create trial
```

Keep `.workspace.id`, `.workspace.source`, and `.checkout.revision` from the response. `create` registers a preview but starts nothing. It uses jj when present, never falls back to Git after a jj failure, and otherwise creates a detached Git worktree. Git defaults to `HEAD`; jj defaults to `@` and snapshots according to its normal rules. Use `--revision REV` or `--directory DIR` only for the requested source and destination. Existing destinations and names are refused. Do not copy ignored secrets yourself as a workaround.

Use `--workspace "$ENVIRONMENT_ID"` for every subsequent operation. `doctor` accepts that selector too. A failed creation may retain a checkout; report its path instead of deleting or silently reusing it.

## Reproduce declared data

Read `data.files`, `data.sqlite`, setup hooks and seed/export/import tasks first. Snapshots include only declared paths under `PARCELA_DATA_DIR`; they do not establish that external databases or queues are isolated. Declared application records may still be sensitive.

With explicit permission to capture the selected stopped environment:

```sh
parcela --workspace "$ENVIRONMENT_ID" snapshot baseline
parcela --json --path "$CHECKOUT" create trial --from baseline
```

SQLite uses a consistent backup, including WAL contents. Snapshots reject linked paths, common secret files and raw SQLite file copies. Names are immutable. `restore baseline` fills only a fresh registered environment with no data directory; never remove existing data to make it pass. Restore starts nothing and does not mark setup complete. Review the next start's setup and migration commands for their effects on restored data.

Seed and external data operations remain project-defined tasks, such as `parcela --workspace "$ENVIRONMENT_ID" task seed`. Do not infer permission to run them from permission to create a checkout. `down` stops the selected graph; `rm` still retains source and runtime data. There is no implicit source or snapshot deletion.

## Start and verify

```sh
parcela --path "$CHECKOUT" up --detach
parcela --path "$CHECKOUT" status --current
parcela --path "$CHECKOUT" url api
```

- If the environment is already `ready`, reuse it unless the user asks for a restart. `up` on a ready environment prints its URLs and exits 0.
- For HTTP environments, `up` starts the shared HTTPS proxy when none runs. Watcher-only environments need no proxy or certificate trust. The shared proxy serves HTTP environments on the machine: do not stop or restart it unless asked. `parcela proxy status` reports it.
- Use the URLs Parcela returns; they carry the proxy port (`:1443` by default). Use declared service names, never an assumed port.
- `state: ready` means hooks completed, services started, and explicit readiness checks passed. Watcher-only environments have no URL; use process status and logs to verify them. For HTTP verification, request the application path over the returned HTTPS URL with `curl --cacert "$(parcela trust --print)"`. Never disable TLS verification.
- `parcela trust` changes system trust; run it only when the user asks. Without a terminal, `up` never trusts anything itself. `parcela proxy install` adds a system service; same rule.
- If an HTTP environment needs Caddy and it is missing, report it. Do not install software, start Docker or a GUI, change firewall rules or take over a busy port.
- `run -- ARGV` runs one foreground command as the only web service. It is not a substitute for `up`.

Report the environment ID, the URLs, what you verified and what you did not.

## Share with other devices

Only when the user asks for it, for the environment they name:

```sh
parcela --path "$CHECKOUT" up --detach --lan        # https://<name>.local:1443 on the same network
parcela --path "$CHECKOUT" up --detach --tailscale  # https://<machine>.<tailnet>.ts.net on the tailnet
parcela --path "$CHECKOUT" up --detach --domain dev.example.com  # https://<name>.dev.example.com via the user's own proxy
```

Shared URLs appear under each service in `up` and `status` output, and in `.live.endpoints.<service>.shared`. Sharing lasts for that run and `down` ends it. Domain defaults may be explicitly declared in `sharing.domain` and `sharing.domain_port` in `parcela.yaml`; inspect them before starting. CLI and environment settings override project defaults. `--no-share` disables all sharing; use `restart --no-share` to withdraw an existing shared run. Do not add sharing defaults unless asked. Never use Tailscale Funnel. `--domain` serves plain HTTP on `127.0.0.1:1480` (`--domain-port`) for a front proxy the user already runs; do not set up that proxy or its DNS unasked. A dev server that rejects the host needs `PARCELA_SHARE_HOSTS` in its allowed hosts; that is a project change, so propose it rather than make it unasked.

## Diagnose

```sh
parcela --workspace "$ENVIRONMENT_ID" status
parcela --workspace "$ENVIRONMENT_ID" logs api --tail 100
```

`logs` without a name prints every process, each line prefixed with `name |`. `logs --follow` streams until interrupted, so do not use it in a command that must return.

If `status` says `unknown`, and starting is authorized, run `up --detach`: it takes back services that are still running instead of starting a second copy, and says so on stderr. If it says the run was being stopped, run `parcela down` first. When Parcela refuses, report its message. Never kill processes by PID, use broad `pkill`, delete sockets or locks, or edit the state directory.

## Restart, stop and tasks

Only for the environment the user selected:

```sh
parcela --workspace "$ENVIRONMENT_ID" restart --detach
parcela --workspace "$ENVIRONMENT_ID" down
```

- `restart` checks the new configuration, then restarts every service. It cannot restart one service alone.
- `down` stops the services and keeps source and data. Do not stop other environments as cleanup. A refused stop is not a successful one.
- `down --all` stops the project's environments and `--all-projects` every one; use them only when asked.
- `rm` forgets a stopped environment and frees its URLs; `rm --prune` does so for every one whose checkout is gone. Both keep the runtime directory, but `hooks.teardown`, if declared, runs first and may delete data; read it before `rm`. Use them only on an explicit cleanup request.
- Before `task NAME -- ARGS`, read that task's command and effects in `parcela.yaml`. Run only the requested task.

## CLI updates

`parcela upgrade` updates an install.sh installation and refreshes an older running proxy automatically. Homebrew and Cargo installations print their own update command; the next regular CLI invocation refreshes an older proxy after that update. App PIDs, data, hooks, the CA, and the front door stay in the same run. URLs briefly disconnect while routes return. Stopped proxies stay stopped. Do not run an extra proxy restart as an upgrade step.

Interactive commands offer stable-release updates at most once a day. Piped commands, `--json`, internal owners, and shutdown commands never check or prompt. `PARCELA_NO_UPDATE=1` disables release checks, while local proxy maintenance still happens. Existing CLIs without this behavior need one explicit upgrade. Do not install or upgrade the CLI unless the user asks.

## Limits

Parcela is a local development runner, not a sandbox. It does not restart individual services, snapshot undeclared or external data, transfer secrets, or prove browser trust. This skill and `--help` are not runtime proof.

Parcela sends anonymous usage data unless it is turned off (https://parcela.run/docs/telemetry/). Leave `parcela telemetry` and `DO_NOT_TRACK` as the user set them unless they ask.
