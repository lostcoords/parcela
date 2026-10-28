"""Owner, proxy and process crash, stop and recovery races. Each case runs in its own fixture."""
import json
import os
from pathlib import Path
import signal
import shutil
import socket
import sqlite3
import ssl
import subprocess
import threading
import time
import traceback
import socketserver
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from http.client import HTTPResponse

from live.common import BINARY, Live, free_port, wait_for

class Server(ThreadingHTTPServer):
    def server_bind(self):
        # HTTPServer.server_bind does a reverse DNS lookup, which takes 35s on some CI runners.
        socketserver.TCPServer.server_bind(self)


SERVICE = '''import json, os, signal, socketserver, time
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
root = Path(os.environ['PARCELA_RUNTIME_DIR'])
def stop(*_):
    with (root / 'stop-signals').open('a') as log:
        log.write('TERM\\n')
    (root / 'shutdown-started').write_text('started')
    time.sleep(float(os.environ.get('STOP_DELAY', '0')))
    (root / 'shutdown-completed').write_text('completed')
    raise SystemExit(0)
signal.signal(signal.SIGTERM, stop)
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == '/stream':
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.end_headers()
            for tick in range(12):
                self.wfile.write(f'data: {tick}\\n\\n'.encode())
                self.wfile.flush()
                time.sleep(1)
            return
        data = json.dumps({'id': os.environ['PARCELA_ENVIRONMENT_ID'], 'pid': os.getpid(), 'generation': os.environ['PARCELA_GENERATION'], 'revision': os.environ.get('REVISION')}).encode()
        gated = self.path == '/ready' and os.environ.get('READINESS_GATED') == 'yes' and not (root / 'allow-ready').exists()
        self.send_response(503 if gated else 200)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)
    def log_message(self, *_): pass
class Server(ThreadingHTTPServer):
    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
with (root / 'service-starts').open('a') as log:
    log.write(str(os.getpid()) + '\\n')
server = Server(('127.0.0.1', int(os.environ['PORT'])), Handler)
(root / 'service-started').write_text(str(os.getpid()))
server.serve_forever()
'''

def socket_path(app, kind):
    return Path(json.loads((app['runtime'] / 'context.json').read_text())['sockets'][kind])

def socket_live(path):
    with socket.socket(socket.AF_UNIX) as probe:
        return probe.connect_ex(str(path)) == 0

def listener_pids(port):
    binary = '/usr/sbin/lsof' if Path('/usr/sbin/lsof').exists() else 'lsof'
    result = subprocess.run([binary, '-nP', f'-iTCP:{port}', '-sTCP:LISTEN', '-Fp'], capture_output=True, text=True, timeout=5)
    if result.returncode == 1 and not result.stdout:
        return []
    assert result.returncode == 0, result.stderr
    return [int(line[1:]) for line in result.stdout.splitlines() if line.startswith('p')]

def listening(port):
    return bool(listener_pids(port))

class Fixture(Live):
    def __init__(self, caddy):
        super().__init__('r', caddy)
        self.apps = []
    def cli(self, path, *args, check=False, timeout=25):
        return super().cli(path, *args, check=check, timeout=timeout)
    def app(self, label, delay=0):
        path = self.root / label
        path.mkdir()
        (path / 'service.py').write_text(SERVICE)
        manifest = {'version': 1, 'project': 'review', 'env': {'STOP_DELAY': str(delay)},
                    'services': {'web': {'command': f"python3 '{path / 'service.py'}'", 'http': {'healthcheck': '/ready'}}}}
        (path / 'parcela.yaml').write_text(json.dumps(manifest))
        identity = self.register(path, 'preview', label)['id']
        app = {'path': path, 'id': identity, 'host': f'{label}-review.localhost', 'runtime': self.state / 'environments' / identity,
               'port': free_port(), 'manifest': manifest}
        self.apps.append(app)
        self.ports.append(app['port'])
        return app
    def start(self, app):
        owner = self.spawn(app['path'], 'up', '--backend-port', str(app['port']), '--readiness-timeout', '10')
        app['owner'] = owner
        wait_for(lambda: self.probe(app)[0] == 200)
        return owner
    def start_pending(self, app):
        app['manifest']['env']['READINESS_GATED'] = 'yes'
        (app['path'] / 'parcela.yaml').write_text(json.dumps(app['manifest']))
        owner = self.spawn(app['path'], 'up', '--backend-port', str(app['port']), '--readiness-timeout', '30')
        app['owner'] = owner
        def recorded():
            pid = int((app['runtime'] / 'service-started').read_text())
            record = json.loads((app['runtime'] / 'owner.json').read_text())
            return record['phase'] == 'starting' and 'web' in record['children'] and any(process['pid'] == pid for process in record['processes'])
        wait_for(recorded)
        assert self.session(app)['status'] == 'starting'
        assert self.probe(app)[0] == 503
        return owner
    def gated_ps(self):
        """An environment whose `ps` pauses once while `gate-enabled` exists: recovery reads the
        process tree with it right after it takes the ownership lock."""
        tools = self.root / 'gate-tools'
        tools.mkdir(exist_ok=True)
        wrapper = tools / 'ps'
        wrapper.write_text(f'''#!/usr/bin/env python3
import os, sys, time
from pathlib import Path
root = Path({str(self.root)!r})
if '-A' in sys.argv and (root / 'gate-enabled').exists() and not (root / 'gate-entered').exists():
    (root / 'gate-entered').write_text('entered')
    time.sleep(.5)
os.execv({shutil.which('ps')!r}, ['ps', *sys.argv[1:]])
''')
        wrapper.chmod(0o700)
        return {**os.environ, 'PATH': str(tools) + os.pathsep + os.environ['PATH']}
    def stop_signals(self, app):
        path = app['runtime'] / 'stop-signals'
        return path.read_text().splitlines() if path.exists() else []
    def session(self, app):
        with sqlite3.connect(f'file:{self.state / "registry.sqlite3"}?mode=ro', uri=True) as db:
            db.row_factory = sqlite3.Row
            return dict(db.execute('SELECT * FROM sessions WHERE environment_id=?', (app['id'],)).fetchone())
    def probe(self, app):
        code, _, body = self.fetch(app['host'], timeout=2)
        return code, body.decode()
    def caddy_config(self):
        with socket.socket(socket.AF_UNIX) as stream:
            stream.settimeout(3)
            stream.connect(json.loads((self.state / 'proxy/caddy.json').read_text())['admin']['listen'].removeprefix('unix/'))
            stream.sendall(b'GET /config/ HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n')
            with HTTPResponse(stream) as response:
                response.begin()
                assert response.status == 200
                return json.loads(response.read())
    def kill_service(self, app):
        """Kills an app's service directly, as a crash would, without Parcela."""
        os.kill(int((app['runtime'] / 'service-started').read_text()), signal.SIGKILL)
        wait_for(lambda: not listening(app['port']))
    def close(self):
        for app in self.apps:
            if (app['runtime'] / 'context.json').exists():
                try: self.cli(app['path'], 'down')
                except (OSError, subprocess.TimeoutExpired): pass
        super().close()
        for app in self.apps:
            if (app['runtime'] / 'context.json').exists():
                path = socket_path(app, 'control')
                if path.is_socket() and not socket_live(path): path.unlink()
                try: path.parent.rmdir()
                except OSError: pass


