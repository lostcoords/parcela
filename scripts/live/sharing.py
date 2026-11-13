"""Domain sharing: a run served at <name>.<domain> on a loopback port for the person's own proxy.

LAN and Tailscale sharing publish to other devices, so they are not run here.
"""
import http.client
import json

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
        return {'https_port': live.https, 'domain_port': domain_port, 'cases': cases}
