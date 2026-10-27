// Shared by the backend, property inspector and tests. Choice identities are
// structured JSON, never display strings or catalog positions.
export const selectorKey = s => JSON.stringify([s.category, s.node_kind, s.node, s.target, s.action, s.plugin ?? '']);
export const nodeKey = s => JSON.stringify([s.node_kind, s.node]);
export const targetKey = s => JSON.stringify([s.target, s.plugin ?? '']);
export const specs = [
  ['application', 'Application', false, 'application'],
  ['nodes', 'Inputs, outputs & nodes', false, 'node'],
  ['effects', 'Rack effects', false, 'effect'],
  ['switches', 'Switch destinations', false, 'switch'],
  ['sounds', 'Soundboard', false, 'soundboard'],
  ['recordings', 'Recording slots', false, 'recording_slot'],
  ['extensions', 'Extension actions', false, 'extension_action'],
  ['volume', 'Volume', true, 'mixer', 'volume'],
  ['pan', 'Pan', true, 'mixer', 'pan'],
  ['parameters', 'Effect & VST parameters', true, ''],
  ['ducking', 'Auto Ducking parameters', true, 'ducking_parameter'],
  ['nodeparameters', 'Extension node parameters', true, 'extension_parameter'],
].map(([id, name, adjustment, category, parameter]) => ({ id, name, adjustment, category, parameter }));

export function entriesFor(entries, spec) {
  return entries.filter(e => e.adjustment === spec.adjustment &&
    (!spec.category || e.selector.category === spec.category) &&
    (!spec.parameter || e.selector.action === spec.parameter) &&
    (spec.id !== 'parameters' || !['mixer', 'ducking_parameter', 'extension_parameter'].includes(e.selector.category)));
}
export function choices(entries, settings = {}, filter = '') {
  const counts = new Map();
  for (const e of entries) { const key = selectorKey(e.selector); counts.set(key, (counts.get(key) || 0) + 1); }
  const unique = (list, key, label) => [...new Map(list.map(e => [key(e.selector), { value: key(e.selector), label: label(e) }])).values()];
  return {
    nodes: unique(entries.filter(e => `${e.selector.node} ${e.selector.node_kind}`.toLowerCase().includes(filter.toLowerCase())), nodeKey,
      e => e.selector.node ? `${e.selector.node} (${e.selector.node_kind})` : 'PirateWave'),
    targets: unique(entries.filter(e => nodeKey(e.selector) === settings.node), targetKey, e => e.selector.target || 'This node'),
    actions: unique(entries.filter(e => nodeKey(e.selector) === settings.node && targetKey(e.selector) === settings.target), selectorKey,
      e => e.label + (counts.get(selectorKey(e.selector)) > 1 ? ' — Ambiguous target' : '')),
  };
}
export function select(settings, field, value, entries) {
  const next = { ...settings, schema: 1, [field]: value };
  if (field === 'node') { next.target = ''; delete next.selector; }
  if (field === 'target') delete next.selector;
  if (field === 'action') {
    const entry = entries.find(e => selectorKey(e.selector) === value);
    if (entry) next.selector = { ...entry.selector };
    else delete next.selector;
  }
  delete next.action;
  return next;
}
export function find(entries, selector) {
  if (!selector) return { error: 'Configure action' };
  const matches = entries.filter(e => selectorKey(e.selector) === selectorKey(selector));
  return matches.length === 1 ? { entry: matches[0] } : { error: matches.length ? 'Ambiguous target' : 'Missing target' };
}
export function press(entries, entry) {
  if (entry.selector.category === 'mixer' && entry.selector.action === 'volume') {
    const mute = entries.filter(e => e.selector.category === 'node' && e.selector.node === entry.selector.node &&
      e.selector.node_kind === entry.selector.node_kind && e.selector.action === 'toggle_mute');
    return mute.length === 1 ? { op: 'execute', selector: mute[0].selector } : undefined;
  }
  return { op: 'reset', selector: entry.selector };
}
export function feedback(client, settings) {
  if (!client.connected) return { title: settings.selector?.node || 'PirateWave', detail: client.status, color: '#CBD5E1' };
  if (settings.schema !== undefined && settings.schema !== 1) return { title: 'PirateWave', detail: 'Unsupported assignment version', color: '#FBBF24' };
  const { entry, error } = find(client.snapshot.entries, settings.selector);
  if (!entry) return { title: settings.selector?.node || 'PirateWave', detail: error, color: '#FBBF24' };
  const title = entry.selector.target || entry.selector.node || 'PirateWave';
  if (!['Connected', 'Accepted'].includes(client.status)) return { title, detail: client.status, color: '#FBBF24' };
  let detail = entry.state;
  if (entry.adjustment) {
    detail = entry.choices.length ? entry.choices[Math.max(0, Math.min(entry.choices.length - 1, Math.round(entry.value)))] : `${Number(entry.value.toFixed(3))} ${entry.unit}`;
    if (entry.selector.category === 'mixer' && entry.selector.action === 'pan') detail = entry.value === 0 ? 'Center' : `${Math.round(Math.abs(entry.value) * 100)}% ${entry.value < 0 ? 'L' : 'R'}`;
    if (entry.state) detail += ` · ${entry.state}`;
  }
  const active = !entry.adjustment && typeof entry.active === 'boolean' ? entry.active : undefined;
  if (active !== undefined) detail = `${active ? 'ON' : 'OFF'} · ${detail}`;
  return { title, label: entry.label, detail, color: active === undefined ? '#CBD5E1' : active ? '#4ADE80' : '#F87171', active,
    value: entry.value, min: entry.min, max: entry.max };
}
export const xml = text => String(text).replace(/[&<>"']/g, ch => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&apos;' })[ch]);
const images = new Map();
export function image(view) {
  const key = JSON.stringify(view);
  if (images.has(key)) return images.get(key);
  const wrap = (line, count) => (String(line).match(/.{1,18}/gu) || ['']).slice(0, count);
  const lines = [...wrap(view.title, 2), ...wrap(view.label || '', 1), ...wrap(view.detail, 3)];
  const result = 'data:image/svg+xml;base64,' + Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" width="144" height="144"><rect width="144" height="144" rx="10" fill="#191A1F"/><g fill="${view.color}" font-family="Arial" font-size="15" text-anchor="middle">${lines.map((line, i) => `<text x="72" y="${27 + i * 19}">${xml(line)}</text>`).join('')}</g><rect x="0" y="138" width="144" height="6" fill="${view.color}"/></svg>`).toString('base64');
  if (images.size >= 256) images.delete(images.keys().next().value);
  images.set(key, result);
  return result;
}