def owner_crash(f):
    app = f.app('crash')
    f.start_proxy()
    owner = f.start(app)
    owner.kill()
    assert owner.wait(timeout=5) == -signal.SIGKILL
    status = json.loads(f.cli(app['path'], 'status').stdout)
    assert status['state'] == 'unknown'
    wait_for(lambda: f.probe(app)[0] == 503, timeout=10)
    down = f.cli(app['path'], 'down')
    assert down.returncode == 0, down.stderr
    assert not listening(app['port']), 'recovered stop must finish the actual graph'
    class Foreign(BaseHTTPRequestHandler):
        def do_GET(self):
            data = b'FOREIGN-BACKEND'
            self.send_response(200)
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        def log_message(self, *_): pass
    server = Server(('127.0.0.1', app['port']), Foreign)
    thread = threading.Thread(target=server.serve_forever)
    thread.start()
    try:
        code, body = f.probe(app)
        assert code == 503 and 'FOREIGN-BACKEND' not in body, 'stale hostname must not serve an unowned reused port'
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=3)
    return {'owner_crash_route': 503, 'recovered_down_exit': down.returncode, 'foreign_reused_port_route': code}

def graceful_stop(f):
    app = f.app('grace', delay=7)
    f.start_proxy()
    owner = f.start(app)
    down = f.cli(app['path'], 'down')
    assert down.returncode == 0, down.stderr
    assert f.session(app)['status'] == 'stopped'
    assert (app['runtime'] / 'shutdown-completed').exists(), 'stop success requires a completed service shutdown'
    assert not listener_pids(app['port']), 'stop success must not leave a supervised listener'
    assert not socket_path(app, 'control').exists(), 'normal shutdown must remove its control socket'
    assert owner.wait(timeout=5) == 0
    retry = f.cli(app['path'], 'up', '--detach', '--backend-port', str(app['port']), '--readiness-timeout', '10')
    assert retry.returncode == 0, retry.stderr
    return {'down_exit': down.returncode, 'shutdown_completed': True, 'subsequent_up_exit': retry.returncode}

def duplicate_stop(f):
    app = f.app('duplicate', delay=3)
    f.start_proxy()
    owner = f.start(app)
    command = [str(BINARY), '--path', str(app['path']), '--state-dir', str(f.state), 'down']
    first = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    wait_for(lambda: (app['runtime'] / 'shutdown-started').exists())
    second = f.cli(app['path'], 'down')
    out, err = first.communicate(timeout=15)
    assert first.returncode == 0, err
    assert second.returncode == 0, second.stderr
    assert f.session(app)['status'] == 'stopped'
    assert owner.wait(timeout=5) == 0
    return {'first_down_exit': first.returncode, 'duplicate_down_exit': second.returncode, 'final_state': 'stopped'}

def restart_preflight(f):
    app = f.app('preflight')
    f.start_proxy()
    owner = f.start(app)
    original = f.session(app)
    # An absent LAN address fails the replacement's preflight.
    restart = f.cli(app['path'], 'restart', '--detach', '--lan-ip', '192.0.2.1')
    assert restart.returncode != 0
    assert owner.poll() is None, 'failed replacement preflight must preserve the original owner'
    assert f.session(app)['token'] == original['token']
    assert f.probe(app)[0] == 200, 'failed preflight must preserve the healthy route'
    return {'restart_exit': restart.returncode, 'original_owner_alive': True, 'https_code': 200}

def proxy_outage(f):
    apps = [f.app('first'), f.app('second')]
    f.start_proxy()
    owners = [f.start(app) for app in apps]
    original = [f.session(app) for app in apps]
    f.proxy.terminate()
    f.proxy.wait(timeout=10)
    time.sleep(1)
    assert all(owner.poll() is None for owner in owners), 'front-door loss must not stop unrelated application graphs'
    assert all(listening(app['port']) for app in apps)
    for app, old in zip(apps, original):
        wait_for(lambda: f.probe(app)[0] == 200)
        assert f.session(app)['token'] == old['token']
        assert f.session(app)['generation'] == old['generation']
    assert f.cli(f.root, 'proxy', 'stop').returncode == 0, 'the owners started the proxy again in the background'
    time.sleep(3)
    assert not listening(f.https), 'a deliberately stopped proxy stays stopped'
    return {'application_owners_preserved': True, 'proxy_revived_by_owners': True, 'routes_recovered': 2,
            'application_generations_preserved': True, 'deliberate_stop_respected': True}

def lost_stop_reply(f):
    app = f.app('lost', delay=2)
    f.start_proxy()
    owner = f.start(app)
    session = f.session(app)
    with socket.socket(socket.AF_UNIX) as client:
        client.connect(str(socket_path(app, 'control')))
        client.sendall((json.dumps({'environment_id': app['id'], 'token': session['token'], 'generation': session['generation'], 'action': {'type': 'stop'}}) + '\n').encode())
    wait_for(lambda: f.session(app)['status'] in ['stopping', 'stopped'])
    retry = f.cli(app['path'], 'down')
    assert retry.returncode == 0, retry.stderr
    assert owner.wait(timeout=5) == 0
    return {'lost_reply_retry_exit': 0, 'shutdown_completed': (app['runtime'] / 'shutdown-completed').exists()}

