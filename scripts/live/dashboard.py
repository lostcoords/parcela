"""Read-only dashboard over real Caddy, optional browser checks, and cleanup."""
import gzip
import http.client
import json
import os
from pathlib import Path
import subprocess

from live.common import FIXTURES, ROOT, Live, free_port, wait_for

HOST = 'parcela-ui.localhost'


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
        assert b'Your environments' in fetch()[2]
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
                                     capture_output=True, text=True, timeout=90)
            assert browser.returncode == 0, browser.stdout + browser.stderr
            cases.append(json.loads(browser.stdout))
        live.cli(preview, 'down')
        wait_for(lambda: any(env['name'] == 'review' and env['state'] == 'stopped' for env in snapshot()['environments']))
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
