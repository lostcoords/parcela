"""Routes, streams, ownership, process lifecycle, logs and status against real Caddy."""
import json
from pathlib import Path
import signal
import socket
import subprocess

from live.common import BINARY, SERVICE, Live, wait_for

MANIFEST = f'''version: 1
project: live
services:
  web:
    command: python3 {SERVICE}
    http:
      healthcheck: /ready
  worker:
    command: python3 -c 'import time; print("cli-worker-ready", flush=True); time.sleep(600)'
'''
WEBSOCKET = 'Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n'


def control_socket(runtime):
    return Path(json.loads((runtime / 'context.json').read_text())['sockets']['control'])


def run(caddy, **_):
    with Live('core', caddy) as live:
        cli, request, fetch = live.cli, live.request, live.fetch
        copies = []
        for name, role in [('primary', 'primary'), ('auth', 'preview'), ('compose', 'preview')]:
            path = live.checkout(name, MANIFEST)
            copies.append((path, live.register(path, role, name)['id']))
        (primary_path, primary_id), (auth_path, auth_id), (compose_path, compose_id) = copies
        proxy = live.start_proxy()
        primary = live.spawn(primary_path, 'run', '--', 'python3', str(SERVICE), 'literal argument with spaces', '--flag')
        preview = live.spawn(auth_path, 'run', '--', 'python3', str(SERVICE))
        wait_for(lambda: request('live.localhost'), [proxy, primary, preview])
        wait_for(lambda: request('auth-live.localhost'), [proxy, primary, preview])
        assert request('live.localhost')['id'] == primary_id
        assert request('auth-live.localhost')['id'] == auth_id
        assert request('live.localhost')['argv'] == ['literal argument with spaces', '--flag']
        spoofed = json.loads(fetch('live.localhost', headers='X-Forwarded-Proto: http\r\nX-Forwarded-Host: evil.example\r\n')[2])
        assert spoofed['headers']['X-Forwarded-Proto'] == 'https', spoofed
        assert spoofed['headers']['X-Forwarded-Host'] == 'live.localhost', spoofed
        assert b'data: ready' in fetch('live.localhost', '/sse')[2]
        assert fetch('live.localhost', '/ws', headers=WEBSOCKET)[0] == 101
        assert fetch('live.localhost', host_header='unknown.localhost')[0] == 404, 'an unknown Host cannot pick a route by SNI'
        with socket.create_connection(('127.0.0.1', live.https), timeout=3) as plain:
            plain.sendall(b'GET /a?b=1 HTTP/1.1\r\nHost: live.localhost\r\nConnection: close\r\n\r\n')
            redirect = b''
            while part := plain.recv(65536):
                redirect += part
        assert redirect.split(b'\r\n')[0].endswith(b' 308 Permanent Redirect'), redirect
        assert b'\r\nLocation: https://live.localhost/a?b=1\r\n' in redirect, redirect
        duplicate = cli(primary_path, 'run', '--', 'python3', str(SERVICE), check=False)
        assert duplicate.returncode != 0 and primary.poll() is None
        primary.send_signal(signal.SIGTERM)
        assert primary.wait(timeout=10) == 143
        assert request('auth-live.localhost')['id'] == auth_id
        assert fetch('auth-live.localhost', host_header='live.localhost')[0] == 503
        assert cli(primary_path, 'run', '--', 'python3', '-c', 'raise SystemExit(7)', check=False).returncode == 7
        with socket.socket() as foreign:
            foreign.bind(('127.0.0.1', 0))
            foreign.listen()
            port = foreign.getsockname()[1]
            assert cli(primary_path, 'run', '--backend-port', str(port), '--', 'python3', str(SERVICE), check=False).returncode != 0
            assert foreign.getsockname()[1] == port

        owner = live.spawn(compose_path, 'up')
        wait_for(lambda: request('compose-live.localhost'), [proxy, preview, owner])
        assert request('compose-live.localhost')['id'] == compose_id
        status = live.json(compose_path, 'status', '--current')
        assert status['state'] == 'ready' and status['live_health_verified']
        assert status['live']['http_ready'] and status['live']['listener_owned']
        assert {item['name']: item['state'] for item in status['processes']} == {'web': 'running', 'worker': 'running'}
        assert all(item['pid'] for item in status['processes']), status['processes']
        wait_for(lambda: 'cli-worker-ready' in cli(compose_path, 'logs', 'worker', '--tail', '10').stdout, [owner])
        assert 'PROCESS' in live.human(compose_path, 'status', '--current')
        assert 'worker | cli-worker-ready' in cli(compose_path, 'logs', '--tail', '10').stdout
        follower = subprocess.Popen(live.argv(compose_path, 'logs', '--follow', '--tail', '10'), stdout=subprocess.PIPE, text=True)
        assert any('worker | cli-worker-ready' in follower.stdout.readline() for _ in range(20)), 'logs --follow prints every process'
        follower.send_signal(signal.SIGINT)
        assert follower.wait(timeout=10) == 0, 'logs --follow exits cleanly on Ctrl-C'
        assert cli(compose_path, 'logs', 'unknown', check=False).returncode != 0
        assert cli(auth_path, 'restart', '--detach', check=False).returncode != 0
        assert preview.poll() is None
        assert cli(compose_path, 'restart', '--detach', '--readiness-timeout', '0', check=False).returncode == 2
        assert cli(compose_path, 'restart', '--detach', '--backend-port', '80', check=False).returncode == 2
        assert owner.poll() is None
        assert cli(compose_path, 'url').stdout.strip() == status['recorded_session']['public_url']
        runtime = live.state / 'environments' / compose_id
        (runtime / 'data' / 'retain.txt').write_text('preserve application data')
        (runtime / 'unhealthy').touch()
        unhealthy = live.json(compose_path, 'status', '--current')
        assert unhealthy['state'] == 'unhealthy' and unhealthy['recorded_session']['status'] == 'ready'
        assert unhealthy['live_health_verified'] and not unhealthy['live']['http_ready']
        (runtime / 'unhealthy').unlink()
        stopped = live.json(compose_path, 'down')
        assert stopped['state'] == 'stopped' and owner.wait(timeout=10) == 0
        assert not control_socket(runtime).exists()
        assert fetch('auth-live.localhost', host_header='compose-live.localhost')[0] == 503
        assert live.json(compose_path, 'down')['already_stopped']

        first = live.json(compose_path, 'up', '--detach')
        assert first['detached'] and first['live']['state'] == 'ready'
        assert first['workspace']['generation'] == first['session']['generation']
        again = live.json(compose_path, 'up', '--detach')
        assert again['already_running'] and again['session']['token'] == first['session']['token']
        second = live.json(compose_path, 'restart', '--detach')
        assert second['session']['generation'] > first['session']['generation']
        assert second['session']['token'] != first['session']['token']
        assert request('compose-live.localhost')['generation'] == str(second['session']['generation'])
        with socket.socket(socket.AF_UNIX) as stale:
            stale.settimeout(3)
            stale.connect(str(control_socket(runtime)))
            stale.sendall((json.dumps({'environment_id': compose_id, 'token': first['session']['token'],
                                       'generation': first['session']['generation'], 'action': {'type': 'stop'}}) + '\n').encode())
            assert 'stale' in json.loads(stale.makefile().readline())['error']
        assert request('compose-live.localhost')['id'] == compose_id
        inventory = live.json(live.root, 'ls')['workspaces']
        assert {row['workspace']['label']: row['state'] for row in inventory} == {'primary': 'stopped', 'auth': 'ready', 'compose': 'ready'}
        assert 'compose' in live.human(live.root, 'ls')
        selected = subprocess.run([str(BINARY), '--state-dir', str(live.state), '--workspace', compose_id, 'status'],
                                  capture_output=True, text=True, check=True, timeout=15)
        assert json.loads(selected.stdout)['workspace']['id'] == compose_id
        assert live.json(compose_path, 'stop')['state'] == 'stopped'
        assert (runtime / 'data' / 'retain.txt').read_text() == 'preserve application data'
        with socket.socket(socket.AF_UNIX) as foreign_control:
            foreign_control.bind(str(control_socket(runtime)))
            failed = cli(compose_path, 'up', check=False)
            assert failed.returncode != 0 and control_socket(runtime).exists()
        control_socket(runtime).unlink()
        assert cli(compose_path, 'up', '--detach', '--lan-ip', '192.0.2.1', check=False).returncode != 0
        assert live.json(compose_path, 'status', '--current')['state'] == 'stopped'
        assert not control_socket(runtime).exists()
        assert request('auth-live.localhost')['id'] == auth_id
        return {'https_port': live.https, 'verified': [
            'owned Caddy TLS/CA without trust', 'two simultaneous routes', 'literal argv', 'forwarded scheme/host spoof rejection',
            'SSE', 'WebSocket upgrade', 'unknown host rejection', 'duplicate ownership', 'SIGTERM', 'exit status', 'foreign port safety',
            'supervised web+worker graph', 'independent teardown', 'owner-mediated down/stop', 'detached startup/readiness',
            'restart generation', 'unhealthy probe separate from recorded ready', 'stale command rejection',
            'live/aggregate status and UUID targeting', 'service/log inspection', 'data retention',
            'foreign control socket refusal', 'failed detached startup cleanup']}
