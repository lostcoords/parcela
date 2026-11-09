(() => {
  'use strict';
  const $ = id => document.getElementById(id);
  const ESCAPE = String.fromCharCode(27);
  const CODES = new RegExp(`${ESCAPE}\\[([0-9;:]*)([A-Za-z])`);
  const ALL_CODES = new RegExp(`${ESCAPE}\\[[0-9;:]*[A-Za-z]`, 'g');
  const PALETTE = { 30: '#9a9ca3', 31: '#ef7f68', 32: '#6fc58a', 33: '#dda35a', 34: '#7ba7f0', 35: '#c792ea', 36: '#5fc3d6', 37: '#d6d6d2' };
  const TONES = { ready: 'success', stopped: 'neutral', unregistered: 'neutral', unhealthy: 'destructive' };
  const environment = new URLSearchParams(location.search).get('environment');
  let process = null;
  let lines = [];

  const severity = line => /\b(ERROR|FATAL|Exception|Traceback|panicked)\b/.test(line) ? 'text-destructive' : /\bWARN(ING)?\b/.test(line) ? 'text-warning' : '';

  // Programs color their own output with ANSI escapes. 256-color and truecolor are left out.
  const render = text => {
    const node = document.createElement('div');
    node.className = severity(text.replace(ALL_CODES, ''));
    const pieces = text.split(new RegExp(CODES.source));
    let color = null;
    let bold = false;
    for (let index = 0; index < pieces.length; index += 3) {
      if (pieces[index]) {
        const span = document.createElement('span');
        span.textContent = pieces[index];
        if (color) span.style.color = color;
        if (bold) span.style.fontWeight = '700';
        node.append(span);
      }
      if (pieces[index + 2] !== 'm') continue;
      const params = (pieces[index + 1] || '0').split(';');
      for (let at = 0; at < params.length; at++) {
        const [code, ...sub] = params[at].split(':').map(Number);
        if (code === 38 || code === 48 || code === 58) {
          if (!sub.length) at += params[at + 1] === '5' ? 2 : params[at + 1] === '2' ? 4 : 0;
          continue;
        }
        if (code === 0) { color = null; bold = false; }
        else if (code === 1) bold = true;
        else if (code === 22) bold = false;
        else if (code === 39) color = null;
        else if (PALETTE[code % 60]) color = PALETTE[code % 60];
      }
    }
    return node;
  };

  const say = (text, tone = 'text-muted-foreground') => {
    const node = document.createElement('div');
    node.className = tone;
    node.textContent = text;
    $('output').replaceChildren(node);
  };

  const draw = () => {
    const output = $('output');
    const following = output.scrollHeight - output.scrollTop - output.clientHeight < 40;
    const needle = $('filter').value.trim().toLowerCase();
    const shown = needle ? lines.filter(text => text.replace(ALL_CODES, '').toLowerCase().includes(needle)) : lines;
    if (shown.length) output.replaceChildren(...shown.map(render));
    else say(needle ? 'No lines match.' : 'No logs yet.');
    if (following) output.scrollTop = output.scrollHeight;
  };

  const processes = names => {
    $('processes').replaceChildren(...names.map(name => {
      const node = $('process').content.firstElementChild.cloneNode(true);
      node.textContent = name;
      node.setAttribute('aria-pressed', String(name === process));
      node.addEventListener('click', () => { process = name; load(); });
      return node;
    }));
  };

  const load = async () => {
    const query = new URLSearchParams({ environment, lines: '1000' });
    if (process) query.set('process', process);
    try {
      const response = await fetch(`/_parcela/logs?${query}`, { cache: 'no-store', signal: AbortSignal.timeout(5000) });
      if (!response.ok) throw new Error(await response.text());
      const result = await response.json();
      process = result.process;
      processes(result.processes);
      if (result.lines.length && result.lines.join('\n') === lines.join('\n')) return;
      lines = result.lines;
      draw();
    } catch {
      say('Could not read logs. Retrying.', 'text-destructive');
    }
  };

  const describe = async () => {
    try {
      const response = await fetch('/_parcela/environments', { cache: 'no-store', signal: AbortSignal.timeout(5000) });
      const found = (await response.json()).environments.find(row => row.key === environment);
      if (!found) return;
      const title = `${found.project} · ${found.name}`;
      $('title').textContent = title;
      document.title = `${title} logs`;
      const badge = $(`badge-${TONES[found.state] || 'warning'}`).content.firstElementChild.cloneNode(true);
      badge.textContent = found.state;
      $('state').replaceChildren(badge);
    } catch { /* The next poll tries again. */ }
  };

  $('filter').addEventListener('input', draw);
  $('wrap').addEventListener('click', () => {
    const on = $('wrap').getAttribute('aria-pressed') !== 'true';
    $('wrap').setAttribute('aria-pressed', String(on));
    $('output').classList.toggle('whitespace-pre-wrap', on);
    $('output').classList.toggle('break-words', on);
  });
  if (!environment) { say('No environment chosen.', 'text-destructive'); return; }
  load();
  describe();
  setInterval(load, 1500);
  setInterval(describe, 5000);
})();
