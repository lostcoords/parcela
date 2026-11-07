# Configure your app

Put the commands your app already uses in `parcela.yaml`. Configure data once so every checkout can start with `parcela up`.

## Services and setup

```yaml
version: 1
project: shop

services:
  web:
    command: bin/rails server -p $PORT -b 127.0.0.1
    http:
      healthcheck: /up
  jobs:
    command: bin/jobs

hooks:
  setup:
    command: bin/setup
  prestart:
    command: bin/rails db:migrate
```

Give each process a service. HTTP services listen on `127.0.0.1:$PORT` and return 2xx at their healthcheck path. For another HTTP service, set `http.subdomain`, such as `api`. Workers need no `http` section.

Use `depends_on` when a service or hook needs another service ready first. `setup` runs until it succeeds once. `prestart` runs on every new start. Make setup safe to retry. If needed, copy untracked configuration from `$PARCELA_PRIMARY_DIR` without printing secrets.

## Separate data

URLs and ports do not isolate databases, queues, or uploads. Configure each app component explicitly. Files under `PARCELA_DATA_DIR` belong to one registered environment and survive worktree removal. Shared `.env` values can still point multiple copies at the same database.

### SQLite

Keep the file in `PARCELA_DATA_DIR`, so it survives removing the worktree:

```yaml
env:
  DATABASE_PATH: ${PARCELA_DATA_DIR}/development.sqlite3
hooks:
  setup:
    command: bin/rails db:prepare
```

Rails, in `config/database.yml`:

```yaml
development:
  adapter: sqlite3
  database: <%= ENV.fetch("DATABASE_PATH", "storage/development.sqlite3") %>
```

Django, in `settings.py`, with `python manage.py migrate` as the setup command:

```python
DATABASES = {
    "default": {
        "ENGINE": "django.db.backends.sqlite3",
        "NAME": os.environ.get("DATABASE_PATH", BASE_DIR / "db.sqlite3"),
    }
}
```

### PostgreSQL

Choose a server per environment, a container per environment, or one database per environment on a shared server.

