import { test } from 'node:test';
import assert from 'node:assert/strict';
import { WebSocketServer } from 'ws';
import { spawn } from 'node:child_process';
import { resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

test('packaged SDK backend starts, registers all actions and handles host callbacks', { timeout: 12000 }, async t => {
  const server = new WebSocketServer({ host: '127.0.0.1', port: 0 });
  await new Promise(resolve => server.once('listening', resolve));
  let ws; const messages = [];
  server.on('connection', socket => { ws = socket; socket.on('message', bytes => {
    const message = JSON.parse(bytes); messages.push(message);
    if (message.event === 'getSettings') socket.send(JSON.stringify({ event: 'didReceiveSettings', action: 'com.piratewave.streamdeck.nodes', context: 'sdk-key', payload: { settings: {} } }));
  }); });
  const child = spawn(process.execPath, ['bin/plugin.mjs', '-port', String(server.address().port), '-pluginUUID', 'com.piratewave.streamdeck', '-registerEvent', 'registerPlugin', '-info', JSON.stringify({ application: { version: '7.1.0', language: 'en', platform: 'windows', platformVersion: '10' }, plugin: { version: '1.0.0.0' }, devices: [{ id: 'simulated-device', name: 'Simulated Stream Deck+', type: 7, size: { columns: 4, rows: 2 } }], devicePixelRatio: 1 })], {
    cwd: resolve('com.piratewave.streamdeck.sdPlugin'), windowsHide: true, stdio: ['ignore','pipe','pipe'],
  });
  let output = ''; child.stdout.on('data', b => output += b); child.stderr.on('data', b => output += b);
  t.after(async () => { child.kill(); ws?.terminate(); await new Promise(resolve => server.close(resolve)); });
  async function until(predicate) {
    const start = Date.now(); while (!predicate()) { if (child.exitCode !== null || Date.now() - start > 8000) throw new Error(`SDK callback did not arrive: ${output}`); await delay(10); }
  }
  await until(() => messages.some(m => m.event === 'registerPlugin'));
  ws.send(JSON.stringify({ event: 'willAppear', action: 'com.piratewave.streamdeck.nodes', context: 'sdk-key', device: 'simulated-device', payload: { controller: 'Keypad', settings: {}, coordinates: { column: 0, row: 0 }, isInMultiAction: false } }));
  await until(() => messages.some(m => m.event === 'setImage'));
  assert.equal(messages.find(m => m.event === 'setImage').context, 'sdk-key');
  // An unconfigured press is safe even if a developer happens to have PirateWave
  // running. It cannot resolve a target or send an engine command.
  ws.send(JSON.stringify({ event: 'keyDown', action: 'com.piratewave.streamdeck.nodes', context: 'sdk-key', device: 'simulated-device', payload: { controller: 'Keypad', settings: {}, coordinates: { column: 0, row: 0 }, isInMultiAction: false } }));
  await until(() => messages.some(m => m.event === 'showAlert'));
  ws.send(JSON.stringify({ event: 'propertyInspectorDidAppear', action: 'com.piratewave.streamdeck.nodes', context: 'sdk-key', device: 'simulated-device' }));
  await until(() => messages.some(m => m.event === 'sendToPropertyInspector'));
  assert(Array.isArray(messages.find(m => m.event === 'sendToPropertyInspector').payload.entries));
  ws.send(JSON.stringify({ event: 'willDisappear', action: 'com.piratewave.streamdeck.nodes', context: 'sdk-key', device: 'simulated-device', payload: { controller: 'Keypad', settings: {}, coordinates: { column: 0, row: 0 }, isInMultiAction: false } }));
  assert.equal(child.exitCode, null);
});
