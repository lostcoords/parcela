"""Multi-service graphs: one route group per checkout, per-endpoint health, recovery and partial-start failure.

Lifecycle commands target workspaces by UUID, as the operating skill tells agents to.
"""
import json
import os
from pathlib import Path
import signal
import socket
import subprocess

from live.common import BINARY, SERVICE, Live, wait_for

MANIFEST = f'''version: 1
project: sets
services:
  frontend:
    command: python3 {SERVICE} frontend
    working_dir: frontend
    http:
      healthcheck: /ready
  backend:
    # Restarts the server when the test kills it, so its listener changes within the same run.
    command: while :; do python3 {SERVICE} backend; done
    working_dir: backend
    http:
      subdomain: api
      healthcheck: /ready
  worker:
    command: python3 -c 'import time; print("service-set-worker", flush=True); time.sleep(600)'
tasks:
  probe:
    command: python3 -c 'import json,sys;print(json.dumps(sys.argv[1:]))'
    timeout_seconds: 5
env:
  API_ORIGIN: '${{PARCELA_SERVICE_BACKEND_URL}}'
'''
HOOKS = f'''version: 1
project: sets
services:
  web:
    command: python3 {SERVICE}
    http:
      healthcheck: /ready
hooks:
  setup:
    command: printf x >> "$PARCELA_DATA_DIR/setup-runs" && echo seeded
  prestart:
    command: echo "migrated $(wc -c < "$PARCELA_DATA_DIR/setup-runs" | tr -d ' ')"
  teardown:
    command: echo "dropped $PARCELA_ENVIRONMENT_NAME" > "$PARCELA_DATA_DIR/teardown"
'''
BY_UUID = {'status', 'url', 'logs', 'restart', 'recover', 'down', 'task'}


