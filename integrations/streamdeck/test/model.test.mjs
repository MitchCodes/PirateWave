import { test } from 'node:test';
import assert from 'node:assert/strict';
import { choices, select, selectorKey, nodeKey, targetKey, specs, entriesFor, find, feedback, press, image } from '../src/model.mjs';
export const selector = (overrides = {}) => ({ category: 'node', node_kind: 'input', node: 'Mic | / α', target: '', action: 'toggle_mute', plugin: '', ...overrides });
export const entry = (overrides = {}) => ({ selector: selector(), label: 'Toggle mute', state: 'Unmuted', adjustment: false, active: false, value: 0, min: -60, max: 12, choices: [], unit: '', ...overrides });

test('dependent selections use exact structured identities, clear children, preserve unrelated settings', () => {
  const e = entry(), fx = entry({ selector: selector({ category: 'effect', target: 'vocoder', plugin: 'vst:α', action: 'toggle' }) });
  const list = [e, fx];
  let settings = select({ sensitivity: 2 }, 'node', nodeKey(e.selector), list);
  assert.equal(choices(list, settings).targets.length, 2);
  settings = select(settings, 'target', targetKey(fx.selector), list);
  assert.deepEqual(choices(list, settings).actions.map(a => a.value), [selectorKey(fx.selector)]);
  settings = select(settings, 'action', selectorKey(fx.selector), list);
  assert.deepEqual(settings.selector, fx.selector);
  assert.equal(settings.sensitivity, 2);
  assert.equal(settings.schema, 1);
  const saved = structuredClone(settings);
  choices([], settings, 'not found');
  assert.deepEqual(settings, saved, 'refresh must not rewrite saved assignments');
  assert.equal(select(settings, 'node', 'different', list).selector, undefined);
  assert.equal(select(settings, 'target', 'different', list).selector, undefined);
});
test('exact name matching handles project switches, ambiguity, recreation and plugin replacement', () => {
  const e = entry();
  assert.equal(find([], e.selector).error, 'Missing target');
  assert.equal(find([e, e], e.selector).error, 'Ambiguous target');
  assert.equal(find([structuredClone(e)], e.selector).entry.label, e.label);
  assert.equal(find([entry({ selector: selector({ node: 'Renamed' }) })], e.selector).error, 'Missing target');
  assert.equal(find([entry({ selector: selector({ node: 'mic | / α' }) })], e.selector).error, 'Missing target');
  assert.equal(find([entry({ selector: selector({ plugin: 'replacement' }) })], e.selector).error, 'Missing target');
});
test('every catalog category maps to a button or adjustment action', () => {
  for (const [category, adjustment] of [['application',false],['node',false],['effect',false],['switch',false],['soundboard',false],['recording_slot',false],['extension_action',false],['mixer',true],['ducking_parameter',true],['extension_parameter',true],['effect_parameter',true],['vst_parameter',true]]) {
    const e = entry({ selector: selector({ category, action: category === 'mixer' ? 'volume' : 'param' }), adjustment });
    assert.equal(specs.filter(spec => entriesFor([e], spec).length).length, 1, category);
  }
});
test('empty slots are available, duplicate effects stay visibly ambiguous', () => {
  const slot = entry({ selector: selector({ category: 'recording_slot', target: 'Empty slot', action: 'capture' }), state: 'Empty' });
  assert.equal(entriesFor([slot], specs.find(s => s.id === 'recordings')).length, 1);
  const settings = { node: nodeKey(slot.selector), target: targetKey(slot.selector) };
  assert.match(choices([slot, slot], settings).actions[0].label, /Ambiguous/);
});
test('feedback uses authoritative flags, neutral errors and capture outcomes', () => {
  const e = entry(), client = { connected: true, status: 'Accepted', snapshot: { entries: [e] } }, settings = { selector: e.selector };
  assert.equal(feedback(client, settings).color, '#F87171');
  e.active = true; assert.equal(feedback(client, settings).color, '#4ADE80');
  assert.match(feedback(client, settings).detail, /ON/);
  client.status = 'Capture failed'; assert.equal(feedback(client, settings).active, undefined);
  client.status = 'Accepted'; delete e.active; e.state = 'Capturing';
  assert.equal(feedback(client, settings).detail, 'Capturing');
  e.state = 'Captured'; assert.equal(feedback(client, settings).detail, 'Captured');
  client.connected = false; assert.equal(feedback(client, settings).active, undefined);
  const svg = Buffer.from(image({ title: '<&', detail: '🙂', color: '#F87171' }).split(',')[1], 'base64').toString();
  assert.match(svg, /&lt;&amp;/);
});
test('dial presses resolve only a unique matching mute and otherwise reset', () => {
  const mute = entry(), volume = entry({ selector: selector({ category: 'mixer', action: 'volume' }), adjustment: true });
  assert.deepEqual(press([mute, volume], volume), { op: 'execute', selector: mute.selector });
  assert.equal(press([mute, mute, volume], volume), undefined);
  const pan = entry({ selector: selector({ category: 'mixer', action: 'pan' }), adjustment: true });
  assert.equal(press([pan], pan).op, 'reset');
});
