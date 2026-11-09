(() => {
  'use strict';
  if (document.getElementById('parcela-switcher')) return;
  if (!document.currentScript) return;
  const page = document.currentScript.hasAttribute('data-page');
  // A shared name is read-only and serves no logs or opened files; the server enforces it.
  const shared = !location.hostname.endsWith('.localhost');
  const dashboard = '__PARCELA_DASHBOARD__';
  const RECENT_MS = 15000;
  const TOAST_MS = 8000;
  const SEEN = 'parcela:seen-opens';
  const CLEARED = 'parcela:cleared-opens';
  const TAB = 'parcela:tab';
  const LARGE = 'parcela:large';
  const POSITION = 'parcela:position';

  const EDGES = ['top', 'right', 'bottom', 'left'];
  const MARGIN = 8;
  const RADIUS = 12;
  const GAP = 6;
  // Within this many pixels of a corner, the badge settles in it.
  const CORNER_PULL = 24;

  const STYLE = `
    :host{all:initial;color-scheme:dark;--surface:#17181b;--raised:#26272b;--hover:#313237;--selected:#3a3b41;--well:#101114;--line:#ffffff1a;--text:#ecece8;--muted:#9a9ca3;--ready:#58b878;--busy:#dda35a;--failed:#ef7f68;font:13px/1.4 system-ui,-apple-system,sans-serif;color:var(--text)}
    *{box-sizing:border-box}
    button,select{font:inherit;color:inherit;border:0;background:none;cursor:pointer}
    button:focus-visible,a:focus-visible,select:focus-visible,section:focus-visible{outline:2px solid var(--text);outline-offset:2px}
    [hidden]{display:none !important}
    .badge{position:fixed;z-index:2147483647;display:flex;align-items:center;justify-content:center;min-width:40px;height:40px;padding:0 8px;border-radius:12px;background:#1a1a1a;box-shadow:0 0 0 1px #ffffff1f,0 2px 10px #15171b33;touch-action:none;user-select:none;-webkit-user-select:none}
    .badge:hover{background:#2e2e2e}
    .badge.dragging{cursor:grabbing;box-shadow:0 0 0 1px #ffffff1f,0 6px 20px #15171b33}
    .badge img{display:block;width:24px;height:auto;aspect-ratio:96/85;pointer-events:none;transform:translateY(-.5px)}
    .wordmark{color:#ecece8;font:500 12px/18px ui-monospace,SFMono-Regular,Menlo,monospace}
    .badge .count{position:absolute;top:-4px;right:-4px;min-width:15px;padding:0 4px;border-radius:8px;background:#b45d3c;color:#fff;font:600 10px/15px system-ui,-apple-system,sans-serif;text-align:center}
    .hidden-label{position:absolute;width:1px;height:1px;overflow:hidden;clip-path:inset(50%);white-space:nowrap}
    :host([data-count-x=left]) .badge .count{right:auto;left:-4px}
    :host([data-count-y=bottom]) .badge .count{top:auto;bottom:-4px}
    .guide{position:fixed;z-index:2147483646;width:40px;height:40px;border:1.5px dashed #b45d3c;border-radius:12px;background:#b45d3c33;pointer-events:none}
    .panel{position:fixed;z-index:2147483647;width:min(440px,calc(100vw - 16px));max-height:min(560px,calc(100dvh - 70px));display:flex;flex-direction:column;border-radius:14px;background:var(--surface);box-shadow:0 0 0 1px var(--line),0 16px 48px #0000004d;overflow:hidden}
    .panel.tall{height:min(560px,calc(100dvh - 70px))}
    .panel.large{inset:8px;width:auto;height:auto;max-height:none}
    .panel.page{position:static;width:100%;height:100dvh;max-height:none;border-radius:0;box-shadow:none}
    .bar{flex:none;display:flex;align-items:center;gap:2px;padding:8px}
    .bar .tabs{margin-right:6px}
    .tabs{flex:1;display:flex;gap:2px;padding:3px;border-radius:10px;background:var(--well)}
    .tabs button{flex:1;display:flex;align-items:center;justify-content:center;gap:6px;padding:5px 8px;border-radius:8px;font-weight:550;font-size:12px;color:var(--muted)}
    .tabs button:hover{color:var(--text)}
    .tabs button[aria-selected=true]{background:var(--selected);color:var(--text)}
    .tabs svg{width:14px;height:14px}
    .title{flex:1;padding-left:8px;font-weight:600}
    .icon{flex:none;display:inline-flex;align-items:center;justify-content:center;width:28px;height:28px;border-radius:8px;color:var(--muted);text-decoration:none}
    .icon:hover{background:var(--hover);color:var(--text)}
    .icon.danger:hover{color:var(--failed)}
    .icon:disabled{opacity:.4;cursor:default;background:none;color:var(--muted)}
    .icon[aria-pressed=true]{background:var(--selected);color:var(--text)}
    svg{display:block;width:16px;height:16px;fill:none;stroke:currentColor;stroke-width:2;stroke-linecap:round;stroke-linejoin:round}
    .view{flex:1;min-height:0;display:flex;flex-direction:column}
    .toolbar{flex:none;display:flex;align-items:center;gap:6px;padding:0 8px 8px}
    .action{padding:5px 10px;border-radius:8px;background:var(--raised);font-size:12px;font-weight:550;color:var(--text);text-decoration:none;white-space:nowrap}
    .action:hover{background:var(--hover)}
    .scroll{flex:1;min-height:0;overflow:auto;padding:0 6px 8px}
    ol{list-style:none;margin:0;padding:0}
    li{display:flex;align-items:center;gap:6px;border-radius:9px}
    .tray{flex:none;margin:0 8px 8px;padding:2px 2px 4px;border-radius:10px;background:var(--raised)}
    .tray header{display:flex;align-items:center;justify-content:space-between;padding-right:2px}
    .tray h3{margin:0 8px}
    .tray ol{max-height:136px;overflow:auto}
    .opened li:hover{background:var(--hover)}
    .opened a{flex:1;min-width:0;display:flex;align-items:baseline;gap:10px;padding:6px 4px 6px 8px;color:inherit;text-decoration:none}
    .label{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
    .when{flex:none;color:var(--muted);font-size:12px}
    .empty{margin:0;padding:8px 10px 16px;color:var(--muted)}
    h3{margin:6px 10px 4px;font-size:11px;font-weight:600;letter-spacing:.06em;text-transform:uppercase;color:var(--muted)}
    .environment{padding:6px 4px 6px 10px}
    .environment.current{background:var(--raised)}
    .dot{flex:none;width:8px;height:8px;border-radius:50%;background:#5c5e64}
    .dot.ready{background:var(--ready)}.dot.busy{background:var(--busy)}.dot.failed{background:var(--failed)}
    .who{flex:1;min-width:0}
    .who strong{display:block;font-weight:600;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
    .who small{display:block;color:var(--muted);font-size:12px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
    .tag{margin-left:6px;padding:1px 6px;border-radius:6px;background:var(--selected);font-size:11px;font-weight:550;color:var(--muted)}
    .buttons{flex:none;display:flex;gap:2px}
    select{flex:1;min-width:0;padding:6px 8px;border-radius:8px;background:var(--raised);font-size:12px}
    .filter{flex:1;min-width:0;padding:6px 9px;border:0;border-radius:8px;background:var(--raised);font:inherit;font-size:12px;color:inherit}
    .filter::placeholder{color:var(--muted)}
    .chips{flex:none;display:flex;gap:4px;flex-wrap:wrap;padding:0 8px 8px}
    .chips button{padding:3px 9px;border-radius:7px;background:var(--raised);font-size:12px;color:var(--muted)}
    .chips button[aria-pressed=true]{background:var(--selected);color:var(--text)}
    pre{flex:1;min-height:0;margin:0 8px 8px;padding:8px 0;overflow:auto;border-radius:10px;background:var(--well);font:11.5px/1.55 ui-monospace,SFMono-Regular,Menlo,monospace;white-space:pre}
    pre.wrap{white-space:pre-wrap;word-break:break-word}
    pre div{padding:0 10px}
    pre div:hover{background:var(--raised)}
    pre .error{background:#3a1f1a}
    pre .warn{background:#33291a}
    .t,.k{color:#7d8087}.m{font-weight:700}.ok{color:#6fc58a}.redirect,.d{color:#7ba7f0}.client{color:#dda35a}.server,.e{color:#ef7f68;font-weight:600}
    .toast{position:fixed;z-index:2147483647;width:min(340px,calc(100vw - 16px));display:flex;align-items:center;gap:8px;padding:8px 8px 8px 14px;border-radius:12px;background:var(--surface);box-shadow:0 0 0 1px var(--line),0 12px 36px #0000004d}
    .toast span{flex:1;min-width:0}
    .toast small{display:block;color:var(--muted);font-size:11px}
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

  // Built node by node: an app's Trusted Types policy can refuse innerHTML.
  const GLYPHS = {
    close: ['M6 6l12 12', 'M18 6L6 18'],
    expand: ['M14 4h6v6', 'M20 4l-6 6', 'M10 20H4v-6', 'M4 20l6-6'],
    collapse: ['M4 14h6v6', 'M10 14l-6 6', 'M20 10h-6V4', 'M14 10l6-6'],
    external: ['M14 4h6v6', 'M20 4l-9 9', 'M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5'],
    grid: ['M4 4h7v7H4z', 'M13 4h7v7h-7z', 'M4 13h7v7H4z', 'M13 13h7v7h-7z'],
    layers: ['M12 3l9 5-9 5-9-5z', 'M3 13l9 5 9-5'],
    terminal: ['M4 17l6-5-6-5', 'M12 19h8'],
    start: ['M7 4.5v15l12-7.5z'],
    stop: ['M6 6h12v12H6z'],
    restart: ['M21 12a9 9 0 1 1-2.64-6.36L21 8', 'M21 3v5h-5'],
    trash: ['M4 7h16', 'M9 7V4h6v3', 'M6 7l1 13h10l1-13'],
    wrap: ['M4 6h16', 'M4 12h13a3 3 0 0 1 0 6h-4', 'M15 15l-3 3 3 3', 'M4 18h5'],
  };
  const glyph = name => {
    const SVG = 'http://www.w3.org/2000/svg';
    const svg = document.createElementNS(SVG, 'svg');
    svg.setAttribute('viewBox', '0 0 24 24');
    svg.setAttribute('aria-hidden', 'true');
    for (const d of GLYPHS[name]) {
      const path = document.createElementNS(SVG, 'path');
      path.setAttribute('d', d);
      svg.append(path);
    }
    return svg;
  };
  const named = (node, name, label, danger) => {
    node.className = `icon${danger ? ' danger' : ''}`;
    node.title = label;
    node.setAttribute('aria-label', label);
    node.replaceChildren(glyph(name));
    return node;
  };
  const iconButton = (name, label, onclick, danger) => named(element('button', { onclick }), name, label, danger);
  const iconLink = (name, label, href) => named(link(href), name, label);

  let anchor = () => {};

  // An app whose CSP blocks same-origin images still gets a badge it can see.

  const icon = () => {
    const image = element('img', { src: '/_parcela/badge.png', alt: '', draggable: false });
    image.addEventListener('error', () => image.replaceWith(element('span', { className: 'wordmark', textContent: 'parcela' })));
    return image;
  };

  const mount = () => {
    if (document.getElementById('parcela-switcher')) return;
    const host = element('div', { id: 'parcela-switcher' });
    const shadow = host.attachShadow({ mode: 'open' });
    const count = element('span', { className: 'count', hidden: true });
    const badge = element('button', { className: 'badge', title: 'Parcela' }, icon(), element('span', { className: 'hidden-label', textContent: 'Parcela' }), count);
    badge.setAttribute('aria-expanded', 'false');
    badge.setAttribute('aria-controls', 'parcela-panel');

    const tray = shared ? null : opened(count);
    const views = shared ? { environments: environments(tray) } : { environments: environments(tray), logs: logs() };
    const labels = { environments: ['layers', 'Environments'], logs: ['terminal', 'Logs'] };
    const tabs = element('div', { className: 'tabs', role: 'tablist' });
    let current = localStorage.getItem(TAB) in views ? localStorage.getItem(TAB) : 'environments';
    const select = name => {
      current = name;
      localStorage.setItem(TAB, name);
      panel.classList.toggle('tall', name === 'logs');
      for (const [key, view] of Object.entries(views)) {
        view.element.hidden = key !== name;
        if (key !== name) view.hidden?.();
        tabs.querySelector(`[data-tab=${key}]`).setAttribute('aria-selected', String(key === name));
      }
      views[name].shown();
    };
    for (const name of Object.keys(views)) {
      const button = element('button', { role: 'tab', onclick: () => select(name) }, glyph(labels[name][0]), labels[name][1]);
      button.dataset.tab = name;
      tabs.append(button);
    }
    const close = iconButton('close', 'Close', () => hide());
    const large = localStorage.getItem(LARGE) === 'true';
    const grow = iconButton(large ? 'collapse' : 'expand', large ? 'Shrink' : 'Expand', () => {
      const now = panel.classList.toggle('large');
      localStorage.setItem(LARGE, String(now));
      named(grow, now ? 'collapse' : 'expand', now ? 'Shrink' : 'Expand');
      anchor(panel);
    });
    const away = iconButton('external', 'Open in a new tab', () => {
      window.open(`/_parcela/#${current}${views.logs?.chosen() ? `:${encodeURIComponent(views.logs.chosen())}` : ''}`, '_blank', 'noopener');
      hide();
    });
    const manage = location.hostname.endsWith('.localhost') && dashboard.startsWith('https://')
      ? iconLink('grid', 'All environments', `${dashboard}/`)
      : null;
    const panel = element('section', { id: 'parcela-panel', className: `panel${page ? ' page' : large ? ' large' : ''}`, hidden: true, tabIndex: -1 },
      element('div', { className: 'bar' },
        Object.keys(views).length > 1 ? tabs : element('span', { className: 'title', textContent: 'Environments' }),
        manage, ...(page ? [] : [grow, away, close])),
      ...Object.values(views).map(view => view.element));
    // Its height follows what it lists, so a panel above the badge has to move as that changes.
    new ResizeObserver(() => { if (expanded()) anchor(panel); }).observe(panel);
    panel.setAttribute('aria-label', 'Parcela');
    views.environments.onLogs = key => { views.logs?.choose(key); select('logs'); };

    const expanded = () => badge.getAttribute('aria-expanded') === 'true';
    const hide = () => {
      panel.hidden = true;
      badge.setAttribute('aria-expanded', 'false');
      Object.values(views).forEach(view => view.hidden?.());
    };
    // Where the badge rests: an edge of the window, and how far along it, from 0 to 1.
    const clamp = (value, low, high) => Math.min(Math.max(value, low), Math.max(low, high));
    const resting = () => {
      try {
        const { edge, at } = JSON.parse(localStorage.getItem(POSITION));
        if (EDGES.includes(edge) && at >= 0 && at <= 1) return { edge, at };
      } catch {}
      return { edge: 'top', at: 1 };
    };
    let position = resting();
    const css = ({ edge, at }) => {
      // Flush with the edge, and with the next edge at either end: only free corners are rounded.
      const free = (edges, end) => (edges.includes(edge) || at === end ? 0 : RADIUS);
      const radius = edge === 'top' || edge === 'bottom'
        ? [free(['top'], 0), free(['top'], 1), free(['bottom'], 1), free(['bottom'], 0)]
        : [free(['left'], 0), free(['right'], 0), free(['right'], 1), free(['left'], 1)];
      const along = edge === 'top' || edge === 'bottom'
        ? `left:${at * 100}%;transform:translateX(${-at * 100}%)`
        : `top:${at * 100}%;transform:translateY(${-at * 100}%)`;
      return `${edge}:0;${along};border-radius:${radius.join('px ')}px`;
    };
    // The count sits on a corner of the badge that is away from the window's edges.
    const settle = ({ edge, at }) => {
      const horizontal = edge === 'top' || edge === 'bottom';
      host.dataset.countX = edge === 'right' || (horizontal && at >= 0.5) ? 'left' : 'right';
      host.dataset.countY = edge === 'top' || (!horizontal && at < 0.5) ? 'bottom' : 'top';
    };
    const place = next => {
      position = next;
      badge.style.cssText = css(next);
      settle(next);
      localStorage.setItem(POSITION, JSON.stringify(next));
      if (expanded()) anchor(panel);
    };
    // Beside the badge, kept inside the window. An expanded panel fills the window instead.
    anchor = target => {
      if (target.classList.contains('large')) { target.style.inset = ''; return; }
      const from = badge.getBoundingClientRect();
      const [width, height] = [target.offsetWidth, target.offsetHeight];
      const left = { left: from.right + GAP, right: from.left - GAP - width }[position.edge] ?? from.left + from.width / 2 - width / 2;
      const top = { top: from.bottom + GAP, bottom: from.top - GAP - height }[position.edge] ?? from.top + from.height / 2 - height / 2;
      target.style.inset = `${clamp(top, MARGIN, innerHeight - height - MARGIN)}px auto auto ${clamp(left, MARGIN, innerWidth - width - MARGIN)}px`;
    };
    badge.style.cssText = css(position);
    settle(position);
    addEventListener('resize', () => {
      if (expanded()) anchor(panel);
      if (tray && !tray.toast.hidden) anchor(tray.toast);
    });

    // A small movement is still a click. Past that the badge follows the pointer, and lands on the
    // nearest edge at the point it was dropped; the guide shows where.
    const guide = element('div', { className: 'guide', hidden: true });
    const destination = event => {
      const distances = { top: event.clientY, right: innerWidth - event.clientX, bottom: innerHeight - event.clientY, left: event.clientX };
      const edge = EDGES.reduce((nearest, next) => (distances[next] < distances[nearest] ? next : nearest));
      const { width, height } = badge.getBoundingClientRect();
      const horizontal = edge === 'top' || edge === 'bottom';
      const size = horizontal ? width : height;
      const room = (horizontal ? innerWidth : innerHeight) - size;
      const offset = (horizontal ? event.clientX : event.clientY) - size / 2;
      if (offset < CORNER_PULL) return { edge, at: 0 };
      if (offset > room - CORNER_PULL) return { edge, at: 1 };
      return { edge, at: Math.round(offset / room * 1000) / 1000 };
    };
    let press = null;
    let dragged = false;
    const release = () => {
      badge.classList.remove('dragging');
      badge.style.cssText = css(position);
      guide.hidden = true;
      press = null;
    };
    badge.addEventListener('pointerdown', event => {
      if (event.button !== 0) return;
      press = { x: event.clientX, y: event.clientY, moving: false };
      dragged = false;
      badge.setPointerCapture(event.pointerId);
    });
    badge.addEventListener('pointermove', event => {
      if (!press) return;
      if (!press.moving && Math.hypot(event.clientX - press.x, event.clientY - press.y) < 6) return;
      if (!press.moving) {
        press.moving = dragged = true;
        hide();
        badge.classList.add('dragging');
        guide.hidden = false;
      }
      guide.style.cssText = css(destination(event));
      const { width, height } = badge.getBoundingClientRect();
      const left = clamp(event.clientX - width / 2, 0, innerWidth - width);
      const top = clamp(event.clientY - height / 2, 0, innerHeight - height);
      badge.style.cssText = `inset:${top}px auto auto ${left}px`;
    });
    badge.addEventListener('pointerup', event => {
      if (press?.moving) place(destination(event));
      release();
    });
    badge.addEventListener('pointercancel', release);
    // Arrow keys walk along an edge and round its ends, or cross straight to the opposite edge.
    badge.addEventListener('keydown', event => {
      const direction = { ArrowUp: 'top', ArrowRight: 'right', ArrowDown: 'bottom', ArrowLeft: 'left' }[event.key];
      if (!direction) return;
      event.preventDefault();
      const { edge, at } = position;
      const horizontal = edge === 'top' || edge === 'bottom';
      const alongEdge = horizontal === (direction === 'left' || direction === 'right');
      if (!alongEdge) { place({ edge: direction, at }); return; }
      const forward = direction === 'right' || direction === 'bottom';
      const next = Math.round((at + (forward ? 0.1 : -0.1)) * 10) / 10;
      if (next >= 0 && next <= 1) place({ edge, at: next });
      else place({ edge: direction, at: edge === 'top' || edge === 'left' ? 0 : 1 });
    });
    badge.addEventListener('click', () => {
      if (dragged) { dragged = false; return; }
      if (expanded()) { hide(); return; }
      if (tray) tray.toast.hidden = true;
      badge.setAttribute('aria-expanded', 'true');
      panel.hidden = false;
      anchor(panel);
      panel.focus();
      // New items appear on Environments, so that is where the count leads.
      select(count.hidden ? current : 'environments');
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
    shadow.append(...style, ...(page ? [] : [guide, badge]), panel, ...(page || !tray ? [] : [tray.toast]));
    document.body.append(host);
    if (page) {
      const [tab, key] = location.hash.slice(1).split(':');
      if (key) views.logs?.choose(decodeURIComponent(key));
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
    const clear = iconButton('trash', 'Clear all', () => forget(visible().map(open => open.id)));
    // Only there when the app has opened something.
    const element_ = element('div', { className: 'tray', hidden: true },
      element('header', {}, element('h3', { textContent: 'Opened by your app' }), clear),
      items);
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
      count.textContent = String(unseen);
      element_.hidden = opens.length === 0;
      items.replaceChildren(...opens.map(open => {
        const remove = iconButton('close', `Remove ${open.label}`, () => forget([open.id]));
        return element('li', {},
          link(open.href,
            element('span', { className: 'label', textContent: open.label, title: open.label }),
            element('span', { className: 'when', textContent: ago(latest.now, open.at_unix_ms) })),
          remove);
      }));
    };
    const remember = ids => { ids.forEach(id => seen.add(id)); store(sessionStorage, SEEN, seen); render(); };
    const forget = ids => { ids.forEach(id => cleared.add(id)); store(localStorage, CLEARED, cleared); render(); };

    const announce = open => {
      const action = link(open.href, 'Open');
      action.className = 'action';
      action.addEventListener('click', () => { toast.hidden = true; remember([open.id]); });
      toast.replaceChildren(
        element('span', {}, element('small', { textContent: 'Your app opened' }), element('strong', { textContent: open.label, title: open.label })),
        action);
      toast.hidden = false;
      anchor(toast);
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

  // Behind a front proxy, a sibling answers on the scheme and port this page came through.
  const sameEntry = url => {
    if (!shared) return url;
    const target = new URL(url);
    target.protocol = location.protocol;
    target.port = location.port;
    return target.href;
  };

  const environments = tray => {
    const list = element('div', { className: 'scroll' });
    const view = {
      element: element('div', { className: 'view', hidden: true }, tray?.element, list),
      onLogs: null,
    };
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
      const button = (name, label, onclick, danger) => Object.assign(iconButton(name, label, onclick, danger), { disabled: running });
      const buttons = [];
      if (ready && environment.url && !environment.current) buttons.push(iconLink('external', 'Open', sameEntry(environment.url)));
      // Pages act only on registered environments they may manage: an app page its own, the
      // dashboard's page every one. `parcela up` in a checkout registers it.
      if (environment.key && environment.manageable) {
        if (stopped) buttons.push(button('start', 'Start', () => act(environment, 'start')));
        else buttons.push(button('restart', 'Restart', () => act(environment, 'restart')), button('stop', 'Stop', () => act(environment, 'stop'), true));
        if (environment.processes.length) buttons.push(button('terminal', 'Logs', () => view.onLogs?.(environment.key)));
      }
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
    view.shown = () => { tray?.shown(); refresh(); clearInterval(timer); timer = setInterval(refresh, 2000); };
    view.hidden = () => { tray?.hidden(); clearInterval(timer); };
    return view;
  };

  const PALETTE = { 30: '#9a9ca3', 31: '#ef7f68', 32: '#6fc58a', 33: '#dda35a', 34: '#7ba7f0', 35: '#c792ea', 36: '#5fc3d6', 37: '#d6d6d2' };
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
    const wrap = iconButton('wrap', 'Wrap lines');
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
