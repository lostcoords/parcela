# Example app

Build a page, a SQLite-backed API, and a worker. Then run a second copy in a worktree with its own database.

You need macOS or Linux, Python, Git, and [Parcela with Caddy installed](/docs/getting-started/#install-it-by-hand). This tutorial uses a new directory. The finished app is in [examples/docs-demo](https://github.com/lostcoords/parcela/tree/main/examples/docs-demo).

## Create the app

Create an empty directory and enter it:

```sh
mkdir hello
cd hello
```

Create `index.html`:

```html
<h1>Hello from Parcela</h1>
```

Create `db.py`. Setup writes one greeting to this environment's database. The `list` command reads it.

```python
import os
import sqlite3
import sys

database = sqlite3.connect(os.environ["DATABASE_PATH"])

if sys.argv[1] == "setup":
    database.execute("CREATE TABLE IF NOT EXISTS messages (text TEXT)")
    database.execute("INSERT INTO messages VALUES (?)", (f"hello from {os.environ['PARCELA_ENVIRONMENT_NAME']}",))
    database.commit()
    print("database ready")
else:
    for (text,) in database.execute("SELECT text FROM messages"):
        print(text)
```

Create `api.py`. It returns the greeting and the API's public URL.

```python
import json
import os
import sqlite3
from http.server import BaseHTTPRequestHandler, HTTPServer


class Hello(BaseHTTPRequestHandler):
    def do_GET(self):
        database = sqlite3.connect(os.environ["DATABASE_PATH"])
        (message,) = database.execute("SELECT text FROM messages LIMIT 1").fetchone()
        body = {"message": message, "url": os.environ["PARCELA_SERVICE_API_URL"]}
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(body).encode() + b"\n")


HTTPServer(("127.0.0.1", int(os.environ["PORT"])), Hello).serve_forever()
```

Create `worker.py`. It prints a tick every five seconds.

```python
import time

tick = 0
while True:
    tick += 1
    print(f"tick {tick}", flush=True)
    time.sleep(5)
```

## Describe the processes

Create `parcela.yaml`:

```yaml
# yaml-language-server: $schema=https://parcela.run/schema/v1.json
version: 1
project: hello

env:
  DATABASE_PATH: ${PARCELA_DATA_DIR}/hello.sqlite3

services:
  web:
    command: python3 -m http.server $PORT --bind 127.0.0.1
    http:
      healthcheck: /
  api:
    command: python3 api.py
    http:
      subdomain: api
      healthcheck: /
  ticker:
    command: python3 worker.py

hooks:
  setup:
    command: python3 db.py setup

tasks:
  messages:
    command: python3 db.py list
```

The `web` and `api` services get ports and HTTPS URLs. `ticker` has no HTTP endpoint. The setup hook creates a SQLite database under `PARCELA_DATA_DIR` on the environment's first start. The `messages` task reads it.

## Start the first environment

Check the tools and configuration:

```sh
parcela doctor
```

Fix any failed checks, then start the app:

```sh
parcela up --detach
```

With the default proxy port, the page is at `https://hello.localhost:1443` and the API at `https://api-hello.localhost:1443`. If your proxy uses another port, use the URLs Parcela prints.

Open the page. You see "Hello from Parcela". A terminal's first `up` offers system CA trust behind a password prompt. The following command verifies HTTPS with the certificate directly and needs no system trust change:

```sh
curl --cacert "$(parcela trust --print)" "$(parcela url api)"
```

The JSON has `message: "hello from primary"` and the API's URL. Read the worker's output and query the database:

```sh
parcela logs ticker --tail 2
parcela task messages
```

The log contains `tick 1` or a later tick. The task prints `hello from primary`.

## Start a second checkout

Commit the files in this new tutorial repository and create a worktree:

```sh
git init -q
git add .
git commit -qm "hello app"
git worktree add ../signup
cd ../signup
parcela up --detach
```

The page is now at `https://signup-hello.localhost:1443`, and the API has the prefix `api-signup-hello`. The primary environment keeps running.

Request this checkout's API:

```sh
curl --cacert "$(parcela trust --print)" "$(parcela url api)"
parcela task messages
```

Both report `hello from signup`. The new checkout has its own database because setup ran with a different `PARCELA_DATA_DIR`.

List both environments:

```sh
parcela ls
```

The table includes `primary` and `signup`, both ready.

## Restart without resetting data

```sh
parcela restart --detach
parcela task messages
```

The task still prints `hello from signup`. Setup already succeeded, so a normal restart keeps the database and skips setup.

## Stop and clean up

Stop both environments of the tutorial project:

```sh
parcela down --all
cd ../hello
git worktree remove ../signup
parcela rm --prune
```

`rm --prune` forgets the removed checkout and releases its names. It retains the runtime directory and reports where the data remains. Source files in `hello` remain too.

The shared proxy stays running. To stop it, run `parcela proxy stop` when no other environment needs its URLs.

For your own app, continue with [Configure your app](/docs/frameworks/#services-and-setup). To use jj workspaces or coding agents, see [Worktrees and agents](/docs/worktrees/).
