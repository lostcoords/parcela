# Worktrees and agents

Start with a configured project. Give each agent its own checkout, then run the same `parcela.yaml` in each one.

## Names and isolation

| Checkout of `shop` | Default URL |
| --- | --- |
| Main checkout | `https://shop.localhost:1443` |
| jj workspace or Git worktree `signup` | `https://signup-shop.localhost:1443` |
| HTTP service with `subdomain: api` in `signup` | `https://api-signup-shop.localhost:1443` |

Parcela remembers identity on first registration. Renaming a branch keeps the same URL. A conflicting name is refused. Use `--name review` on first start to choose a name, or `--primary` for the main URL after the old holder releases it with `rm`. Long derived names are shortened to fit DNS limits and get a hash suffix.

Each environment has its own processes, ports, logs, and runtime directory. Processes still run as your user. Separate databases and queues in [your app configuration](frameworks.md#separate-data).

## Run several agents at once

Create the checkouts through Parcela, then start them independently:

```sh
parcela create signup
parcela create billing
parcela -w signup up --detach
parcela -w billing up --detach
```

Parcela chooses jj for a jj repository, including a colocated repository, or Git otherwise. `create` registers the name but starts nothing. With Git it creates a detached worktree at `HEAD`; with jj it creates a workspace on `@`. `--revision REV` chooses another source revision and `--directory DIR` chooses a new location. Git's uncommitted changes are not copied. jj follows its normal working-copy snapshot rules.

Use `--json` and keep the returned `.workspace.id` and `.workspace.source` for each agent. From any directory, use `parcela --workspace "$ENVIRONMENT_ID" status`, `logs`, `task`, `restart`, or `down`. Names and URLs also work as selectors when unambiguous.

You can still create checkouts with Git or jj yourself, then run `parcela up --detach` in each one.

### Reproduce a test dataset

First bind the app's databases and uploads to `PARCELA_DATA_DIR` and declare [snapshot data](configuration.md#data). Configure a `tasks.seed` command if the project needs synthetic fixtures. Review its effects before running it.

```sh
parcela create baseline
parcela -w baseline up --detach
parcela -w baseline task seed
parcela -w baseline down
parcela -w baseline snapshot migration-fixture
parcela create migration-trial --from migration-fixture
parcela -w migration-trial up --detach
parcela -w migration-trial task test
parcela -w migration-trial down
```

The new checkout gets independent data, processes, ports, and URLs. Snapshotting requires a stopped environment. Restore fills only a new data directory and never overwrites an existing one. It restores data, not source: choose `--revision` when reproducing an exact code revision. Setup still runs on first start, so make its commands safe for restored data.

See everything that runs on the machine:

```sh
parcela ls
```

```text
PROJECT  NAME     STATE    URL                                  PATH
shop     billing  ready    https://billing-shop.localhost:1443  /Users/you/code/shop-billing
shop     signup   ready    https://signup-shop.localhost:1443   /Users/you/code/shop-signup
shop     primary  stopped  https://shop.localhost:1443          /Users/you/code/shop
```

## Clean up

```sh
parcela --path ../billing down
git worktree remove ../billing
parcela rm --prune
```

This stops and removes only `billing`, then forgets missing stopped checkouts. With jj, use `jj workspace forget billing` before removing its directory yourself.

To stop all environments of this project, use `parcela down --all`. To stop all projects, use `parcela down --all-projects`.

`down` keeps everything on disk. `rm` forgets a stopped environment and frees its URLs. It keeps the runtime directory, but a configured teardown hook runs first and can delete external data.

## Set up a coding agent

Parcela ships instructions for coding agents inside the binary. Print them with:

```sh
parcela --skill
```

Use the same operating instructions with Claude Code, Codex, Cursor, or another coding agent. The output is Markdown. It tells the agent how to pick the right checkout, start it with `--detach`, read `--json` output, find URLs and logs and stop only its own environment. You can also [read the operating skill](../../.agents/skills/parcela/SKILL.md) here or [download it](../../.agents/skills/parcela/SKILL.md?raw=1).

### Install the skill for Claude Code

[Claude Code discovers skills](https://code.claude.com/docs/en/skills) in `.claude/skills/<name>/SKILL.md`. Run this from the project root:

```sh
(
	set -eu
	skill_root=.claude
	skill_dir="$skill_root/skills/parcela"
	for skill_path in "$skill_root" "$skill_root/skills" "$skill_dir"; do
		if test -L "$skill_path"; then
			printf 'Refusing symlinked skill directory: %s\n' "$skill_path" >&2
			exit 1
		fi
	done
	mkdir -p "$skill_dir"
	stage=$(mktemp -d "$skill_dir/.install.XXXXXX")
	trap 'rm -f "$stage/SKILL.md"; rmdir "$stage"' EXIT
	parcela --skill > "$stage/SKILL.md"
	test -s "$stage/SKILL.md"
	ln "$stage/SKILL.md" "$skill_dir/"
)
```

It writes `.claude/skills/parcela/SKILL.md` and refuses to replace an existing one. To install it for every project, replace the `skill_root=.claude` line with `skill_root="$HOME/.claude"`. After upgrading Parcela, compare `parcela --skill` with the installed copy and update it.

Then ask Claude Code something bounded:

```text
/parcela Show this checkout's status and URLs. Don't start or stop anything.
```

### Configure a project with an agent

[Get started](getting-started.md#install-it-with-your-agent) has a prompt that installs Parcela and writes `parcela.yaml` for an existing project.

## Share with another device

Sharing is off by default and lasts for one run. Pass the flag again after a restart.

### Share on the same network

```text
$ parcela up --detach --lan
✓ hello is running at https://hello.localhost:1443
            https://hello.local:1443
  api       https://api-hello.localhost:1443
            https://api-hello.local:1443
```

Each service also answers at a `.local` name, which the device on your network resolves over mDNS. Parcela publishes the names with `dns-sd` on macOS or `avahi-publish` on Linux, and Caddy listens on this machine's LAN address with the same port. Devices on the network reach only the environments started with `--lan`.

The device does not trust Parcela's certificate authority, so it warns first. To stop the warning, install the certificate from `parcela trust --print` on the device. If the network changes, run `parcela restart --lan`. `--lan-ip IP` picks the address when the detected one is wrong, for example behind a VPN. `--lan` refuses a public address, so a machine with one, such as a cloud VM, cannot serve its app to the internet this way. `PARCELA_LAN=1` in the environment works like `--lan`, and `PARCELA_TAILSCALE=1` like `--tailscale`.

### Share on your tailnet

```text
$ parcela up --detach --tailscale
✓ hello is running at https://hello.localhost:1443
            https://mb.tail1234.ts.net
  api       https://api-hello.localhost:1443
            https://mb.tail1234.ts.net:8443
```

Each service gets its own HTTPS port on this machine's tailnet name, through `tailscale serve`. Tailscale's certificate is trusted everywhere, so there is no warning. Only devices on your tailnet can open it. It never uses Funnel. It needs the Tailscale CLI, a connected machine and HTTPS certificates turned on for the tailnet. Parcela picks ports that `tailscale serve status` shows free, and leaves the rest of your serve configuration alone.

### Share under your own domain

```text
$ parcela up --detach --domain dev.example.com
✓ hello is running at https://hello.localhost:1443
            https://hello.dev.example.com
  api       https://api-hello.localhost:1443
            https://api-hello.dev.example.com
```

Every environment's name becomes one label under your domain, so one wildcard certificate covers all of them. Parcela answers these names in plain HTTP on `127.0.0.1:1480` only (`--domain-port N` changes the port). Parcela never opens the port to other machines and never holds the certificate or DNS credentials. Your own HTTPS proxy does that, so it decides who can reach the names. For example, with a Caddy that has a DNS provider module, listening on this machine's tailnet address:

```text
*.dev.example.com {
    bind 100.101.102.103
    tls {
        dns cloudflare {env.CLOUDFLARE_API_TOKEN}
    }
    reverse_proxy 127.0.0.1:1480
}
```

Point a wildcard DNS record, `*.dev.example.com`, at the address the proxy listens on. A tailnet address in public DNS is still reachable only from your tailnet. Configure the proxy once: new environments need no change there. Parcela trusts `X-Forwarded-Proto` and `X-Forwarded-Host` from loopback, so the app sees `https` and the name the browser asked for. `PARCELA_DOMAIN=dev.example.com` in the environment works like `--domain`. To make the domain a project default, set [`sharing.domain`](configuration.md#sharing) in `parcela.yaml`. Use `--no-share` for a local-only run.

### Let the dev server accept the names

Dev servers such as Vite and Rails refuse hostnames they do not know. Parcela puts the shared ones in `PARCELA_SHARE_HOSTS`, separated by commas. For Vite:

```js
server: {
  allowedHosts: (process.env.PARCELA_SHARE_HOSTS ?? '').split(',').filter(Boolean),
},
```

The publishers show up in `parcela status` and `parcela logs` as `parcela-lan-<service>` and `parcela-tailscale-<service>`. `--domain` has no publisher: the proxy serves its names. A link that the app builds from `PARCELA_URL` still points at `.localhost`, which other devices cannot open.

### Stop sharing

Run `parcela down` in the shared checkout. This stops the environment and its mDNS or Tailscale publishers. To keep the app running locally, start it again with `parcela up --detach --no-share`. This overrides sharing flags, environment variables and project domain defaults.

Check `parcela status`. For Tailscale, also check `tailscale serve status` to confirm the run's ports are gone. Parcela leaves unrelated serve configuration alone.

If a device cannot connect, see [shared URL troubleshooting](troubleshooting.md#another-device-cannot-open-a-shared-url).
