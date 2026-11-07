// The telemetry endpoint (worker.mjs) keeps only the documented fields, and the card renderer
// draws 1200x630 PNGs.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import telemetry from '../worker.mjs';
import { validate, capture } from '../worker.mjs';
import { renderCard } from '../og.mjs';

const event = {
  schema: 1, event: 'command', command: 'proxy install', flags: ['--json'], exit: 0, duration: '<1s',
  version: '0.5.0', channel: 'stable', os: 'macos', arch: 'aarch64', install: 'homebrew',
  ci: false, terminal: true, json: true, install_id: '4208b56a-3d78-451f-8c9d-889a2e13a766',
};

test('a CLI event is accepted and anything else is rejected', () => {
  assert.deepEqual(validate(event), event);
  for (const [name, value] of [['path', '/Users/me'], ['command', 'up /Users/me'], ['flags', ['--name=client']], ['version', '1.0.0 /tmp'], ['install_id', 'me@example.com']]) {
    assert.equal(validate({ ...event, [name]: value }), null, `${name}=${JSON.stringify(value)} is rejected`);
  }
  assert.equal(capture(event, 'key').properties.$process_person_profile, false, 'events are anonymous');
});

test('the endpoint takes valid events and refuses the rest', async () => {
  const respond = (body) => telemetry.fetch(new Request('https://parcela.run/api/telemetry', { method: 'POST', body }), {}, { waitUntil() {} });
  assert.equal((await respond(JSON.stringify(event))).status, 204);
  assert.equal((await respond('{"schema":1}')).status, 400);
  assert.equal((await respond('x'.repeat(5000))).status, 413);
});

test('cards are 1200x630', () => {
  const png = renderCard({ title: 'Configuration', description: 'Parcela reads parcela.yaml from the root of your checkout.' });
  assert.deepEqual([png.readUInt32BE(16), png.readUInt32BE(20)], [1200, 630]);
});
