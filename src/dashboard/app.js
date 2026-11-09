(() => {
  'use strict';
  const $ = id => document.getElementById(id);
  const STALE_MS = 15000;
  const TONES = { ready: 'success', stopped: 'neutral', unregistered: 'neutral', failed: 'destructive' };
  let environments = [];
  let checked = null;
  let problem = null;
  let drawn = null;
  const refused = {};

  const clone = id => $(id).content.firstElementChild.cloneNode(true);
  const badge = (text, tone) => {
    const node = clone(`badge-${tone}`);
    node.textContent = text;
    return node;
  };
  const named = (node, label, action) => {
    node.title = label;
    node.setAttribute('aria-label', label);
    node.dataset.control = action;
    return node;
  };
  const opener = (url, label) => {
    const node = named(clone('open'), label, `open:${url}`);
    node.href = url;
    return node;
  };

  const act = async (environment, name) => {
    refused[environment.id] = null;
    try {
      const response = await fetch(`/_parcela/environments/${encodeURIComponent(environment.id)}/${name}`, { method: 'POST', headers: { 'X-Parcela-Action': '1' } });
      if (!response.ok) refused[environment.id] = (await response.text()).trim();
    } catch {
      refused[environment.id] = 'Could not reach Parcela.';
    }
    refresh();
  };

  const controls = environment => {
    const running = environment.action?.state === 'running';
    const ready = environment.state === 'ready';
    const stopped = environment.state === 'stopped' || environment.state === 'unregistered';
    const nodes = ready ? environment.services.map(service => opener(service.url, environment.services.length > 1 ? `Open ${service.name}` : 'Open')) : [];
    if (environment.id && environment.manageable) {
      const button = (name, label) => {
        const node = named(clone(name), label, name);
        node.disabled = running;
        node.addEventListener('click', () => act(environment, name));
        return node;
      };
      if (environment.processes.length) {
        const logs = named(clone('logs'), 'Logs', 'logs');
        logs.href = `/logs#logs:${encodeURIComponent(environment.id)}`;
        nodes.push(logs);
      }
      if (stopped) nodes.push(button('start', 'Start'));
      else nodes.push(button('restart', 'Restart'), button('stop', 'Stop'));
    }
    return nodes;
  };

  const status = environment => {
    if (environment.action?.state === 'running') return badge(environment.action.name === 'stop' ? 'stopping' : 'starting', 'warning');
    return badge(environment.state, TONES[environment.state] || 'warning');
  };

  const failure = environment => refused[environment.id]
    || (environment.action?.state === 'failed' ? `${environment.action.name} failed: ${environment.action.message || 'see logs'}` : null);

  const row = environment => {
    const node = clone('row');
    node.dataset.environment = environment.id || `${environment.project}/${environment.name}`;
    node.querySelector('[data-slot=list-title]').textContent = environment.name;
    const processes = environment.processes.length;
    const subtitle = [environment.url?.replace(/^https:\/\//, ''), processes ? `${processes} process${processes === 1 ? '' : 'es'}` : null].filter(Boolean).join(' · ');
    const line = node.querySelector('[data-slot=list-subtitle]');
    if (subtitle) line.textContent = subtitle;
    else line.remove();
    const message = failure(environment);
    if (message) {
      const text = clone('problem');
      text.textContent = message;
      node.querySelector('[data-slot=list-text]').append(text);
    }
    node.querySelector('[data-slot=controls]').append(status(environment), ...controls(environment));
    return node;
  };

  const project = (name, members) => {
    const node = clone('project');
    node.querySelector('h2').textContent = name;
    node.querySelector('ul').append(...members.map(row));
    return node;
  };

  const draw = () => {
    const term = $('filter').value.trim().toLowerCase();
    const rows = environments.filter(environment => `${environment.project} ${environment.name} ${environment.state}`.toLowerCase().includes(term));
    const signature = JSON.stringify([term, rows, refused]);
    if (signature === drawn) return;
    drawn = signature;
    const focused = document.activeElement?.closest?.('[data-slot=list-row]')?.dataset.environment;
    const control = document.activeElement?.dataset?.control;
    const projects = new Map();
    for (const environment of [...rows].sort((a, b) => a.project.localeCompare(b.project) || a.name.localeCompare(b.name))) {
      if (!projects.has(environment.project)) projects.set(environment.project, []);
      projects.get(environment.project).push(environment);
    }
    const nodes = [...projects].map(([name, members]) => project(name, members));
    if (!nodes.length && checked) {
      const empty = clone('empty');
      empty.textContent = term ? 'No environments match.' : 'No environments yet. Run `parcela up` in a project.';
      nodes.push(empty);
    }
    $('environments').replaceChildren(...nodes);
    if (focused && control) {
      for (const node of $('environments').querySelectorAll('[data-slot=list-row]')) {
        if (node.dataset.environment === focused) node.querySelector(`[data-control="${CSS.escape(control)}"]`)?.focus();
      }
    }
    const ready = environments.filter(environment => environment.state === 'ready').length;
    $('summary').replaceChildren(...(checked ? [badge(`${ready} ready`, ready ? 'success' : 'neutral')] : []));
  };

  const health = () => {
    const stale = checked && Date.now() - checked > STALE_MS;
    $('health').textContent = problem || (stale ? 'Status may be out of date.' : '');
    $('health').hidden = !problem && !stale && Boolean(checked);
  };

  const refresh = async () => {
    try {
      const [status, listed] = await Promise.all(['/api/status', '/_parcela/environments'].map(async path => {
        const response = await fetch(path, { cache: 'no-store', signal: AbortSignal.timeout(5000) });
        if (!response.ok) throw new Error(path);
        return response.json();
      }));
      const rows = new Map(listed.environments.map(row => [row.key, row]));
      environments = status.environments.map(environment => {
        const { action = null, manageable = false } = rows.get(environment.id) || {};
        return { ...environment, action, manageable };
      });
      problem = status.error ? 'Could not read the environments.' : null;
      if (status.checked_at_unix_ms) checked = status.checked_at_unix_ms;
    } catch {
      problem = 'Could not reach Parcela. Retrying.';
    }
    draw();
    health();
  };

  $('filter').addEventListener('input', draw);
  refresh();
  setInterval(refresh, 2000);
})();
