import { choices, select, selectorKey, nodeKey, targetKey, find } from './model.mjs';
let socket, context, action, settings = {}, entries = [], connected = false, adjustment = false;
const el = id => document.getElementById(id);
function send(event, payload) {
  if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ event, context, action, payload }));
}
function fill(id, options, selected) {
  const field = el(id);
  const signature = JSON.stringify([options, selected, connected]);
  if (field.dataset.signature === signature) return;
  field.dataset.signature = signature;
  field.replaceChildren(new Option('Select…', ''));
  if (selected && !options.some(option => option.value === selected)) field.add(new Option('Saved target — missing or filtered', selected));
  for (const option of options) field.add(new Option(option.label, option.value));
  field.value = selected || ''; field.disabled = !connected;
}
function render() {
  // Recover parent selections from the durable selector when restoring settings.
  // This is presentation only; a catalog update never writes host settings.
  const selection = { ...settings };
  if (selection.selector) { selection.node = nodeKey(selection.selector); selection.target = targetKey(selection.selector); }
  const list = choices(entries, selection, el('filter').value);
  fill('node', list.nodes, selection.node);
  fill('target', list.targets, selection.target);
  fill('action', list.actions, selection.selector && selectorKey(selection.selector));
  el('adjustment').hidden = !adjustment;
  el('action-label').textContent = adjustment ? 'Parameter' : 'Action';
  el('operation').value = settings.operation || 'increase';
  if (document.activeElement !== el('sensitivity')) el('sensitivity').value = settings.sensitivity ?? 1;
  const { error } = find(entries, settings.selector);
  el('target-status').textContent = connected ? (error || 'Assignment ready. No keyboard shortcut required.') : 'Run PirateWave and enable external controllers.';
}
window.connectElgatoStreamDeckSocket = (port, uuid, registerEvent, info, actionInfo) => {
  const initial = JSON.parse(actionInfo);
  // The inspector registration UUID and action instance context are different
  // identities. Commands/settings must address the action, not the UI window.
  settings = initial.payload?.settings || {}; context = initial.context || uuid; action = initial.action;
  socket = new WebSocket(`ws://127.0.0.1:${port}`);
  socket.onopen = () => {
    socket.send(JSON.stringify({ event: registerEvent, uuid }));
    send('sendToPlugin', { event: 'catalog' });
    send('getSettings', {});
  };
  socket.onmessage = event => {
    try {
      const message = JSON.parse(event.data);
      if (message.context && message.context !== context) return;
      if (message.event === 'didReceiveSettings') { settings = message.payload.settings || {}; render(); }
      if (message.event === 'sendToPropertyInspector' && message.payload.event === 'catalog') {
        const data = message.payload;
        if (data.context !== context) return;
        entries = data.entries; connected = data.connected; adjustment = data.adjustment;
        el('status').textContent = data.status + (connected && !entries.length ? ' — No matching controls in this project' : ''); render();
      }
    } catch { el('status').textContent = 'Invalid controller response'; }
  };
  socket.onclose = () => { connected = false; el('status').textContent = 'Stream Deck connection closed'; render(); };
  render();
};
for (const field of ['node', 'target', 'action', 'operation', 'sensitivity']) el(field).addEventListener('change', () => {
  let value = el(field).value;
  if (field === 'sensitivity') {
    value = Number(value);
    if (!Number.isFinite(value) || value < .01 || value > 100) { render(); return; }
  }
  settings = select(settings, field, value, entries);
  send('setSettings', settings); render();
});
el('filter').addEventListener('input', render);
