# Environment overview and page switcher

Design exploration, 2026-10-28. The implementation follows the overview and
optional-script candidate, and adds an iframe workspace with direct navigation
as the fallback. The current CLI and browser behavior is documented in
[CLI reference](site/cli.md#status). The exploration below records the original
alternatives; it is not a second command reference. Independent design review
was not performed.

The user needs to find running worktrees and their URLs without remembering
checkout directories. A page switcher should also show environment health while
the user works in an application.

## What exists

`parcela status` resolves one environment from the current checkout or
`--workspace`. It asks that environment's owner for live status and process
details. `parcela` without a subcommand uses this same command inside a
registered checkout.

`parcela ls` already lists every registered environment in the selected state
directory, across projects. Inside a project, it also discovers that project's
unregistered jj workspaces or Git worktrees and predicts their URLs. Discovery
does not register or start anything. It does not discover unregistered
worktrees belonging to other projects.

The registry supplies identity and saved sessions. The owner supplies live
health. An unreachable owner produces `unknown`, not a claim that a saved
`ready` session is healthy. The proxy renews route leases and Caddy serves the
leased routes. Caddy currently forwards application responses without HTML
transformation. Tailnet sharing bypasses Caddy and reaches backends directly.

The relevant code is:

- `src/cli/commands.rs`: command dispatch and single-environment selection.
- `src/cli/inventory.rs`: inventory collection and checkout discovery.
- `src/cli/output.rs`: the existing overview table.
- `src/lifecycle.rs`: live status and process details.
- `src/control.rs`: authenticated, generation-bound owner requests over Unix sockets.
- `src/router.rs`: loopback, LAN, and domain Caddy routes.

## Proposed CLI experience

```sh
parcela status                           # overview across registered projects
parcela status --current                 # today's detailed checkout view
parcela --workspace mascot-animation-wariwari status
                                         # detailed view of an explicit selection
parcela ls                               # retain the existing inventory command
```

Use the compact inventory table for the overview, with project, environment,
state, URL, and path. Mark the current checkout and group its project first.
Keep stopped, unknown, and not-started environments visible and distinct.
Show the process table only for a selected environment.

Two candidates are viable. Keeping `status` scoped and adding `status --all`
preserves existing scripts, but leaves the discovery problem in the default
command. Making `status` an overview matches the reported expectation. Prefer
the second, with explicit selection for details.

Both overview commands should call one inventory collector. Do not implement
another registry scan inside `status`. Extract collection from CLI rendering
so the browser can consume the same interpretation of health later.

Before changing the default, decide the JSON migration. Today `status` returns
one object and `ls` returns an inventory envelope. Prefer a consistent inventory
envelope for overview status and the existing object for explicit selection.
Document the change for scripts using `status --json`. Also choose explicitly
whether bare `parcela` adopts the overview or retains its current detail view.

## Proposed page experience

A small Parcela tab sits at the right edge of the application. Click or keyboard
activation opens a panel. Hover may preview it but must not be required.
Escape closes the panel and returns focus to the tab.

```text
Parcela                                  [close]
[ Find an environment...                      ]

wariwari
  mascot-animation     ready       current
  landing-bento        ready       open
  landing-editorial    ready       open
  landing-glass        ready       open
  primary              not started

Current environment
  setup completed    prestart completed
  css running        web running
```

Group by project, place the current project first, and list each exposed HTTP
service under its environment. Use returned URLs, including their ports. Show
stopped or unknown environments without implying their URLs currently work.
Switching opens the target's returned root URL; preserving the application path
is a separate feature because different apps may have unrelated routes.

A list fits environment names, health, and several service links better than a
radial menu. A radial menu is a competing interaction to prototype if the list
feels too large. Start with navigation and status. Lifecycle controls require a
separate design for browser authentication and generation-bound mutations.

## Integration candidates

| Candidate | What it hides | Cost or limitation |
| --- | --- | --- |
| Dedicated local dashboard with an optional app script | One status API and UI work across frameworks; Caddy routes ordinary requests | Apps opt into the script; strict CSP can block the widget |
| Automatic HTML injection using a custom Caddy module | Apps need no markup changes | Parcela must distribute a custom Caddy build and handle response transformation compatibility |
| Dashboard that embeds apps in an iframe | Application responses stay untouched | Apps can refuse framing; embedding changes navigation and browser behavior |

Prefer a dedicated local dashboard and an optional app script for the first
prototype. It gives the status UI a usable standalone fallback and keeps Caddy
as the routing and TLS backend. If automatic injection is essential, prototype
the custom-module candidate before committing to its distribution cost.

Caddy's [replace-response module](https://github.com/caddyserver/replace-response)
performs response replacements, but it is
[non-standard](https://caddyserver.com/docs/modules/http.handlers.replace_response)
and requires a custom build. Its documentation explains buffering, streaming,
and compressed-response constraints. Injection needs explicit checks for
compression, content length, caching validators, HTML streaming, CSP, and
non-HTML responses. WebSocket and event-stream traffic must remain untouched.

The built-in [templates directive](https://caddyserver.com/docs/caddyfile/directives/templates)
evaluates response content as templates. It is unsuitable as a general-purpose
injector over arbitrary application HTML. Caddy explicitly warns about evaluating
user-generated content.

## Ownership and boundaries

Keep one inventory implementation over the registry and live owner protocol.
The CLI and a new local HTTP adapter consume it. The HTTP adapter returns an
allowlisted view of names, health, service URLs, and process states. It must not
serialize saved sessions, owner tokens, commands, or environment variables.

The shared proxy process is the candidate owner of the local HTTP adapter.
Caddy routes a dedicated control origin to it. Caddy's admin socket and the
environment control sockets remain private. Slow or unreachable owners must
not block the proxy's route-renewal loop. Existing control reads can wait six
seconds per owner, so the browser needs bounded collection outside that loop
and a visible timestamp or stale state.

Serve the dashboard and its JSON on the dedicated origin. If the widget uses
an iframe, it can render that UI without granting application JavaScript
cross-origin access to the inventory. Do not add permissive CORS or pass owner
credentials to the page. Validate the control Host and browser request origin.
Keep the control routes off the LAN and domain servers and out of Tailscale
sharing. Limit any frame messages to declared UI actions.

The control hostname needs collision handling in the existing name reservation
system. Do not assume `parcela.localhost` is free: this machine already uses it
for the Parcela project. Any app-side script path also needs an explicit
namespace contract. A strict app CSP may refuse the script or iframe; retain
the standalone dashboard instead of weakening the application's policy.

## Verified evidence and next steps

The installed `parcela --json --path
/Users/davafons/workspace/wariwari-mascot-animation ls` returned four ready
WariWari previews: `landing-bento`, `landing-editorial`, `landing-glass`, and
`mascot-animation`. It also returned the unregistered primary checkout as
`not started`. Other registered projects appeared, some with `unknown` health.
This verifies CLI inventory collection, not browser reachability or injection.

The existing `cargo test --test inventory ls_lists_every_project_in_one_table`
test passed. No runtime code changed in this exploration.

First implement the CLI overview using the existing collector, with checks for
explicit selection, invocation outside a project, unknown owners, and unchanged
registration state. Resolve JSON behavior in that change. Then prototype the
standalone dashboard and list interaction on disposable environments. Compare
it with automatic injection using compressed HTML, streaming, strict CSP,
WebSockets, and shared routes before choosing the widget delivery mechanism.


## Implementation evidence

The implementation keeps bare `parcela` detailed, makes `status` an overview,
and adds `status --current` and `ui`. The dashboard uses a private Unix socket
owned by the proxy. It supports an iframe workspace and an optional app script;
there is no automatic response transformation.

Verification passed with `scripts/check`, the documentation-site check, the
`core`, `docs-example`, and `dashboard` live suites, and the `proxy_outage` and
`guardian_death` recovery cases. The dashboard suite ran Chromium interactions
for search, process details, iframe navigation, the optional tab, keyboard
closing and focus, direct navigation when framing is blocked, strict CSP,
stale status recovery, and the narrow layout. It also verified that the control
origin is absent from a shared domain and its socket is removed on shutdown.
All live runs used disposable sources, isolated state, and test ports. Existing
user environments and the installed executable were not changed.
