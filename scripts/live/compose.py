"""A compose service: two copies of one project run the same Compose file side by side."""
import json
import subprocess
import urllib.request
import uuid

from live.common import SERVICE, Live, wait_for

# A fixed container name, host port, volume name and network name, which two copies could not share
# without Parcela's override.
COMPOSE = '''services:
  files:
    image: ${FILES_IMAGE:?set in parcela.yaml}
    container_name: parcela-live-files
    command: ["sh", "-c", "echo \\"$$CHECKOUT\\" > /www/index.html && exec httpd -f -p 8080 -h /www"]
    environment:
      CHECKOUT: ${CHECKOUT:-none}
    ports: ["8080:8080"]
    volumes: [www:/www]
    healthcheck:
      test: ["CMD", "wget", "-q", "-O", "/dev/null", "http://127.0.0.1:8080/"]
      interval: 1s
volumes:
  www:
    name: parcela-live-www
networks:
  default:
    name: parcela-live-net
'''


def manifest(project):
    return f'''version: 1
project: {project}
env:
  CHECKOUT: ${{PARCELA_ENVIRONMENT_NAME}}
  FILES_IMAGE: busybox:1.37
services:
  deps:
    compose:
      file: docker-compose.yml
      services: [files]
      http:
        site: {{service: files, port: 8080, subdomain: site, healthcheck: /}}
  web:
    command: python3 {SERVICE} web
    http:
      healthcheck: /
    depends_on: [deps]
tasks:
  containers:
    command: docker compose ps --format '{{{{.Name}}}} {{{{.State}}}}'
    timeout_seconds: 30
'''


def docker(*argv):
    return subprocess.run(['docker', *argv], capture_output=True, text=True, check=True).stdout


def run(caddy, **_):
    project = f'compose-{uuid.uuid4().hex[:8]}'
    with Live('compose', caddy) as live:
        cases = []
        try:
            alpha = live.checkout('alpha', manifest(project))
            beta = live.checkout('beta', manifest(project))
            for source in (alpha, beta):
                (source / 'docker-compose.yml').write_text(COMPOSE)
                (source / 'docker-compose.override.yml').write_text('services:\n  files:\n    labels: [parcela.live.override]\n')
            doctor = live.cli(alpha, '--json', 'doctor', check=False)
            assert next(check for check in json.loads(doctor.stdout)['checks'] if check['name'] == 'config')['ok'], doctor.stdout
            live.start_proxy()
            live.cli(alpha, 'up', '--detach', timeout=180)
            live.cli(beta, 'up', '--detach', '--name', 'beta', timeout=180)
            ports = {}
            for source, label in ((alpha, 'primary'), (beta, 'beta')):
                session = live.json(source, 'status', '--current')['recorded_session']
                port = session['ports']['files']
                ports[label] = port
                body = urllib.request.urlopen(f'http://127.0.0.1:{port}/', timeout=5).read().decode().strip()
                assert body == label, (label, body)
                published = docker('compose', '-p', f'{project}-{label}', 'port', 'files', '8080').strip()
                assert published == f'127.0.0.1:{port}', published
            assert ports['primary'] != ports['beta'], ports
            for label in ('primary', 'beta'):
                docker('volume', 'inspect', f'{project}-{label}_www')
                docker('network', 'inspect', f'{project}-{label}_default')
            cases.append('each-copy-runs-its-own-containers-volumes-and-network-on-a-leased-loopback-port')

            for host, label in ((f'site-{project}.localhost', 'primary'), (f'site-beta-{project}.localhost', 'beta')):
                status, _, body = live.fetch(host, '/')
                assert (status, body.decode().strip()) == (200, label), (host, status, body)
            cases.append('a-container-port-is-served-at-its-own-https-url')

            labels = docker('inspect', '--format', '{{json .Config.Labels}}', f'{project}-primary-files-1')
            assert 'parcela.live.override' in labels, labels
            cases.append('the-default-override-file-applies-as-with-plain-docker-compose')

            listed = live.cli(alpha, 'task', 'containers').stdout.split()
            assert listed == [f'{project}-primary-files-1', 'running'], listed
            cases.append('tasks-reach-the-environments-own-compose-project')

            live.cli(alpha, 'down', timeout=60)
            stopped = docker('ps', '-a', '--filter', f'label=com.docker.compose.project={project}-primary', '--format', '{{.State}}').split()
            assert stopped == ['exited'], stopped
            docker('volume', 'inspect', f'{project}-primary_www')
            assert docker('compose', '-p', f'{project}-beta', 'ps', '--format', '{{.State}}').split() == ['running']
            cases.append('down-stops-only-its-own-containers-and-keeps-their-volumes')

            live.cli(alpha, 'up', '--detach', timeout=180)
            port = live.json(alpha, 'status', '--current')['recorded_session']['ports']['files']
            assert port == ports['primary'], (port, ports)
            wait_for(lambda: urllib.request.urlopen(f'http://127.0.0.1:{port}/', timeout=5).read().decode().strip() == 'primary')
            cases.append('up-again-reuses-the-port-and-the-containers')

            live.cli(beta, 'down', timeout=60)
            removed = json.loads(live.cli(beta, 'rm', timeout=60).stdout)
            assert removed['forgotten'][0]['kept']['volumes'] == [f'{project}-beta_www'], removed
            assert docker('ps', '-a', '-q', '--filter', f'label=com.docker.compose.project={project}-beta') == ''
            assert subprocess.run(['docker', 'network', 'inspect', f'{project}-beta_default'], capture_output=True).returncode != 0
            docker('volume', 'inspect', f'{project}-beta_www')
            cases.append('rm-removes-the-containers-and-network-and-keeps-the-volumes')
        finally:
            for label in ('primary', 'beta'):
                subprocess.run(['docker', 'compose', '-p', f'{project}-{label}', 'down', '-v'], capture_output=True)
        return {'https_port': live.https, 'project': project, 'cases': cases}