def recover_surviving_graph(f):
    app = f.app('reattach')
    f.start_proxy()
    owner = f.start(app)
    before = f.session(app)
    service = json.loads(f.probe(app)[1])
    owner.kill()
    owner.wait(timeout=5)
    recovered = f.cli(app['path'], 'recover', '--detach')
    assert recovered.returncode == 0, recovered.stderr
    after = f.session(app)
    assert after['token'] == before['token'] and after['generation'] == before['generation']
    assert json.loads(f.probe(app)[1])['pid'] == service['pid'], 'recovery must not replay or replace service processes'
    return {'controller_recovered': True, 'service_pid_preserved': True, 'generation_preserved': True}

def up_recovers_surviving_graph(f):
    app = f.app('upreattach')
    f.start_proxy()
    owner = f.start(app)
    before = f.session(app)
    service = json.loads(f.probe(app)[1])
    owner.kill()
    owner.wait(timeout=5)
    recovered = f.cli(app['path'], 'up', '--detach', '--backend-port', str(app['port']))
    assert recovered.returncode == 0, recovered.stderr
    assert 'taking back control of its running services' in recovered.stderr, recovered.stderr
    result = json.loads(recovered.stdout)
    assert result['recovered'] is True and result['detached'] is True and result['live']['state'] == 'ready', result
    after = f.session(app)
    assert (after['token'], after['generation'], after['status']) == (before['token'], before['generation'], 'ready')
    assert json.loads(f.probe(app)[1])['pid'] == service['pid'], 'up must reattach, not replay the service'
    status = json.loads(f.cli(app['path'], 'status').stdout)
    assert status['state'] == 'ready' and [process['name'] for process in status['processes']] == ['web'], status
    assert status['processes'][0]['state'] == 'running' and status['processes'][0]['pid'], status
    duplicate = f.cli(app['path'], 'up', '--detach', '--backend-port', str(app['port']))
    assert duplicate.returncode == 0 and json.loads(duplicate.stdout)['already_running'], duplicate.stderr
    assert f.session(app)['token'] == before['token'] and json.loads(f.probe(app)[1])['pid'] == service['pid'], 'a live owner is never replaced'
    down = f.cli(app['path'], 'down')
    assert down.returncode == 0 and json.loads(down.stdout)['state'] == 'stopped', down.stderr
    assert not listening(app['port'])
    return {'up_detach_recovered': True, 'service_pid_preserved': True, 'generation_preserved': True, 'duplicate_up_reported': True}

def up_foreground_recovers_surviving_graph(f):
    app = f.app('upforeground')
    f.start_proxy()
    owner = f.start(app)
    before = f.session(app)
    service = json.loads(f.probe(app)[1])
    owner.kill()
    owner.wait(timeout=5)
    recovered = f.spawn(app['path'], 'up', '--backend-port', str(app['port']))
    def reattached():
        live = json.loads(f.cli(app['path'], 'status').stdout)['live']
        return live and live['owner_pid'] == recovered.pid and live['state'] == 'ready'
    wait_for(reattached)
    after = f.session(app)
    assert (after['token'], after['generation']) == (before['token'], before['generation'])
    assert json.loads(f.probe(app)[1])['pid'] == service['pid'], 'up must reattach, not replay the service'
    down = f.cli(app['path'], 'down')
    assert down.returncode == 0, down.stderr
    assert recovered.wait(timeout=20) == 0
    assert not listening(app['port'])
    return {'up_foreground_recovered': True, 'service_pid_preserved': True, 'foreground_exit': 0}

def guardian_death(f):
    app = f.app('guardian')
    f.start_proxy()
    owner = f.start(app)
    original = f.session(app)
    f.proxy.kill()
    f.proxy.wait(timeout=5)
    wait_for(lambda: f.probe(app)[0] == 503, timeout=10)
    assert owner.poll() is None and listening(app['port'])
    status = json.loads(f.cli(app['path'], 'status').stdout)
    assert status['state'] == 'unhealthy' and not status['live']['route_published']
    f.start_proxy()
    wait_for(lambda: f.probe(app)[0] == 200)
    assert f.session(app)['token'] == original['token']
    return {'lease_expired_without_guardian': True, 'existing_caddy_recovered': True, 'application_preserved': True}

def interrupted_stop(f):
    app = f.app('interrupted', delay=3)
    f.start_proxy()
    owner = f.start(app)
    session = f.session(app)
    with socket.socket(socket.AF_UNIX) as client:
        client.connect(str(socket_path(app, 'control')))
        client.sendall((json.dumps({'environment_id': app['id'], 'token': session['token'], 'generation': session['generation'], 'action': {'type': 'stop'}}) + '\n').encode())
    wait_for(lambda: (app['runtime'] / 'shutdown-started').exists())
    owner.kill()
    owner.wait(timeout=5)
    wait_for(lambda: (app['runtime'] / 'shutdown-completed').exists())
    wait_for(lambda: not listening(app['port']))
    retry = f.cli(app['path'], 'down')
    assert retry.returncode == 0, retry.stderr
    assert f.session(app)['status'] == 'stopped'
    assert f.probe(app)[0] == 503
    return {'interrupted_down_retry_exit': 0, 'shutdown_completed': True, 'route_withdrawn': True}


def ongoing_stop_retry(f):
    app = f.app('ongoing', delay=7)
    f.start_proxy()
    owner = f.start(app)
    session = f.session(app)
    with socket.socket(socket.AF_UNIX) as client:
        client.connect(str(socket_path(app, 'control')))
        client.sendall((json.dumps({'environment_id': app['id'], 'token': session['token'], 'generation': session['generation'], 'action': {'type': 'stop'}}) + '\n').encode())
    wait_for(lambda: (app['runtime'] / 'shutdown-started').exists())
    owner.kill()
    owner.wait(timeout=5)
    assert not (app['runtime'] / 'shutdown-completed').exists(), 'retry must start while the service is still stopping'
    assert listening(app['port']), 'fixture graph must still be running at retry'
    retry = f.cli(app['path'], 'down')
    assert retry.returncode == 0, retry.stderr
    assert (app['runtime'] / 'shutdown-completed').exists(), 'successful retry must await the service shutdown'
    assert not listening(app['port'])
    assert f.session(app)['status'] == 'stopped'
    assert f.session(app)['token'] == session['token']
    assert f.probe(app)[0] == 503
    return {'retry_during_shutdown_exit': 0, 'shutdown_completed': True, 'generation_preserved': True}


