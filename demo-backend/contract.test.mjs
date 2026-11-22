// Contract test for the demo backend: it must behave like the real server as described in
// openapi.json and docs/api/asyncapi.json, so that the public demo shows what the product
// does and the web interface is developed against real behaviour.
//
//   node --test        (in demo-backend/)
//
// It starts server.js on a free port with the playback simulation switched off, checks every
// GET route of the REST API, the write operations the web interface uses, the error body,
// the rate limit, and every WebSocket event the demo sends.
import { spawn } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { after, before, describe, it } from 'node:test';
import assert from 'node:assert/strict';
import Ajv2020 from 'ajv/dist/2020.js';
import WebSocket from 'ws';

const here = dirname(fileURLToPath(import.meta.url));
const openapi = JSON.parse(readFileSync(join(here, '../openapi.json'), 'utf8'));
const asyncapi = JSON.parse(readFileSync(join(here, '../docs/api/asyncapi.json'), 'utf8'));

// ── Schemas ──────────────────────────────────────────────────

const ajv = new Ajv2020({ strict: false, validateFormats: false, allErrors: true });

/** Make a schema self-contained: `#/components/schemas/X` refs point into `$defs`. */
function standalone(schema, components) {
  const text = JSON.stringify({ ...schema, $defs: components })
    .replaceAll('#/components/schemas/', '#/$defs/');
  return JSON.parse(text);
}

const validators = new Map();
function validator(key, schema, components) {
  if (!validators.has(key)) validators.set(key, ajv.compile(standalone(schema, components)));
  return validators.get(key);
}

function assertValid(key, schema, components, value, what) {
  const validate = validator(key, schema, components);
  assert.ok(validate(value), `${what}: ${ajv.errorsText(validate.errors)}\n${JSON.stringify(value).slice(0, 400)}`);
}

// ── Server ───────────────────────────────────────────────────

let child;
let base;

before(async () => {
  child = spawn(process.execPath, [join(here, 'server.js')], {
    env: { ...process.env, PORT: '0', DEMO_SIMULATION: 'off' },
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  base = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('the demo backend did not start')), 10_000);
    child.stdout.on('data', (chunk) => {
      const match = /listening on port (\d+)/.exec(String(chunk));
      if (match) {
        clearTimeout(timer);
        resolve(`127.0.0.1:${match[1]}`);
      }
    });
    child.on('exit', (code) => reject(new Error(`the demo backend exited with ${code}`)));
  });
});

after(() => child?.kill());

