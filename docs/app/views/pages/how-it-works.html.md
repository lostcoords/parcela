# How it works

Parcela is a CLI, one supervisor process per running environment, and one shared proxy. Caddy does TLS and routing; Parcela tells it what to route.

## The pieces

```diagram
  you ── parcela up / down / ls / ui …
               │ registers, leases ports
               ▼
  ┌────────────────────────────────────────────────────────────────┐
  │ state directory   ~/.local/state/parcela                       │
  │   registry.sqlite3     projects, environments, names,          │
  │                        port leases, sessions                   │
  │   environments/<id>/   owner.json, logs, opens.json            │
  │   proxy/               caddy.json, leases/, CA                 │
  └────────────────────────────────────────────────────────────────┘
               │ starts one                           ▲
               ▼ per environment                      │ reads sessions
  ┌─────────────────────────┐ publishes  ┌────────────┴────────────┐
  │ environment owner       │ ─────────► │ parcela proxy run       │
  │  holds owner.lock       │   routes   │  supervises Caddy       │
  │  runs the processes     │            │  renews leases (1 s)    │
  │  readiness checks       │            │  serves the dashboard   │
  └────────────┬────────────┘            └────────────┬────────────┘
               │ spawns                               │ admin socket
               ▼                                      ▼
  ┌─────────────────────────┐            ┌─────────────────────────┐
  │ your processes          │ ◄───────── │ Caddy 127.0.0.1:1443    │
  │  web  127.0.0.1:2xxxx   │ reverse    │  TLS, private CA        │
  │  api  127.0.0.1:2xxxx   │  proxy     │  a route per name       │
  │  worker (no port)       │            └────────────┬────────────┘
  └─────────────────────────┘                         ▲
                                                      │
                                         https://shop.localhost:1443
```

Settings for this machine (domains, LAN, ports, proxy, trust) live in `~/.config/parcela/config.yaml`, managed with `parcela config`. Each process gets its port in `$PORT`. Ports come from 20000–29999 unless `ports.backend` says otherwise.

## Names

```diagram
  checkout          service    hostname
  ────────────────  ─────────  ─────────────────────────
  shop  (main)      web        shop.localhost
  shop  (main)      api        api-shop.localhost
  shop-signup       web        signup-shop.localhost
  shop-signup       api        api-signup-shop.localhost
  (none)            dashboard  parcela-ui.localhost
```

Names stay one label deep under the project, so `*.<domain>` covers every environment when sharing under a domain. `.localhost` resolves to loopback on its own, so no DNS or `/etc/hosts` changes are needed. `shop-signup` is a jj workspace or Git worktree named `signup`; `api` has `subdomain: api`.

## A request

```diagram
                              browser
                                 │
               ┌─────────────────┴─────────────────┐
               │                                   │
     https://shop.localhost          https://shop.localhost:1443
       (relay installed)                     (no relay)
               │                                   │
               ▼                                   │
    ┌─────────────────────┐                        │
    │ relay  :443 → :1443 │  parcela relay         │
    │ launchd or systemd  │  (optional)            │
    └──────────┬──────────┘                        │
               └─────────────────┬─────────────────┘
                                 ▼
                     ┌───────────────────────┐
                     │ Caddy :1443           │   lease expired
                     │ hostname → route      │ ───────────────► 503
                     └───────────┬───────────┘
                                 │
                 /_parcela/*  ◄──┼──►  everything else
                      │                    │
                      ▼                    ▼
              dashboard socket      127.0.0.1:2xxxx
              (debug menu)          your web process
```

The proxy renews a lease file per route every second. Caddy serves a route only while the current second's file exists, so a route dies five seconds after its owner does, even if the proxy dies too.

## Sharing

Off by default. Each kind is turned on per run with a flag, or for every run on the machine with `parcela config`. `--no-share` turns all of them off for a run.

```diagram
                     ┌────────────────────────────────────────────┐
  same machine ────► │ 127.0.0.1:1443    *.localhost  always      │
                     │                                            │
  LAN device ──────► │ <LAN IP>:1443     *.local      --lan       │
                     │                                            │
  tailnet device ──► │ tailscale serve   *.ts.net     --tailscale │
                     │                                            │
  your own proxy ──► │ 127.0.0.1:<port>  *.<domain>   --domain    │
                     └────────────────────────────────────────────┘
```

LAN names are published over mDNS with `dns-sd` or `avahi`. Caddy serves the LAN and domain names; `tailscale serve` forwards to the backend ports itself, and Parcela never uses Funnel. A domain listener is plain HTTP for your own proxy (cloudflared, nginx) to put TLS in front of. Domain listeners stay on loopback; something you run in front of them makes them public.

## Lifecycle

```diagram
  parcela up
     │
     ├─ find parcela.yaml, register the checkout
     ├─ reserve names, lease ports
     ├─ start the proxy if a service speaks HTTP
     ├─ start the owner ──► setup, prestart ──► services in order
     ├─ wait for readiness checks
     └─ publish routes ──► print URLs

  parcela down
     │
     ├─ withdraw routes
     ├─ stop commands, SIGTERM each process group, SIGKILL after 10 s
     └─ mark the session stopped

  reboot ──► owners gone, sessions still "ready"
         ──► at login, parcela autostart (if enabled) runs
             up --detach for each
```

[Architecture](/docs/architecture/) has the details: locks, recovery, proxy updates and the source map.
