
# Telemetry

Parcela sends anonymous command events to help maintainers understand use and failures. Events exclude paths, project names, environment names, service names, URLs, ports, argument values, environment variables, and error messages.

## Controls

| Control | Effect |
| --- | --- |
| `parcela telemetry status` | Shows the effective setting and why it is disabled, if applicable. |
| `parcela telemetry disable` | Saves a disabled preference, `telemetry: false` in the [host config](/docs/cli/#config). |
| `parcela telemetry enable` | Saves an enabled preference. Environment overrides still apply. |
| `DO_NOT_TRACK=1` | Disables telemetry for commands that inherit the variable. |
| `PARCELA_TELEMETRY_DISABLED=1` | Disables telemetry for commands that inherit the variable. |
| `PARCELA_TELEMETRY_DEBUG=1` | Prints the event to stderr instead of sending it. |

The saved preference is `telemetry` in the host config, `~/.config/parcela/config.yaml`, and applies to every state directory. A preference saved by an older release in the state directory's `telemetry.json` still applies until the host config sets one.

## When it is sent

Nothing is sent the first time Parcela runs. That run prints a notice instead:

```text
Parcela collects anonymous usage data: commands, exit codes and versions, never paths or names.
Turn it off with `parcela telemetry disable` or DO_NOT_TRACK=1. https://parcela.run/docs/telemetry/
```

After that, each command sends one event when it finishes. A background `curl` posts it to `https://parcela.run/api/telemetry` and gives up after 3 seconds; Parcela never waits for it, and a failed or blocked request changes nothing. Builds from source in debug mode, including `cargo test`, never send. `parcela telemetry`, `--help`, `--version` and Parcela's own internal re-runs are not reported.

## What an event contains

`PARCELA_TELEMETRY_DEBUG=1` prints the event to stderr instead of sending it:

```text
$ PARCELA_TELEMETRY_DEBUG=1 parcela up --detach --name review
{
  "schema": 1,
  "event": "command",
  "command": "up",
  "flags": ["--detach", "--name"],
  "exit": 0,
  "duration": "5-30s",
  "version": "0.5.0",
  "channel": "stable",
  "os": "macos",
  "arch": "aarch64",
  "install": "install.sh",
  "ci": false,
  "terminal": true,
  "json": false,
  "install_id": "4208b56a-3d78-451f-8c9d-889a2e13a766"
}
```

| Field | What it is |
| --- | --- |
| `schema`, `event` | `1` and `command`, identifying the event format. |
| `command` | The subcommand, such as `up` or `relay install`. |
| `flags` | The names of the flags given on the command line, never their values: `--name review` is reported as `--name`. |
| `exit` | The exit code. |
| `duration` | How long the command ran: `<1s`, `1-5s`, `5-30s`, `30s-2m` or `>2m`. |
| `version`, `channel` | Parcela's version, and `stable` or `nightly`. |
| `os`, `arch` | The operating system and CPU architecture. |
| `install` | How Parcela was installed: `install.sh`, `homebrew`, `cargo` or `unknown`. |
| `ci` | Whether a CI provider's usual variable, such as `CI` or `GITHUB_ACTIONS`, is set. |
| `terminal`, `json` | Whether the output went to a terminal, and whether `--json` was given. |
| `install_id` | A random ID created on the second run, so installs can be counted without knowing whose they are. |

## Where it goes

`parcela.run/api/telemetry` is a Cloudflare Worker in this repository (`docs/worker.mjs`). It rejects any event with fields or values outside the table above, and forwards the rest to [PostHog](https://posthog.com) Cloud US as anonymous events. Person profiles and location enrichment are disabled. PostHog keeps them according to the configured plan's retention policy. The address is ours, so a firewall or proxy rule for that one URL also stops it.
