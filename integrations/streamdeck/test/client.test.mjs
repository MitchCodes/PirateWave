import { test } from 'node:test';
import assert from 'node:assert/strict';
import net from 'node:net';
import { randomUUID } from 'node:crypto';
import { ControlClient, Decoder, frame, MAX_FRAME } from '../src/client.mjs';
import { setTimeout as delay } from 'node:timers/promises';
const selector = { category: 'node', node_kind: 'input', node: 'Mic', target: '', action: 'toggle_mute', plugin: '' };
export async function until(predicate, label = 'condition', timeout = 5000) {
  const start = Date.now(); while (!predicate()) { if (Date.now() - start > timeout) throw new Error(`Timed out: ${label}`); await delay(10); }
}
test('framing supports split and combined frames; rejects oversized and malformed input', () => {
  const decoder = new Decoder(), bytes = frame({ generation: '1792746915180750500' });
  assert.deepEqual(decoder.push(bytes.subarray(0, 3)), []);
  assert.deepEqual(decoder.push(Buffer.concat([bytes.subarray(3), frame({ ok: true })])), [{ generation: '1792746915180750500' }, { ok: true }]);
  const bad = Buffer.alloc(4); bad.writeUInt32LE(MAX_FRAME + 1);
  assert.throws(() => new Decoder().push(bad), /frame/);
  assert.throws(() => new Decoder().push(Buffer.alloc(4)), /frame/);
});
test('bounded queue coalesces adjacent same-direction deltas, preserves buttons/reset/direction and rejects overflow', () => {
  const client = new ControlClient('unused'); client.connected = true; client.snapshot.generation = '1792746915180750500';
  assert(client.submit('adjust', selector, 1)); assert(client.submit('adjust', selector, 2));
  assert.equal(client.queue[0].ticks, 3);
  client.submit('execute', selector); client.submit('adjust', selector, -1); client.submit('reset', selector); client.submit('adjust', selector, -1);
  assert.deepEqual(client.queue.map(i => i.op), ['adjust','execute','adjust','reset','adjust']);
  while (client.queue.length < 128) assert(client.submit('execute', selector));
  assert.equal(client.submit('execute', selector), false); assert.match(client.status, /queue full/);
  assert.equal(client.submit('adjust', selector, 1, Infinity), false);
  client.stop(); assert.equal(client.queue.length, 0);
});
test('async transport handshakes, preserves generation, rejects obsolete queues, reports failures, reconnects without replay', async t => {
  const path = process.platform === 'win32' ? `\\\\.\\pipe\\PirateWave.StreamDeck.Test.${randomUUID()}` : `/tmp/piratewave-${randomUUID()}.sock`;
  const sockets = new Set(), commands = []; let generation = '1792746915180750500', connections = 0, fail = false;
  const server = net.createServer(socket => {
    sockets.add(socket); connections++;
    socket.on('close', () => sockets.delete(socket)); socket.on('error', () => {});
    const decoder = new Decoder();
    socket.on('data', chunk => { for (const request of decoder.push(chunk)) {
      if (request.op === 'hello') socket.write(frame({ id: request.id, ok: true, version: 2 }));
      else if (['subscribe','state'].includes(request.op)) socket.write(frame({ id: request.id, ok: true, state: { generation, entries: [], notice: '' } }));
      else { commands.push(request); socket.write(frame({ id: request.id, ok: !fail, error: 'Capture failed' })); }
    } });
  });
  await new Promise(resolve => server.listen(path, resolve));
  const client = new ControlClient(path, 20); t.after(async () => { client.stop(); for (const socket of sockets) socket.destroy(); await new Promise(resolve => server.close(resolve)); });
  client.start(); await until(() => client.connected);
  client.submit('execute', selector); await until(() => commands.length === 1);
  assert.equal(commands[0].generation, generation);
  fail = true; client.submit('execute', selector); await until(() => client.status === 'Capture failed');
  // Change generation before next queued intent can be sent.
  client.pending = { id: -1, at: Date.now() }; client.submit('execute', selector);
  generation = '1792746915180750501';
  for (const socket of sockets) socket.write(frame({ event: 'state', state: { generation, entries: [] } }));
  await until(() => client.snapshot.generation === generation); assert.equal(client.queue.length, 0);
  for (const socket of sockets) socket.destroy();
  await until(() => connections >= 2 && client.connected);
  await delay(50); assert.equal(commands.length, 2);
});
test('incompatible v1-only host gives upgrade status rather than a lossy fallback', async t => {
  const path = process.platform === 'win32' ? `\\\\.\\pipe\\PirateWave.StreamDeck.Test.${randomUUID()}` : `/tmp/piratewave-${randomUUID()}.sock`;
  const server = net.createServer(socket => { const decoder = new Decoder(); socket.on('data', chunk => { for (const r of decoder.push(chunk)) socket.write(frame({ id: r.id, ok: false, version: 1 })); }); socket.on('error', () => {}); });
  await new Promise(resolve => server.listen(path, resolve));
  const client = new ControlClient(path, 100); t.after(async () => { client.stop(); await new Promise(resolve => server.close(resolve)); });
  client.start(); await until(() => client.status.includes('Upgrade')); assert.equal(client.connected, false); assert.equal(client.queue.length, 0);
});