def concurrent_crash_stop(f):
    app = f.app('election', delay=2)
    gated = f.gated_ps()
    f.start_proxy()
    owner = f.start(app)
    session = f.session(app)
    owner.kill()
    owner.wait(timeout=5)
    (f.root / 'gate-enabled').write_text('enabled')
    first = f.spawn(app['path'], 'down', env=gated)
    wait_for(lambda: (f.root / 'gate-entered').exists())
    second = f.spawn(app['path'], 'down', env=gated)
    first_code = first.wait(timeout=20)
    second_code = second.wait(timeout=20)
    first_log = (f.root / f'child-{f.children.index(first)}.log').read_text()
    second_log = (f.root / f'child-{f.children.index(second)}.log').read_text()
    assert first_code == 0, first_log
    assert second_code == 0, second_log
    assert f.session(app)['status'] == 'stopped'
    assert f.session(app)['token'] == session['token']
    assert (app['runtime'] / 'shutdown-completed').exists()
    assert not listening(app['port'])
    assert f.stop_signals(app) == ['TERM'], 'only the elected controller may stop the service'
    return {'first_recovery_exit': first_code, 'contending_recovery_exit': second_code, 'same_generation_stopped': True, 'stop_signals': 1}


def recover_before_readiness(f):
    app = f.app('pending')
    f.start_proxy()
    owner = f.start_pending(app)
    session = f.session(app)
    pid = int((app['runtime'] / 'service-started').read_text())
    owner.kill()
    owner.wait(timeout=5)
    recovered = f.spawn(app['path'], 'recover')
    def starting_owner():
        status = f.cli(app['path'], 'status')
        live = json.loads(status.stdout).get('live')
        return live and live['owner_pid'] == recovered.pid and live['state'] == 'starting'
    wait_for(starting_owner)
    assert f.probe(app)[0] == 503, 'recovery must not advertise an unready service'
    (app['runtime'] / 'allow-ready').write_text('ready')
    wait_for(lambda: f.probe(app)[0] == 200)
    assert json.loads(f.probe(app)[1])['pid'] == pid, 'recovery must not replay the service'
    assert f.session(app)['token'] == session['token']
    return {'pre_readiness_controller_recovered': True, 'unready_route': 503, 'service_pid_preserved': True}


def stop_before_readiness(f):
    app = f.app('pending-stop')
    f.start_proxy()
    owner = f.start_pending(app)
    session = f.session(app)
    owner.kill()
    owner.wait(timeout=5)
    down = f.cli(app['path'], 'down')
    assert down.returncode == 0, down.stderr
    assert f.session(app)['status'] == 'stopped'
    assert f.session(app)['token'] == session['token']
    assert (app['runtime'] / 'shutdown-completed').exists()
    assert not listening(app['port'])
    assert f.probe(app)[0] == 503
    return {'pre_readiness_stop_exit': 0, 'shutdown_completed': True, 'generation_preserved': True}


def allocation_without_launch_evidence(f):
    """A run whose owner died after leasing ports and before saving its record, which it does
    before starting any process: no record and nothing running."""
    app = f.app('allocation')
    marker = app['path'] / 'owned-source-data'
    marker.write_text('preserve allocation fixture')
    f.start_proxy()
    owner = f.start_pending(app)
    session = f.session(app)
    owner.kill()
    owner.wait(timeout=5)
    f.kill_service(app)
    (app['runtime'] / 'owner.json').unlink()
    assert f.session(app)['status'] == 'starting'
    down = f.cli(app['path'], 'down')
    assert down.returncode == 0, down.stderr
    assert f.session(app)['token'] == session['token'] and f.session(app)['status'] == 'stopped'
    assert marker.read_text() == 'preserve allocation fixture'
    (app['runtime'] / 'allow-ready').write_text('ready')
    retry = f.cli(app['path'], 'up', '--detach', '--backend-port', str(app['port']))
    assert retry.returncode == 0, retry.stderr
    assert f.probe(app)[0] == 200
    assert f.cli(app['path'], 'down').returncode == 0
    return {'unrecorded_allocation_released': True, 'retry_exit': 0, 'source_data_preserved': True}


def recover_racing_stop(f):
    app = f.app('mixed')
    gated = f.gated_ps()
    f.start_proxy()
    owner = f.start(app)
    session = f.session(app)
    pid = (app['runtime'] / 'service-started').read_text()
    owner.kill()
    owner.wait(timeout=5)
    (f.root / 'gate-enabled').write_text('enabled')
    recovered = f.spawn(app['path'], 'recover', env=gated)
    wait_for(lambda: (f.root / 'gate-entered').exists())
    stopper = f.spawn(app['path'], 'down')
    try:
        stop_code = stopper.wait(timeout=15)
        stop_log = (f.root / f'child-{f.children.index(stopper)}.log').read_text()
        assert stop_code == 0, stop_log
        assert recovered.wait(timeout=5) == 0
        assert f.session(app)['status'] == 'stopped'
        assert f.session(app)['token'] == session['token']
        assert (app['runtime'] / 'service-started').read_text() == pid, 'recovery must not replay the service'
        assert f.stop_signals(app) == ['TERM']
        assert not listening(app['port'])
    finally:
        (f.root / 'gate-enabled').unlink()
    return {'mixed_stop_exit': 0, 'recovery_controller_exit': 0, 'stop_signals': 1, 'no_service_replay': True}


def completed_stop_receipt(f):
    app = f.app('receipt')
    f.start_proxy()
    owner = f.start(app)
    down = f.cli(app['path'], 'down')
    assert down.returncode == 0, down.stderr
    assert owner.wait(timeout=5) == 0
    record = json.loads((app['runtime'] / 'owner.json').read_text())
    assert record['phase'] == 'stopped'
    with sqlite3.connect(f.state / 'registry.sqlite3') as db:
        db.execute('UPDATE sessions SET status=? WHERE environment_id=?', ('stopping', app['id']))
    retry = f.cli(app['path'], 'down')
    assert retry.returncode == 0, retry.stderr
    assert f.session(app)['status'] == 'stopped'
    assert not listening(app['port'])
    return {'completed_receipt_reconciled': True, 'retry_exit': 0}


