# Parcela

Workspace/environment organization and named HTTPS for arbitrary local services.
Use jj for version control; preserve unrelated work and never use mutating Git commands.

- Read docs/architecture.md before architecture changes.
- Keep Caddy as the maintained routing/TLS backend. Parcela's own supervisor runs each environment's processes.
- Defaults: HTTPS on the no-sudo port 1443, loopback only, flat names, tenants off.
- Never take over listeners, trust a CA, expose LAN/tailnet services, or delete source/data implicitly. Sharing is per run. LAN/Tailscale require explicit flags or environment settings. Domain routes may use explicit project `sharing` defaults and remain loopback-only; `--no-share` disables all sharing. Never Tailscale Funnel.
- CA trust only from `parcela trust` or the first `up` on a terminal, behind the system password prompt; never without a terminal. System services only from `parcela proxy install`.
- Tests that give Parcela a terminal set `PARCELA_NO_TRUST=1`.
- Share live tests only on disposable environments, and check `down` removes the mDNS names and `tailscale serve` config.
- Keep project commands/data policy in configuration; no framework assumptions in core.
- Run cargo fmt --check, cargo clippy --all-targets -- -D warnings, cargo test, and scripts/check for code changes.
- Test live processes only in disposable directories with isolated state and explicit test ports.
- No GUI focus, clipboard changes, test audio, or system-wide input.
- Describe verified results and explicit unsupported/unverified functionality; docs are not runtime proof.
