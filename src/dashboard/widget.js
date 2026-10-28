(() => {
  'use strict';
  if (document.getElementById('parcela-switcher')) return;
  const script = document.currentScript;
  if (!script) return;
  const origin = new URL(script.src).origin;
  const mount = () => {
    if (document.getElementById('parcela-switcher')) return;
    const host = document.createElement('div');
    host.id = 'parcela-switcher';
    const shadow = host.attachShadow({ mode: 'open' });
    const style = document.createElement('style');
    style.textContent = ':host{all:initial;position:fixed;right:0;top:35%;z-index:2147483647;font-family:system-ui,sans-serif}button{font:600 11px system-ui;color:#355232;background:#eef3e3;border:1px solid #bacbad;border-right:0;border-radius:8px 0 0 8px;padding:14px 9px;cursor:pointer;writing-mode:vertical-rl;letter-spacing:1px}button:focus-visible{outline:2px solid #477b57;outline-offset:3px}iframe{position:fixed;right:12px;top:12px;width:min(390px,calc(100vw - 24px));height:calc(100dvh - 24px);border:1px solid #c1cdb7;border-radius:12px;box-shadow:0 12px 60px #20301940;background:#fffefa}iframe[hidden]{display:none}';
    const button = document.createElement('button');
    button.textContent = 'parcela';
    button.setAttribute('aria-label', 'Open Parcela environment switcher');
    button.setAttribute('aria-expanded', 'false');
    button.setAttribute('aria-controls', 'parcela-panel');
    const frame = document.createElement('iframe');
    frame.id = 'parcela-panel';
    frame.title = 'Parcela environment switcher';
    frame.hidden = true;
    frame.referrerPolicy = 'no-referrer';
    const close = () => { frame.hidden = true; button.setAttribute('aria-expanded', 'false'); button.focus(); };
    button.addEventListener('click', () => {
      if (!frame.hidden) { close(); return; }
      if (!frame.src) frame.src = `${origin}/?embed=1#from=${encodeURIComponent(location.origin)}`;
      frame.hidden = false;
      button.setAttribute('aria-expanded', 'true');
      frame.focus();
    });
    window.addEventListener('message', event => {
      if (event.origin === origin && event.source === frame.contentWindow && event.data?.type === 'parcela:close') close();
    });
    document.addEventListener('keydown', event => { if (event.key === 'Escape' && !frame.hidden) close(); });
    shadow.append(style, button, frame);
    document.body.append(host);
  };
  if (document.body) mount();
  else document.addEventListener('DOMContentLoaded', mount, { once: true });
})();