def replaced_process_birth(f):
    """A recorded identity that no longer matches its pid is not this run's process: recovery
    never signals it, and the run counts as gone."""
    app = f.app('birth')
    f.start_proxy()
    owner = f.start(app)
    owner.kill()
    owner.wait(timeout=5)
    path = app['runtime'] / 'owner.json'
    record = json.loads(path.read_text())
    record['children']['web']['identity']['birth'] += 1
    for process in record['processes']:
        process['birth'] += 1
    path.write_text(json.dumps(record))
    down = f.cli(app['path'], 'down')
    assert down.returncode == 0, down.stderr
    assert listening(app['port']), 'a mismatched process identity must not be signaled'
    assert f.stop_signals(app) == []
    wait_for(lambda: f.probe(app)[0] == 503)
    f.kill_service(app)
    return {'mismatched_birth_not_signaled': True, 'route_withdrawn': True}


def long_stream_lease(f):
    app = f.app('stream')
    f.start_proxy()
    owner = f.start(app)
    loaded = f.caddy_config()
    context = ssl.create_default_context(cafile=str(f.state / 'proxy/data/pki/authorities/local/root.crt'))
    with context.wrap_socket(socket.create_connection(('127.0.0.1', f.https), timeout=3), server_hostname=app['host']) as stream:
        stream.settimeout(3)
        stream.sendall(f'GET /stream HTTP/1.1\r\nHost: {app["host"]}\r\nConnection: close\r\n\r\n'.encode())
        with stream.makefile('rb') as response:
            assert response.readline().startswith(b'HTTP/1.1 200')
            while True:
                header = response.readline()
                assert header, 'stream closed before response headers'
                if header == b'\r\n':
                    break
            for tick in range(12):
                line = response.readline()
                while not line.startswith(b'data: '):
                    assert line, f'stream closed before event {tick}'
                    line = response.readline()
                assert line.strip() == f'data: {tick}'.encode()
                assert f.probe(app)[0] == 200, 'lease renewal must keep new requests routed'
    assert owner.poll() is None
    assert f.caddy_config() == loaded, 'lease renewal must not change actual Caddy config'
    return {'stream_events': 12, 'lease_windows_crossed': True, 'config_unchanged': True}


def occupied_replacement_port(f):
    app = f.app('occupied')
    f.start_proxy()
    owner = f.start(app)
    original = f.session(app)
    with socket.socket() as foreign:
        foreign.bind(('127.0.0.1', 0))
        foreign.listen()
        port = foreign.getsockname()[1]
        restart = f.cli(app['path'], 'restart', '--detach', '--backend-port', str(port))
        assert restart.returncode != 0
        assert owner.poll() is None, 'occupied replacement preflight must preserve original owner'
        assert f.session(app)['token'] == original['token']
        assert f.session(app)['generation'] == original['generation']
        assert f.probe(app)[0] == 200, 'occupied replacement must preserve original HTTPS'
        with socket.create_connection(('127.0.0.1', port), timeout=2):
            pass
    return {'restart_exit': restart.returncode, 'original_owner_alive': True, 'https_code': 200, 'foreign_listener_preserved': True}

def invalid_replacement_port(f):
    app = f.app('invalid')
    f.start_proxy()
    owner = f.start(app)
    original = f.session(app)
    for port in [0, 80, f.https]:
        restart = f.cli(app['path'], 'restart', '--detach', '--backend-port', str(port))
        assert restart.returncode != 0, f'invalid replacement port {port} must be refused'
        assert owner.poll() is None, f'invalid replacement port {port} stopped original owner'
        assert f.session(app)['token'] == original['token']
        assert f.probe(app)[0] == 200
    return {'invalid_endpoints_refused': 3, 'original_owner_alive': True, 'https_code': 200}

def restart_owned_port(f):
    app = f.app('reuse')
    f.start_proxy()
    owner = f.start(app)
    original = f.session(app)
    restart = f.cli(app['path'], 'restart', '--detach', '--backend-port', str(app['port']))
    assert restart.returncode == 0, restart.stderr
    assert owner.wait(timeout=5) == 0
    current = f.session(app)
    assert current['token'] != original['token']
    assert current['generation'] == original['generation'] + 1
    assert current['backend_port'] == app['port']
    assert f.probe(app)[0] == 200
    return {'restart_exit': 0, 'owned_endpoint_reused': True, 'new_generation': current['generation'], 'https_code': 200}

def leased_replacement_port(f):
    app = f.app('leased')
    other = f.app('reserved')
    f.start_proxy()
    owner = f.start(app)
    other_owner = f.start(other)
    original = f.session(app)
    other_owner.kill()
    other_owner.wait(timeout=5)
    f.kill_service(other)
    assert f.session(other)['status'] != 'stopped'
    restart = f.cli(app['path'], 'restart', '--detach', '--backend-port', str(other['port']))
    assert restart.returncode != 0
    assert owner.poll() is None, 'reserved replacement must preserve original owner'
    assert f.session(app)['token'] == original['token']
    assert f.probe(app)[0] == 200
    return {'reserved_unbound_endpoint_refused': True, 'original_owner_alive': True, 'https_code': 200}

def detached_recover_running(f):
    app = f.app('rundetach')
    f.start_proxy()
    owner = f.start(app)
    original = f.session(app)
    before = json.loads(f.probe(app)[1])
    owner.kill()
    owner.wait(timeout=5)
    recover = f.cli(app['path'], 'recover', '--detach', timeout=25)
    assert recover.returncode == 0, recover.stderr
    result = json.loads(recover.stdout)
    assert result['live']['state'] == 'ready' and result['detached'] is True
    current = f.session(app)
    assert current['token'] == original['token'] and current['generation'] == original['generation']
    assert json.loads(f.probe(app)[1])['pid'] == before['pid'], 'detached recovery must not replay services'
    assert f.cli(app['path'], 'down').returncode == 0
    assert not listening(app['port'])
    return {'recover_exit': 0, 'state': 'ready', 'service_pid_preserved': True, 'same_generation': True}

