// Real Windows pipe/service integration. The isolated host never opens audio
// hardware, reads user settings, or shares an endpoint with installed plugins.
import assert from 'node:assert/strict';
import net from 'node:net';
import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { setTimeout as delay } from 'node:timers/promises';
import { resolve } from 'node:path';
import { ControlClient, Decoder, frame, pipeName } from '../src/client.mjs';
import { find, selectorKey } from '../src/model.mjs';

if (process.platform !== 'win32') throw new Error('Native transport tests require Windows');
const exe = resolve(process.argv[2] || '../../target/debug/piratewave-desktop.exe');
const id = randomUUID().replaceAll('-', '');
const path = await pipeName() + `.test.${id}`;
const env = { ...process.env, PIRATEWAVE_CONTROLLER_TEST_ID: id };
const hosts = new Set(), clients = new Set(), peers = new Set();
async function until(predicate, label, timeout = 8000) {
  const start = Date.now(); while (!predicate()) { if (Date.now() - start > timeout) throw new Error(`Timed out: ${label}`); await delay(10); }
}
function host() {
  const child = spawn(exe, ['--controller-test-host'], { env, windowsHide: true, stdio: ['ignore','pipe','pipe'] });
  child.output = ''; child.stdout.on('data', b => child.output += b); child.stderr.on('data', b => child.output += b);
  hosts.add(child); return child;
}
async function kill(child) { if (child.exitCode === null) { child.kill(); await until(() => child.exitCode !== null || child.signalCode !== null, 'host exit'); } }
async function raw() {
  const socket = net.createConnection(path), decoder = new Decoder(), pending = new Map(); let next = 0;
  peers.add(socket); socket.on('error', () => {});
  socket.on('data', chunk => { for (const m of decoder.push(chunk)) { if (pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } } });
  await new Promise((res, rej) => { socket.once('connect', res); socket.once('error', rej); });
  return { socket, request: data => new Promise((res, rej) => {
    const id = ++next; const timer = setTimeout(() => { pending.delete(id); rej(new Error('Raw request timed out')); }, 4000);
    pending.set(id, m => { clearTimeout(timer); res(m); }); socket.write(frame({ ...data, id }));
  }) };
}
const client = new ControlClient(path, 50); clients.add(client);
try {
  client.start(); await delay(80); assert.equal(client.connected, false, 'client-first startup should initially be offline');
  let first = host(); await until(() => client.connected && client.snapshot.entries.length > 0, 'client-first connection');
  const baseline = client.snapshot.generation; assert.equal(baseline, '1792746915180750500');
  const legacy = await raw();
  assert.equal((await legacy.request({ op: 'hello', version: 1, client: 'loupedeck' })).version, 1);
  assert.equal(typeof (await legacy.request({ op: 'state' })).state.generation, 'number');
  const second = new ControlClient(path, 50); clients.add(second); second.start();
  await until(() => second.connected && second.snapshot.entries.length, 'server-first second controller');
  const volume = client.snapshot.entries.find(e => e.selector.category === 'mixer' && e.selector.action === 'volume' && e.selector.node === 'Source'); assert(volume);
  const stateEntry = () => find(client.snapshot.entries, volume.selector).entry;
  assert(client.submit('adjust', volume.selector, -3, 2));
  await until(() => stateEntry()?.value === -6, 'volume decrease and sensitivity');
  assert(client.submit('adjust', volume.selector, 10000, 100));
  await until(() => stateEntry()?.value === 12 && find(second.snapshot.entries, volume.selector).entry?.value === 12, 'clamping and shared feedback');
  assert(client.submit('reset', volume.selector)); await until(() => stateEntry()?.value === 0, 'v2 volume default');
  const mute = client.snapshot.entries.find(e => e.selector.node === 'Source' && e.selector.action === 'toggle_mute');
  assert(client.submit('execute', mute.selector)); await until(() => find(second.snapshot.entries, mute.selector).entry?.active === true, 'toggle synchronization');
  // Legacy reset must still mute, without changing the volume default.
  assert((await legacy.request({ op: 'reset', generation: baseline, selector: volume.selector })).ok);
  await until(() => find(client.snapshot.entries, mute.selector).entry?.active === false, 'legacy volume press compatibility');
  for (const category of ['effect_parameter','vst_parameter','ducking_parameter','extension_parameter']) {
    const e = client.snapshot.entries.find(e => e.selector.category === category);
    if (!e) { assert.equal(category, 'extension_parameter', `Missing ${category}`); continue; }
    assert(client.submit('adjust', e.selector, e.value < e.max ? 1 : -1));
    await until(() => find(client.snapshot.entries, e.selector).entry?.value !== e.value, `${category} adjustment`);
    assert(client.submit('reset', e.selector)); await until(() => find(client.snapshot.entries, e.selector).entry?.value === e.default, `${category} reset`);
  }
  const discrete = client.snapshot.entries.find(e => e.adjustment && e.choices.length > 1);
  assert(discrete); assert(client.submit('adjust', discrete.selector, 1)); await until(() => find(client.snapshot.entries, discrete.selector).entry?.value !== discrete.value, 'discrete choice');
  for (const slot of client.snapshot.entries.filter(e => e.selector.category === 'recording_slot')) {
    assert.equal(slot.state, 'Empty');
    const result = await legacy.request({ op: 'execute', generation: baseline, selector: slot.selector }); assert(result.ok, slot.selector.action);
  }
  const bad = await raw(); assert.equal((await bad.request({ op: 'hello', version: 99 })).ok, false); bad.socket.destroy();
  const duplicate = host(); await until(() => duplicate.exitCode !== null, 'exclusive PirateWave ownership'); assert.notEqual(duplicate.exitCode, 0);
  // Generation increments simulate a project switch. Stale commands fail on the
  // server even if an adapter hasn't received the refreshed subscription yet.
  const start = client.snapshot.entries.find(e => e.selector.category === 'application' && e.selector.action === 'start_audio');
  assert(start); assert(client.submit('execute', start.selector)); await until(() => client.snapshot.generation !== baseline, 'project generation publication');
  const stale = await legacy.request({ op: 'execute', generation: baseline, selector: mute.selector }); assert(!stale.ok); assert.match(stale.error, /Project changed/);
  // Fill all eight slots, including an unresponsive peer. They cannot prevent
  // existing subscribed controllers from executing or receiving live state.
  await delay(30); // Allow the rejected peer's pipe instance to accept again.
  const extra = []; for (let n = 0; n < 5; n++) extra.push(await raw());
  const slow = extra[0];
  assert((await slow.request({ op: 'hello', version: 2, client: 'streamdeck' })).ok);
  assert((await slow.request({ op: 'subscribe' })).ok);
  slow.socket.pause();
  const countBefore = client.snapshot.notice;
  assert(client.submit('execute', mute.selector)); await until(() => client.snapshot.notice !== countBefore, 'action with eight clients');
  const ninth = net.createConnection(path); peers.add(ninth);
  const saturated = await new Promise(resolve => { ninth.once('error', e => resolve(e.code)); ninth.once('connect', () => resolve('connected')); setTimeout(() => resolve('pending'), 200); });
  assert.notEqual(saturated, 'connected', 'ninth client must not exceed service bound'); ninth.destroy();
  await delay(3000); // Exhaust the paused reader's pipe and its write deadline.
  const afterSlow = client.snapshot.notice;
  assert(client.submit('execute', mute.selector));
  await until(() => client.snapshot.notice !== afterSlow, 'slow reader must not block other controllers');
  slow.socket.resume();
  await until(() => slow.socket.destroyed, 'slow reader timed out independently');
  for (const p of extra) p.socket.destroy();
  legacy.socket.destroy(); second.stop();
  await kill(first); await until(() => !client.connected, 'disconnect');
  assert.equal(client.queue.length, 0); assert.equal(client.submit('execute', mute.selector), false);
  first = host(); await until(() => client.connected && client.snapshot.generation === baseline, 'reconnect');
  await delay(200); assert.equal(client.snapshot.notice, 'Accepted 0', 'no pending commands may replay');
  assert.equal(find(client.snapshot.entries, volume.selector).entry.value, 0);
  assert.equal(selectorKey(find(client.snapshot.entries, volume.selector).entry.selector), selectorKey(volume.selector));
  console.log('PASS: real Windows pipe v1/v2 coexistence, eight-client bound, ownership, lossless generations, defaults/legacy press, knobs/choices/clamps, shared feedback, empty-slot actions, stale-project rejection, both startup orders and restart without replay. Physical hardware not exercised.');
} catch (e) { for(const h of hosts) console.log(h.output); throw e; } finally {
  for (const c of clients) c.stop(); for (const p of peers) p.destroy();
  for (const h of hosts) await kill(h);
}
