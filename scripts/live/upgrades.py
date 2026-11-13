"""Environments started by earlier releases keep running through an upgrade to this build.

Downloads the two latest releases older than this build (or the tags in PARCELA_UPGRADE_FROM),
starts environments and the proxy with each, then drives them with this build: the old owners must
answer it, the proxy must be replaced without touching them, and down, restart and up must work.
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import tarfile
import time
import urllib.request

from live.common import BINARY, SERVICE, Live, listening, wait_for

REPOSITORY = 'lostcoords/parcela'
MANIFEST = f'''version: 1
project: upgrade
services:
  web:
    command: python3 {SERVICE}
    http: {{healthcheck: /ready}}
hooks:
  setup:
    command: echo setup >> "$PARCELA_DATA_DIR/setup-runs"
'''


def version(text):
    return tuple(int(part) for part in re.findall(r'\d+', text)[:3])


def previous_releases(count=2):
    if os.environ.get('PARCELA_UPGRADE_FROM'):
        return os.environ['PARCELA_UPGRADE_FROM'].split()
    current = version(subprocess.run([str(BINARY), '--version'], capture_output=True, text=True, check=True).stdout)
    request = urllib.request.Request(f'https://api.github.com/repos/{REPOSITORY}/releases?per_page=30',
                                     headers={'Accept': 'application/vnd.github+json'})
    if os.environ.get('GITHUB_TOKEN'):
        request.add_header('Authorization', f'Bearer {os.environ["GITHUB_TOKEN"]}')
    with urllib.request.urlopen(request, timeout=30) as response:
        releases = json.load(response)
    tags = sorted((release['tag_name'] for release in releases
                   if not release['draft'] and not release['prerelease'] and re.fullmatch(r'v\d+\.\d+\.\d+', release['tag_name'])
                   and version(release['tag_name']) < current), key=version, reverse=True)
    return tags[:count]


def download(tag, directory):
    target = {('Darwin', 'arm64'): 'aarch64-apple-darwin', ('Darwin', 'x86_64'): 'x86_64-apple-darwin',
              ('Linux', 'x86_64'): 'x86_64-unknown-linux-gnu', ('Linux', 'aarch64'): 'aarch64-unknown-linux-gnu'}[(platform.system(), platform.machine())]
    url = f'https://github.com/{REPOSITORY}/releases/download/{tag}/parcela-{target}.tar.gz'
    with urllib.request.urlopen(url, timeout=60) as response:
        archive = response.read()
    with urllib.request.urlopen(f'{url}.sha256', timeout=30) as response:
        expected = response.read().decode().split()[0]
    assert hashlib.sha256(archive).hexdigest() == expected, f'{tag}: checksum mismatch'
    directory.mkdir(parents=True)
    bundle = directory / 'release.tar.gz'
    bundle.write_bytes(archive)
    with tarfile.open(bundle) as opened:
        member = opened.getmember(f'parcela-{target}/parcela')
        binary = directory / 'parcela'
        binary.write_bytes(opened.extractfile(member).read())
    binary.chmod(0o755)
    return binary


def run(caddy, **_):
    report = {}
    current_version = version(subprocess.run([str(BINARY), '--version'], capture_output=True, text=True, check=True).stdout)
    for tag in previous_releases():
        with Live(f'upgrade-{tag}', caddy) as live:
            old = download(tag, live.root / 'old')

            def old_cli(path, *argv, check=True):
                return subprocess.run([str(old), '--path', str(path), '--state-dir', str(live.state), *argv],
                                      capture_output=True, text=True, check=check, timeout=60)

            # The old release's proxy, as a person would have running before upgrading.
            log = open(live.root / 'old-proxy.log', 'w')
            live.logs.append(log)
            proxy = subprocess.Popen([str(old), '--path', str(live.root), '--state-dir', str(live.state),
                                      'proxy', 'run', '--caddy', caddy, '--port', str(live.https)], stdout=log, stderr=log)
            live.children.append(proxy)
            wait_for(lambda: old_cli(live.root, 'proxy', 'status', check=False).returncode == 0, [proxy])
            apps = {}
            for name in ('kept', 'cycled'):
                source = live.checkout(name, MANIFEST)
                started = json.loads(old_cli(source, '--json', 'up', '--detach', '--name', name).stdout)
                live.ports.extend(endpoint['backend_port'] for endpoint in started['session']['endpoints'])
                host = f'{name}-upgrade.localhost'
                wait_for(lambda: live.request(host))
                apps[name] = {'source': source, 'host': host, 'pid': live.request(host)['pid'],
                              'generation': started['session']['generation']}

            # This build reads the old state, talks to the old owners, and replaces the old proxy.
            began = time.monotonic()
            first = {row['workspace']['label']: row['state'] for row in live.json(live.root, 'ls')['workspaces']}
            gaps = []
            def all_answer():
                codes = [live.fetch(app['host'])[0] for app in apps.values()]
                if any(code != 200 for code in codes):
                    gaps.append(round(time.monotonic() - began, 2))
                return all(code == 200 for code in codes)
            wait_for(all_answer, timeout=15)
            def all_ready():
                return {row['workspace']['label']: row['state'] for row in live.json(live.root, 'ls')['workspaces']} == {'kept': 'ready', 'cycled': 'ready'}
            wait_for(all_ready, timeout=15)
            # Owners from earlier releases look for a new proxy every two seconds.
            assert not gaps or gaps[-1] < 4, (tag, first, gaps)
            for name, app in apps.items():
                status = live.json(app['source'], 'status', '--current')
                assert status['state'] == 'ready' and status['live_health_verified'], (tag, name, status)
                assert {process['name'] for process in status['processes']} >= {'web'}, status
            proxy_status = live.json(live.root, 'proxy', 'status')
            assert version(proxy_status['version']) == current_version and proxy_status['pid'] != proxy.pid, proxy_status
            for app in apps.values():
                wait_for(lambda: live.request(app['host']))
                assert live.request(app['host'])['pid'] == app['pid'], 'the upgrade restarted an app'
            listed = live.fetch(apps['kept']['host'], '/_parcela/environments')[2].decode()
            assert '"current":true' in listed, listed

            # An old owner stops on this build's `down`; restart and up start this build's owners.
            cycled = apps['cycled']
            restarted = live.json(cycled['source'], 'restart', '--detach')
            assert restarted['session']['generation'] > cycled['generation']
            wait_for(lambda: live.request(cycled['host'])['pid'] != cycled['pid'])
            assert live.json(cycled['source'], 'down')['state'] == 'stopped'
            assert live.json(cycled['source'], 'up', '--detach')['live']['state'] == 'ready'
            wait_for(lambda: live.request(cycled['host']))
            # This build's owners put their routes on a new proxy within a tick, not seconds later.
            live.cli(live.root, 'proxy', 'restart')
            began = time.monotonic()
            wait_for(lambda: live.fetch(cycled['host'])[0] == 200, timeout=10)
            assert time.monotonic() - began < 1, time.monotonic() - began
            assert live.json(cycled['source'], 'down')['state'] == 'stopped'
            kept = apps['kept']
            assert live.json(kept['source'], 'down')['state'] == 'stopped'
            assert not any(listening(port) for port in live.ports), 'every app process stopped'
            data = live.state / 'environments'
            assert all((runtime / 'data/setup-runs').read_text() == 'setup\n' for runtime in data.iterdir()
                       if (runtime / 'data/setup-runs').exists()), 'setup never ran again'
            live.cli(live.root, 'proxy', 'stop')
            report[tag] = {'old_owners_answered': 2, 'proxy_replaced': True, 'apps_kept_running': True,
                           'down_restart_up': True}
    assert report, 'no earlier release to upgrade from'
    return report
