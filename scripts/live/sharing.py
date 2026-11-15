"""Domain sharing: a run served at <name>.<domain> on a loopback port for the person's own proxy.
LAN sharing behind device pairing, with a stand-in for dns-sd so no name is published.

Tailscale sharing publishes to other devices, so it is not run here.
"""
import http.client
import ipaddress
import json
import os
import socket
import ssl

from live.common import SERVICE, Live, free_port, listening, wait_for

MANIFEST = f'''version: 1
project: shop
services:
  web:
    command: python3 {SERVICE}
    http:
      healthcheck: /ready
  api:
    command: python3 {SERVICE} api
    http:
      subdomain: api
      healthcheck: /ready
'''


def get(port, host, headers=None):
    connection = http.client.HTTPConnection('127.0.0.1', port, timeout=5)
    try:
        connection.request('GET', '/', headers={'Host': host, **(headers or {})})
        response = connection.getresponse()
        return response.status, response.read()
    finally:
        connection.close()


def lan_address():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        try:
            probe.connect(('192.0.2.1', 9))
        except OSError:
            return None
        address = probe.getsockname()[0]
    return address if ipaddress.ip_address(address).is_private and not address.startswith('127.') else None


def lan_fetch(live, ip, host, headers='', path='/'):
    """One HTTPS request to the run's LAN listener, as another device sends it."""
    context = ssl.create_default_context(cafile=str(live.state / 'proxy/data/pki/authorities/local/root.crt'))
    with context.wrap_socket(socket.create_connection((ip, live.https), timeout=5), server_hostname=host) as sock:
        connection = 'Upgrade' if path == '/ws' else 'close'
        sock.sendall(f'GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: {connection}\r\n{headers}\r\n'.encode())
        data = b''
        while part := sock.recv(65536):
            data += part
            if path == '/ws':
                break
    head, _, body = data.partition(b'\r\n\r\n')
    return int(head.split(b' ', 2)[1]), head.decode(), body


def lan_pairing(live, source, cases):
    ip = lan_address()
    if ip is None:
        cases.append('lan-pairing-skipped-without-a-private-address')
        return
    bin_dir = live.root / 'bin'
    bin_dir.mkdir()
    (bin_dir / 'dns-sd').write_text('#!/bin/sh\nexec sleep 86400\n')
    (bin_dir / 'dns-sd').chmod(0o755)
    path = os.environ['PATH']
    os.environ['PATH'] = f'{bin_dir}:{path}'
    try:
        live.cli(source, 'up', '--detach', '--lan-ip', ip, timeout=60)
    finally:
        os.environ['PATH'] = path
    spoofed = 'X-Forwarded-For: 203.0.113.9\r\nForwarded: for=203.0.113.9\r\nX-Real-IP: 203.0.113.9\r\nTrue-Client-IP: 203.0.113.9\r\n'

    def local(headers):
        """Whether the app saw a request as this machine would send it, on the .local name over HTTPS."""
        names = {name.lower(): value for name, value in headers.items()}
        named = {'x-forwarded-for', 'forwarded', 'x-real-ip', 'true-client-ip'} & names.keys()
        return not named and names.get('x-forwarded-proto') == 'https' and names.get('x-forwarded-host') == 'shop.local' \
            and 'parcela_device' not in names.get('cookie', '')

    code, head, body = wait_for(lambda: (lambda answer: answer[0] == 403 and answer)(
        lan_fetch(live, ip, 'shop.local', 'Sec-Fetch-Dest: document\r\nUser-Agent: LiveTest\r\n' + spoofed)), [live.proxy])
    assert b'Waiting for approval' in body, body[:300]
    cookie = next(line.split(':', 1)[1].strip() for line in head.split('\r\n') if line.lower().startswith('set-cookie:'))
    assert cookie.endswith('Secure; HttpOnly; SameSite=Lax'), cookie
    token = cookie.split(';', 1)[0].split('=', 1)[1]
    cases.append('an-unpaired-device-gets-the-pairing-page-and-a-cookie')

    code, _, body = lan_fetch(live, ip, 'shop.local', 'Sec-Fetch-Dest: image\r\n')
    assert code == 403 and b'not paired' in body, (code, body[:200])
    devices = live.json(source, 'devices')['devices']
    assert [(device['standing'], device['host'], device['user_agent']) for device in devices] == [('pending', 'shop.local', 'LiveTest')], devices
    cases.append('only-a-page-load-asks-and-the-request-is-listed')

    device = devices[0]['id']
    live.cli(source, 'devices', 'approve', device)
    code, _, body = lan_fetch(live, ip, 'shop.local', f'Cookie: theme=dark; parcela_device={token}\r\n' + spoofed)
    assert code == 200, (code, body[:300])
    answer = json.loads(body)
    assert local(answer['headers']), answer['headers']
    assert 'theme=dark' in answer['headers'].get('Cookie', ''), answer['headers']
    cases.append('an-approved-device-reaches-the-app-as-this-machine-with-forged-headers-gone')

    upgrade = f'Cookie: parcela_device={token}\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n'
    code, _, _ = lan_fetch(live, ip, 'shop.local', upgrade, path='/ws')
    assert code == 101, code
    cases.append('an-approved-device-opens-a-websocket')

    code, _, _ = lan_fetch(live, ip, 'api-shop.local', f'Cookie: parcela_device={token}\r\n')
    assert code == 403, code
    cases.append('approval-holds-for-one-local-name')

    live.cli(source, 'devices', 'revoke', device)
    code, _, _ = lan_fetch(live, ip, 'shop.local', f'Cookie: parcela_device={token}\r\n')
    assert code == 403, code
    cases.append('a-revoked-device-is-refused')
    live.cli(source, 'down', timeout=60)


