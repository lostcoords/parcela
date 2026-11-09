(() => {
  'use strict';
  const $ = id => document.getElementById(id);
  const STALE_MS = 15000;
  const TONES = { ready: 'success', stopped: 'neutral', 'not started': 'neutral', failed: 'destructive', unhealthy: 'destructive' };
  const NOTES = {
    interrupted: 'Parcela lost track of this run. Start takes it back.',
    unknown: 'Parcela can’t reach this run. Start takes it back.',
  };
  let environments = [];
  let checked = null;
  let problem = null;
  let drawn = null;
  const refused = {};
  const remembered = () => { try { return localStorage.getItem('parcela:stopped') === 'open'; } catch { return false; } };
  let showStopped = remembered();

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

  const acting = environment => environment.action?.state === 'running';
  const resting = environment => environment.state === 'stopped' || environment.state === 'not started';
  // The reason, without the path to a log the Logs button already opens.
  const failure = environment => refused[environment.id]
    || (environment.action?.state === 'failed'
      ? `${environment.action.name} failed: ${(environment.action.message || 'see logs').replace(/^parcela: /, '').replace(/;?\s*full log: \S+/, '')}`
      : null);
  const quiet = environment => resting(environment) && !acting(environment) && !failure(environment);
  // What needs a person first, then what is changing, then what is running.
  const rank = environment => {
    if (failure(environment) || NOTES[environment.state] || environment.state === 'unhealthy') return 0;
    if (acting(environment) || environment.state !== 'ready') return 1;
    return 2;
  };
  const compare = (a, b) => rank(a) - rank(b)
    || Number(b.name === 'primary') - Number(a.name === 'primary')
    || a.name.localeCompare(b.name);

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

  const openers = environment => {
    if (environment.state !== 'ready') return [];
    const several = environment.services.length > 1;
    return environment.services.map(service => {
      const label = several ? `Open ${service.name}` : 'Open';
      const node = named(clone(several ? 'open-service' : 'open'), label, `open:${service.name}`);
      node.href = service.url;
      if (several) node.querySelector('[data-slot=service]').textContent = service.name;
      return node;
    });
  };

  const controls = environment => {
    const nodes = openers(environment);
    if (!environment.id || !environment.manageable) return nodes;
    const button = (name, label) => {
      const node = named(clone(name), label, name);
      node.disabled = acting(environment);
      node.addEventListener('click', () => act(environment, name));
      return node;
    };
    if (environment.processes.length || failure(environment)) {
      const logs = named(clone('logs'), 'Logs', 'logs');
      logs.href = `/logs?environment=${encodeURIComponent(environment.id)}`;
      nodes.push(logs);
    }
    if (resting(environment)) nodes.push(button('start', 'Start'));
    else if (NOTES[environment.state]) nodes.push(button('start', 'Start'), button('stop', 'Stop'));
    else nodes.push(button('restart', 'Restart'), button('stop', 'Stop'));
    return nodes;
  };

  const status = environment => {
    if (acting(environment)) return badge(environment.action.name === 'stop' ? 'stopping' : 'starting', 'warning');
    return badge(environment.state, TONES[environment.state] || 'warning');
  };

  const row = environment => {
    const node = clone('row');
    node.dataset.environment = environment.id || `${environment.project}/${environment.name}`;
    node.querySelector('[data-slot=list-title]').textContent = environment.name;
    const processes = environment.processes.length;
    const subtitle = [environment.url?.replace(/^https:\/\//, ''), processes ? `${processes} process${processes === 1 ? '' : 'es'}` : null, environment.state === 'not started' ? 'run parcela up there to register it' : null].filter(Boolean).join(' · ');
    const line = node.querySelector('[data-slot=list-subtitle]');
    if (subtitle) line.textContent = subtitle;
    else line.remove();
    const message = failure(environment);
    const note = acting(environment) ? null : NOTES[environment.state];
    if (message || note) {
      const text = clone(message ? 'problem' : 'note');
      text.textContent = message || note;
      node.querySelector('[data-slot=list-text]').append(text);
    }
    node.querySelector('[data-slot=controls]').append(status(environment), ...controls(environment));
    return node;
  };

  const projects = rows => {
    const grouped = new Map();
    for (const environment of [...rows].sort((a, b) => rank(a) - rank(b) || a.project.localeCompare(b.project))) {
      if (!grouped.has(environment.project)) grouped.set(environment.project, []);
      grouped.get(environment.project).push(environment);
    }
    return [...grouped].map(([name, members]) => {
      const node = clone('project');
      node.querySelector('h2').textContent = name;
      node.querySelector('ul').append(...members.sort(compare).map(row));
      return node;
    });
  };

  const draw = () => {
    const term = $('filter').value.trim().toLowerCase();
    const rows = environments.filter(environment => `${environment.project} ${environment.name} ${environment.state}`.toLowerCase().includes(term));
    const signature = JSON.stringify([term, rows, refused]);
    if (signature === drawn) return;
    drawn = signature;
    const focused = document.activeElement?.closest?.('[data-slot=list-row]')?.dataset.environment;
    const control = document.activeElement?.dataset?.control;
    const active = rows.filter(environment => !quiet(environment));
    const stopped = rows.filter(quiet);
    const nodes = projects(active);
    if (!rows.length && checked) {
      const empty = clone('empty');
      empty.textContent = term ? 'No environments match.' : 'No environments yet. Run `parcela up` in a project.';
      nodes.push(empty);
    }
    $('environments').replaceChildren(...nodes);
    $('environments').hidden = !nodes.length;
    $('stopped').hidden = !stopped.length;
    $('stopped-count').textContent = `${stopped.length} stopped`;
    $('stopped-list').replaceChildren(...projects(stopped));
    $('stopped').open = Boolean(term) || !active.length || showStopped;
    if (focused && control) {
      for (const node of document.querySelectorAll('[data-slot=list-row]')) {
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
  $('stopped').querySelector('summary').addEventListener('click', () => {
    showStopped = !$('stopped').open;
    try { localStorage.setItem('parcela:stopped', showStopped ? 'open' : 'closed'); } catch { /* A convenience only. */ }
  });
  refresh();
  setInterval(refresh, 2000);
})();