**A server per environment.** Run `postgres` as a service on a named port, as in [the Rails and PostgreSQL example](https://github.com/lostcoords/parcela/blob/main/examples/rails-postgres/parcela.yaml). Its files live in `PARCELA_DATA_DIR` and `down` stops it.

**A container per environment.** Run the image your `docker-compose.yml` already pins, so every worktree and every teammate gets the same version. The Compose file stays as it is:

```yaml
# docker-compose.yml
services:
  postgres:
    image: postgres:17
    environment:
      POSTGRES_PASSWORD: postgres
    ports: ["5432:5432"]
    volumes: [db:/var/lib/postgresql/data]
    healthcheck:
      test: ["CMD", "pg_isready", "-U", "postgres"]
      interval: 2s
volumes:
  db:
```

```yaml
# parcela.yaml
env:
  DATABASE_URL: postgres://postgres:postgres@127.0.0.1:${PARCELA_PORT_POSTGRES}/shop_development
services:
  db:
    compose:
      file: docker-compose.yml
      services: [postgres]
  web:
    command: bin/rails server -p $PORT -b 127.0.0.1
    http:
      healthcheck: /up
    depends_on: [db]
hooks:
  setup:
    command: bin/rails db:prepare
    depends_on: [db]
```

Each environment gets its own container and volume, `shop-signup-postgres-1` and `shop-signup_db`, on its own loopback port. `docker compose exec postgres psql -U postgres` in a task reaches the environment's own server. [Docker Compose](#docker-compose) has the details.

**One shared server, one database per environment.** Name the database after the environment. libpq tools such as `psql`, `createdb` and `dropdb` read `PGHOST`:

```yaml
env:
  PGHOST: 127.0.0.1
  DATABASE_URL: postgres://127.0.0.1/shop_${PARCELA_ENVIRONMENT_NAME}
hooks:
  setup:
    command: bin/rails db:prepare
  prestart:
    command: bin/rails db:migrate
  teardown:
    command: dropdb --if-exists "shop_$PARCELA_ENVIRONMENT_NAME"
```

`bin/rails db:prepare` creates the database. Django does not create databases, so do it in `setup` before migrating:

```yaml
hooks:
  setup:
    command: >-
      psql -d postgres -tAc "SELECT 1 FROM pg_database WHERE datname = 'shop_$PARCELA_ENVIRONMENT_NAME'" | grep -q 1
      || createdb "shop_$PARCELA_ENVIRONMENT_NAME";
      python manage.py migrate
```

- `teardown` runs on `parcela rm` of any environment, the main checkout included, where it drops `shop_primary`. Leave it out if that is not what you want.
- Environment names can contain hyphens. `createdb`, `dropdb` and Rails quote the name. Quote it yourself in raw SQL.
- To start a worktree from a copy of the main checkout's data, run `createdb -T shop_primary "shop_$PARCELA_ENVIRONMENT_NAME"` in `setup` instead. PostgreSQL refuses while anything is connected to `shop_primary`, so stop the main checkout first.

### Docker Compose

Keep the backing services in `docker-compose.yml` and the apps native. A [compose](configuration.md#compose) service starts the Compose services an environment needs, and the apps depend on it:

```yaml
env:
  DATABASE_URL: postgres://postgres@127.0.0.1:${PARCELA_PORT_POSTGRES}/shop
  ELASTICSEARCH_URL: http://127.0.0.1:${PARCELA_PORT_SEARCH}

readiness_timeout_seconds: 600

services:
  deps:
    compose:
      file: docker-compose.yml
      services: [postgres, search]
  web:
    command: bin/rails server -p $PORT -b 127.0.0.1
    http:
      healthcheck: /up
    depends_on: [deps]
```

- Each environment is its own Compose project, `shop-primary` or `shop-signup`, with its own containers, network and volumes. Every `docker compose` in hooks and tasks uses it.
- The ports come from the Compose file: each port a started container publishes is leased. The first one of a service is named after it, like `PARCELA_PORT_SEARCH`, and the others add the container port, like `PARCELA_PORT_SEARCH_9300`. Parcela publishes it on `127.0.0.1` only, through an override file, so fixed host ports, `container_name` and volume and network `name`s in the Compose file stay. `external` volumes and networks stay shared.
- `deps` is ready once every listed service runs and is healthy. Give the Compose services a `healthcheck`, or the app may start before the database accepts connections. Raise `readiness_timeout_seconds` for one that is slow on its first start, like Elasticsearch.
- `down` stops the containers, even if `docker compose up` was killed or Parcela crashed, and keeps the volumes.
- A built image used by several worktrees needs an `image:` name next to `build:`, or Compose builds one per environment.
- Compose reads `.env` next to the Compose file for its own variables, such as passwords. Variables from `parcela.yaml` take precedence. The containers start before `setup`, so a new worktree cannot copy it first: point Compose at the main checkout's with `COMPOSE_ENV_FILES: ${PARCELA_PRIMARY_DIR}/.env`.
- Nothing deletes an environment's volumes. `parcela rm` removes its containers and network and lists the volumes to delete with `docker volume rm`.
- A container that serves HTTP, like Mailpit, MinIO's console or Kibana, gets its own URL through `http` under `compose`. See [Containers with URLs](configuration.md#containers-with-urls).

### Redis, Sidekiq and queues

Run a Redis per environment on a named port. Its dump goes to `PARCELA_DATA_DIR`:

```yaml
ports: [redis]

env:
  REDIS_URL: redis://127.0.0.1:${PARCELA_PORT_REDIS}/0

services:
  redis:
    command: exec redis-server --port "$PARCELA_PORT_REDIS" --bind 127.0.0.1 --dir "$PARCELA_DATA_DIR"
    healthcheck: redis-cli -p "$PARCELA_PORT_REDIS" ping
  web:
    command: bin/rails server -p $PORT -b 127.0.0.1
    http:
      healthcheck: /up
    depends_on: [redis]
  sidekiq:
    command: bundle exec sidekiq
    depends_on: [redis]
```

Pass `REDIS_URL` to each component that uses Redis, including Rails' cache store and Action Cable's Redis adapter. Sidekiq reads it when no other URL is configured. Rails' generated development Cable configuration uses the `async` adapter until changed.

Keep job queues separate. Workers sharing the same queue can consume each other's jobs. Sidekiq 7 does not support namespaces. On a shared Redis server, assign a distinct database index to each environment. For caches that accept namespaces, use a prefix such as `shop:${PARCELA_ENVIRONMENT_NAME}:`.

Queues that live in the database, such as Solid Queue, GoodJob or Oban, are separate once the database is.

### Uploads and Active Storage

Point the upload root at `PARCELA_DATA_DIR`:

```yaml
env:
  STORAGE_ROOT: ${PARCELA_DATA_DIR}/storage
```

Rails, in `config/storage.yml`:

```yaml
local:
  service: Disk
  root: <%= ENV.fetch("STORAGE_ROOT") { Rails.root.join("storage").to_s } %>
```

Django: `MEDIA_ROOT = os.environ.get("STORAGE_ROOT", BASE_DIR / "media")`.


`down` keeps data. `rm` forgets the registration and retains its runtime directory, but a declared teardown hook can delete external data first. A later registration gets a new directory. Retained files are not automatically reused.

## Framework settings

Local `.localhost` names work with the defaults below. Add `PARCELA_SHARE_HOSTS` for LAN or tailnet access. Keep these settings in development configuration.

### Rails

Rails 6 and later allow `.localhost` hosts in development. Add the shared names, and set URLs for mailers and jobs, which have no request to take them from. In `config/environments/development.rb`:

```ruby
config.hosts.concat(ENV.fetch("PARCELA_SHARE_HOSTS", "").split(","))

if ENV["PARCELA_URL"]
  url = URI(ENV["PARCELA_URL"])
  config.action_mailer.default_url_options = { host: url.host, port: url.port, protocol: url.scheme }
end
```

Action Cable normally accepts the page's own origin. If it rejects that origin, set `config.action_cable.allowed_request_origins = [ENV["PARCELA_URL"]]`.

### Django

When adding shared hosts, retain the local names too. Trust the proxy's HTTPS scheme to avoid a mismatch during CSRF checks. In development `settings.py`:

```python
SECURE_PROXY_SSL_HEADER = ("HTTP_X_FORWARDED_PROTO", "https")
ALLOWED_HOSTS = [".localhost", "127.0.0.1", "[::1]"] + [
    host for host in os.environ.get("PARCELA_SHARE_HOSTS", "").split(",") if host
]
CSRF_TRUSTED_ORIGINS = [os.environ["PARCELA_URL"]] if "PARCELA_URL" in os.environ else []
```

Start it with `python manage.py runserver 127.0.0.1:$PORT`.

### Vite

Vite [allows](https://vite.dev/config/server-options#server-allowedhosts) `localhost`, its subdomains and IP addresses. Add the shared names:

```js
server: {
  host: '127.0.0.1',
  port: Number(process.env.PORT),
  strictPort: true,
  allowedHosts: (process.env.PARCELA_SHARE_HOSTS ?? '').split(',').filter(Boolean),
},
```

Keep HMR on the HTTP service's port. Vite connects to the page's host over `wss://`, and Caddy forwards the WebSocket. A separate `server.hmr.port` would need its own route.

If the project already overrides `server.ws.clientPort`, or `server.hmr.clientPort` on older Vite, set it to `new URL(process.env.PARCELA_URL).port || 443`.

When Vite runs next to a backend, as with Vite Ruby or Laravel's plugin, give it its own HTTP service, so it gets a free port and its own HTTPS URL:

```yaml
services:
  vite:
    command: npx vite --host 127.0.0.1 --port $PORT --strictPort
    http:
      subdomain: vite
      healthcheck: /@vite/client
```

Set `server.origin: process.env.PARCELA_SERVICE_VITE_URL` so asset URLs point at it. The plugin decides how the page finds the dev server:

- **Laravel.** `laravel-vite-plugin` writes `server.origin` to its hot file, and `@vite` loads assets from there.
- **Vite Ruby.** Rails proxies assets to `VITE_RUBY_HOST` and `VITE_RUBY_PORT`, and the plugin sets Vite's port from them. Set them in the top-level `env`, to `127.0.0.1` and `${PARCELA_SERVICE_VITE_PORT}`. The plugin also points the HMR websocket at that port unless `clientPort` is set.

### Next.js

Next.js blocks dev-only requests from origins it does not know. It [allows](https://nextjs.org/docs/app/api-reference/config/next-config-js/allowedDevOrigins) `localhost` and its subdomains. Add the shared names, in `next.config.js`:

```js
module.exports = {
  allowedDevOrigins: (process.env.PARCELA_SHARE_HOSTS ?? '').split(',').filter(Boolean),
};
```

Start it with `next dev --port $PORT --hostname 127.0.0.1`.

### Phoenix

Set the endpoint's `url` so generated URLs use the proxy. In `config/dev.exs`:

```elixir
parcela = URI.parse(System.get_env("PARCELA_URL", "http://localhost:4000"))

config :my_app, MyAppWeb.Endpoint,
  http: [ip: {127, 0, 0, 1}, port: String.to_integer(System.get_env("PORT", "4000"))],
  url: [host: parcela.host, port: parcela.port, scheme: parcela.scheme]
```

A new app's `config/dev.exs` sets `check_origin: false`. If you turn the check on, the default compares the `Origin` host with `url`'s host, so list the shared names too. `check_origin: :conn` also compares the scheme, which is `http` behind the proxy, so it refuses the page.

### Laravel

```yaml
env:
  APP_URL: ${PARCELA_URL}
services:
  web:
    command: php artisan serve --host=127.0.0.1 --port=$PORT --no-reload
    http:
      healthcheck: /up
```

Keep `--no-reload` so `php artisan serve` passes Parcela's environment through when `.env` exists. In `bootstrap/app.php`, [trust the loopback proxy](https://laravel.com/docs/12.x/requests#configuring-trusted-proxies) so generated URLs use HTTPS:

```php
->withMiddleware(fn ($middleware) => $middleware->trustProxies(at: '127.0.0.1'))
```

## Keep browser sessions separate

Use host-only cookies to keep logins separate between `signup-shop.localhost` and `shop.localhost`. HTTPS allows `Secure` cookies.

`localhost` is not on the [Public Suffix List](https://publicsuffix.org/), so browsers treat it as a top-level domain, and `shop.localhost` and `api-shop.localhost` are different sites. A request from one to the other is cross-site, and cookies with `SameSite=Lax`, Rails' and Django's default, are not sent with it. Either keep the browser on one origin, for example with a dev-server proxy to `http://127.0.0.1:${PARCELA_SERVICE_API_PORT}`, or use `SameSite=None; Secure` cookies with credentialed CORS.

With `wildcard: true`, tenants such as `kobe.signup-shop.localhost` are subdomains of the environment's host. A cookie for `Domain=signup-shop.localhost` reaches every tenant of that worktree and no other worktree.

## Connect services

For server-side calls, use the other service's loopback port. This avoids DNS and certificate configuration:

```yaml
env:
  API_INTERNAL_URL: http://127.0.0.1:${PARCELA_SERVICE_API_PORT}
  API_PUBLIC_URL: ${PARCELA_SERVICE_API_URL}
```

Use the public URL for anything a browser follows. The request arrives with `Host: 127.0.0.1:<port>`, which Rails and Django allow in development and the Django settings above list.

## Configure callbacks and webhooks

Every worktree has its own origin, so its OAuth callback is its own URL, such as `https://signup-shop.localhost:1443/auth/callback`. Build it from the request or from `PARCELA_URL`, never from a fixed host.

- Most providers match redirect URIs exactly, port included, and few accept wildcards. Register the callback for each name you use. Names stay after the first `up`, and `parcela up --name NAME` picks one, so a few registered names can be reused.
- `parcela relay install` drops the `:1443` from URLs. Register the URLs you will actually see.
- With `wildcard: true`, register one callback on the environment's host and send the tenant along in the OAuth `state`. A provider cannot be given every tenant host.
- Some providers refuse `*.localhost`. [Google](https://developers.google.com/identity/protocols/oauth2/web-server#uri-validation) requires a top-level domain on the Public Suffix List, which `localhost` is not.

`.localhost` URLs answer only on this machine, so a webhook from the internet cannot reach them. `--tailscale` serves your tailnet only, and Parcela never uses Funnel. Use the provider's forwarding CLI as a service and point it at the backend port:

```yaml
services:
  stripe:
    command: stripe listen --forward-to "http://127.0.0.1:$PARCELA_SERVICE_WEB_PORT/webhooks"
```

## Verify the configuration

Run `parcela doctor`, then `parcela up --detach`. Open every returned URL and read `parcela logs`. Before giving agents parallel checkouts, check that their data is separate as intended. After editing configuration, use `parcela restart --detach`.

The [configuration reference](configuration.md) lists every field. The [CLI reference](cli.md) lists commands, flags, and task syntax.
