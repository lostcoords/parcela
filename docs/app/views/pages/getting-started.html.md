
# Get started

Set up Parcela so `parcela up` starts your app. Open it at a local HTTPS URL such as `https://shop.localhost:1443`.

Start with a project you can already run locally on macOS or Linux. Use the agent prompt below, or [follow the manual steps](#install-it-by-hand).

To try Parcela with a small sample project, follow the [example app](/docs/tutorial/).

## Install it with your agent

From the root of your project, paste this prompt into your coding agent:

```text
Set up Parcela for this project, so that `parcela up` alone runs
the whole app in any checkout or worktree, each at its own HTTPS URL.

1. If `parcela --version` fails, install it:
   curl -fsSL https://parcela.run/install.sh | sh
   Then run `parcela doctor` and tell me what it says is missing.
2. Read `parcela --skill` and https://parcela.run/docs/configuration/.
   Save the skill where you load skills (for Claude Code,
   .claude/skills/parcela/SKILL.md) unless one is there.
3. Find how the project runs today: Procfile, bin/dev,
   docker-compose.yml, package.json scripts, Makefile, README.
   Don't read .env files or secrets.
4. Write parcela.yaml with the project's real commands:
   - one service per process; HTTP services listen on
     127.0.0.1:$PORT and have an http.healthcheck;
   - data that belongs to each environment: SQLite under
     $PARCELA_DATA_DIR, or one database per environment;
   - databases and other containers already in docker-compose.yml
     as one service with `compose: {file, services}`, the apps
     native, as in
     https://parcela.run/docs/frameworks/#docker-compose;
   - hooks.setup to install packages, copy .env from
     $PARCELA_PRIMARY_DIR and create and seed the database;
     hooks.prestart to run migrations.
   Ask me before deciding how databases, uploads or queues are shared.
5. Run `parcela doctor`, then `parcela up --detach`. Check every URL
   with curl and read `parcela logs`. Fix what fails.
6. Show me parcela.yaml and what you verified. Leave the app running.
```

The prompt asks before choosing how to share data. Review the resulting configuration and verification report. [Install the operating skill](/docs/worktrees/#set-up-a-coding-agent) to give later agents the same instructions.

## Install it by hand

1. Install Parcela. The installer also installs Caddy when needed and writes to `~/.local/bin` without `sudo`.

   ```sh
   curl -fsSL https://parcela.run/install.sh | sh
   ```

   With Homebrew, use `brew install lostcoords/tap/parcela` instead. For a nightly build, add `sh -s -- --nightly` to the installer command. For a source build, use `cargo install --git https://github.com/lostcoords/parcela --locked` and install [Caddy](https://caddyserver.com/docs/install) separately.

2. From your project root, create the configuration.

   ```sh
   parcela init
   ```

   Review the generated `parcela.yaml`. Parcela guesses a command for Node, Rails, or Django projects and uses a Python file server otherwise. Replace the guess if needed. Each HTTP service must listen on `127.0.0.1:$PORT` and answer its `http.healthcheck` path with a 2xx response.

3. Check the configuration and tools.

   ```sh
   parcela doctor
   ```

   Fix any failed checks before starting. [Configure your app](/docs/frameworks/) covers workers, databases, and setup hooks. Separate URLs do not separate data.

4. Start the environment.

   ```sh
   parcela up --detach
   ```

   Open the HTTPS URL in the output. The first `up` in a terminal offers to trust the local certificate authority behind your system's password prompt. Scripts and agents never change trust. If your browser warns, run `parcela trust` in a terminal, then restart the browser. For a command-line check without system trust, use `curl --cacert "$(parcela trust --print)" "$(parcela url)"`.

5. Read the output and stop the environment when finished.

   ```sh
   parcela logs
   parcela down
   ```

   `down` stops the processes and keeps your source and data.

## Everyday commands

| Goal | Command |
| --- | --- |
| Check health | `parcela status` |
| Read one service's output | `parcela logs web --tail 50` |
| Apply configuration changes | `parcela restart --detach` |
| Run a configured task | `parcela task test` |
| Print an API URL | `parcela url api` |
| List checkouts | `parcela ls` |

[Configure your app](/docs/frameworks/) to add services and data. [Worktrees and agents](/docs/worktrees/) covers parallel copies. For a complete example, follow the [tutorial](/docs/tutorial/). Upgrades and removal are in the [FAQ](/docs/faq/#upgrade-parcela).
