// Shown while nothing serves this name. Starting is always a click, never a page load.
(() => {
  const $ = id => document.getElementById(id);
  $('all').href = '__PARCELA_DASHBOARD__/';
  let failure = null;
  let environment = null;

  const answering = async () => {
    try {
      const response = await fetch(location.href, { method: 'HEAD', cache: 'no-store', redirect: 'manual' });
      return response.status !== 503;
    } catch {
      return false;
    }
  };

  const act = async name => {
    failure = null;
    $('action').disabled = true;
    try {
      const response = await fetch(`/_parcela/environments/${encodeURIComponent(environment.key)}/${name}`, { method: 'POST', headers: { 'X-Parcela-Action': '1' } });
      if (!response.ok) failure = (await response.text()).trim();
    } catch {
      failure = 'Could not reach Parcela.';
    }
    refresh();
  };

  const show = (dot, text) => {
    $('dot').className = dot;
    $('status').textContent = text;
  };

  const refresh = async () => {
    let listed;
    try {
      const response = await fetch('/_parcela/environments', { cache: 'no-store' });
      listed = (await response.json()).environments;
    } catch {
      $('action').hidden = true;
      show('failed', 'Could not reach Parcela.');
      return;
    }
    environment = listed.find(row => row.current);
    if (!environment) {
      $('action').hidden = true;
      show('', 'This name is not registered.');
      return;
    }
    $('project').textContent = environment.project;
    $('project').hidden = false;
    $('name').textContent = environment.name;
    document.title = `${environment.project} · ${environment.name}`;
    $('logs').hidden = !environment.processes.length;

    const running = environment.action?.state === 'running';
    const stopped = environment.state === 'stopped' || environment.state === 'unregistered';
    const name = stopped ? 'start' : 'restart';
    $('action').hidden = !environment.manageable;
    $('action').disabled = running;
    $('action').textContent = name === 'start' ? 'Start' : 'Restart';
    $('action').onclick = () => act(name);

    if (running) show('busy', environment.action.name === 'stop' ? 'Stopping…' : 'Starting…');
    else if (failure) show('failed', failure);
    else if (environment.action?.state === 'failed') show('failed', `${environment.action.name} failed: ${environment.action.message || 'see logs'}`);
    else if (environment.state === 'ready') {
      show('busy', 'Starting…');
      if (await answering()) location.reload();
    } else if (stopped) show('', 'Stopped');
    else show('busy', `Not ready: ${environment.state}`);
  };

  refresh();
  setInterval(refresh, 1000);
})();
