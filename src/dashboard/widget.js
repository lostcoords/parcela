(() => {
  'use strict';
  if (document.getElementById('parcela-switcher')) return;
  if (!document.currentScript) return;
  const page = document.currentScript.hasAttribute('data-page');
  const dashboard = '__PARCELA_DASHBOARD__';
  const RECENT_MS = 15000;
  const TOAST_MS = 8000;
  const SEEN = 'parcela:seen-opens';
  const CLEARED = 'parcela:cleared-opens';
  const TAB = 'parcela:tab';
  const LARGE = 'parcela:large';

  const STYLE = `
    :host{all:initial;font:13px/1.4 system-ui,-apple-system,sans-serif;color:#15171b}
    *{box-sizing:border-box}
    button,select{font:inherit;color:inherit;border:0;background:none;cursor:pointer}
    button:focus-visible,a:focus-visible,select:focus-visible,section:focus-visible{outline:2px solid #15171b;outline-offset:2px}
    [hidden]{display:none !important}
    .badge{position:fixed;z-index:2147483647;top:0;right:0;display:flex;align-items:center;gap:6px;padding:4px 10px;border-radius:0 0 0 9px;background:#ecece8;font:500 12px/1.4 ui-monospace,SFMono-Regular,Menlo,monospace;box-shadow:0 2px 10px #15171b14}
    .badge:hover{background:#e2e2dd}
    .muted{color:#6b6e75}
    .panel{position:fixed;z-index:2147483647;top:30px;right:8px;width:min(440px,calc(100vw - 16px));height:min(560px,calc(100dvh - 40px));display:flex;flex-direction:column;border-radius:14px;background:#fff;box-shadow:0 16px 48px #15171b26,0 2px 8px #15171b14;overflow:hidden}
    .panel.large{left:8px;width:auto;height:calc(100dvh - 38px)}
    .panel.page{position:static;width:100%;height:100dvh;border-radius:0;box-shadow:none}
    .bar{display:flex;align-items:center;gap:4px;padding:10px 10px 8px 12px}
    .bar .tabs{margin-right:4px}
    .tabs{flex:1;display:flex;gap:2px;padding:3px;border-radius:10px;background:#f1f1ee}
    .tabs button{flex:1;padding:5px 8px;border-radius:8px;font-weight:550;font-size:12px;color:#6b6e75}
    .tabs button[aria-selected=true]{background:#fff;color:#15171b;box-shadow:0 1px 3px #15171b1f}
    .close{width:28px;height:28px;border-radius:8px;color:#6b6e75;font-size:16px}
    .close:hover{background:#f1f1ee;color:#15171b}
    .view{flex:1;min-height:0;display:flex;flex-direction:column}
    .toolbar{display:flex;align-items:center;gap:6px;padding:2px 12px 8px}
    .toolbar .grow{flex:1}
    .action{padding:5px 10px;border-radius:8px;background:#f1f1ee;font-size:12px;font-weight:550;color:#15171b;text-decoration:none;white-space:nowrap}
    .action:hover{background:#e6e6e1}
    .action:disabled{opacity:.45;cursor:default}
    .scroll{flex:1;min-height:0;overflow:auto;padding:0 6px 8px}
    ol{list-style:none;margin:0;padding:0}
    li{display:flex;align-items:center;gap:6px;border-radius:9px}
    .opened li:hover{background:#f4f4f1}
    .opened a{flex:1;min-width:0;display:flex;align-items:baseline;gap:10px;padding:9px 4px 9px 10px;color:inherit;text-decoration:none}
    .label{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
    .when{flex:none;color:#6b6e75;font-size:12px}
    .remove{flex:none;width:30px;height:30px;border-radius:7px;color:#6b6e75;font-size:16px}
    .remove:hover{background:#e6e6e1;color:#15171b}
    .empty{margin:0;padding:8px 10px 16px;color:#6b6e75}
    h3{margin:10px 10px 4px;font-size:11px;font-weight:600;letter-spacing:.06em;text-transform:uppercase;color:#6b6e75}
    .environment{padding:8px 6px 8px 10px}
    .environment.current{background:#f4f4f1}
    .dot{flex:none;width:8px;height:8px;border-radius:50%;background:#c6c7c2}
    .dot.ready{background:#3f9b5b}.dot.busy{background:#d79a2b}.dot.failed{background:#c8553d}
    .who{flex:1;min-width:0}
    .who strong{display:block;font-weight:600;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
    .who small{display:block;color:#6b6e75;font-size:12px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
    .tag{margin-left:6px;padding:1px 6px;border-radius:6px;background:#ecece8;font-size:11px;font-weight:550;color:#6b6e75}
    .buttons{flex:none;display:flex;gap:4px}
    select{flex:1;min-width:0;padding:6px 8px;border-radius:8px;background:#f1f1ee;font-size:12px}
    .filter{flex:1;min-width:0;padding:6px 9px;border:0;border-radius:8px;background:#f1f1ee;font:inherit;font-size:12px;color:inherit}
    .toggle[aria-pressed=true]{background:#15171b;color:#fff}
    .chips{display:flex;gap:4px;flex-wrap:wrap;padding:0 12px 8px}
    .chips button{padding:3px 9px;border-radius:7px;background:#f1f1ee;font-size:12px;color:#6b6e75}
    .chips button[aria-pressed=true]{background:#15171b;color:#fff}
    pre{flex:1;min-height:0;margin:0 8px 8px;padding:8px 0;overflow:auto;border-radius:10px;background:#f6f6f3;font:11.5px/1.55 ui-monospace,SFMono-Regular,Menlo,monospace;white-space:pre}
    pre.wrap{white-space:pre-wrap;word-break:break-word}
    pre div{padding:0 10px}
    pre div:hover{background:#ecece8}
    pre .error{background:#fbeae6}
    pre .warn{background:#fbf3e2}
    .t,.k{color:#8a8d93}.m{font-weight:700}.ok{color:#2f7d48}.redirect{color:#3b6fd8}.client{color:#a8681a}.server,.e{color:#b4432f;font-weight:600}.d{color:#3b6fd8}
    .toast{position:fixed;z-index:2147483647;top:30px;right:8px;width:min(340px,calc(100vw - 16px));display:flex;align-items:center;gap:8px;padding:8px 8px 8px 14px;border-radius:12px;background:#fff;box-shadow:0 12px 36px #15171b26,0 2px 6px #15171b14}
    .toast span{flex:1;min-width:0}
    .toast small{display:block;color:#6b6e75;font-size:11px}
    .toast strong{display:block;font-weight:550;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  `;

  const element = (tag, properties = {}, ...children) => {
    const node = Object.assign(document.createElement(tag), properties);
    node.append(...children.filter(child => child != null));
    return node;
  };
  const stored = (storage, key) => {
    try { return new Set(JSON.parse(storage.getItem(key) || '[]')); } catch { return new Set(); }
  };
  const store = (storage, key, set) => storage.setItem(key, JSON.stringify([...set].slice(-100)));
  const ago = (now, at) => {
    const minutes = Math.floor((now - at) / 60000);
    if (minutes < 1) return 'just now';
    if (minutes < 60) return `${minutes} min ago`;
    const hours = Math.floor(minutes / 60);
    return hours < 24 ? `${hours} h ago` : `${Math.floor(hours / 24)} d ago`;
  };
  const link = (href, ...children) => element('a', { href, target: '_blank', rel: 'noopener' }, ...children);
  const fetchJson = async (path, options) => {
    const response = await fetch(path, { cache: 'no-store', credentials: 'omit', ...options });
    if (!response.ok) throw new Error(await response.text());
    return response.json();
  };
  const ANSI = new RegExp(String.fromCharCode(27) + '\\[[0-9;:]*[A-Za-z]', 'g');

  const mount = () => {
    if (document.getElementById('parcela-switcher')) return;
    const host = element('div', { id: 'parcela-switcher' });
    const shadow = host.attachShadow({ mode: 'open' });
    const count = element('span', { className: 'muted', hidden: true });
    const badge = element('button', { className: 'badge' }, 'parcela', count);
    badge.setAttribute('aria-expanded', 'false');
    badge.setAttribute('aria-controls', 'parcela-panel');

    const views = { environments: environments(), opened: opened(count), logs: logs() };
    const labels = { environments: 'Environments', opened: 'Opened', logs: 'Logs' };
    const tabs = element('div', { className: 'tabs', role: 'tablist' });
    let current = localStorage.getItem(TAB) in views ? localStorage.getItem(TAB) : 'environments';
    const select = name => {
      current = name;
      localStorage.setItem(TAB, name);
      for (const [key, view] of Object.entries(views)) {
        view.element.hidden = key !== name;
        if (key !== name) view.hidden?.();
        tabs.querySelector(`[data-tab=${key}]`).setAttribute('aria-selected', String(key === name));
      }
      views[name].shown();
    };
    for (const name of Object.keys(views)) {
      const button = element('button', { role: 'tab', textContent: labels[name], onclick: () => select(name) });
      button.dataset.tab = name;
      tabs.append(button);
    }
    const close = element('button', { className: 'close', textContent: '×', title: 'Close', onclick: () => hide() });
    close.setAttribute('aria-label', 'Close Parcela');
    const grow = element('button', { className: 'close', textContent: '⤢', title: 'Expand', onclick: () => {
      panel.classList.toggle('large');
      localStorage.setItem(LARGE, String(panel.classList.contains('large')));
    } });
    grow.setAttribute('aria-label', 'Expand Parcela');
    const away = element('button', { className: 'close', textContent: '↗', title: 'Open in a new tab', onclick: () => {
      window.open(`/_parcela/#${current}${views.logs.chosen() ? `:${encodeURIComponent(views.logs.chosen())}` : ''}`, '_blank', 'noopener');
      hide();
    } });
    away.setAttribute('aria-label', 'Open Parcela in a new tab');
    const panel = element('section', { id: 'parcela-panel', className: `panel${page ? ' page' : localStorage.getItem(LARGE) === 'true' ? ' large' : ''}`, hidden: true, tabIndex: -1 },
      element('div', { className: 'bar' }, tabs, ...(page ? [] : [grow, away, close])), ...Object.values(views).map(view => view.element));
    panel.setAttribute('aria-label', 'Parcela');
    views.environments.onLogs = key => { views.logs.choose(key); select('logs'); };

    const expanded = () => badge.getAttribute('aria-expanded') === 'true';
    const hide = () => {
      panel.hidden = true;
      badge.setAttribute('aria-expanded', 'false');
      Object.values(views).forEach(view => view.hidden?.());
    };
    badge.addEventListener('click', () => {
      if (expanded()) { hide(); return; }
      views.opened.toast.hidden = true;
      badge.setAttribute('aria-expanded', 'true');
      panel.hidden = false;
      panel.focus();
      select(current);
    });
    document.addEventListener('keydown', event => {
      if (event.key === 'Escape' && expanded()) { hide(); badge.focus(); }
    });
    document.addEventListener('click', event => {
      if (expanded() && !event.composedPath().includes(host)) hide();
    });

    // A constructed sheet is outside the app's `style-src`; older browsers get a <style>.
    let style = [];
    try {
      const sheet = new CSSStyleSheet();
      sheet.replaceSync(STYLE);
      shadow.adoptedStyleSheets = [sheet];
    } catch {
      style = [element('style', { textContent: STYLE })];
    }
    shadow.append(...style, ...(page ? [] : [badge]), panel, ...(page ? [] : [views.opened.toast]));
    document.body.append(host);
    if (page) {
      const [tab, key] = location.hash.slice(1).split(':');
      if (key) views.logs.choose(decodeURIComponent(key));
      panel.hidden = false;
      select(tab in views ? tab : current);
      return;
    }
    // Turbo and similar libraries swap in a new <body> without running head scripts again.
    new MutationObserver(() => {
      if (!host.isConnected && document.body) document.body.append(host);
    }).observe(document.documentElement, { childList: true });
  };

  const opened = count => {
    const seen = stored(sessionStorage, SEEN);
    const cleared = stored(localStorage, CLEARED);
    const items = element('ol', { className: 'opened' });
    const clear = element('button', { className: 'action', textContent: 'Clear all' });
    const element_ = element('div', { className: 'view', hidden: true },
      element('div', { className: 'toolbar' }, element('span', { className: 'grow' }), clear),
      element('div', { className: 'scroll' }, items));
    const toast = element('div', { className: 'toast', hidden: true, role: 'status' });
    let since = null;
    let latest = { now: 0, opens: [] };
    let toastTimer = null;
    let showing = false;

    const visible = () => latest.opens.filter(open => !cleared.has(open.id)).reverse();
    const render = () => {
      const opens = visible();
      const unseen = opens.filter(open => !seen.has(open.id) && open.at_unix_ms >= since).length;
      count.hidden = unseen === 0;
      count.textContent = `×${unseen}`;
      clear.disabled = opens.length === 0;
      items.replaceChildren(...opens.map(open => {
        const remove = element('button', { className: 'remove', textContent: '×', onclick: () => forget([open.id]) });
        remove.setAttribute('aria-label', `Remove ${open.label}`);
        return element('li', {},
          link(open.href,
            element('span', { className: 'label', textContent: open.label, title: open.label }),
            element('span', { className: 'when', textContent: ago(latest.now, open.at_unix_ms) })),
          remove);
      }));
      if (opens.length === 0) items.replaceChildren(element('p', { className: 'empty', textContent: 'Nothing yet. When your app opens a page or file in a browser, such as a mail preview, it shows up here.' }));
    };
    const remember = ids => { ids.forEach(id => seen.add(id)); store(sessionStorage, SEEN, seen); render(); };
    const forget = ids => { ids.forEach(id => cleared.add(id)); store(localStorage, CLEARED, cleared); render(); };
    clear.addEventListener('click', () => forget(visible().map(open => open.id)));

    const announce = open => {
      const action = link(open.href, 'Open');
      action.className = 'action';
      action.addEventListener('click', () => { toast.hidden = true; remember([open.id]); });
      toast.replaceChildren(
        element('span', {}, element('small', { textContent: 'Your app opened' }), element('strong', { textContent: open.label, title: open.label })),
        action);
      toast.hidden = false;
      clearTimeout(toastTimer);
      toastTimer = setTimeout(() => { toast.hidden = true; }, TOAST_MS);
    };
    const announced = new Set();
    const poll = async () => {
      if (document.visibilityState !== 'visible') return;
      try {
        const { now_unix_ms: now, opens } = await fetchJson('/_parcela/opens');
        since ??= now - RECENT_MS;
        latest = { now, opens };
        const fresh = visible().filter(open => open.at_unix_ms >= since && !seen.has(open.id) && !announced.has(open.id));
        fresh.forEach(open => announced.add(open.id));
        if (showing) remember(visible().map(open => open.id));
        else if (fresh.length) announce(fresh[0]);
        render();
      } catch {}
    };
    render();
    poll();
    setInterval(poll, 1500);
    document.addEventListener('visibilitychange', poll);
    return {
      element: element_,
      toast,
      shown: () => { showing = true; remember(visible().map(open => open.id)); },
      hidden: () => { showing = false; },
    };
  };

  const environments = () => {
    const list = element('div', { className: 'scroll' });
    const view = {
      element: element('div', { className: 'view', hidden: true },
        location.hostname.endsWith('.localhost') && dashboard.startsWith('https://')
          ? element('div', { className: 'toolbar' }, element('span', { className: 'grow' }), link(dashboard, 'Dashboard ↗'))
          : null,
        list),
      onLogs: null,
    };
    view.element.querySelector('.toolbar a')?.classList.add('action');
    let timer = null;
    let failures = {};

    const act = async (environment, name) => {
      failures[environment.key] = null;
      try {
        await fetchJson(`/_parcela/environments/${encodeURIComponent(environment.key)}/${name}`, { method: 'POST', headers: { 'X-Parcela-Action': '1' } });
      } catch (error) {
        failures[environment.key] = error.message.trim();
      }
      refresh();
    };
    const row = environment => {
      const running = environment.action?.state === 'running';
      const ready = environment.state === 'ready';
      const stopped = environment.state === 'stopped' || environment.state === 'unregistered';
      const status = running ? `${environment.action.name === 'stop' ? 'stopping' : 'starting'}…`
        : failures[environment.key] || (environment.action?.state === 'failed' ? `${environment.action.name} failed: ${environment.action.message || 'see logs'}` : environment.state);
      const dot = running || (!ready && !stopped) ? 'busy' : environment.action?.state === 'failed' ? 'failed' : ready ? 'ready' : '';
      const button = (text, onclick) => element('button', { className: 'action', textContent: text, disabled: running, onclick });
      const buttons = [];
      if (ready && environment.url && !environment.current) buttons.push(link(environment.url, 'Open'));
      // Pages act only on registered environments; `parcela up` in a checkout registers it.
      if (environment.key) {
        if (stopped) buttons.push(button('Start', () => act(environment, 'start')));
        else buttons.push(button('Restart', () => act(environment, 'restart')), button('Stop', () => act(environment, 'stop')));
        if (environment.processes.length) buttons.push(button('Logs', () => view.onLogs?.(environment.key)));
      }
      buttons.filter(node => node.tagName === 'A').forEach(node => { node.className = 'action'; });
      return element('li', { className: `environment${environment.current ? ' current' : ''}` },
        element('span', { className: `dot ${dot}` }),
        element('span', { className: 'who' },
          element('strong', {}, environment.name || 'unregistered', environment.current ? element('span', { className: 'tag', textContent: 'this page' }) : null),
          element('small', { textContent: status })),
        element('span', { className: 'buttons' }, ...buttons));
    };
    const refresh = async () => {
      try {
        const { environments } = await fetchJson('/_parcela/environments');
        const projects = new Map();
        for (const environment of environments) {
          if (!projects.has(environment.project)) projects.set(environment.project, []);
          projects.get(environment.project).push(environment);
        }
        list.replaceChildren(...[...projects].flatMap(([project, members]) => [
          element('h3', { textContent: project }),
          element('ol', {}, ...members.map(row)),
        ]));
        if (!environments.length) list.replaceChildren(element('p', { className: 'empty', textContent: 'Parcela is still collecting environments…' }));
      } catch {
        list.replaceChildren(element('p', { className: 'empty', textContent: 'Could not reach Parcela.' }));
      }
    };
    view.shown = () => { refresh(); clearInterval(timer); timer = setInterval(refresh, 2000); };
    view.hidden = () => clearInterval(timer);
    return view;
  };

  const PALETTE = { 30: '#15171b', 31: '#b4432f', 32: '#2f7d48', 33: '#a8681a', 34: '#3b6fd8', 35: '#8c45b8', 36: '#1f7f91', 37: '#6b6e75' };
  const HIGHLIGHT = /(\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?(?:Z|[+-]\d{2}:?\d{2})?)|(\[\d+:[^\]]*\])|\b(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\b|(?:\b(?:status[=:]\s?"?|Completed\s|HTTP\/[\d.]+"?\s)([1-5]\d\d)\b)|(\d+(?:\.\d+)?\s?ms\b)|\b(ERROR|FATAL|Error|Exception|Traceback|panicked|failed|WARN(?:ING)?)\b/g;
  const STATUS = code => ({ 2: 'ok', 3: 'redirect', 4: 'client', 5: 'server' })[code[0]] || '';
  const severity = line => /\b(ERROR|FATAL|Exception|Traceback|panicked)\b|^\S+ \S+ [EF] /.test(line) ? 'error' : /\bWARN(ING)?\b|^\S+ \S+ W /.test(line) ? 'warn' : '';

  const highlight = (text, color, bold) => {
    const parts = [];
    let last = 0;
    const plain = chunk => {
      if (!chunk) return;
      parts.push(color || bold ? element('span', { textContent: chunk, style: `${color ? `color:${color};` : ''}${bold ? 'font-weight:700' : ''}` }) : chunk);
    };
    for (const match of text.matchAll(HIGHLIGHT)) {
      plain(text.slice(last, match.index));
      const [whole, time, tag, method, status, duration, word] = match;
      if (time) parts.push(element('span', { className: 't', textContent: whole }));
      else if (tag) parts.push(element('span', { className: 'k', textContent: whole }));
      else if (method) parts.push(element('span', { className: 'm', textContent: whole }));
      else if (status) {
        plain(whole.slice(0, whole.length - status.length));
        parts.push(element('span', { className: STATUS(status), textContent: status }));
      } else if (duration) parts.push(element('span', { className: 'd', textContent: whole }));
      else if (word) parts.push(element('span', { className: word.startsWith('WARN') ? 'client' : 'e', textContent: whole }));
      last = match.index + whole.length;
    }
    plain(text.slice(last));
    return parts;
  };
  // Programs color their own output with ANSI escapes; keep those colors, then add ours.
  const line = text => {
    const parts = [];
    let color = null;
    let bold = false;
    const segments = text.split(new RegExp(String.fromCharCode(27) + '\\[([0-9;:]*)([A-Za-z])'));
    for (let index = 0; index < segments.length; index += 3) {
      parts.push(...highlight(segments[index], color, bold));
      if (segments[index + 2] !== 'm') continue;
      const params = (segments[index + 1] || '0').split(';');
      for (let at = 0; at < params.length; at++) {
        const [code, ...sub] = params[at].split(':').map(Number);
        // 256-color and truecolor codes carry their own arguments, never basic colors.
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
    return element('div', { className: severity(text.replace(ANSI, '')) }, ...parts);
  };

  const logs = () => {
    const picker = element('select', { ariaLabel: 'Environment' });
    const filter = element('input', { className: 'filter', type: 'search', placeholder: 'Filter lines', ariaLabel: 'Filter log lines' });
    const wrap = element('button', { className: 'action toggle', textContent: 'Wrap' });
    wrap.setAttribute('aria-pressed', 'false');
    const chips = element('div', { className: 'chips' });
    const output = element('pre', { tabIndex: 0 });
    if (matchMedia('(max-width: 640px)').matches) {
      output.classList.add('wrap');
      wrap.setAttribute('aria-pressed', 'true');
    }
    const view = {
      element: element('div', { className: 'view', hidden: true },
        element('div', { className: 'toolbar' }, picker), chips,
        element('div', { className: 'toolbar' }, filter, wrap), output),
    };
    let chosen = null;
    let process = null;
    let timer = null;
    let lines = [];

    const draw = () => {
      const following = output.scrollHeight - output.scrollTop - output.clientHeight < 40;
      const needle = filter.value.trim().toLowerCase();
      const shown = needle ? lines.filter(text => text.replace(ANSI, '').toLowerCase().includes(needle)) : lines;
      output.replaceChildren(...(shown.length ? shown.map(line) : [element('div', { className: 'k', textContent: needle ? 'No matching lines.' : 'No output yet.' })]));
      if (following) output.scrollTop = output.scrollHeight;
    };
    const load = async () => {
      if (!chosen) return;
      const query = new URLSearchParams({ environment: chosen, lines: '600' });
      if (process) query.set('process', process);
      try {
        const result = await fetchJson(`/_parcela/logs?${query}`);
        process = result.process;
        chips.replaceChildren(...result.processes.map(name => {
          const chip = element('button', { textContent: name, onclick: () => { process = name; load(); } });
          chip.setAttribute('aria-pressed', String(name === process));
          return chip;
        }));
        if (result.lines.join('\n') === lines.join('\n')) return;
        lines = result.lines;
        draw();
      } catch {
        output.replaceChildren(element('div', { className: 'e', textContent: 'Could not read logs.' }));
      }
    };
    filter.addEventListener('input', draw);
    wrap.addEventListener('click', () => {
      output.classList.toggle('wrap');
      wrap.setAttribute('aria-pressed', String(output.classList.contains('wrap')));
    });
    picker.addEventListener('change', () => { chosen = picker.value; process = null; lines = []; output.replaceChildren(); load(); });
    view.choose = key => { chosen = key; process = null; lines = []; };
    view.chosen = () => chosen;
    view.shown = async () => {
      try {
        const { environments } = await fetchJson('/_parcela/environments');
        const runnable = environments.filter(environment => environment.key);
        chosen ??= runnable.find(environment => environment.current)?.key ?? runnable[0]?.key ?? null;
        picker.replaceChildren(...runnable.map(environment => element('option', {
          value: environment.key,
          textContent: `${environment.project} · ${environment.name}`,
          selected: environment.key === chosen,
        })));
      } catch {}
      output.scrollTop = output.scrollHeight;
      load();
      clearInterval(timer);
      timer = setInterval(load, 2000);
    };
    view.hidden = () => clearInterval(timer);
    return view;
  };

  if (document.body) mount();
  else document.addEventListener('DOMContentLoaded', mount, { once: true });
})();