def run(caddy, **_):
    with Live('sharing', caddy) as live:
        cases = []
        source = live.checkout('shop', MANIFEST)
        domain_port = free_port()
        live.ports.append(domain_port)
        live.start_proxy()
        live.cli(source, 'up', '--detach', '--primary', '--domain', 'example.test', '--domain-port', str(domain_port), timeout=60)
        status = live.json(source, 'status', '--current')
        live.ports.extend(endpoint['backend_port'] for endpoint in status['recorded_session']['endpoints'])
        shared = {name: endpoint['shared'] for name, endpoint in status['live']['endpoints'].items()}
        assert shared == {'web': ['https://shop.example.test'], 'api': ['https://api-shop.example.test']}, shared
        cases.append('status-lists-each-service-domain-url')

        for host, argv in (('shop.example.test', []), ('api-shop.example.test', ['api'])):
            code, body = get(domain_port, host, {'X-Forwarded-Proto': 'https', 'X-Forwarded-Host': host})
            assert code == 200, (host, code, body[:200])
            answer = json.loads(body)
            assert answer['argv'] == argv, answer
            assert answer['headers'].get('X-Forwarded-Proto') == 'https', answer['headers']
            assert answer['headers'].get('X-Forwarded-For') == '127.0.0.1', answer['headers']
        cases.append('each-service-answers-on-its-domain-name-with-forwarded-headers')

        code, _ = get(domain_port, 'other.example.test')
        assert code != 200, code
        cases.append('an-unknown-name-is-not-routed')

        live.cli(source, 'down', timeout=60)
        wait_for(lambda: not listening(domain_port), [live.proxy], timeout=15)
        cases.append('down-closes-the-domain-listener')

        live.cli(source, 'up', '--detach', '--no-share', timeout=60)
        assert live.json(source, 'status', '--current')['live']['endpoints']['web'].get('shared', []) == []
        assert not listening(domain_port)
        cases.append('a-run-without-sharing-opens-no-domain-listener')
        live.cli(source, 'down', timeout=60)
        lan_pairing(live, source, cases)
        return {'https_port': live.https, 'domain_port': domain_port, 'cases': cases}
