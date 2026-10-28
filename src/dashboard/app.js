(() => {
  'use strict';
  const $ = id => document.getElementById(id);
  const embedded = new URLSearchParams(location.search).has('embed');
  const query = new URLSearchParams(location.search);
  const from = new URLSearchParams(location.hash.slice(1)).get('from');
  let environments = [];
  let selected = query.get('environment');
  let selectedService = query.get('service');
  let checkedAt = null;
  const expanded = new Set();
  const element = (tag, text, className) => {
    const node = document.createElement(tag);
    if (text !== undefined) node.textContent = text;
    if (className) node.className = className;
    return node;
  };
  const ready = env => env.state === 'ready';
  const isCurrent = env => env.services.some(service => service.url === from) || env.url === from;
  const workspaceUrl = (env, service) => `/workspace?environment=${encodeURIComponent(env.id)}&service=${encodeURIComponent(service.name)}`;
  function anchor(text, url, target) {
    const node = element('a', text);
    node.href = url;
    node.target = target;
    node.rel = 'noopener noreferrer';
    return node;
  }
  function render() {
    const term = $('search').value.trim().toLowerCase();
    const currentProject = environments.find(isCurrent)?.project;
    const rows = environments.filter(env => `${env.project} ${env.name} ${env.state}`.toLowerCase().includes(term));
    rows.sort((a, b) => Number(b.project === currentProject) - Number(a.project === currentProject) || a.project.localeCompare(b.project) || Number(isCurrent(b)) - Number(isCurrent(a)) || a.name.localeCompare(b.name));
    const list = $('environments');
    list.replaceChildren();
    let project;
    let group;
    for (const env of rows) {
      if (project !== env.project) {
        project = env.project;
        group = element('section', undefined, 'project');
        group.append(element('h2', project));
        list.append(group);
      }
      const current = isCurrent(env);
      const card = element('article', undefined, `environment${current ? ' current' : ''}`);
      const heading = element('div', undefined, 'env-heading');
      heading.append(element('span', env.name, 'name'), element('span', env.state, `state ${env.state}`));
      card.append(heading);
      if (current) card.append(element('div', 'You are here', 'current-label'));
      if (env.url) card.append(element('div', env.url, 'url'));
      for (const service of env.services) {
        const row = element('div', undefined, 'service');
        row.append(element('span', service.name || 'web', 'service-name'));
        if (ready(env)) {
          const actions = element('div', undefined, 'actions');
          actions.append(anchor('Open directly ↗', service.url, embedded ? '_top' : '_blank'));
          if (env.id) actions.append(anchor('In workspace', workspaceUrl(env, service), embedded ? '_top' : '_self'));
          row.append(actions);
        }
        card.append(row);
      }
      if (env.processes.length) {
        const details = element('details', undefined, 'processes');
        details.open = expanded.has(env.id) || (current && !expanded.has(`closed:${env.id}`));
        const summary = element('summary', `${env.processes.length} processes`);
        summary.dataset.focusKey = env.id;
        details.append(summary);
        const table = element('table');
        const head = element('tr');
        for (const title of ['Process', 'State', 'PID']) head.append(element('th', title));
        table.append(head);
        for (const process of env.processes) {
          const row = element('tr');
          for (const value of [process.name, process.state, process.pid ?? '—']) row.append(element('td', value));
          table.append(row);
        }
        details.append(table);
        details.addEventListener('toggle', () => {
          if (!details.isConnected) return;
          if (details.open) { expanded.add(env.id); expanded.delete(`closed:${env.id}`); }
          else { expanded.delete(env.id); expanded.add(`closed:${env.id}`); }
        });
        card.append(details);
      }
      group.append(card);
    }
    if (!rows.length) list.append(element('p', term ? 'No environments match your search.' : 'No environments registered. Run parcela up in a project to get started.', 'empty'));
    renderWorkspace();
  }
  function renderWorkspace() {
    if (embedded || !selected) return;
    const env = environments.find(env => env.id === selected);
    const service = env?.services.find(service => service.name === selectedService) || env?.services[0];
    const available = env && service && ready(env);
    $('workspace').hidden = !available;
    $('welcome').hidden = Boolean(available);
    document.body.classList.toggle('has-workspace', Boolean(available));
    if (available) {
      $('workspace-title').textContent = `${env.project} / ${env.name} · ${service.name}`;
      $('direct').href = service.url;
      if ($('application').getAttribute('src') !== service.url) $('application').src = service.url;
    } else {
      $('application').removeAttribute('src');
      $('welcome').querySelector('h2').textContent = env ? `${env.name} is ${env.state}.` : 'Environment unavailable.';
    }
  }
  async function refresh() {
    try {
      const response = await fetch('/api/status', { cache: 'no-store', signal: AbortSignal.timeout(5000) });
      if (!response.ok) throw new Error('Status unavailable');
      const snapshot = await response.json();
      if (snapshot.error) throw new Error(snapshot.error);
      environments = snapshot.environments;
      checkedAt = snapshot.checked_at_unix_ms;
      const stale = !checkedAt || Date.now() - checkedAt > 10000;
      $('health').textContent = checkedAt ? `${environments.filter(ready).length} ready · Checked ${new Date(checkedAt).toLocaleTimeString()}${stale ? ' · Status may be stale' : ''}` : 'Reading environment status…';
      $('health').classList.toggle('stale', stale);
      const focused = document.activeElement;
      const focusKey = focused?.tagName === 'A' ? focused.href : null;
      const summaryKey = focused?.dataset.focusKey;
      render();
      if (focusKey && !focused.isConnected) Array.from($('environments').querySelectorAll('a')).find(link => link.href === focusKey)?.focus({ preventScroll: true });
      if (summaryKey && !focused.isConnected) Array.from($('environments').querySelectorAll('summary')).find(summary => summary.dataset.focusKey === summaryKey)?.focus({ preventScroll: true });
    } catch (_) {
      $('health').textContent = 'Connection lost. Displayed status may be stale. Retrying…';
      $('health').classList.add('stale');
      for (const link of $('environments').querySelectorAll('a')) link.removeAttribute('href');
    } finally {
      setTimeout(refresh, 3000);
    }
  }
  if (embedded) {
    document.body.classList.add('embedded');
    $('close').hidden = false;
    const close = () => {
      if (from) {
        try { parent.postMessage({ type: 'parcela:close' }, new URL(from).origin); } catch (_) { /* Invalid parent origins cannot receive messages. */ }
      }
    };
    $('close').addEventListener('click', close);
    document.addEventListener('keydown', event => { if (event.key === 'Escape') close(); });
  }
  $('search').addEventListener('input', render);
  refresh();
})();
