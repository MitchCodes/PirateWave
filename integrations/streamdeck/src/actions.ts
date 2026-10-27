import { SingletonAction, type Action, type WillAppearEvent, type WillDisappearEvent,
  type DidReceiveSettingsEvent, type KeyDownEvent, type DialDownEvent, type DialRotateEvent,
  type TouchTapEvent, type PropertyInspectorDidAppearEvent, type SendToPluginEvent } from '@elgato/streamdeck';
import { entriesFor, find, press, feedback, image } from './model.mjs';
import { ControlClient } from './client.mjs';
import type { JsonObject } from '@elgato/utils';

export type Selector = { category: string; node_kind: string; node: string; target: string; action: string; plugin: string };
export type Settings = JsonObject & { schema?: number; selector?: Selector; node?: string; target?: string; operation?: string; sensitivity?: number };
export type Spec = { id: string; name: string; adjustment: boolean; category: string; parameter?: string };
type UI = { action?: { id: string }; sendToPropertyInspector(payload: JsonObject): Promise<void> };
type Context = { action: Action<Settings>; settings: Settings; rendered: string; rendering: boolean; refresh: boolean };

// Exported so SDK callback tests exercise the production adapter with a simulated
// device surface; there is no alternate test-only command implementation.
export class PirateWaveAction extends SingletonAction<Settings> {
  override readonly manifestId: string;
  private contexts = new Map<string, Context>();
  private editorContext = '';
  private editorSignature = '';
  private listener = () => { for (const context of this.contexts.values()) void this.render(context); void this.editor(); };
  constructor(readonly spec: Spec, private client: ControlClient, private ui: UI, private log: (error: unknown) => void) {
    super(); this.manifestId = `com.piratewave.streamdeck.${spec.id}`;
  }
  override async onWillAppear(ev: WillAppearEvent<Settings>) {
    const context: Context = { action: ev.action, settings: ev.payload.settings, rendered: '', rendering: false, refresh: false };
    this.contexts.set(ev.action.id, context);
    if (this.contexts.size === 1) this.client.on('changed', this.listener);
    await this.render(context);
    // SDK v3 getSettings returns the result; it does not emit didReceiveSettings.
    const initial = context.settings;
    try {
      const saved = await ev.action.getSettings();
      if (this.contexts.get(ev.action.id) === context && context.settings === initial) { context.settings = saved; await this.render(context); }
    } catch (e) { this.log(e); }
  }
  override onWillDisappear(ev: WillDisappearEvent<Settings>) {
    this.contexts.delete(ev.action.id);
    if (!this.contexts.size) this.client.off('changed', this.listener);
    if (this.editorContext === ev.action.id) this.editorContext = '';
  }
  override async onDidReceiveSettings(ev: DidReceiveSettingsEvent<Settings>) {
    const context = this.contexts.get(ev.action.id);
    if (context) { context.settings = ev.payload.settings; await this.render(context); }
    this.editorSignature = ''; await this.editor();
  }
  override async onPropertyInspectorDidAppear(ev: PropertyInspectorDidAppearEvent<Settings>) {
    this.editorContext = ev.action.id; this.editorSignature = ''; await this.editor();
  }
  override onPropertyInspectorDidDisappear() { this.editorContext = ''; this.editorSignature = ''; }
  override async onSendToPlugin(ev: SendToPluginEvent<JsonObject, Settings>) {
    if (ev.payload?.event === 'catalog') { this.editorContext = ev.action.id; this.editorSignature = ''; await this.editor(); }
  }
  private async editor() {
    if (!this.editorContext || this.ui.action?.id !== this.editorContext) return;
    const entries = entriesFor(this.client.snapshot.entries, this.spec);
    const signature = JSON.stringify([this.client.connected, this.client.status, entries.map((e: { selector: Selector; label: string }) => [e.selector, e.label])]);
    if (signature === this.editorSignature) return;
    this.editorSignature = signature;
    try { await this.ui.sendToPropertyInspector({ event: 'catalog', context: this.editorContext, entries, adjustment: this.spec.adjustment, connected: this.client.connected, status: this.client.status }); }
    catch (e) { this.editorSignature = ''; this.log(e); }
  }
  private async render(context: Context) {
    if (context.rendering) { context.refresh = true; return; }
    context.rendering = true;
    try {
      do {
        context.refresh = false;
        if (this.contexts.get(context.action.id) !== context) return;
        let view = feedback(this.client, context.settings);
        if (context.action.isKey() && this.spec.adjustment && 'label' in view) {
          const operation = context.settings.operation === 'reset' ? 'Reset' : context.settings.operation === 'decrease' ? '−' : '+';
          view = { ...view, label: `${operation} ${view.label}` };
        }
        const signature = JSON.stringify(view);
        if (signature === context.rendered) continue;
        if (context.action.isDial()) {
          const range = 'min' in view && 'max' in view && view.max !== view.min ? (view.value! - view.min!) / (view.max! - view.min!) * 100 : 0;
          await context.action.setFeedback({ title: [view.title, view.label].filter(Boolean).join(" · "), value: view.detail, indicator: Math.max(0, Math.min(100, range)) });
        } else if (context.action.isKey()) {
          await context.action.setTitle('');
          await context.action.setImage(image(view));
        }
        context.rendered = signature;
      } while (context.refresh);
    } catch (e) { this.log(e); }
    finally { context.rendering = false; }
  }
  private async run(action: Action<Settings>, settings: Settings, ticks?: number) {
    const available = entriesFor(this.client.snapshot.entries, this.spec);
    const { entry, error } = find(available, settings.selector);
    let ok = false;
    if (!this.client.connected) this.client.reject('PirateWave offline');
    else if (!entry) this.client.reject(error || 'Missing target');
    else if (settings.schema !== undefined && settings.schema !== 1) this.client.reject('Unsupported assignment version');
    else if (entry.adjustment) {
      if (ticks !== undefined) ok = this.client.submit('adjust', entry.selector, ticks, settings.sensitivity ?? 1);
      else if (!action.isDial() && settings.operation === 'reset') ok = this.client.submit('reset', entry.selector);
      else if (action.isDial()) {
        const command = press(this.client.snapshot.entries, entry);
        if (command) ok = this.client.submit(command.op, command.selector);
        else this.client.reject('Missing mute target');
      } else ok = this.client.submit('adjust', entry.selector, settings.operation === 'decrease' ? -1 : 1, settings.sensitivity ?? 1);
    } else ok = this.client.submit('execute', entry.selector);
    if (!ok && (action.isKey() || action.isDial())) await action.showAlert();
  }
  override onKeyDown(ev: KeyDownEvent<Settings>) { return this.run(ev.action, ev.payload.settings); }
  override onDialDown(ev: DialDownEvent<Settings>) { return this.run(ev.action, ev.payload.settings); }
  override onTouchTap(ev: TouchTapEvent<Settings>) { return this.run(ev.action, ev.payload.settings); }
  override onDialRotate(ev: DialRotateEvent<Settings>) { return this.run(ev.action, ev.payload.settings, ev.payload.ticks); }
}