def recovered_owner_initialization(f):
    app = f.app('initialize')
    f.start_proxy()
    owner = f.start(app)
    original = f.session(app)
    owner.kill()
    owner.wait(timeout=5)
    tools = f.root / 'tools'
    tools.mkdir()
    ps = tools / 'ps'
    ps.write_text(f"""#!/usr/bin/env python3
import json, os, sys, time
from pathlib import Path
root = Path({str(f.root)!r})
record_path = Path({str(app['runtime'] / 'owner.json')!r})
if '-A' in sys.argv and not (root / 'refresh-paused').exists():
    record = json.loads(record_path.read_text())
    if record['phase'] == 'starting' and record['owner']['pid'] != {owner.pid}:
        (root / 'refresh-paused').write_text('paused before recovered control admission')
        time.sleep(1)
os.execv({shutil.which('ps')!r}, ['ps', *sys.argv[1:]])
""")
    ps.chmod(0o700)
    recovery = f.spawn(app['path'], 'recover', '--detach', env={**os.environ, 'PATH': str(tools) + os.pathsep + os.environ['PATH']})
    wait_for(lambda: (f.root / 'refresh-paused').exists())
    record = json.loads((app['runtime'] / 'owner.json').read_text())
    assert record['phase'] == 'starting'
    current = f.session(app)
    assert current['status'] == 'starting', 'new recovered owner must not inherit former ready intent before initialization'
    assert current['token'] == original['token'] and current['generation'] == original['generation']
    assert recovery.wait(timeout=20) == 0
    assert f.probe(app)[0] == 200
    return {'initializing_state': 'starting', 'same_generation': True, 'https_code': 200}

def detached_recover_stopping(f):
    return detached_stopped_recovery(f, False)

def detached_recover_quiescent(f):
    return detached_stopped_recovery(f, True)

def detached_stopped_recovery(f, finish_first):
    app = f.app('stopdetach', delay=7)
    f.start_proxy()
    owner = f.start(app)
    original = f.session(app)
    with socket.socket(socket.AF_UNIX) as control:
        control.settimeout(5)
        control.connect(str(socket_path(app, 'control')))
        control.sendall((json.dumps({'environment_id': app['id'], 'generation': original['generation'], 'token': original['token'], 'action': {'type': 'stop'}}) + '\n').encode())
        with control.makefile('rb') as response:
            reply = json.loads(response.readline())
        assert reply['result']['stop_accepted'] is True
    wait_for(lambda: (app['runtime'] / 'shutdown-started').exists())
    owner.kill()
    owner.wait(timeout=5)
    if finish_first:
        wait_for(lambda: (app['runtime'] / 'shutdown-completed').exists())
        wait_for(lambda: not listening(app['port']))
    recover = f.cli(app['path'], 'recover', '--detach', timeout=25)
    assert recover.returncode == 0, recover.stderr
    result = json.loads(recover.stdout)
    assert result['state'] == 'stopped' and result['recovered'] is True and result['detached'] is True
    current = f.session(app)
    assert current['token'] == original['token'] and current['generation'] == original['generation']
    assert current['status'] == 'stopped'
    assert (app['runtime'] / 'shutdown-completed').exists()
    assert not listening(app['port'])
    return {'recover_exit': 0, 'state': 'stopped', 'same_generation': True, 'finished_before_recover': finish_first}


def crash_whole_tree(f, app):
    f.start_proxy()
    owner = f.start(app)
    before = f.session(app)
    service = int((app['runtime'] / 'service-started').read_text())
    wait_for(lambda: any(process['pid'] == service for process in json.loads((app['runtime'] / 'owner.json').read_text())['processes']))
    record = json.loads((app['runtime'] / 'owner.json').read_text())
    owner.kill()
    owner.wait(timeout=5)
    for process in record['processes']:
        try: os.kill(process['pid'], signal.SIGKILL)
        except ProcessLookupError: pass
    wait_for(lambda: not listening(app['port']))
    assert socket_path(app, 'control').exists(), 'a crash leaves a stale control socket'
    assert f.session(app)['status'] == 'ready'
    return before

def crashed_tree_down(f):
    app = f.app('crashdown')
    before = crash_whole_tree(f, app)
    first = f.cli(app['path'], 'down')
    assert first.returncode == 0, first.stderr
    assert json.loads(first.stdout)['state'] == 'stopped'
    second = f.cli(app['path'], 'down')
    assert second.returncode == 0 and json.loads(second.stdout)['already_stopped'] is True, second.stderr
    assert not socket_path(app, 'control').exists()
    assert f.probe(app)[0] == 503
    retry = f.cli(app['path'], 'up', '--detach', '--backend-port', str(app['port']), '--readiness-timeout', '10')
    assert retry.returncode == 0, retry.stderr
    after = f.session(app)
    assert after['generation'] == before['generation'] + 1 and after['status'] == 'ready'
    assert f.probe(app)[0] == 200
    assert f.cli(app['path'], 'down').returncode == 0
    return {'first_down': 'stopped', 'second_down': 'already_stopped', 'next_generation': after['generation'], 'https_code': 200}