async function request(method, path, body) {
  const res = await fetch(`http://${base}${path}`, {
    // Covers redirect to an image host: never leave the machine in a test.
    redirect: 'manual',
    method,
    headers: body === undefined ? {} : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  let json;
  try { json = text ? JSON.parse(text) : undefined; } catch { json = undefined; }
  return { status: res.status, type: res.headers.get('content-type') ?? '', text, json };
}

/** Check a response against the operation in openapi.json. */
function assertDocumented(method, template, res) {
  const op = openapi.paths[template]?.[method.toLowerCase()];
  assert.ok(op, `${method} ${template} is not in openapi.json`);
  const documented = op.responses[String(res.status)];
  assert.ok(documented, `${method} ${template} answered ${res.status}, documented: ${Object.keys(op.responses).join(', ')}`);
  const schema = documented.content?.['application/json']?.schema;
  if (schema) {
    assert.ok(res.json !== undefined, `${method} ${template} ${res.status}: expected JSON, got ${JSON.stringify(res.text.slice(0, 80))}`);
    assertValid(`${method} ${template} ${res.status}`, schema, openapi.components.schemas, res.json, `${method} ${template} ${res.status}`);
  }
}

// ── WebSocket ────────────────────────────────────────────────

function openSocket() {
  const ws = new WebSocket(`ws://${base}/ws`);
  const events = [];
  ws.on('message', (data) => events.push(JSON.parse(String(data))));
  return new Promise((resolve, reject) => {
    ws.on('open', () => resolve({ ws, events }));
    ws.on('error', reject);
  });
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function until(what, check) {
  for (let i = 0; i < 100; i++) {
    const found = check();
    if (found) return found;
    await sleep(20);
  }
  assert.fail(`timed out waiting for ${what}`);
}

/** Every event must be a live AsyncAPI message with all required fields. */
function assertEventsMatchAsyncApi(events) {
  for (const event of events) {
    const message = asyncapi.components.messages[event.type];
    assert.ok(message, `the demo sent an event the server does not have: ${event.type}`);
    assert.ok(!message['x-reserved'], `the demo sent the reserved event ${event.type}`);
    assertValid(`event ${event.type}`, message.payload, asyncapi.components.schemas, event, `event ${event.type}`);
  }
}

// ── Tests ────────────────────────────────────────────────────

describe('demo backend contract', () => {
  let socket;
  before(async () => { socket = await openSocket(); });
  after(() => socket.ws.close());

  it('sends nothing on connect', async () => {
    const fresh = await openSocket();
    await sleep(300);
    fresh.ws.close();
    assert.deepEqual(fresh.events, []);
  });

  it('answers every GET route as documented', async () => {
    const values = {
      zone_index: 1,
      client_index: 1,
      playlist_index: 1,
      track_index: 0,
      band_index: 0,
      playlist_id: 'subsonic:demo-1',
      cover_id: 'demo-cover',
      name: 'KEF LS50 Meta',
    };
    const query = { q: 'lo', kind: 'track', limit: '5', offset: '0' };
    const binary = new Set([
      '/api/v1/knx/knxprod',
      '/api/v1/zones/{zone_index}/cover',
      '/api/v1/media/cover/{cover_id}',
      '/api/v1/media/playlists/{playlist_index}/cover',
      '/api/v1/media/playlists/{playlist_index}/tracks/{track_index}/cover',
    ]);
    const failures = [];
    for (const [template, ops] of Object.entries(openapi.paths)) {
      if (!ops.get || template.startsWith('/api/v1/openapi') || template === '/ws') continue;
      const path = template.replace(/\{(\w+)\}/g, (_, name) => encodeURIComponent(String(values[name] ?? 1)));
      const params = (ops.get.parameters ?? []).filter((p) => p.in === 'query' && query[p.name]);
      const search = params.length ? `?${params.map((p) => `${p.name}=${query[p.name]}`).join('&')}` : '';
      const res = await request('GET', path + search);
      try {
        if (binary.has(template)) {
          // The demo serves images by redirecting to where they live.
          const redirect = res.status >= 300 && res.status < 400;
          assert.ok(redirect || ops.get.responses[String(res.status)], `GET ${template} answered ${res.status}`);
        } else {
          assertDocumented('GET', template, res);
        }
      } catch (e) {
        failures.push(e.message);
      }
    }
    assert.deepEqual(failures, []);
  });

  it('answers an unknown zone with an ErrorBody', async () => {
    const res = await request('GET', '/api/v1/zones/99');
    assert.equal(res.status, 404);
    assertDocumented('GET', '/api/v1/zones/{zone_index}', res);
  });

  it('answers the track metadata of an idle zone with 200', async () => {
    await request('POST', '/api/v1/zones/3/stop');
    const res = await request('GET', '/api/v1/zones/3/track/metadata');
    assert.equal(res.status, 200);
    assertDocumented('GET', '/api/v1/zones/{zone_index}/track/metadata', res);
  });

  it('answers library search with results', async () => {
    const res = await request('GET', '/api/v1/media/search?q=lo');
    assert.equal(res.status, 200);
    assertDocumented('GET', '/api/v1/media/search', res);
    assert.ok(res.json.items.length > 0, 'the seed data has matches for "lo"');
  });

  it('answers the write operations of the web interface as documented', async () => {
    const writes = [
      ['PUT', '/api/v1/zones/{zone_index}/volume', 30],
      ['PUT', '/api/v1/zones/{zone_index}/mute', false],
      ['POST', '/api/v1/zones/{zone_index}/mute/toggle'],
      ['POST', '/api/v1/zones/{zone_index}/play'],
      ['POST', '/api/v1/zones/{zone_index}/pause'],
      ['PUT', '/api/v1/zones/{zone_index}/shuffle', true],
      ['PUT', '/api/v1/zones/{zone_index}/repeat', 'playlist'],
      ['PUT', '/api/v1/zones/{zone_index}/eq', { enabled: true, bands: [{ freq: 1000, gain: 3, q: 1, type: 'peaking' }] }],
      ['POST', '/api/v1/zones/{zone_index}/eq/preset', 'flat'],
      ['PUT', '/api/v1/clients/{client_index}/volume', 40],
      ['PUT', '/api/v1/clients/{client_index}/mute', false],
      ['PUT', '/api/v1/clients/{client_index}/latency', 20],
      ['PUT', '/api/v1/clients/{client_index}/eq', { enabled: true, bands: [] }],
      ['PUT', '/api/v1/knx/programming-mode', false],
      ['POST', '/api/v1/system/updates/check'],
    ];
    const failures = [];
    for (const [method, template, body] of writes) {
      const path = template.replace('{zone_index}', '1').replace('{client_index}', '1');
      try {
        assertDocumented(method, template, await request(method, path, body));
      } catch (e) {
        failures.push(e.message);
      }
    }
    assert.deepEqual(failures, []);
  });

  it('confirms a seek with zone_changed', async () => {
    await request('POST', '/api/v1/zones/1/play/playlist', { index: 1 });
    const from = socket.events.length;
    const res = await request('PUT', '/api/v1/zones/1/track/position', { position_ms: 12_000 });
    assertDocumented('PUT', '/api/v1/zones/{zone_index}/track/position', res);
    await until('the seek event', () => socket.events.slice(from)
      .find((e) => e.type === 'zone_changed' && e.zone === 1 && e.position_ms === 12_000));
  });

  it('announces a rename with the new name', async () => {
    const from = socket.events.length;
    const res = await request('PUT', '/api/v1/clients/2/name', 'Desk');
    assertDocumented('PUT', '/api/v1/clients/{client_index}/name', res);
    await until('the rename event', () => socket.events.slice(from)
      .find((e) => e.type === 'client_state_changed' && e.client === 2 && e.name === 'Desk'));
  });

  it('announces client EQ and speaker changes', async () => {
    const from = socket.events.length;
    await request('PUT', '/api/v1/clients/1/eq', { enabled: false, bands: [] });
    await request('PUT', '/api/v1/clients/1/speaker', { speaker: null });
    await until('client_eq_changed', () => socket.events.slice(from).find((e) => e.type === 'client_eq_changed'));
    await until('client_speaker_changed', () => socket.events.slice(from).find((e) => e.type === 'client_speaker_changed'));
  });

  it('lets one EQ drag through without 429', async () => {
    const band = { freq: 1000, gain: 0, q: 1, type: 'peaking' };
    for (let i = 0; i < 150; i++) {
      const res = await request('PUT', '/api/v1/zones/2/eq/0', { ...band, gain: (i % 24) / 2 - 6 });
      assert.notEqual(res.status, 429, `request ${i} was rate limited`);
    }
  });

  it('sends only live AsyncAPI events with every required field', () => {
    assert.ok(socket.events.length > 0, 'the operations above produced events');
    assertEventsMatchAsyncApi(socket.events);
  });
});
