---
description: What to check when an environment does not start, a URL does not load or the browser does not trust the certificate, starting with parcela doctor and parcela status --current.
---

# Troubleshooting

Check tools and configuration, then inspect the selected environment:

```sh
parcela doctor
parcela status --current
```

## The browser warns about the certificate

Your system does not trust Parcela's local certificate authority yet. Run `parcela trust` once, then restart the browser. For `curl`, pass `--cacert "$(parcela trust --print)"`. Don't use `curl -k`. It hides real certificate errors.

## The proxy port is busy

```text
parcela: port 1443 is already in use by Python (pid 57668); choose another port with `parcela proxy start --port <n>`
```

Another program listens on the port. Parcela never stops it. Stop it yourself, or pick another port with `parcela proxy start --port 4443`. Parcela saves the port for later starts.

## `up` times out

```text
parcela: not ready after 60s: web: GET /up returned 404
```

With `--detach`, the same message comes from the background owner, under the last lines of its log:

```text
parcela: last lines of /Users/you/.local/state/parcela/environments/<id>/log/owner.log:
  web | Listening on http://127.0.0.1:3000
  parcela: not ready after 60s: web: GET /up returned 404
parcela: services stopped before they were ready; full log: /Users/you/.local/state/parcela/environments/<id>/log/owner.log
```

The message says what each service did instead: `nothing listens on its $PORT`, a status code, or that another process holds the port. Check the service configuration:

- Pass `$PORT` to the server command.
- Bind to `127.0.0.1`.
- Set `http.healthcheck` to a path that returns 2xx.

`parcela logs web` shows the service's own output, also after the environment stopped.

## A hook failed

```text
parcela: hooks.setup failed with exit code 1; its output is above
```

The setup or prestart command exited nonzero, so no other service started and the environment stopped. Its output is printed above the error, prefixed with `setup |`, and `parcela logs setup` shows it again. Fix the cause and run `parcela up` again. A setup that failed runs again on the next `up`. `parcela up --setup` reruns one that already succeeded.

## The URL answers 503

The environment is registered but not running, or it is still starting. Run `parcela status --current`. If it says `stopped`, run `parcela up --detach`.

## Another device cannot open a shared URL

Run `parcela status --current` and look at the `parcela-lan-*` and `parcela-tailscale-*` processes. `parcela logs parcela-tailscale-web` shows why one stopped.

- **A `.local` name does not resolve.** The device must be on the same network, and some guest or office Wi-Fi blocks mDNS. After the laptop changes networks, run `parcela restart --detach --lan`.
- **The page says the host is not allowed.** The dev server refuses the shared name. Add `PARCELA_SHARE_HOSTS` to its allowed hosts, as shown in [the sharing guide](worktrees.md#let-the-dev-server-accept-the-names).
- **`--tailscale` refuses to start.** It says what is missing: the CLI, `tailscale up`, or HTTPS certificates in the tailnet's DNS settings.

## The dev server rejects the host

Rails, Django and Vite accept `*.localhost` names in development by default, but not the `.local`, tailnet and domain names of `--lan`, `--tailscale` and `--domain`, nor any name once you set an allow list yourself. Add the names from `PARCELA_SHARE_HOSTS`, or derive them from `PARCELA_URL`. [Configure frameworks](frameworks.md#framework-settings) shows the setting for each framework.

## Safari searches instead of opening the URL

Type the scheme: `https://shop.localhost:1443`. Without it, Safari may treat `shop.localhost:1443` as a search.

## A preview name is taken

```text
parcela: preview name signup is already used by /Users/you/code/signup; choose another with --name <name>
```

Another checkout owns that name. Start this one with another name: `parcela up --name signup-2`. To free the name, stop the other checkout and run `parcela rm` in it.

## A checkout keeps its old name

Parcela remembers a checkout's name after the first `up`. To rename it, run `parcela down`, `parcela rm`, then `parcela up --name NEW`. Read any teardown hook before `rm`, because it can delete external data. The new registration gets a new runtime directory. Move retained data yourself if needed.

## `logs` has nothing to show

```text
parcela: no logs yet; run parcela up first
```

`logs` reads what the latest run's processes wrote, so it works while the environment runs and after it stops, until the next `up`. A checkout that never started has none. Parcela's own messages from a detached start are in `log/owner.log` in the environment's runtime directory.

## The proxy is from another Parcela version

```text
parcela: the running proxy is from Parcela <old>, older than this <new>; run `parcela proxy restart` to update it
```

Normal foreground commands refresh an older running proxy automatically after a CLI update. Diagnostic commands such as `proxy status` still show its recorded version without changing it. Run a regular command such as `parcela status` to perform the refresh.

If automatic refresh fails, Parcela prints the reason. Check that Caddy 2 is available on PATH. Missing prerequisites leave the working proxy alone; unknown or unverifiable ownership is never taken over. `parcela proxy restart` remains available for an explicit restart.

If the message says the proxy is newer, another Parcela on this machine is newer than the one you ran. Run `parcela upgrade`.

## The state is `unknown`

Parcela lost contact with the process that supervises the environment, for example after a crash. Run `parcela up`. If the services are still running, `up` takes control of them again without restarting them. If they are all gone, it starts them fresh. If `up` says the run was being stopped, finish the stop with `parcela down`, then run `parcela up`.

If `down` says `shutdown incomplete: a process did not exit`, a process outlived the shutdown timeout. Run `parcela down` again: it stops whatever is left, by the process identity Parcela recorded.

Don't kill processes by PID or delete files in the state directory to fix this. Parcela refuses to act when it cannot prove which processes it started, and it keeps the record so nothing is lost.

## Something else

Run `parcela status --current --json` and read its `diagnostic` field. Review the output and matching `owner.log` for secrets or private paths before including them in a [bug report](https://github.com/lostcoords/parcela/issues).