def rebooted_device_down(f):
    """Simulate persisted pre-reboot evidence; no host reboot or mount changes."""
    app = f.app('rebootdown')
    app['manifest']['services']['web']['stop'] = 'echo stopped >> "$PARCELA_RUNTIME_DIR/stop-command-runs"'
    (app['path'] / 'parcela.yaml').write_text(json.dumps(app['manifest']))
    before = crash_whole_tree(f, app)
    path = app['runtime'] / 'owner.json'
    record = json.loads(path.read_text())
    identities = [record['owner'], *record['processes'],
                  *(child['identity'] for child in record['children'].values() if child['identity'])]
    for identity in identities:
        identity['boot'] = '00000000-0000-4000-8000-000000000001'
    record['lock']['device'] += 1
    path.write_text(json.dumps(record))
    socket_path(app, 'control').unlink()
    sentinel = app['runtime'] / 'data' / 'keep.txt'
    sentinel.parent.mkdir(exist_ok=True)
    sentinel.write_text('keep environment data')
    down = f.cli(app['path'], 'down')
    assert down.returncode == 0, down.stderr
    assert f.session(app)['status'] == 'stopped'
    again = f.cli(app['path'], 'down')
    assert again.returncode == 0 and json.loads(again.stdout)['already_stopped'] is True, again.stderr
    assert (app['runtime'] / 'stop-command-runs').read_text().splitlines() == ['stopped']
    retry = f.cli(app['path'], 'up', '--detach', '--backend-port', str(app['port']), '--readiness-timeout', '10')
    assert retry.returncode == 0, retry.stderr
    after = f.session(app)
    assert after['generation'] == before['generation'] + 1 and after['status'] == 'ready'
    assert f.probe(app)[0] == 200
    assert sentinel.read_text() == 'keep environment data'
    assert f.cli(app['path'], 'down').returncode == 0
    assert not listening(app['port'])
    return {'simulated_previous_boot': True, 'changed_device_same_inode': True, 'repeated_down': 'already_stopped',
            'owed_stop_command_ran_once': True, 'data_preserved': True, 'new_generation_https': 200}


def crashed_tree_up(f):
    app = f.app('crashup')
    before = crash_whole_tree(f, app)
    retry = f.cli(app['path'], 'up', '--detach', '--backend-port', str(app['port']), '--readiness-timeout', '10')
    assert retry.returncode == 0, retry.stderr
    after = f.session(app)
    assert after['generation'] == before['generation'] + 1 and after['status'] == 'ready'
    assert f.probe(app)[0] == 200
    assert f.cli(app['path'], 'down').returncode == 0
    return {'up_after_crash_exit': 0, 'next_generation': after['generation'], 'https_code': 200}

DAEMON = """import os, sys, time
from pathlib import Path
pidfile = Path(os.environ['PARCELA_RUNTIME_DIR']) / 'daemon.pid'
if os.fork() == 0:
    os.setsid()
    if os.fork() == 0:
        pidfile.write_text(str(os.getpid()))
        while True: time.sleep(1)
    os._exit(0)
os.wait()
os.execvp('sleep', ['sleep', '100000'])
"""

def stop_command_after_crash(f):
    """A service that leaves a daemon outside its process tree, like `docker compose up` leaves
    containers: its `stop` command must still run after the owner and the tree crashed, and a
    failed one must leave the run interrupted until `down` succeeds."""
    app = f.app('stopcmd')
    (app['path'] / 'daemon.py').write_text(DAEMON)
    pidfile = app['runtime'] / 'daemon.pid'
    gate = app['runtime'] / 'stop-may-succeed'
    app['manifest']['services']['daemon'] = {
        'command': f"exec python3 '{app['path'] / 'daemon.py'}'",
        'healthcheck': 'test -f "$PARCELA_RUNTIME_DIR/daemon.pid"',
        'stop': 'test -f "$PARCELA_RUNTIME_DIR/stop-may-succeed" || exit 7; '
                'test ! -f "$PARCELA_RUNTIME_DIR/daemon.pid" || { kill "$(cat "$PARCELA_RUNTIME_DIR/daemon.pid")"; rm "$PARCELA_RUNTIME_DIR/daemon.pid"; }',
    }
    app['manifest']['services']['web']['depends_on'] = ['daemon']
    (app['path'] / 'parcela.yaml').write_text(json.dumps(app['manifest']))
    crash_whole_tree(f, app)
    daemon = int(pidfile.read_text())
    os.kill(daemon, 0)
    failed = f.cli(app['path'], 'down')
    assert failed.returncode != 0 and 'services.daemon.stop failed with exit code 7' in failed.stderr, failed.stderr
    assert f.session(app)['status'] == 'interrupted'
    os.kill(daemon, 0)
    gate.write_text('yes')
    down = f.cli(app['path'], 'down')
    assert down.returncode == 0, down.stderr
    wait_for(lambda: _dead(daemon))
    assert f.session(app)['status'] == 'stopped'
    again = f.cli(app['path'], 'down')
    assert again.returncode == 0 and json.loads(again.stdout)['already_stopped'] is True, again.stderr
    retry = f.cli(app['path'], 'up', '--detach', '--backend-port', str(app['port']), '--readiness-timeout', '10')
    assert retry.returncode == 0, retry.stderr
    second = int(pidfile.read_text())
    graceful = f.cli(app['path'], 'down')
    assert graceful.returncode == 0, graceful.stderr
    assert not pidfile.exists()
    wait_for(lambda: _dead(second))
    return {'failed_stop_exit': failed.returncode, 'retried_down_exit': down.returncode, 'graceful_down_exit': graceful.returncode}

def concurrent_up_same_checkout(f):
    app = f.app('same-checkout')
    f.start_proxy()
    owner = f.start_pending(app)
    starting = f.session(app)
    contender = f.cli(app['path'], 'up', '--detach', '--backend-port', str(app['port']))
    assert contender.returncode == 1, contender.stderr
    assert contender.stderr.strip() in {
        'parcela: this environment is already running or was interrupted; see `parcela status`',
        'parcela: backend endpoint is not owned by the current routable generation',
    }, contender.stderr
    assert owner.poll() is None
    (app['runtime'] / 'allow-ready').touch()
    wait_for(lambda: f.probe(app)[0] == 200)
    current = f.session(app)
    assert current['token'] == starting['token'] and current['generation'] == starting['generation']
    assert (app['runtime'] / 'service-starts').read_text().splitlines() == [str(json.loads(f.probe(app)[1])['pid'])]
    again = f.cli(app['path'], 'up', '--detach')
    assert again.returncode == 0, again.stderr
    assert f.session(app)['token'] == current['token']
    return {'contender_exit': contender.returncode, 'contender_error': contender.stderr.strip(),
            'contender_refused_while_starting': True, 'one_service_launch': True, 'repeated_up_keeps_generation': True}


