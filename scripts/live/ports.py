"""Named non-HTTP ports across two parallel copies of one project."""
import json
import socket

from live.common import FIXTURES, SERVICE, Live, wait_for

MANIFEST = f'''version: 1
project: ports-demo
services:
  web:
    command: python3 {SERVICE} web
    http:
      healthcheck: /
  db:
    command: python3 {FIXTURES / 'port_listener.py'} $PARCELA_PORT_DB
ports: [db, vite-hmr]
env:
  DATABASE_URL: postgres://127.0.0.1:${{PARCELA_PORT_DB}}/app
tasks:
  ports:
    command: >-
      python3 -c 'import json,os;print(json.dumps({{k: v for k, v in os.environ.items() if k.startswith("PARCELA_PORT_")}}))'
    timeout_seconds: 10
'''


def run(caddy, **_):
    with Live('ports', caddy) as live:
        cases, runs = [], []

        def observe(source, label):
            """Every view of one run's named ports must agree: process argv and env, context.json, context and status."""
            status = live.json(source, 'status', '--current')
            session = status['recorded_session']
            runtime = live.state / 'environments' / status['workspace']['id']
            record_path = runtime / 'port-listener.json'
            record = wait_for(lambda: (record := json.loads(record_path.read_text()))['generation'] == str(session['generation']) and record)
            ports = session['ports']
            assert set(ports) == {'db', 'vite-hmr'}, session
            expected = {'PARCELA_PORT_DB': str(ports['db']), 'PARCELA_PORT_VITE_HMR': str(ports['vite-hmr'])}
            assert record['argv'] == [str(ports['db'])], (record, ports)
            assert record['environment'] == expected, (record, ports)
            environment = json.loads((runtime / 'context.json').read_text())['environment']
            assert {key: environment[key] for key in expected} == expected, environment
            assert environment['DATABASE_URL'] == f"postgres://127.0.0.1:{ports['db']}/app", environment
            assert live.json(source, 'context')['ports'] == ports
            human = live.human(source, 'status', '--current')
            assert f"{ports['db']}" in human and 'PARCELA_PORT_VITE_HMR' in human, human
            assert 'PARCELA_PORT_DB' in live.human(source, 'context')
            with socket.socket() as probe:
                assert probe.connect_ex(('127.0.0.1', ports['db'])) == 0, 'the stand-in database listens on its leased port'
            endpoints = {endpoint['backend_port'] for endpoint in session['endpoints']} or {session['backend_port']}
            assert not endpoints & set(ports.values()), session
            live.ports.extend(endpoints | set(ports.values()))
            runs.append({'copy': label, 'generation': session['generation'], 'ports': ports, 'endpoints': sorted(endpoints)})
            return ports, endpoints

        alpha, beta = live.checkout('alpha', MANIFEST), live.checkout('beta', MANIFEST)
        doctor = live.cli(alpha, '--json', 'doctor', check=False)
        assert next(check for check in json.loads(doctor.stdout)['checks'] if check['name'] == 'config')['ok']
        live.start_proxy()
        live.cli(alpha, 'up', '--detach')
        live.cli(beta, 'up', '--detach', '--name', 'beta')
        first, first_endpoints = observe(alpha, 'alpha')
        other, other_endpoints = observe(beta, 'beta')
        assert not (set(first.values()) | first_endpoints) & (set(other.values()) | other_endpoints), (first, other)
        cases.append('parallel-copies-get-distinct-named-ports-that-every-view-agrees-on')

        task = live.json(alpha, 'task', 'ports')
        assert task == {'PARCELA_PORT_DB': str(first['db']), 'PARCELA_PORT_VITE_HMR': str(first['vite-hmr'])}, task
        cases.append('task-environment-carries-named-ports')

        live.cli(alpha, 'down')
        live.cli(alpha, 'up', '--detach')
        assert observe(alpha, 'alpha')[0] == first
        restarted = live.json(alpha, 'restart', '--detach')
        assert restarted['session']['ports'] == first, restarted['session']
        assert observe(alpha, 'alpha')[0] == first
        cases.append('down-up-and-restart-reuse-the-previous-named-ports')

        live.cli(alpha, 'down')
        with socket.socket() as foreign:
            foreign.bind(('127.0.0.1', first['db']))
            foreign.listen()
            live.cli(alpha, 'up', '--detach')
            moved, _ = observe(alpha, 'alpha')
            assert moved['db'] != first['db'] and moved['vite-hmr'] == first['vite-hmr'], (moved, first)
        cases.append('a-port-taken-meanwhile-is-replaced-and-the-rest-stay')
        return {'https_port': live.https, 'cases': cases, 'runs': runs}
