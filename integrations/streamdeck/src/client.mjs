import net from 'node:net';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { EventEmitter } from 'node:events';
import { selectorKey } from './model.mjs';

export const MAX_FRAME = 16 * 1024 * 1024;
export function frame(value) {
  const json = Buffer.from(JSON.stringify(value));
  if (!json.length || json.length > MAX_FRAME) throw new Error('Invalid controller frame');
  const header = Buffer.alloc(4); header.writeUInt32LE(json.length);
  return Buffer.concat([header, json]);
}
export class Decoder {
  buffer = Buffer.alloc(0);
  push(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    const messages = [];
    while (this.buffer.length >= 4) {
      const count = this.buffer.readUInt32LE();
      if (!count || count > MAX_FRAME) throw new Error('Invalid controller frame');
      if (this.buffer.length < count + 4) break;
      messages.push(JSON.parse(this.buffer.subarray(4, count + 4).toString('utf8')));
      this.buffer = this.buffer.subarray(count + 4);
    }
    return messages;
  }
}
export async function pipeName() {
  const { stdout } = await promisify(execFile)(`${process.env.SystemRoot || 'C:\\Windows'}\\System32\\whoami.exe`, ['/user', '/fo', 'csv', '/nh'], { windowsHide: true });
  const sid = stdout.match(/S-1-\d+(?:-\d+)+/)?.[0];
  if (!sid) throw new Error('Cannot determine current Windows user');
  return `\\\\.\\pipe\\PirateWave.Control.v1.${sid}`;
}
// All action contexts share one asynchronous connection. SDK callbacks only
// enqueue intents; transport failure discards every intent, never replays it.
export class ControlClient extends EventEmitter {
  snapshot = { generation: '0', entries: [], project: '', notice: '' };
  connected = false;
  status = 'PirateWave offline';
  queue = [];
  socket;
  pending;
  nextId = 0;
  timer;
  stopped = false;
  notifying = false;
  constructor(path, retryMs = 1000) {
    super(); this.path = path; this.retryMs = retryMs;
    // One listener per visible action type, not per key. The catalog has twelve
    // types, which legitimately exceeds EventEmitter's default warning limit.
    this.setMaxListeners(32);
  }
  notify() {
    if (this.notifying) return;
    this.notifying = true;
    setImmediate(() => {
      this.notifying = false;
      for (const listener of this.listeners('changed')) {
        try { listener(); } catch (e) { this.emit('diagnostic', `Feedback failed: ${e.message}`); }
      }
    });
  }
  reject(message) { this.status = message; this.notify(); return false; }
  submit(op, selector, ticks = 0, sensitivity = 1) {
    if (!this.connected || this.snapshot.generation === '0') return this.reject('PirateWave offline');
    if (!Number.isFinite(sensitivity) || sensitivity < 0.01 || sensitivity > 100 || !Number.isInteger(ticks) || Math.abs(ticks) > 10000) return this.reject('Invalid adjustment');
    const intent = { op, selector, ticks, sensitivity, generation: this.snapshot.generation };
    const last = this.queue.at(-1);
    if (op === 'adjust' && last?.op === op && selectorKey(last.selector) === selectorKey(selector) && last.sensitivity === sensitivity && last.generation === intent.generation && Math.sign(last.ticks) === Math.sign(ticks) && Math.abs(last.ticks + ticks) <= 10000) last.ticks += ticks;
    else {
      if (this.queue.length >= 128) return this.reject('Controller queue full; action rejected');
      this.queue.push(intent);
    }
    return true;
  }
  start() { if (!this.timer && !this.stopped) this.connect(); }
  stop() { this.stopped = true; clearInterval(this.timer); clearTimeout(this.retry); this.socket?.destroy(); this.queue = []; this.pending = undefined; }
  send(request) {
    this.pending = { ...request, id: ++this.nextId, at: Date.now() };
    const { at, ...wire } = this.pending;
    this.socket.write(frame(wire));
  }
  connect() {
    if (this.stopped) return;
    const socket = net.createConnection(this.path);
    this.socket = socket;
    const decoder = new Decoder();
    this.pending = undefined;
    this.lastReceived = Date.now();
    socket.on('connect', () => this.send({ op: 'hello', version: 2, client: 'streamdeck' }));
    socket.on('data', chunk => {
      try { for (const message of decoder.push(chunk)) this.receive(message); }
      catch (e) { this.status = e.message; socket.destroy(); }
    });
    socket.on('error', error => {
      if (this.lastDiagnostic !== error.message) { this.lastDiagnostic = error.message; this.emit('diagnostic', error.message); }
    });
    socket.on('close', () => {
      clearInterval(this.timer); this.timer = undefined;
      this.connected = false; this.queue = []; this.pending = undefined;
      if (!this.status.includes('Upgrade') && !this.status.includes('frame')) this.status = 'PirateWave offline';
      this.notify();
      if (!this.stopped) this.retry = setTimeout(() => this.connect(), this.retryMs);
    });
    this.timer = setInterval(() => {
      if (Date.now() - this.lastReceived > 4000 || (this.pending && Date.now() - this.pending.at > 3000)) {
        socket.destroy(); return;
      }
      if (!this.connected || this.pending) return;
      const intent = this.queue.shift();
      if (intent) {
        if (intent.generation !== this.snapshot.generation) this.reject('Project changed; action discarded');
        else this.send(intent);
      } else if (Date.now() - this.lastSent > 1000) this.send({ op: 'state' });
    }, 5);
  }
  receive(message) {
    this.lastReceived = Date.now();
    if (message.state) {
      const state = message.state;
      if (typeof state.generation !== 'string' || !/^\d+$/.test(state.generation) || !Array.isArray(state.entries)) throw new Error('Upgrade PirateWave: protocol v2 required');
      if (this.snapshot.generation !== state.generation) this.queue = [];
      this.snapshot = state; this.notify();
    }
    if (!this.pending || message.id !== this.pending.id) return;
    const op = this.pending.op; this.pending = undefined; this.lastSent = Date.now();
    if (op === 'hello') {
      if (!message.ok || message.version !== 2) { this.status = 'Upgrade PirateWave: protocol v2 required'; this.socket.destroy(); return; }
      this.send({ op: 'subscribe' });
    } else if (op === 'subscribe') {
      if (!message.ok || !message.state) throw new Error('PirateWave subscription failed');
      this.connected = true; this.status = 'Connected'; this.lastDiagnostic = ''; this.notify();
    } else if (op !== 'state') {
      this.status = message.ok ? 'Accepted' : (message.error || 'Operation failed'); this.notify();
    }
  }
}
