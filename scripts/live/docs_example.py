"""The Getting started app from examples/docs-demo in a main checkout and a Git worktree, then cleanup."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

from live.common import ROOT, Live, wait_for

SOURCE = ROOT / 'examples/docs-demo'
INPUTS = ['parcela.yaml', 'index.html', 'api.py', 'db.py', 'worker.py']


def digests():
    return {name: hashlib.sha256((SOURCE / name).read_bytes()).hexdigest() for name in INPUTS}


def run(caddy, **_):
    before = digests()
    with Live('docs', caddy) as live:
        primary, signup = live.checkout('hello'), live.root / 'signup'
        live.sources.append(signup)

        def git(*argv):
            subprocess.run(['git', '-C', str(primary), '-c', 'user.name=docs', '-c', 'user.email=docs@example.invalid', *argv],
                           check=True, capture_output=True, timeout=30)

        def remember(result):
            session = result['session']
            assert {endpoint['service'] for endpoint in session['endpoints']} == {'web', 'api'}
            live.ports.extend(endpoint['backend_port'] for endpoint in session['endpoints'])
            return session

        for name in INPUTS:
            shutil.copyfile(SOURCE / name, primary / name)
        git('init', '-q')
        git('add', '.')
        git('commit', '-qm', 'hello app')
        git('worktree', 'add', '-q', str(signup))
        proxy = live.start_proxy()
        sessions = []
        for path, base, name in [(primary, 'hello.localhost', 'primary'), (signup, 'signup-hello.localhost', 'signup')]:
            sessions.append(remember(live.json(path, 'up', '--detach')))
            status = live.json(path, 'status', '--current')
            assert status['state'] == 'ready' and status['live_health_verified']
            assert all(endpoint['listener_owned'] and endpoint['http_ready'] for endpoint in status['live']['endpoints'].values())
            assert live.cli(path, 'url').stdout.strip() == f'https://{base}:{live.https}'
            assert live.cli(path, 'url', 'api').stdout.strip() == f'https://api-{base}:{live.https}'
            assert b'Hello from Parcela' in live.request(base, path='/')
            assert json.loads(live.request(f'api-{base}', path='/')) == {'message': f'hello from {name}', 'url': f'https://api-{base}:{live.https}'}
            assert {entry['name']: entry['state'] for entry in status['processes']} == {'web': 'running', 'api': 'running', 'ticker': 'running', 'setup': 'completed'}
            wait_for(lambda: 'tick 1' in live.cli(path, 'logs', 'ticker', '--tail', '10').stdout, [proxy])
            assert live.cli(path, 'task', 'messages').stdout == f'hello from {name}\n'
            assert 'setup  | database ready' in live.cli(path, 'logs', '--tail', '5').stdout
        listed = live.json(primary, 'ls')['workspaces']
        assert {Path(entry['workspace']['source']).resolve() for entry in listed} >= {primary, signup}
        restarted = remember(live.json(primary, 'restart', '--detach'))
        assert restarted['generation'] == sessions[0]['generation'] + 1 and restarted['token'] != sessions[0]['token']
        assert live.cli(primary, 'task', 'messages').stdout == 'hello from primary\n', 'restart keeps the database and skips setup'
        live.cli(primary, 'down')
        assert live.json(primary, 'status', '--current')['state'] == 'stopped'
        live.request('hello.localhost', 503, path='/')
        assert b'Hello from Parcela' in live.request('signup-hello.localhost', path='/')
        live.cli(primary, 'down', '--all')
        git('worktree', 'remove', str(signup))
        live.sources.remove(signup)
        live.cli(primary, 'rm', '--prune')
        remaining = {Path(entry['workspace']['source']).resolve() for entry in live.json(primary, 'ls')['workspaces']}
        assert signup not in remaining and primary in remaining, remaining
        assert {name: hashlib.sha256((primary / name).read_bytes()).hexdigest() for name in INPUTS} == before
    assert digests() == before, 'examples/docs-demo is unchanged'
    return {'verified': ['primary and Git worktree environments', 'named HTTPS web and API per checkout',
                         'per-service URL variables', 'worker logs', 'a SQLite database per checkout from hooks.setup',
                         'restart keeps the data and skips setup', 'independent stop and route withdrawal',
                         'down --all, worktree removal and rm --prune', 'source retention']}
