//! Bounded, UI-only field edits. Runtime replies and controller changes are not
//! captured, and undo checks the expected value before replacing a field.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
const MAX_ENTRIES: usize = 100;
const MAX_BYTES: usize = 16 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Serialize)]
enum Part {
    Key(String),
    Id(u64, usize),
}
#[derive(Clone, Serialize)]
struct Change {
    path: Vec<Part>,
    before: Option<Value>,
    after: Option<Value>,
}
struct Entry {
    changes: Vec<Change>,
    bytes: usize,
}
impl Entry {
    fn new(changes: Vec<Change>) -> Self {
        let bytes = serde_json::to_vec(&changes).map_or(MAX_BYTES + 1, |v| v.len());
        Self { changes, bytes }
    }
}
#[derive(Default)]
pub(super) struct Editing {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    group: Option<egui::Id>,
    pointer_group: bool,
    last: Option<Instant>,
    replayed: bool,
    pub clipboard: Option<Clipboard>,
    pub routing_selection: HashSet<u64>,
    paste_count: usize,
    devices: Option<mpsc::Receiver<DeviceResult>>,
}
type DeviceResult = Result<(Devices, Vec<crate::windows_audio::EndpointState>), String>;
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Clipboard {
    format: String,
    version: u32,
    nodes: Vec<Node>,
    connections: Vec<piratewave_core::project::Connection>,
}
impl Editing {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn refreshing(&self) -> bool {
        self.devices.is_some()
    }
    pub fn clear_workspace(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.routing_selection.clear();
        self.group = None;
        self.pointer_group = false;
        self.replayed = true;
    }
    fn record(&mut self, before: Value, after: Value, group: Option<egui::Id>, pointer: bool) {
        let mut changes = vec![];
        diff(Some(before), Some(after), &mut vec![], &mut changes);
        if changes.is_empty() {
            return;
        }
        let same_targets = self.undo.last().is_some_and(|entry| {
            changes
                .iter()
                .all(|c| entry.changes.iter().any(|old| old.path == c.path))
        });
        let coalesce = same_targets
            && self
                .last
                .is_some_and(|t| t.elapsed() < Duration::from_millis(750))
            && ((group.is_some() && group == self.group) || self.pointer_group && pointer);
        if coalesce {
            if let Some(previous) = self.undo.last_mut() {
                for change in changes {
                    if let Some(old) = previous
                        .changes
                        .iter_mut()
                        .find(|c| c.path == change.path && c.after == change.before)
                    {
                        old.after = change.after;
                    } else {
                        previous.changes.push(change);
                    }
                }
                previous.changes.retain(|c| c.before != c.after);
                previous.bytes =
                    serde_json::to_vec(&previous.changes).map_or(MAX_BYTES + 1, |v| v.len());
            }
        } else {
            self.undo.push(Entry::new(changes));
        }
        self.redo.clear();
        self.group = group;
        self.pointer_group = pointer;
        self.last = Some(Instant::now());
        self.trim();
    }
    fn trim(&mut self) {
        let size = |e: &Entry| e.bytes;
        while self.undo.len() + self.redo.len() > MAX_ENTRIES
            || self.undo.iter().chain(&self.redo).map(size).sum::<usize>() > MAX_BYTES
        {
            if !self.undo.is_empty() {
                self.undo.remove(0);
            } else if !self.redo.is_empty() {
                self.redo.remove(0);
            } else {
                break;
            }
        }
    }
}
fn diff(
    before: Option<Value>,
    after: Option<Value>,
    path: &mut Vec<Part>,
    changes: &mut Vec<Change>,
) {
    if before == after {
        return;
    }
    // Live results and recording transport are not configuration edits.
    if matches!(path.last(),Some(Part::Key(k)) if k=="take_ids" || k=="asset")
        || path == &[Part::Key("project".into()), Part::Key("name".into())]
        || matches!(path.last(),Some(Part::Key(k)) if k=="enabled")
            && path
                .iter()
                .any(|p| matches!(p,Part::Key(k) if k=="recording"))
    {
        return;
    }
    match (&before, &after) {
        (Some(Value::Object(a)), Some(Value::Object(b))) => {
            let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).cloned().collect();
            for k in keys {
                path.push(Part::Key(k.clone()));
                diff(a.get(&k).cloned(), b.get(&k).cloned(), path, changes);
                path.pop();
            }
        }
        (Some(Value::Array(a)), Some(Value::Array(b)))
            if a.iter()
                .chain(b)
                .all(|v| v.get("id").and_then(Value::as_u64).is_some()) =>
        {
            let ids: std::collections::BTreeSet<_> =
                a.iter().chain(b).filter_map(|v| v["id"].as_u64()).collect();
            for id in ids {
                let ai = a.iter().position(|v| v["id"] == id);
                let bi = b.iter().position(|v| v["id"] == id);
                path.push(Part::Id(id, ai.or(bi).unwrap()));
                diff(
                    ai.map(|i| a[i].clone()),
                    bi.map(|i| b[i].clone()),
                    path,
                    changes,
                );
                path.pop();
            }
            // Effect order is audible and must be restored too. Ordinary edits
            // retain ordering; separate ordering edits replace this array atomically.
            let old: Vec<_> = a.iter().map(|v| &v["id"]).collect();
            let new: Vec<_> = b.iter().map(|v| &v["id"]).collect();
            if old.len() == new.len() && old != new && old.iter().all(|id| new.contains(id)) {
                changes.retain(|c| !c.path.starts_with(path));
                changes.push(Change {
                    path: path.clone(),
                    before,
                    after,
                });
            }
        }
        _ => changes.push(Change {
            path: path.clone(),
            before,
            after,
        }),
    }
}
fn replace(
    root: &mut Value,
    path: &[Part],
    expected: &Option<Value>,
    value: &Option<Value>,
) -> bool {
    let Some((part, rest)) = path.split_first() else {
        return false;
    };
    match part {
        Part::Key(key) => {
            let Some(map) = root.as_object_mut() else {
                return false;
            };
            if rest.is_empty() {
                if map.get(key) != expected.as_ref() {
                    return false;
                }
                if let Some(v) = value {
                    map.insert(key.clone(), v.clone());
                } else {
                    map.remove(key);
                }
                true
            } else {
                map.get_mut(key)
                    .is_some_and(|v| replace(v, rest, expected, value))
            }
        }
        Part::Id(id, index) => {
            let Some(items) = root.as_array_mut() else {
                return false;
            };
            let found = items.iter().position(|v| v["id"] == *id);
            if rest.is_empty() {
                if found.map(|i| &items[i]) != expected.as_ref() {
                    return false;
                }
                match (found, value) {
                    (Some(i), Some(v)) => items[i] = v.clone(),
                    (Some(i), None) => {
                        items.remove(i);
                    }
                    (None, Some(v)) => items.insert((*index).min(items.len()), v.clone()),
                    _ => {}
                }
                true
            } else {
                found.is_some_and(|i| replace(&mut items[i], rest, expected, value))
            }
        }
    }
}
impl PirateWaveApp {
    fn edit_snapshot(&self) -> Value {
        serde_json::json!({"project":self.project,"order":self.app_settings.ui.mixer_orders.get(self.path.to_string_lossy().as_ref())})
    }
    pub(super) fn begin_edit_frame(&mut self, ctx: &egui::Context) -> Option<Value> {
        self.editing.replayed = false;
        // No workspace cloning/serialization in idle frames or in rendering.
        let active = ctx.input(|i| {
            i.pointer.any_down()
                || i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::PointerButton { .. }
                            | egui::Event::Key { .. }
                            | egui::Event::Text(_)
                            | egui::Event::Paste(_)
                            | egui::Event::Cut
                            | egui::Event::Copy
                            | egui::Event::MouseWheel { .. }
                    )
                })
        });
        active.then(|| self.edit_snapshot())
    }
    pub(super) fn finish_edit_frame(&mut self, ctx: &egui::Context, before: Option<Value>) {
        if self.editing.replayed {
            return;
        }
        if let Some(before) = before {
            let after = self.edit_snapshot();
            self.editing.record(
                before,
                after,
                ctx.memory(|m| m.focused()).filter(|_| {
                    ctx.input(|i| {
                        i.events.iter().any(|e| {
                            matches!(
                                e,
                                egui::Event::Text(_)
                                    | egui::Event::Paste(_)
                                    | egui::Event::Cut
                                    | egui::Event::Key {
                                        key: egui::Key::Backspace | egui::Key::Delete,
                                        ..
                                    }
                            )
                        })
                    })
                }),
                ctx.input(|i| i.pointer.any_down()),
            );
        }
        if !ctx.input(|i| i.pointer.any_down()) {
            self.editing.pointer_group = false;
        }
    }
    pub(super) fn undo_edit(&mut self, redo: bool) {
        let entry = if redo {
            self.editing.redo.pop()
        } else {
            self.editing.undo.pop()
        };
        let Some(entry) = entry else {
            return;
        };
        let mut next = self.edit_snapshot();
        let mut applied = 0;
        let changes: Vec<_> = if redo {
            entry.changes.iter().collect()
        } else {
            entry.changes.iter().rev().collect()
        };
        let mut successful = vec![];
        for c in changes {
            let (expected, value) = if redo {
                (&c.before, &c.after)
            } else {
                (&c.after, &c.before)
            };
            if replace(&mut next, &c.path, expected, value) {
                applied += 1;
                successful.push(c.clone());
            }
        }
        let result = (|| -> Result<(), String> {
            let project: Project =
                serde_json::from_value(next["project"].clone()).map_err(|e| e.to_string())?;
            piratewave_core::graph::validate(&project)?;
            let before = serde_json::to_value(&self.project).map_err(|e| e.to_string())?;
            let mut old_audio = before.clone();
            let mut new_audio = next["project"].clone();
            for p in [&mut old_audio, &mut new_audio] {
                if let Some(nodes) = p["nodes"].as_array_mut() {
                    for n in nodes {
                        n.as_object_mut().unwrap().remove("position");
                    }
                }
            }
            let restart = self.project.nodes.len() != project.nodes.len()
                || self.project.nodes.iter().zip(&project.nodes).any(|(a, b)| {
                    a.id != b.id
                        || a.kind != b.kind
                        || a.plugin.as_ref().map(|p| &p.descriptor)
                            != b.plugin.as_ref().map(|p| &p.descriptor)
                });
            let removed: Vec<_> = self
                .project
                .nodes
                .iter()
                .filter(|n| !project.nodes.iter().any(|v| v.id == n.id))
                .map(|n| n.id)
                .collect();
            let recording_nodes: HashSet<_> = self
                .project
                .nodes
                .iter()
                .chain(&project.nodes)
                .filter(|n| matches!(n.kind, NodeKind::Recording))
                .map(|n| n.id)
                .collect();
            for id in recording_nodes {
                let old = self
                    .project
                    .nodes
                    .iter()
                    .find(|n| n.id == id)
                    .map(|n| &n.recording.slots);
                let new = project
                    .nodes
                    .iter()
                    .find(|n| n.id == id)
                    .map(|n| &n.recording.slots);
                if old != new {
                    self.invalidate_recording_edits(id);
                }
            }
            for id in removed {
                self.remove_audio_node(id);
            }
            self.project = project;
            if old_audio != new_audio {
                if before["workflows"]["rules"] != next["project"]["workflows"]["rules"] {
                    self.arm_automation(false);
                }
                self.changed(restart);
            } else if before != next["project"] {
                self.dirty = true;
                self.changed_at = Instant::now();
            }
            let key = self.path.to_string_lossy().into_owned();
            if next["order"].is_null() {
                self.app_settings.ui.mixer_orders.remove(&key);
            } else {
                self.app_settings.ui.mixer_orders.insert(
                    key,
                    serde_json::from_value(next["order"].clone()).map_err(|e| e.to_string())?,
                );
            }
            self.layout_preferences_changed();
            self.refresh_shortcuts();
            self.pending_port = None;
            self.selected_edge = None;
            if !self.project.nodes.iter().any(|n| n.id == self.selected) {
                self.selected = self.project.nodes.first().map_or(0, |n| n.id);
                self.view = View::Routing;
            }
            self.editing
                .routing_selection
                .retain(|id| self.project.nodes.iter().any(|n| n.id == *id));
            Ok(())
        })();
        self.editing.replayed = true;
        self.editing.group = None;
        self.editing.pointer_group = false;
        match result {
            Ok(()) => {
                if !redo {
                    successful.reverse();
                }
                if applied > 0 {
                    let e = Entry::new(successful);
                    if redo {
                        self.editing.undo.push(e);
                    } else {
                        self.editing.redo.push(e);
                    }
                }
                self.notify(if applied == entry.changes.len() {
                    if redo {
                        "Redone"
                    } else {
                        "Undone"
                    }
                } else {
                    "History applied where possible; newer external changes were kept"
                });
            }
            Err(e) => {
                if redo {
                    self.editing.redo.push(entry);
                } else {
                    self.editing.undo.push(entry);
                }
                self.notify_error(format!("Cannot restore edit: {e}"));
            }
        }
    }
    pub(super) fn edit_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() || self.recorder.is_some() {
            return;
        }
        let redo = ctx.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::Z,
            )) || i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::Y,
            ))
        });
        let undo = !redo
            && ctx.input_mut(|i| {
                i.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND,
                    egui::Key::Z,
                ))
            });
        if redo || undo {
            self.undo_edit(redo);
            return;
        }
        if self.view == View::Routing {
            let copy = ctx.input_mut(|i| {
                let event = i.events.iter().any(|e| matches!(e, egui::Event::Copy));
                event
                    || i.consume_shortcut(&egui::KeyboardShortcut::new(
                        egui::Modifiers::COMMAND,
                        egui::Key::C,
                    ))
            });
            if copy {
                self.copy_routing_nodes(ctx);
            }
            let text = ctx.input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::Paste(s) = e {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
            });
            if let Some(text) = text {
                self.paste_routing_nodes(Some(&text));
            }
        }
    }
    pub(super) fn copy_routing_nodes(&mut self, ctx: &egui::Context) {
        let ids = if self.editing.routing_selection.is_empty() {
            HashSet::from([self.selected])
        } else {
            self.editing.routing_selection.clone()
        };
        let nodes: Vec<_> = self
            .project
            .nodes
            .iter()
            .filter(|n| ids.contains(&n.id))
            .cloned()
            .collect();
        if nodes.is_empty() {
            return;
        }
        let data = Clipboard {
            format: "PirateWave routing nodes".into(),
            version: 1,
            nodes,
            connections: self
                .project
                .connections
                .iter()
                .filter(|c| ids.contains(&c.from) && ids.contains(&c.to))
                .cloned()
                .collect(),
        };
        if let Ok(text) = serde_json::to_string(&data) {
            if text.len() > 8 * 1024 * 1024 {
                self.notify_error("Node clipboard exceeds 8 MiB");
                return;
            }
            ctx.copy_text(text);
            self.editing.clipboard = Some(data);
            self.editing.paste_count = 0;
            self.notify("Routing nodes copied");
        }
    }
    pub(super) fn paste_routing_nodes(&mut self, text: Option<&str>) {
        let data = if let Some(text) = text {
            if text.len() > 8 * 1024 * 1024 {
                self.notify_error("Node clipboard exceeds 8 MiB");
                return;
            }
            match serde_json::from_str::<Clipboard>(text) {
                Ok(c) => c,
                Err(_) => return,
            }
        } else {
            let Some(c) = self.editing.clipboard.clone() else {
                return;
            };
            c
        };
        if data.format != "PirateWave routing nodes"
            || data.version != 1
            || data.nodes.is_empty()
            || data.nodes.len() > 256
        {
            self.notify_error("Unsupported node clipboard");
            return;
        }
        let mut next = self.project.clone();
        let mut ids = HashMap::new();
        for node in &data.nodes {
            if ids
                .insert(node.id, piratewave_core::workflows::new_id())
                .is_some()
            {
                self.notify_error("Duplicate node in clipboard");
                return;
            }
        }
        let offset = 40. * (self.editing.paste_count + 1) as f32;
        for mut node in data.nodes {
            node.id = ids[&node.id];
            let base = node.name.clone();
            let mut number = 2;
            while next
                .nodes
                .iter()
                .any(|n| n.name.eq_ignore_ascii_case(&node.name))
            {
                node.name = format!("{base} {number}");
                number += 1;
            }
            node.position[0] += offset;
            node.position[1] += offset;
            node.keybinds.clear();
            node.switch_keybinds.clear();
            node.extension.keybinds.clear();
            node.recording.enabled = false;
            node.switch_output = node.switch_output.and_then(|id| ids.get(&id).copied());
            for e in &mut node.effects {
                e.id = piratewave_core::workflows::new_id();
                e.keybinds.clear();
                e.extension_keybinds.clear();
            }
            for s in &mut node.recording.slots {
                s.id = piratewave_core::workflows::new_id();
                s.keybinds.clear();
            }
            next.nodes.push(node);
        }
        for mut c in data.connections {
            let (Some(from), Some(to)) = (ids.get(&c.from), ids.get(&c.to)) else {
                self.notify_error("Clipboard connection target missing");
                return;
            };
            c.from = *from;
            c.to = *to;
            next.connections.push(c);
        }
        if let Err(e) = piratewave_core::graph::validate(&next) {
            self.notify_error(format!("Cannot paste nodes: {e}"));
            return;
        }
        self.project = next;
        self.editing.paste_count += 1;
        self.editing.routing_selection = ids.values().copied().collect();
        self.selected = self
            .project
            .nodes
            .iter()
            .rev()
            .find(|n| self.editing.routing_selection.contains(&n.id))
            .unwrap()
            .id;
        self.layout.routing_focus = Some(self.selected);
        self.refresh_shortcuts();
        self.changed(true);
        self.notify("Nodes pasted with new names and no shortcut conflicts");
    }
    pub(super) fn refresh_audio_devices(&mut self) {
        if self.editing.refreshing() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.editing.devices = Some(rx);
        thread::spawn(move || {
            let result = audio_io::devices()
                .map(|devices| {
                    (
                        devices,
                        crate::windows_audio::cable_endpoints().unwrap_or_default(),
                    )
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
    }
    pub(super) fn poll_device_refresh(&mut self) {
        let result = self
            .editing
            .devices
            .as_ref()
            .and_then(|rx| match rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Device refresh worker stopped".into()))
                }
                Err(_) => None,
            });
        if let Some(result) = result {
            self.editing.devices = None;
            match result {
                Ok((devices, states)) => {
                    self.devices = devices;
                    self.endpoint_states = states;
                    self.notify("Audio devices refreshed; device assignments kept");
                }
                Err(e) => self.notify_error(format!("Device discovery failed: {e}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::app;
    use super::*;
    fn edit(app: &mut PirateWaveApp, f: impl FnOnce(&mut PirateWaveApp)) {
        let before = app.edit_snapshot();
        f(app);
        let after = app.edit_snapshot();
        app.editing.record(before, after, None, false);
    }
    #[test]
    fn gain_undo_redo_preserves_unrelated_controller_edits() {
        let mut app = app();
        edit(&mut app, |a| a.project.nodes[0].gain_db = -12.);
        app.project.nodes[1].pan = 0.7;
        app.undo_edit(false);
        assert_eq!(app.project.nodes[0].gain_db, 0.);
        assert_eq!(app.project.nodes[1].pan, 0.7);
        app.undo_edit(true);
        assert_eq!(app.project.nodes[0].gain_db, -12.);
        assert_eq!(app.project.nodes[1].pan, 0.7);
        // A newer change to the same control is preserved too.
        app.project.nodes[0].gain_db = -4.;
        app.undo_edit(false);
        assert_eq!(app.project.nodes[0].gain_db, -4.);
    }
    #[test]
    fn layout_undo_sends_no_audio_commands_and_order_remains_settings_only() {
        let mut app = app();
        let (engine, rx) = Engine::test_channel();
        app.engine = engine;
        let original = app.project.nodes[0].position;
        edit(&mut app, |a| a.project.nodes[0].position = [1200., 900.]);
        app.undo_edit(false);
        assert_eq!(app.project.nodes[0].position, original);
        assert!(rx.try_recv().is_err());
        assert!(!app.needs_apply);
        app.dirty = false;
        edit(&mut app, |a| {
            a.app_settings
                .ui
                .mixer_orders
                .insert(String::new(), vec![3, 2, 1]);
        });
        app.undo_edit(false);
        assert!(!app.app_settings.ui.mixer_orders.contains_key(""));
        assert!(!app.dirty);
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn typing_and_drag_coalesce_and_history_is_bounded_and_workspace_scoped() {
        let mut h = Editing::default();
        let focus = egui::Id::new("field");
        h.record(
            serde_json::json!({"v":0}),
            serde_json::json!({"v":1}),
            Some(focus),
            false,
        );
        h.record(
            serde_json::json!({"v":1}),
            serde_json::json!({"v":2}),
            Some(focus),
            false,
        );
        assert_eq!(h.undo.len(), 1);
        assert_eq!(h.undo[0].changes[0].before, Some(serde_json::json!(0)));
        for i in 2..150 {
            h.record(
                serde_json::json!({"v":i}),
                serde_json::json!({"v":i+1}),
                None,
                false,
            );
        }
        assert_eq!(h.undo.len(), MAX_ENTRIES);
        h.record(
            serde_json::json!({"v":0}),
            serde_json::json!({"v":"x".repeat(MAX_BYTES+1)}),
            None,
            false,
        );
        assert!(h.undo.is_empty());
        h.clear_workspace();
        assert!(!h.can_redo());
        assert!(!h.can_undo());
    }
    #[test]
    fn routing_clipboard_remaps_nodes_effects_slots_routes_and_clears_shortcuts() {
        use piratewave_core::{
            project::{Connection, InputPort, OutputPort},
            recording::RecordSlot,
        };
        let mut app = app();
        app.project.nodes = vec![
            Node::new(1, "Source", NodeKind::Soundboard, [10., 20.]),
            Node::new(2, "Replay", NodeKind::Recording, [300., 20.]),
        ];
        app.project.connections = vec![Connection {
            from: 1,
            to: 2,
            port: OutputPort::Passthrough,
            input: InputPort::Main,
        }];
        app.project.nodes[1]
            .effects
            .push(EffectSpec::new(app.registry.get("gain").unwrap()));
        app.project.nodes[1]
            .recording
            .slots
            .push(RecordSlot::new("Last phrase"));
        app.project.nodes[0]
            .keybinds
            .insert(NodeAction::ToggleMute, "F8".into());
        app.editing.routing_selection = HashSet::from([1, 2]);
        let ctx = egui::Context::default();
        app.copy_routing_nodes(&ctx);
        let before = app.edit_snapshot();
        app.paste_routing_nodes(None);
        let after = app.edit_snapshot();
        app.editing.record(before, after, None, false);
        assert_eq!(app.project.nodes.len(), 4);
        assert_eq!(app.project.connections.len(), 2);
        let a = &app.project.nodes[2];
        let b = &app.project.nodes[3];
        assert_eq!(a.name, "Source 2");
        assert_eq!(b.name, "Replay 2");
        assert!(a.keybinds.is_empty());
        assert_ne!(b.effects[0].id, app.project.nodes[1].effects[0].id);
        assert_ne!(
            b.recording.slots[0].id,
            app.project.nodes[1].recording.slots[0].id
        );
        assert_eq!(app.project.connections[1].from, a.id);
        assert_eq!(app.project.connections[1].to, b.id);
        app.undo_edit(false);
        assert_eq!(app.project.nodes.len(), 2);
        assert_eq!(app.project.connections.len(), 1);
        app.undo_edit(true);
        assert_eq!(app.project.nodes.len(), 4);
        app.paste_routing_nodes(None);
        assert_eq!(app.project.nodes[4].name, "Source 3");
        assert!(app.project.nodes[4].position[0] > app.project.nodes[2].position[0]);
    }
    #[test]
    fn copy_one_node_does_not_duplicate_external_connections_and_paste_validates_atomically() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.selected = app.project.nodes[0].id;
        let routes = app.project.connections.len();
        app.copy_routing_nodes(&ctx);
        app.paste_routing_nodes(None);
        assert_eq!(app.project.connections.len(), routes);
        let original = app.project.nodes.len();
        let mut bad = app.editing.clipboard.clone().unwrap();
        bad.nodes.push(bad.nodes[0].clone());
        app.paste_routing_nodes(Some(&serde_json::to_string(&bad).unwrap()));
        assert_eq!(app.project.nodes.len(), original);
        app.paste_routing_nodes(Some("ordinary clipboard text"));
        assert_eq!(app.project.nodes.len(), original);
    }
    #[test]
    fn device_refresh_replaces_dropdown_choices_without_reassigning_or_updating_audio() {
        let mut app = app();
        let (engine, rx) = Engine::test_channel();
        app.engine = engine;
        let before = serde_json::to_value(&app.project).unwrap();
        let (tx, reply) = mpsc::channel();
        app.editing.devices = Some(reply);
        tx.send(Ok((
            Devices {
                inputs: vec![audio_io::DeviceInfo {
                    id: "new".into(),
                    name: "New mic".into(),
                }],
                outputs: vec![],
            },
            vec![],
        )))
        .unwrap();
        app.poll_device_refresh();
        assert_eq!(app.devices.inputs[0].name, "New mic");
        assert_eq!(serde_json::to_value(&app.project).unwrap(), before);
        assert!(!app.dirty);
        assert!(rx.try_recv().is_err());
        let (tx, reply) = mpsc::channel();
        app.editing.devices = Some(reply);
        tx.send(Err("test failure".into())).unwrap();
        app.poll_device_refresh();
        assert_eq!(app.devices.inputs[0].name, "New mic");
        assert!(!app.editing.refreshing());
    }
    #[test]
    fn standard_shortcuts_work_and_recording_label_is_clear() {
        let mut app = app();
        app.view = View::Routing;
        let ctx = egui::Context::default();
        edit(&mut app, |a| a.project.nodes[0].gain_db = -9.);
        let key = |key, shift| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                command: true,
                ctrl: true,
                shift,
                ..Default::default()
            },
        };
        let _ = ctx.run(
            egui::RawInput {
                events: vec![key(egui::Key::Z, false)],
                ..Default::default()
            },
            |ctx| app.edit_shortcuts(ctx),
        );
        assert_eq!(app.project.nodes[0].gain_db, 0.);
        let _ = ctx.run(
            egui::RawInput {
                events: vec![key(egui::Key::Z, true)],
                ..Default::default()
            },
            |ctx| app.edit_shortcuts(ctx),
        );
        assert_eq!(app.project.nodes[0].gain_db, -9.);
        let _ = ctx.run(
            egui::RawInput {
                events: vec![key(egui::Key::C, false)],
                ..Default::default()
            },
            |ctx| app.edit_shortcuts(ctx),
        );
        let text = serde_json::to_string(app.editing.clipboard.as_ref().unwrap()).unwrap();
        let count = app.project.nodes.len();
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Paste(text)],
                ..Default::default()
            },
            |ctx| app.edit_shortcuts(ctx),
        );
        assert_eq!(app.project.nodes.len(), count + 1);
        assert_eq!(View::Sessions.name(), "Recording");
    }
    #[test]
    fn native_text_undo_retains_keyboard_priority() {
        let mut app = app();
        let ctx = egui::Context::default();
        edit(&mut app, |a| a.project.nodes[0].gain_db = -5.);
        let mut text = "Text".to_owned();
        let id = egui::Id::new("typing");
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add(egui::TextEdit::singleline(&mut text).id(id))
                    .request_focus();
            });
        });
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Z,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }],
                ..Default::default()
            },
            |ctx| {
                app.edit_shortcuts(ctx);
            },
        );
        assert_eq!(app.project.nodes[0].gain_db, -5.);
        assert!(app.editing.can_undo());
    }
}