def run(caddy, **_):
    with Live('sets', caddy) as live:
        workspaces, cases = {}, []

        def cli(path, *argv, check=True, timeout=20):
            if argv[0] in BY_UUID and path in workspaces:
                return subprocess.run([str(BINARY), '--workspace', workspaces[path]['id'], '--state-dir', str(live.state), *argv],
                                      capture_output=True, text=True, check=check, timeout=timeout)
            return live.cli(path, *argv, check=check, timeout=timeout)

        def status(path):
            return json.loads(cli(path, 'status').stdout)

        def remember(session):
            assert len(session['endpoints']) == 2
            live.ports.extend(endpoint['backend_port'] for endpoint in session['endpoints'])
            return session['endpoints']

        request = live.request
        for label, role in [('primary', 'primary'), ('review', 'preview')]:
            source = live.checkout(label, MANIFEST)
            for name in ['frontend', 'backend']:
                (source / name).mkdir()
            workspaces[source] = live.register(source, role, label)
        primary, preview = workspaces
        workspace = workspaces[primary]
        doctor = json.loads(cli(primary, '--json', 'doctor', check=False).stdout)
        assert next(check for check in doctor['checks'] if check['name'] == 'config')['ok']
        context = json.loads(cli(primary, 'context').stdout)
        assert context['workspace']['id'] == workspace['id'] and context['workspace']['role'] == 'primary'
        assert set(context['planned_urls']) == {'frontend', 'backend'}
        inventory = json.loads(cli(live.root, 'ls').stdout)['workspaces']
        assert {row['workspace']['id'] for row in inventory} == {entry['id'] for entry in workspaces.values()}
        assert status(primary)['state'] == 'stopped'
        proxy = live.start_proxy()
        runtime = live.state / 'environments' / workspace['id']
        for directory in (runtime.parent, runtime):
            directory.mkdir(mode=0o700, exist_ok=True)
        gate = runtime / 'unhealthy-backend'
        gate.touch()
        launcher = live.spawn(primary, 'up', '--detach')

        def waiting_for_api():
            observed = status(primary)
            return observed if (observed.get('live') or {}).get('endpoints', {}).get('frontend', {}).get('http_ready') else None
        starting = wait_for(waiting_for_api, [proxy, launcher])
        assert starting['state'] == 'starting' and not starting['live']['endpoints']['backend']['http_ready'], starting
        request('sets.localhost', 503)
        request('api-sets.localhost', 503)
        gate.unlink()
        assert launcher.wait(timeout=20) == 0
        first = json.loads(Path(live.logs[-1].name).read_text())
        cases.append('no-route-before-all-required-endpoints-ready')
        endpoints = remember(first['session'])
        assert first['session']['generation'] == 2
        for host, service in [('sets.localhost', 'frontend'), ('api-sets.localhost', 'backend')]:
            body = request(host)
            assert body['id'] == workspace['id'] and body['argv'] == [service], body
            assert body['cwd'] == str(primary / service), body
            assert body['api_origin'] == f'https://api-sets.localhost:{live.https}', body
            assert body['headers']['X-Forwarded-Host'] == host, body
            assert body['headers']['X-Parcela-Generation'] == '2', body
        assert len({endpoint['backend_port'] for endpoint in endpoints}) == 2
        current = status(primary)
        assert current['live']['state'] == 'ready' and set(current['live']['endpoints']) == {'frontend', 'backend'}, current
        shown = live.human(primary, 'status', '--current')
        assert all(word in shown for word in ['frontend', 'backend', 'ready', f'https://api-sets.localhost:{live.https}']), shown
        shown = live.human(live.root, 'proxy', 'status')
        assert str(live.https) in shown, shown
        assert all(endpoint['http_ready'] and endpoint['listener_owned'] for endpoint in current['live']['endpoints'].values()), current
        assert {entry['name']: entry['state'] for entry in current['processes']} == {'frontend': 'running', 'backend': 'running', 'worker': 'running'}
        assert cli(primary, 'url', 'backend').stdout.strip() == f'https://api-sets.localhost:{live.https}'
        cases.append('two-origins-one-generation-working-directories-and-worker')
        wait_for(lambda: 'service-set-worker' in cli(primary, 'logs', 'worker', '--tail', '10').stdout, [proxy])
        assert json.loads(cli(primary, 'task', 'probe', '--', 'literal argument with spaces').stdout) == ['literal argument with spaces']
        cases.append('agent-context-inventory-uuid-health-logs-and-explicit-task')

        second = json.loads(cli(preview, 'up', '--detach').stdout)
        remember(second['session'])
        assert request('review-sets.localhost')['id'] != workspace['id']
        assert request('api-review-sets.localhost')['id'] != workspace['id']
        cases.append('independent-worktree-service-set')
        collision = live.checkout('collision', MANIFEST)
        refused = cli(collision, 'up', '--name', 'api', '--lan-ip', '192.0.2.1', check=False)
        assert refused.returncode == 1 and 'is already used by' in refused.stderr, refused
        assert request('api-sets.localhost')['id'] == workspace['id']
        cases.append('secondary-host-cannot-be-another-worktree-base')
        unhealthy = runtime / 'unhealthy-backend'
        unhealthy.touch()
        # A healthy answer is reused for a few seconds, so the change shows up shortly after.
        current = wait_for(lambda: (live_status := status(primary)['live'])['state'] == 'unhealthy' and live_status, timeout=8)
        assert not current['endpoints']['backend']['http_ready'], current
        assert current['endpoints']['frontend']['http_ready'], current
        unhealthy.unlink()
        cases.append('per-endpoint-health')
        frontend_pid = request('sets.localhost')['pid']
        backend_pid = request('api-sets.localhost')['pid']
        unhealthy.touch()
        os.kill(backend_pid, signal.SIGTERM)
        wait_for(lambda: status(primary)['state'] == 'starting', [proxy])
        wait_for(lambda: request('sets.localhost', 503), [proxy])
        request('api-sets.localhost', 503)
        unhealthy.unlink()
        wait_for(lambda: status(primary)['state'] == 'ready', [proxy])
        assert request('sets.localhost')['pid'] == frontend_pid
        cases.append('changed-secondary-listener-withdraws-and-reestablishes-group')
        before_pid = request('api-sets.localhost')['pid']
        owner = status(primary)['live']['owner_pid']
        assert owner == first['live']['owner_pid']
        (primary / 'parcela.yaml').write_text(MANIFEST.replace('subdomain: api', 'subdomain: future-api'))
        os.kill(owner, signal.SIGKILL)
        wait_for(lambda: request('api-sets.localhost', 503), [proxy])
        wait_for(lambda: request('sets.localhost', 503), [proxy])
        recovered = json.loads(cli(primary, 'recover', '--detach').stdout)
        assert recovered['session']['token'] == first['session']['token']
        assert request('api-sets.localhost')['pid'] == before_pid
        assert request('sets.localhost')['generation'] == '2'
        assert request('api-sets.localhost')['api_origin'] == f'https://api-sets.localhost:{live.https}'
        (primary / 'parcela.yaml').write_text(MANIFEST)
        cases.append('group-owner-death-and-authenticated-recovery-without-replay')
        with socket.socket() as foreign:
            foreign.bind(('127.0.0.1', 0))
            foreign.listen()
            refused = cli(primary, 'restart', '--detach',
                          '--backend-port', str(foreign.getsockname()[1]), check=False)
            assert refused.returncode != 0
            assert request('api-sets.localhost')['pid'] == before_pid
        restarted = json.loads(cli(primary, 'restart', '--detach').stdout)
        remember(restarted['session'])
        assert restarted['session']['generation'] == 3 and restarted['session']['token'] != first['session']['token']
        assert request('api-sets.localhost')['pid'] != before_pid
        assert request('sets.localhost')['generation'] == '3'
        cases.append('whole-graph-restart-and-foreign-endpoint-refusal')
        (runtime / 'data' / 'retain.txt').write_text('retained')
        cli(primary, 'down')
        request('sets.localhost', 503)
        request('api-sets.localhost', 503)
        assert request('api-review-sets.localhost')['generation'] == '2'
        assert (runtime / 'data' / 'retain.txt').read_text() == 'retained'
        cli(primary, 'down')
        cases.append('group-stop-is-idempotent-and-preserves-other-worktree-and-data')
        (primary / 'parcela.yaml').write_text(MANIFEST.replace(f'python3 {SERVICE} backend', "python3 -c 'import time; time.sleep(600)'"))
        failed = cli(primary, 'up', '--detach', '--readiness-timeout', '2', check=False)
        assert failed.returncode != 0
        request('sets.localhost', 503)
        request('api-sets.localhost', 503)
        final = status(primary)['recorded_session']
        remember(final)
        assert final['status'] == 'stopped', final
        cases.append('partial-startup-failure-never-publishes-and-stops-group')
        hooks = live.checkout('hooks', HOOKS)
        workspaces[hooks] = live.register(hooks, 'preview', 'hooks')
        first = json.loads(cli(hooks, 'up', '--detach').stdout)
        live.ports.append(first['session']['backend_port'])
        runtime = live.state / 'environments' / workspaces[hooks]['id']
        assert (runtime / 'data' / 'setup-runs').read_text() == 'x', 'setup ran once before the service'
        assert (runtime / 'setup-complete').exists()
        logs = cli(hooks, 'logs').stdout
        assert 'setup    | seeded' in logs and 'prestart | migrated 1' in logs, logs
        (hooks / 'parcela.yaml').write_text(HOOKS.replace('hooks:\n', f'''  api:
    command: python3 {SERVICE} api
    http:
      subdomain: api
      healthcheck: /ready
hooks:
''', 1))
        added = json.loads(cli(hooks, 'restart', '--detach').stdout)
        live.ports.extend(endpoint['backend_port'] for endpoint in added['session']['endpoints'])
        assert request('api-hooks-sets.localhost')['id'] == workspaces[hooks]['id'], 'restart reserves an added service'
        (hooks / 'parcela.yaml').write_text(HOOKS)
        shown = live.human(hooks, 'restart', '--detach')
        assert 'hooks-sets.localhost' in shown, shown
        assert set(json.loads(cli(hooks, 'status').stdout)['live']['endpoints']) == {'web'}, 'restart drops a removed service'
        assert (runtime / 'data' / 'setup-runs').read_text() == 'x', 'restart skips setup'
        assert cli(hooks, 'logs', 'prestart').stdout.strip().endswith('migrated 1')
        shown = live.human(hooks, 'down')
        assert 'stopped' in shown.lower(), shown
        (hooks / 'parcela.yaml').write_text(HOOKS.replace('echo seeded', 'echo broken; exit 7'))
        failed = cli(hooks, 'up', '--detach', '--setup', check=False)
        assert failed.returncode != 0 and 'hooks.setup failed with exit code 7; its output is above' in failed.stderr, failed.stderr
        assert cli(hooks, 'rm').returncode == 0
        assert (runtime / 'data' / 'teardown').read_text() == 'dropped hooks\n', 'rm runs hooks.teardown'
        workspaces.pop(hooks)
        cases.append('setup-runs-once-prestart-every-up-a-failed-setup-stops-up-and-rm-runs-teardown')
        shown = live.human(live.root, 'proxy', 'stop')
        assert 'stopped' in shown.lower(), shown
        cases.append('human-output-for-status-restart-down-and-the-proxy')
        return {'https_port': live.https, 'cases': cases}
