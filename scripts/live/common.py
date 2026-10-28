"""Shared harness for the live suites: one disposable root and state directory per fixture."""
import json
import os
from pathlib import Path
import pty
import random
import shutil
import signal
import socket
import ssl
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
BINARY = ROOT / 'target/debug/parcela'
FIXTURES = ROOT / 'tests/fixtures'
SERVICE = FIXTURES / 'http_service.py'

# Below the Linux (32768+) and macOS (49152+) ephemeral ranges and above Parcela's 20000-29999 leases.
TEST_PORTS = range(30000, 32768)
_handed_out = set()


def free_port():
    for port in random.sample(TEST_PORTS, len(TEST_PORTS)):
        if port in _handed_out:
            continue
        with socket.socket() as sock:
            try:
                sock.bind(('127.0.0.1', port))
            except OSError:
                continue
        _handed_out.add(port)
        return port
    raise RuntimeError('no free test port in 30000-32767')


def listening(port):
    with socket.socket() as probe:
        return probe.connect_ex(('127.0.0.1', port)) == 0


def wait_for(check, processes=(), timeout=20):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        for process in processes:
            if process.poll() is not None:
                raise RuntimeError(f'process exited early: {process.args}, code {process.returncode}')
        try:
            if result := check():
                return result
        except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
            last = repr(error)
        time.sleep(.05)
    raise RuntimeError(f'wait timed out: {last}')


class Live:
    """A proxy on a test port, its state directory, and every child it spawns."""

    def __init__(self, name, caddy):
        self.caddy = caddy
        self.root = Path(tempfile.mkdtemp(prefix=f'pcl-live-{name}-', dir='/tmp')).resolve()
        self.state = self.root / 'state'
        self.https = free_port()
        self.children, self.logs, self.sources, self.ports = [], [], [], []
        self.proxy = None

    def __enter__(self):
        return self

    def __exit__(self, failure, *_):
        self.close()
        if failure is None:
            shutil.rmtree(self.root, ignore_errors=True)

    def checkout(self, name, manifest=None):
        path = self.root / name
        path.mkdir(parents=True)
        if manifest is not None:
            (path / 'parcela.yaml').write_text(manifest)
        self.sources.append(path)
        return path

    def argv(self, path, *argv):
        return [str(BINARY), '--path', str(path), '--state-dir', str(self.state), *argv]

    def cli(self, path, *argv, check=True, timeout=30):
        return subprocess.run(self.argv(path, *argv), capture_output=True, text=True, check=check, timeout=timeout)

    def json(self, path, *argv):
        return json.loads(self.cli(path, *argv).stdout)

    def human(self, path, *argv):
        """Run with stdout on a pseudo-terminal, where Parcela prints human output."""
        primary, secondary = pty.openpty()
        try:
            process = subprocess.Popen(self.argv(path, *argv), stdin=subprocess.DEVNULL, stdout=secondary,
                                       stderr=subprocess.DEVNULL, env={**os.environ, 'NO_COLOR': '1', 'PARCELA_NO_TRUST': '1'})
        finally:
            os.close(secondary)
        output = b''
        try:
            while chunk := os.read(primary, 65536):
                output += chunk
        except OSError:
            pass
        finally:
            os.close(primary)
        assert process.wait(timeout=15) == 0, (argv, output)
        return output.decode().replace('\r\n', '\n')

    def spawn(self, path, *argv, env=None):
        log = open(self.root / f'child-{len(self.children)}.log', 'w+')
        self.logs.append(log)
        child = subprocess.Popen(self.argv(path, *argv), stdout=log, stderr=log, env=env)
        self.children.append(child)
        return child

    def start_proxy(self):
        self.proxy = self.spawn(self.root, 'proxy', 'run', '--caddy', self.caddy, '--port', str(self.https))
        wait_for(lambda: self.cli(self.root, 'proxy', 'status', check=False).returncode == 0, [self.proxy])
        return self.proxy

    def register(self, path, role, label):
        """Register a checkout as its first `up` does, stopping at preflight on an absent LAN address."""
        naming = ['--primary'] if role == 'primary' else ['--name', label]
        started = self.cli(path, 'up', *naming, '--lan-ip', '192.0.2.1', check=False)
        assert started.returncode == 1 and 'no network interface has that address' in started.stderr, started.stderr
        workspace = self.json(path, '--json', 'status', '--current')['workspace']
        assert (workspace['role'], workspace['label']) == (role, label), workspace
        return workspace

    def fetch(self, host, path='/ready', headers='', host_header=None, timeout=3):
        """One HTTPS request through the proxy, trusting only this state's CA. Returns (status, head, body)."""
        context = ssl.create_default_context(cafile=str(self.state / 'proxy/data/pki/authorities/local/root.crt'))
        with context.wrap_socket(socket.create_connection(('127.0.0.1', self.https), timeout=timeout), server_hostname=host) as sock:
            sock.sendall(f'GET {path} HTTP/1.1\r\nHost: {host_header or host}\r\nConnection: close\r\n{headers}\r\n'.encode())
            data = b''
            while part := sock.recv(65536):
                data += part
                if path == '/ws':
                    break
        head, _, body = data.partition(b'\r\n\r\n')
        if not head.startswith(b'HTTP/1.1 '):
            raise ValueError(f'no HTTP response from {host}: {data[:80]!r}')
        if b'transfer-encoding: chunked' in head.lower():
            decoded = b''
            while (size := int(body.partition(b'\r\n')[0], 16)):
                chunk = body.partition(b'\r\n')[2]
                decoded, body = decoded + chunk[:size], chunk[size + 2:]
            body = decoded
        return int(head.split(b' ', 2)[1]), head, body

    def request(self, host, expected=200, path='/ready'):
        status, head, body = self.fetch(host, path)
        if status != expected:
            raise ValueError(f'expected HTTP {expected} from {host}, received {head!r}')
        return json.loads(body) if expected == 200 and path == '/ready' else body

    def close(self):
        """Stop every source and child, then fail if any test listener survives. Keeps the root for inspection."""
        for path in self.sources:
            try:
                self.cli(path, 'down', check=False)
            except (subprocess.TimeoutExpired, OSError):
                pass
        for child in reversed(self.children):
            if child.poll() is None:
                child.send_signal(signal.SIGTERM)
                try:
                    child.wait(timeout=12)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=3)
        for log in self.logs:
            log.close()
        ports = {self.https, *self.ports}
        wait_for(lambda: not any(map(listening, ports)), timeout=12)
