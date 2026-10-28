"""Installer upgrades and concurrent automatic proxy refreshes with live app PID checks."""
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile

from live.common import BINARY, ROOT, SERVICE, Live, listening, wait_for

MANIFEST = f'''version: 1
project: update-demo
services:
  web:
    command: python3 {SERVICE}
    http: {{healthcheck: /ready}}
hooks:
  setup:
    command: echo setup >> "$PARCELA_DATA_DIR/setup-runs"
  prestart:
    command: echo prestart >> "$PARCELA_DATA_DIR/prestart-runs"
'''


def run(caddy, **_):
    with Live('updates', caddy) as live:
        source = live.checkout('app', MANIFEST)
        live.start_proxy()
        started = live.json(source, 'up', '--detach')
        live.ports.extend(endpoint['backend_port'] for endpoint in started['session']['endpoints'])
        before = live.json(source, 'status', '--current')
        process_pid = next(process['pid'] for process in before['processes'] if process['name'] == 'web')
        runtime = live.state / 'environments' / before['workspace']['id']
        ca = live.state / 'proxy/data/pki/authorities/local/root.crt'
        ca_digest = hashlib.sha256(ca.read_bytes()).hexdigest()
        record = live.state / 'proxy/router.json'

        def proxy():
            return live.json(live.root, 'proxy', 'status')

        def mark_old():
            saved = record.read_text()
            older = json.loads(saved)
            older['version'] = '0.0.1'
            older['protocol'] -= 1
            record.write_text(json.dumps(older))
            return saved

        def app_unchanged():
            wait_for(lambda: live.request('update-demo.localhost'))
            after = live.json(source, 'status', '--current')
            assert after['recorded_session']['generation'] == before['recorded_session']['generation']
            assert next(process['pid'] for process in after['processes'] if process['name'] == 'web') == process_pid
            assert (runtime / 'data/setup-runs').read_text() == 'setup\n'
            assert (runtime / 'data/prestart-runs').read_text() == 'prestart\n'
            assert hashlib.sha256(ca.read_bytes()).hexdigest() == ca_digest
            return after

        try:
            old_proxy = proxy()
            mark_old()
            clients = [subprocess.Popen(live.argv(live.root, 'status'), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) for _ in range(2)]
            replies = [client.communicate(timeout=45) for client in clients]
            assert all(client.returncode == 0 for client in clients), replies
            assert all('workspaces' in json.loads(stdout) for stdout, _ in replies), replies
            assert sum(stderr.count('updating the proxy from') for _, stderr in replies) == 1, replies
            refreshed = proxy()
            assert refreshed['pid'] != old_proxy['pid']
            assert refreshed['front_door'] == old_proxy['front_door']
            app_unchanged()

            # A newer proxy is never replaced by an older foreground CLI.
            saved = record.read_text()
            newer = json.loads(saved)
            for version, protocol in [('99.0.0', newer['protocol']), ('0.0.1', newer['protocol'] + 1)]:
                newer['version'] = version
                newer['protocol'] = protocol
                record.write_text(json.dumps(newer))
                live.json(live.root, 'status')
                assert proxy()['pid'] == refreshed['pid']
            record.write_text(saved)

            # Verify an actual installer round-trip, with downloads replaced by local archives.
            home = live.root / 'home'
            installed = home / '.local/bin/parcela'
            installed.parent.mkdir(parents=True)
            shutil.copy2(BINARY, installed)
            target = {('Darwin', 'arm64'): 'aarch64-apple-darwin', ('Darwin', 'x86_64'): 'x86_64-apple-darwin', ('Linux', 'x86_64'): 'x86_64-unknown-linux-gnu', ('Linux', 'aarch64'): 'aarch64-unknown-linux-gnu'}[(platform.system(), platform.machine())]
            archive = live.root / 'release.tar.gz'
            with tarfile.open(archive, 'w:gz') as bundle:
                bundle.add(BINARY, arcname=f'parcela-{target}/parcela')
            checksum = live.root / 'release.sha256'
            checksum.write_text(hashlib.sha256(archive.read_bytes()).hexdigest() + ' *archive\n')
            tools = live.root / 'tools'
            tools.mkdir()
            curl = tools / 'curl'
            curl.write_text(f'''#!{sys.executable}
import os, pathlib, shutil, sys
args = sys.argv[1:]
url = next(arg for arg in args if arg.startswith('https://'))
output = args[args.index('-o') + 1]
if url == 'https://parcela.run/install.sh': source = os.environ['TEST_INSTALLER']
elif url.endswith('.sha256'): source = os.environ['TEST_CHECKSUM']
else: source = os.environ['TEST_ARCHIVE']
shutil.copyfile(source, output)
''')
            curl.chmod(0o755)
            mark_old()
            environment = {**os.environ, 'HOME': str(home), 'PATH': f'{tools}:{os.environ["PATH"]}', 'PARCELA_SKIP_CADDY': '1', 'TEST_INSTALLER': str(ROOT / 'docs/website/install.sh'), 'TEST_ARCHIVE': str(archive), 'TEST_CHECKSUM': str(checksum), 'PARCELA_STATE_DIR': str(live.root / 'wrong-state')}
            upgraded = subprocess.run([str(installed), '--state-dir', str(live.state), 'upgrade'], env=environment, capture_output=True, text=True, timeout=45)
            assert upgraded.returncode == 0, upgraded.stdout + upgraded.stderr
            assert 'updating the proxy from' in upgraded.stderr
            assert proxy()['pid'] != refreshed['pid']
            assert not (live.root / 'wrong-state').exists()
            assert not (home / '.local/state/parcela').exists()
            app_unchanged()

            # Missing replacement prerequisites leave the working proxy alone.
            bad = tools / 'caddy'
            bad.write_text('#!/bin/sh\nexit 1\n')
            bad.chmod(0o755)
            saved = mark_old()
            current_pid = proxy()['pid']
            refused = subprocess.run(live.argv(live.root, 'status'), env={**os.environ, 'PATH': f'{tools}:{os.environ["PATH"]}'}, capture_output=True, text=True, timeout=10)
            assert 'automatic proxy refresh failed' in refused.stderr, refused.stderr
            assert proxy()['pid'] == current_pid
            record.write_text(saved)
            app_unchanged()

            live.cli(live.root, 'proxy', 'stop')
            live.json(live.root, 'status')
            assert not listening(live.https), 'inspection restarted a deliberately stopped proxy'
            assert listening(live.ports[0]), 'proxy maintenance stopped the application'
            return {'concurrent_refreshes': 'one replacement', 'installer_refresh': True, 'application_pid': process_pid, 'generation_and_hooks_unchanged': True, 'ca_and_frontdoor_unchanged': True, 'newer_proxy_preserved': True, 'failed_preflight_preserves_proxy': True, 'deliberate_stop_preserved': True}
        finally:
            live.cli(source, 'down', check=False)
            live.cli(live.root, 'proxy', 'stop', check=False)