def detached_configuration_changes(f):
    app = f.app('configuration')
    other = f.app('unchanged')
    f.start_proxy()
    f.start(other)
    other_session = f.session(other)
    app['manifest']['env'].update(REVISION='before', READINESS_GATED='yes')
    (app['path'] / 'parcela.yaml').write_text(json.dumps(app['manifest']))
    detached = f.spawn(app['path'], 'up', '--detach', '--backend-port', str(app['port']), '--readiness-timeout', '30')
    wait_for(lambda: (app['runtime'] / 'service-started').exists())
    app['manifest']['env']['REVISION'] = 'after'
    app['manifest']['services']['worker'] = {'command': 'python3 -c "import time; time.sleep(600)"'}
    (app['path'] / 'parcela.yaml').write_text(json.dumps(app['manifest']))
    (app['runtime'] / 'allow-ready').touch()
    assert detached.wait(timeout=40) == 0
    wait_for(lambda: f.probe(app)[0] == 200)
    before = json.loads(f.probe(app)[1])
    assert before['revision'] == 'before', before
    assert 'worker' not in {item['name'] for item in json.loads(f.cli(app['path'], 'status').stdout)['processes']}
    restarted = f.cli(app['path'], 'restart', '--detach', timeout=40)
    assert restarted.returncode == 0, restarted.stderr
    after = json.loads(f.probe(app)[1])
    assert after['revision'] == 'after' and int(after['generation']) > int(before['generation']), after
    assert _dead(before['pid'])
    workers = json.loads(f.cli(app['path'], 'status').stdout)['processes']
    worker_pid = next(item['pid'] for item in workers if item['name'] == 'worker')
    assert worker_pid and not _dead(worker_pid)
    del app['manifest']['services']['worker']
    (app['path'] / 'parcela.yaml').write_text(json.dumps(app['manifest']))
    restarted = f.cli(app['path'], 'restart', '--detach', timeout=40)
    assert restarted.returncode == 0, restarted.stderr
    assert 'worker' not in {item['name'] for item in json.loads(f.cli(app['path'], 'status').stdout)['processes']}
    wait_for(lambda: _dead(worker_pid))
    assert f.session(other)['token'] == other_session['token'] and f.probe(other)[0] == 200
    return {'pending_start_keeps_original_configuration': True, 'restart_reads_changed_environment': True,
            'restart_adds_and_removes_services': True, 'removed_worker_stopped': True,
            'old_process_stopped': True, 'other_environment_unchanged': True}


def concurrent_restart(f):
    app = f.app('restart-race', delay=2)
    f.start_proxy()
    f.start(app)
    before = f.session(app)
    first = f.spawn(app['path'], 'restart', '--detach')
    wait_for(lambda: (app['runtime'] / 'shutdown-started').exists())
    assert first.poll() is None
    second = f.spawn(app['path'], 'restart', '--detach')
    codes = [process.wait(timeout=40) for process in (first, second)]
    assert codes == [0, 0], codes
    wait_for(lambda: f.probe(app)[0] == 200)
    current = f.session(app)
    assert current['generation'] == before['generation'] + 2
    response = json.loads(f.probe(app)[1])
    assert response['generation'] == str(current['generation'])
    launched = [int(pid) for pid in (app['runtime'] / 'service-starts').read_text().splitlines()]
    assert len(launched) == 3, launched
    wait_for(lambda: all(_dead(pid) for pid in launched if pid != response['pid']))
    status = json.loads(f.cli(app['path'], 'status').stdout)
    assert status['state'] == 'ready' and status['live']['listener_owned'], status
    assert f.cli(app['path'], 'down').returncode == 0
    assert not listening(app['port'])
    return {'restart_exit_codes': codes, 'second_started_during_first_stop': True,
            'only_current_service_survives': True, 'current_generation_ready': True, 'owned_listener_closed': True}


def worker_exit_after_readiness(f):
    app = f.app('worker-exit')
    app['manifest']['services']['jobs'] = {
        'command': 'while [ ! -f "$PARCELA_RUNTIME_DIR/exit-worker" ]; do sleep 0.05; done; exit 7'
    }
    (app['path'] / 'parcela.yaml').write_text(json.dumps(app['manifest']))
    f.start_proxy()
    owner = f.start(app)
    (app['runtime'] / 'exit-worker').touch()
    def worker_exited():
        status = json.loads(f.cli(app['path'], 'status').stdout)
        return any(process['name'] == 'jobs' and process['state'] == 'exited' for process in status['processes'])
    wait_for(worker_exited)
    assert json.loads(f.cli(app['path'], 'status').stdout)['state'] == 'ready'
    assert owner.poll() is None and f.probe(app)[0] == 200
    return {'worker_exit_reported': True, 'http_environment_stays_ready': True, 'https_code': 200}


def _dead(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return True
    return False

CASES = [owner_crash, graceful_stop, duplicate_stop, restart_preflight, proxy_outage, lost_stop_reply,
         recover_surviving_graph, up_recovers_surviving_graph, up_foreground_recovers_surviving_graph, guardian_death, interrupted_stop, completed_stop_receipt,
         replaced_process_birth, long_stream_lease, ongoing_stop_retry, concurrent_crash_stop,
         recover_before_readiness, stop_before_readiness, allocation_without_launch_evidence, recover_racing_stop,
         occupied_replacement_port, invalid_replacement_port, restart_owned_port, leased_replacement_port, detached_recover_running,
         detached_recover_stopping, detached_recover_quiescent, recovered_owner_initialization,
         crashed_tree_down, crashed_tree_up, rebooted_device_down, stop_command_after_crash,
         concurrent_up_same_checkout, detached_configuration_changes, concurrent_restart, worker_exit_after_readiness]


def run(caddy, cases=None):
    os.umask(0o077)
    selected = [case for case in CASES if not cases or case.__name__ in cases]
    assert selected, f'no recovery case matches {cases}'
    records = []
    for case in selected:
        record = {'case': case.__name__}
        try:
            with Fixture(caddy) as fixture:
                record['fixture'] = str(fixture.root)
                record['evidence'] = case(fixture)
        except Exception:
            record['error'] = traceback.format_exc()
        records.append(record)
        detail = f" ({record['error'].strip().splitlines()[-1]})" if 'error' in record else ''
        print(f"  recovery {case.__name__}: {'FAIL' if 'error' in record else 'ok'}{detail}", flush=True)
    failed = [record['case'] for record in records if 'error' in record]
    assert not failed, {'failed': failed, 'cases': records}
    return {'cases': records}
