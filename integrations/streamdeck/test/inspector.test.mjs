import { test } from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import { readFile } from 'node:fs/promises';
import { choices, select, selectorKey, nodeKey, targetKey, find } from '../src/model.mjs';

test('actual property inspector restores assignments and updates all dependent dropdowns without refresh writes', async () => {
  const fields = new Map();
  class Option { constructor(label, value) { this.label = label; this.value = value; } }
  const document = { activeElement: null, getElementById(id) {
    if (!fields.has(id)) fields.set(id, { value: '', dataset: {}, options: [], events: {}, hidden: false,
      addEventListener(type, handler) { this.events[type] = handler; },
      replaceChildren(...values) { this.options = values; }, add(option) { this.options.push(option); } });
    return fields.get(id);
  } };
  let socket;
  class WebSocket {
    static OPEN = 1; readyState = 1; sent = [];
    constructor() { socket = this; }
    send(value) { this.sent.push(JSON.parse(value)); }
    receive(event, payload) { this.onmessage({ data: JSON.stringify({ event, context: 'key-context', payload }) }); }
  }
  const context = vm.createContext({ document, window: {}, WebSocket, Option, choices, select, selectorKey, nodeKey, targetKey, find });
  const source = (await readFile('ui/inspector.mjs', 'utf8')).replace(/^import[^\n]+\n/, '');
  vm.runInContext(source, context);
  const e = { selector: { category: 'recording_slot', node_kind: 'recording', node: 'Replay α', target: 'Phrase / 1', action: 'capture', plugin: '' }, label: 'Capture', adjustment: false };
  const settings = { schema: 1, selector: e.selector, sensitivity: 2 };
  context.window.connectElgatoStreamDeckSocket('1234','inspector-window','registerPropertyInspector','{}',JSON.stringify({ context: 'key-context', action: 'com.piratewave.streamdeck.recordings', payload: { settings } }));
  socket.onopen();
  assert.equal(socket.sent[0].uuid, 'inspector-window');
  assert.equal(socket.sent[1].context, 'key-context');
  const catalog = entries => socket.receive('sendToPropertyInspector', { event: 'catalog', context: 'key-context', entries, connected: true, status: 'Connected', adjustment: false });
  catalog([e]);
  assert.equal(fields.get('node').value, nodeKey(e.selector));
  assert.equal(fields.get('target').value, targetKey(e.selector));
  assert.equal(fields.get('action').value, selectorKey(e.selector));
  assert.equal(socket.sent.filter(m => m.event === 'setSettings').length, 0);
  fields.get('node').events.change();
  assert.equal(fields.get('target').value, ''); assert.equal(fields.get('action').value, '');
  fields.get('target').value = targetKey(e.selector); fields.get('target').events.change();
  assert.equal(fields.get('action').options[1].value, selectorKey(e.selector));
  fields.get('action').value = selectorKey(e.selector); fields.get('action').events.change();
  assert.deepEqual(socket.sent.at(-1).payload.selector, e.selector);
  const writes = socket.sent.filter(m => m.event === 'setSettings').length;
  catalog([]); assert.match(fields.get('target-status').textContent, /Missing target/);
  assert.equal(fields.get('action').value, selectorKey(e.selector), 'missing targets remain saved');
  catalog([e, e]); assert.match(fields.get('target-status').textContent, /Ambiguous target/);
  catalog([e]); assert.equal(socket.sent.filter(m => m.event === 'setSettings').length, writes);
  fields.get('filter').value = 'unmatched'; fields.get('filter').events.input();
  assert.equal(socket.sent.filter(m => m.event === 'setSettings').length, writes);
});
