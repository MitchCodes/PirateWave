import { test } from 'node:test';
import assert from 'node:assert/strict';
import { PirateWaveAction } from '../.test-build/actions.mjs';
import { ControlClient } from '../src/client.mjs';
import { specs, nodeKey, targetKey } from '../src/model.mjs';

const selector = (category = 'node', action = 'toggle_mute', target = '') => ({ category, action, target, node: 'Microphone', node_kind: 'input', plugin: '' });
function entry(s, adjustment = false) { return { selector: s, adjustment, label: s.action, state: 'Ready', active: false, value: .5, min: 0, max: 1, choices: [], unit: '' }; }
function surface(id, dial = false) {
  return { id, settings: {}, images: [], values: [], alerts: 0,
    isKey: () => !dial, isDial: () => dial, isNeoInfobar: () => false,
    async getSettings() { return this.settings; }, async setTitle() {}, async setImage(i) { this.images.push(i); },
    async setFeedback(v) { this.values.push(v); }, async showAlert() { this.alerts++; } };
}
function fixture(spec) {
  const client = new ControlClient('unused'); client.connected = true; client.status = 'Connected'; client.snapshot.generation = '1792746915180750500';
  const ui = { action: undefined, messages: [], async sendToPropertyInspector(m) { this.messages.push(m); } };
  return { client, ui, adapter: new PirateWaveAction(spec, client, ui, error => { throw error; }) };
}
test('production SDK adapter routes all command categories with no keyboard bindings', async () => {
  for (const spec of specs.filter(s => !s.adjustment)) {
    const { client, adapter } = fixture(spec), e = entry(selector(spec.category)); client.snapshot.entries = [e];
    const action = surface(spec.id), settings = { schema: 1, selector: e.selector }; action.settings = settings;
    await adapter.onWillAppear({ action, payload: { settings } });
    await adapter.onKeyDown({ action, payload: { settings } });
    assert.equal(client.queue[0].op, 'execute'); assert.deepEqual(client.queue[0].selector, e.selector);
    adapter.onWillDisappear({ action }); assert.equal(client.listenerCount('changed'), 0);
  }
});
test('keys increase/decrease/reset; dial rotation uses sensitivity and press/tap use shared defaults', async () => {
  const { client, adapter } = fixture(specs.find(s => s.id === 'volume'));
  const volume = entry(selector('mixer','volume'), true), mute = entry(selector()); client.snapshot.entries = [volume, mute];
  const action = surface('key'), settings = { schema: 1, selector: volume.selector, sensitivity: 2, operation: 'increase' };
  await adapter.onKeyDown({ action, payload: { settings } });
  await adapter.onKeyDown({ action, payload: { settings: { ...settings, operation: 'decrease' } } });
  await adapter.onKeyDown({ action, payload: { settings: { ...settings, operation: 'reset' } } });
  assert.deepEqual(client.queue.map(i => [i.op,i.ticks]), [['adjust',1],['adjust',-1],['reset',0]]);
  const dial = surface('dial', true);
  await adapter.onDialRotate({ action: dial, payload: { settings, ticks: -3 } });
  await adapter.onDialDown({ action: dial, payload: { settings } });
  await adapter.onTouchTap({ action: dial, payload: { settings } });
  assert.equal(client.queue[3].sensitivity, 2); assert.equal(client.queue[3].ticks, -3);
  assert.deepEqual(client.queue.slice(4).map(i => i.selector), [mute.selector, mute.selector]);
  client.snapshot.entries = [volume, volume]; await adapter.onKeyDown({ action, payload: { settings } }); assert.equal(action.alerts, 1);
});
test('SDK v3 settings restore, per-context isolation, editor dependencies and change-only display updates', async () => {
  const { client, adapter, ui } = fixture(specs.find(s => s.id === 'effects'));
  const e = entry(selector('effect','toggle','vocoder')); client.snapshot.entries = [e];
  const a = surface('a'), b = surface('b');
  a.settings = { selector: e.selector, node: nodeKey(e.selector), target: targetKey(e.selector) };
  await adapter.onWillAppear({ action: a, payload: { settings: {} } });
  await adapter.onWillAppear({ action: b, payload: { settings: {} } });
  assert(a.images.length >= 2, 'getSettings result must restore assignment without a settings event');
  const count = a.images.length;
  client.emit('changed'); await new Promise(setImmediate); assert.equal(a.images.length, count);
  e.active = true; client.emit('changed'); await new Promise(setImmediate);
  assert.match(Buffer.from(a.images.at(-1).split(',')[1], 'base64').toString(), /#4ADE80/);
  assert.match(Buffer.from(b.images.at(-1).split(',')[1], 'base64').toString(), /Configure action/);
  ui.action = a; await adapter.onPropertyInspectorDidAppear({ action: a });
  assert.equal(ui.messages.at(-1).entries.length, 1);
  const messages = ui.messages.length; client.emit('changed'); await new Promise(setImmediate); assert.equal(ui.messages.length, messages);
  e.label = 'Renamed control'; client.emit('changed'); await new Promise(setImmediate); assert.equal(ui.messages.length, messages + 1);
  await adapter.onDidReceiveSettings({ action: a, payload: { settings: {} } });
  assert.match(Buffer.from(a.images.at(-1).split(',')[1], 'base64').toString(), /Configure action/);
  adapter.onWillDisappear({ action: a }); adapter.onWillDisappear({ action: b }); assert.equal(client.listenerCount('changed'), 0);
});
