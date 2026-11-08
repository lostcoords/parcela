// parcela.run: the static site, plus /api/telemetry, which takes the CLI's anonymous usage
// events (docs/app/views/pages/telemetry.html.md), keeps only the documented fields and forwards them to
// PostHog. Every other path is served from the built site without running this code.
const POSTHOG = 'https://us.i.posthog.com/i/v0/e/';
const LIMIT = 2048;

const fields = {
  schema: (value) => value === 1,
  event: (value) => value === 'command',
  command: (value) => typeof value === 'string' && /^([a-z]+( [a-z]+)?)?$/.test(value) && value.length <= 32,
  flags: (value) => Array.isArray(value) && value.length <= 24 && value.every((flag) => typeof flag === 'string' && /^--[a-z][a-z-]{0,31}$/.test(flag)),
  exit: (value) => Number.isInteger(value) && value >= -1 && value <= 255,
  duration: (value) => ['<1s', '1-5s', '5-30s', '30s-2m', '>2m'].includes(value),
  version: (value) => typeof value === 'string' && /^\d+\.\d+\.\d+(\+[0-9a-z.]{1,40})?$/.test(value),
  channel: (value) => value === 'stable' || value === 'nightly',
  os: (value) => typeof value === 'string' && /^[a-z0-9_]{1,16}$/.test(value),
  arch: (value) => typeof value === 'string' && /^[a-z0-9_]{1,16}$/.test(value),
  install: (value) => ['install.sh', 'homebrew', 'cargo', 'unknown'].includes(value),
  ci: (value) => typeof value === 'boolean',
  terminal: (value) => typeof value === 'boolean',
  json: (value) => typeof value === 'boolean',
  install_id: (value) => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value),
};

// The event with exactly the documented fields, or null when anything is missing or off.
export function validate(body) {
  if (!body || typeof body !== 'object' || Array.isArray(body)) return null;
  const event = {};
  for (const [name, valid] of Object.entries(fields)) {
    if (!valid(body[name])) return null;
    event[name] = body[name];
  }
  return Object.keys(body).length === Object.keys(fields).length ? event : null;
}

export function capture(event, key) {
  const { install_id: distinctId, event: name, ...properties } = event;
  return {
    api_key: key,
    event: name,
    distinct_id: distinctId,
    properties: { ...properties, $process_person_profile: false, $geoip_disable: true, $ip: null },
  };
}

export default {
  async fetch(request, env, context) {
    const url = new URL(request.url);
    if (url.pathname !== '/api/telemetry') return new Response('Not found', { status: 404 });
    if (request.method !== 'POST') return new Response('Method not allowed', { status: 405, headers: { allow: 'POST' } });
    const text = await request.text();
    if (text.length > LIMIT) return new Response('Too large', { status: 413 });
    let event = null;
    try {
      event = validate(JSON.parse(text));
    } catch {
      event = null;
    }
    if (!event) return new Response('Bad event', { status: 400 });
    if (env.POSTHOG_KEY) {
      context.waitUntil(fetch(POSTHOG, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(capture(event, env.POSTHOG_KEY)),
      }).catch(() => {}));
    }
    return new Response(null, { status: 204 });
  },
};
