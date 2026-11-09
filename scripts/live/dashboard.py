"""Read-only dashboard over real Caddy, optional browser checks, and cleanup."""
import gzip
import http.client
import json
import os
from pathlib import Path
import signal
import subprocess

from live.common import FIXTURES, ROOT, Live, free_port, wait_for

HOST = 'parcela-dashboard.localhost'


def run(caddy, **_):
    with Live('dashboard', caddy) as live:
        origin = f'https://{HOST}:{live.https}'
        manifest = f'''version: 1
project: dashboard-demo
env:
  TEST_UI_ORIGIN: {origin}
services:
  web:
    command: python3 {FIXTURES / 'dashboard_app.py'}
    http: {{healthcheck: /ready}}
  blocked:
    command: python3 {FIXTURES / 'dashboard_app.py'}
    env: {{BLOCK_FRAME: "1"}}
    http: {{subdomain: blocked, healthcheck: /ready}}
'''
        primary = live.checkout('primary', manifest)
        preview = live.checkout('preview', manifest)
        live.start_proxy()
        domain_port = free_port()
        live.ports.append(domain_port)
        sessions = []
        for source, naming in [(primary, ['--primary', '--domain', 'dashboard.example.test', '--domain-port', str(domain_port)]), (preview, ['--name', 'review'])]:
            started = live.json(source, 'up', '--detach', *naming)
            sessions.append(started)
            live.ports.extend(endpoint['backend_port'] for endpoint in started['session']['endpoints'])

        def fetch(path='/', headers=''):
            return live.fetch(HOST, path, headers=headers, host_header=f'{HOST}:{live.https}')

        def snapshot():
            return json.loads(fetch('/api/status')[2])

        data = wait_for(lambda: (data := snapshot())['checked_at_unix_ms'] and len(data['environments']) == 2 and all(env['state'] == 'ready' for env in data['environments']) and data)
        assert {env['name'] for env in data['environments']} == {'primary', 'review'}, data
        assert all(len(env['services']) == 2 and len(env['processes']) == 2 for env in data['environments']), data
        assert not any(key in json.dumps(data) for key in ['recorded_session', 'router_token', 'environment_id', 'command', 'source']), data
        assert fetch()[0] == 200
        assert b'id="environments"' in fetch()[2]
        assert fetch('/workspace')[0] == 404
        assert b'frame-src' not in fetch()[1]
        assert fetch('/widget.js')[0] == 200
        assert fetch('/api/status', 'Origin: https://evil.example\r\n')[0] == 403
        assert fetch('/api/status', 'Sec-Fetch-Site: cross-site\r\n')[0] == 403
        assert fetch('/widget.js', 'Origin: https://dashboard-demo.localhost\r\n')[0] == 200
        assert fetch('/no-such-route')[0] == 404
        assert live.json(live.root, 'ui')['url'] == origin
        assert len(live.json(live.root, 'status')['workspaces']) == 2
        # A script opt-in does not transform compressed or CSP-protected app responses.
        compressed = live.fetch('dashboard-demo.localhost', '/gzip', headers='Accept-Encoding: gzip\r\n')
        assert compressed[0] == 200 and b'Content-Encoding: gzip' in compressed[1]
        assert b'<h1>Local application</h1>' in gzip.decompress(compressed[2])
        strict = live.fetch('dashboard-demo.localhost', '/strict')
        assert b"script-src 'none'" in strict[1]
        domain = http.client.HTTPConnection('127.0.0.1', domain_port, timeout=3)
        domain.request('GET', '/api/status', headers={'Host': HOST})
        assert domain.getresponse().status == 404
        domain.close()
        cases = ['control_origin_absent_on_shared_domain', 'inventory_and_processes', 'host_origin_and_read_only_api', 'dashboard_and_widget_assets', 'compressed_html_and_csp_preserved']
        module = os.environ.get('PARCELA_PLAYWRIGHT_MODULE')
        if module:
            browser = subprocess.run(['node', str(ROOT / 'scripts/live/dashboard-browser.cjs'), module, origin,
                                      str(live.state / 'proxy/data/pki/authorities/local/root.crt')],
                                     capture_output=True, text=True, timeout=180)
            assert browser.returncode == 0, browser.stdout + browser.stderr
            cases.append(json.loads(browser.stdout))
        def review():
            return next(env for env in snapshot()['environments'] if env['name'] == 'review')

        def act(name):
            return live.fetch(HOST, f"/_parcela/environments/{review()['id']}/{name}", method='POST', host_header=f'{HOST}:{live.https}',
                              headers=f'Origin: {origin}\r\nX-Parcela-Action: 1\r\nContent-Length: 0\r\n')[0]

        def pids():
            return {process['name']: process['pid'] for process in review()['processes']}

        def action():
            listed = json.loads(fetch('/_parcela/environments')[2])['environments']
            return next(row for row in listed if row['key'] == review()['id']).get('action') or {}

        # A dead supervisor leaves its services running: Start takes the same ones back.
        running = pids()
        assert all(running.values()), running
        owner = next(row for row in live.json(preview, 'status')['workspaces'] if row['current'])['live']['owner_pid']
        os.kill(owner, signal.SIGKILL)
        wait_for(lambda: review()['state'] == 'interrupted')
        assert act('start') == 202
        wait_for(lambda: review()['state'] == 'ready' and pids() == running)
        cases.append('start_takes_back_an_interrupted_run')
        # A start that fails says why, and a good manifest starts again.
        live.cli(preview, 'down')
        wait_for(lambda: review()['state'] == 'stopped')
        good = (preview / 'parcela.yaml').read_text()
        (preview / 'parcela.yaml').write_text('version: 1\nproject: dashboard-demo\nservices: {web: {command: exit 3, http: {healthcheck: /}}}\n')
        assert act('start') == 202
        wait_for(lambda: action().get('state') == 'failed')
        assert action()['name'] == 'start' and action()['message'], action()
        (preview / 'parcela.yaml').write_text(good)
        assert act('start') == 202
        wait_for(lambda: review()['state'] == 'ready')
        cases.append('failed_start_is_reported_and_can_be_retried')
        live.cli(preview, 'down')
        wait_for(lambda: review()['state'] == 'stopped')
        cases.append('status_updates_after_down')
        live.cli(live.root, 'proxy', 'stop')
        assert not fetch_socket_exists(live)
        cases.append('dashboard_socket_removed_on_stop')
        return {'cases': cases, 'browser_verified': bool(module)}


def fetch_socket_exists(live):
    evidence = live.state / 'proxy/dashboard-socket.json'
    assert evidence.exists()
    # The socket is under the same per-state directory as Caddy's private admin socket.
    config = json.loads((live.state / 'proxy/caddy.json').read_text())
    return (Path(config['admin']['listen'].removeprefix('unix/')).parent / 'dashboard.sock').exists()
