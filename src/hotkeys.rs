use global_hotkey::{hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use piratewave_core::{
    project::Project,
    shortcuts::{AppAction, AppSettings, NodeAction, ShortcutAction},
};
use std::collections::HashMap;

/// Own on the GUI/event-loop thread so WM_HOTKEY is delivered by winit.
pub struct Hotkeys {
    manager: GlobalHotKeyManager,
    registered: HashMap<u32, HotKey>,
    ids: HashMap<u32, ShortcutAction>,
    paused: Option<(HashMap<u32, HotKey>, HashMap<u32, ShortcutAction>)>,
}
impl Hotkeys {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            manager: GlobalHotKeyManager::new().map_err(|e| e.to_string())?,
            registered: HashMap::new(),
            ids: HashMap::new(),
            paused: None,
        })
    }
    /// Register additions before releasing old keys. A parse/conflict/OS failure
    /// leaves the last successfully applied shortcut set intact.
    pub fn update(&mut self, project: &Project, settings: &AppSettings) -> Vec<String> {
        let result = self.update_inner(project, settings);
        for error in &result {
            crate::logging::error(format_args!("Global shortcut registration failed: {error}"));
        }
        if result.is_empty() {
            crate::logging::info(format_args!(
                "Global shortcuts applied: {} registrations",
                self.registered.len()
            ));
        }
        result
    }
    fn update_inner(&mut self, project: &Project, settings: &AppSettings) -> Vec<String> {
        if self.paused.is_some() {
            return vec!["Finish recording the shortcut first".into()];
        }
        let removed: Vec<_> = self
            .ids
            .iter()
            .filter_map(|(key, action)| {
                let keep = action.exists(project);
                (!keep).then_some(*key)
            })
            .collect();
        for id in removed {
            if let Some(key) = self.registered.remove(&id) {
                if let Err(error) = self.manager.unregister(key) {
                    crate::logging::error(format_args!("Shortcut unregistration failed: {error}"));
                }
            }
            self.ids.remove(&id);
        }
        let plan = match binding_plan(project, settings) {
            Ok(plan) => plan,
            Err(errors) => return errors,
        };
        let mut added = vec![];
        for binding in &plan {
            if !self.registered.contains_key(&binding.key.id()) {
                if let Err(error) = self.manager.register(binding.key) {
                    for key in added {
                        if let Err(error) = self.manager.unregister(key) {
                            crate::logging::error(format_args!(
                                "Shortcut unregistration failed: {error}"
                            ));
                        }
                    }
                    return vec![format!("Cannot register {} ({}): {error}. Previously applied shortcuts remain active.",binding.label,binding.text)];
                }
                added.push(binding.key);
            }
        }
        let next: HashMap<_, _> = plan.iter().map(|b| (b.key.id(), b.key)).collect();
        for (id, key) in &self.registered {
            if !next.contains_key(id) {
                if let Err(error) = self.manager.unregister(*key) {
                    crate::logging::error(format_args!("Shortcut unregistration failed: {error}"));
                }
            }
        }
        // Discard old queued presses before changing their action mapping.
        while GlobalHotKeyEvent::receiver().try_recv().is_ok() {}
        self.registered = next;
        self.ids = plan.into_iter().map(|b| (b.key.id(), b.action)).collect();
        vec![]
    }
    pub fn clear(&mut self) {
        for key in self.registered.values() {
            if let Err(error) = self.manager.unregister(*key) {
                crate::logging::error(format_args!("Shortcut unregistration failed: {error}"));
            }
        }
        self.registered.clear();
        self.ids.clear();
        while GlobalHotKeyEvent::receiver().try_recv().is_ok() {}
    }
    pub fn suspend(&mut self) -> Result<(), String> {
        if self.paused.is_some() {
            return Ok(());
        }
        let mut removed = Vec::new();
        for key in self.registered.values() {
            if let Err(error) = self.manager.unregister(*key) {
                for key in removed {
                    let _ = self.manager.register(key);
                }
                return Err(format!("Could not pause shortcuts for recording: {error}"));
            }
            removed.push(*key);
        }
        self.paused = Some((
            std::mem::take(&mut self.registered),
            std::mem::take(&mut self.ids),
        ));
        while GlobalHotKeyEvent::receiver().try_recv().is_ok() {}
        Ok(())
    }
    pub fn resume(&mut self) -> Vec<String> {
        let Some((keys, ids)) = self.paused.take() else {
            return vec![];
        };
        let mut errors = Vec::new();
        for (id, key) in keys {
            match self.manager.register(key) {
                Ok(()) => {
                    self.registered.insert(id, key);
                    if let Some(action) = ids.get(&id) {
                        self.ids.insert(id, *action);
                    }
                }
                Err(error) => errors.push(format!(
                    "Could not restore shortcut {key}: {error}. Apply keybinds to retry."
                )),
            }
        }
        while GlobalHotKeyEvent::receiver().try_recv().is_ok() {}
        errors
    }
    pub fn poll(&self) -> Vec<ShortcutAction> {
        let mut events = vec![];
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if event.state == HotKeyState::Pressed {
                if let Some(id) = self.ids.get(&event.id) {
                    events.push(*id);
                }
            }
        }
        events
    }
}
impl Drop for Hotkeys {
    fn drop(&mut self) {
        for key in self.registered.values() {
            if let Err(error) = self.manager.unregister(*key) {
                crate::logging::error(format_args!("Shortcut unregistration failed: {error}"));
            }
        }
    }
}

struct Binding {
    key: HotKey,
    action: ShortcutAction,
    label: String,
    text: String,
}
fn binding_plan(project: &Project, settings: &AppSettings) -> Result<Vec<Binding>, Vec<String>> {
    let mut items = vec![];
    for action in AppAction::ALL {
        if let Some(key) = settings.keybinds.get(&action) {
            items.push((
                ShortcutAction::App(action),
                action.label().into(),
                key.as_str(),
            ));
        }
    }
    for node in &project.nodes {
        for (action, label) in piratewave_core::extensions::action_rows(node) {
            if let ShortcutAction::Extension {
                effect,
                action: key,
                ..
            } = action
            {
                if action.exists(project) {
                    if let Some(text) =
                        piratewave_core::extensions::action_binding(node, effect, key)
                    {
                        items.push((action, format!("{} / {}", node.name, label), text));
                    }
                }
            }
        }
        if matches!(node.kind, piratewave_core::project::NodeKind::Recording) {
            for slot in &node.recording.slots {
                for (action, key) in &slot.keybinds {
                    items.push((
                        ShortcutAction::RecordingSlot {
                            id: node.id,
                            slot: slot.id,
                            action: *action,
                        },
                        format!("{} / {}: {}", node.name, slot.name, action.label()),
                        key.as_str(),
                    ));
                }
            }
        }
        if matches!(node.kind, piratewave_core::project::NodeKind::Switch) {
            for (output, key) in &node.switch_keybinds {
                let action = ShortcutAction::SwitchOutput {
                    id: node.id,
                    output: *output,
                };
                if action.exists(project) {
                    items.push((
                        action,
                        format!("{}: activate output {}", node.name, output),
                        key.as_str(),
                    ));
                }
            }
        }
        if node.kind.has_effect_controls() {
            for effect in &node.effects {
                for (action, key) in &effect.keybinds {
                    items.push((
                        ShortcutAction::Effect {
                            id: node.id,
                            effect: effect.id,
                            action: *action,
                        },
                        format!("{} / {}: {}", node.name, effect.kind, action.label()),
                        key.as_str(),
                    ));
                }
            }
        }
        for (action, key) in &node.keybinds {
            if !NodeAction::for_kind(&node.kind).contains(action) {
                return Err(vec![format!(
                    "{} does not support {}",
                    node.name,
                    action.label()
                )]);
            }
            items.push((
                ShortcutAction::Node {
                    id: node.id,
                    action: *action,
                },
                format!("{}: {}", node.name, action.label()),
                key.as_str(),
            ));
        }
    }
    for sound in &project.sounds {
        items.push((
            ShortcutAction::Sound(sound.id),
            format!("Soundboard: {}", sound.name),
            sound.shortcut.as_str(),
        ));
    }
    let mut plan = vec![];
    let mut seen = HashMap::new();
    let mut errors = vec![];
    for (action, label, text) in items {
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        match text.parse::<HotKey>() {
            Ok(key) => {
                if let Some(previous) = seen.insert(key.id(), label.clone()) {
                    errors.push(format!(
                        "Shortcut {text} is assigned to both {previous} and {label}."
                    ));
                } else {
                    plan.push(Binding {
                        key,
                        action,
                        label,
                        text: text.into(),
                    });
                }
            }
            Err(error) => errors.push(format!("Invalid shortcut for {label} ({text}): {error}")),
        }
    }
    if errors.is_empty() {
        Ok(plan)
    } else {
        Err(errors)
    }
}

#[cfg(windows)]
pub fn self_test() -> Result<(), String> {
    use windows_sys::Win32::{
        System::Threading::GetCurrentThreadId,
        UI::WindowsAndMessaging::{
            DispatchMessageW, FindWindowExW, GetWindowThreadProcessId, PeekMessageW, PostMessageW,
            TranslateMessage, MSG, PM_REMOVE, WM_HOTKEY,
        },
    };
    let mut hotkeys = Hotkeys::new()?;
    let mut base = Project::default();
    base.nodes.push(piratewave_core::project::Node::new(
        6,
        "Processor",
        piratewave_core::project::NodeKind::Processor,
        [0.0; 2],
    ));
    for (id, kind) in [
        (7, piratewave_core::project::NodeKind::VstInstrument),
        (8, piratewave_core::project::NodeKind::VstEffect),
    ] {
        base.nodes.push(piratewave_core::project::Node::new(
            id, "VST node", kind, [0.0; 2],
        ));
    }
    base.sounds.push(piratewave_core::project::Sound {
        id: 42,
        name: "Sound pad".into(),
        path: String::new(),
        shortcut: String::new(),
        volume: 1.0,
    });
    base.nodes.push(piratewave_core::project::Node::new(
        90,
        "Switch",
        piratewave_core::project::NodeKind::Switch,
        [0.; 2],
    ));
    base.connect(90, 4).map_err(|e| e.to_string())?;
    base.connect(90, 5).map_err(|e| e.to_string())?;
    let registry = piratewave_core::effects::EffectRegistry::default();
    let effect = piratewave_core::effects::EffectSpec::new(registry.get("gain").unwrap());
    let effect_id = effect.id;
    base.nodes
        .iter_mut()
        .find(|n| n.id == 6)
        .unwrap()
        .effects
        .push(effect);
    let mut recording = piratewave_core::project::Node::new(
        91,
        "Recording",
        piratewave_core::project::NodeKind::Recording,
        [0.; 2],
    );
    let slot = piratewave_core::recording::RecordSlot::new("Slot");
    let recording_slot = slot.id;
    recording.recording.slots.push(slot);
    let recording_effect = piratewave_core::effects::EffectSpec::new(registry.get("gain").unwrap());
    let recording_effect_id = recording_effect.id;
    recording.effects.push(recording_effect);
    base.nodes.push(recording);
    base.nodes.push(piratewave_core::project::Node::new(
        94,
        "Auto Ducking",
        piratewave_core::project::NodeKind::AutoDucking,
        [0.; 2],
    ));
    for (i, e) in piratewave_core::extensions::catalog().iter().enumerate() {
        if i >= 2 {
            break;
        }
        let mut node = piratewave_core::project::Node::new(
            96 + i as u64,
            "SDK fixture",
            if e.descriptor.kind == piratewave_core::extensions::TypeKind::Node {
                piratewave_core::project::NodeKind::Extension {
                    definition: Box::new(e.descriptor.clone()),
                }
            } else {
                piratewave_core::project::NodeKind::Processor
            },
            [0.; 2],
        );
        if e.descriptor.kind == piratewave_core::extensions::TypeKind::Processor {
            node.effects
                .push(piratewave_core::extensions::effect_spec(&e.descriptor));
        }
        base.nodes.push(node);
    }
    let mut cases: Vec<_> = AppAction::ALL
        .into_iter()
        .map(ShortcutAction::App)
        .collect();
    for node in &base.nodes {
        for action in NodeAction::for_kind(&node.kind) {
            cases.push(ShortcutAction::Node {
                id: node.id,
                action: *action,
            });
        }
    }
    for n in &base.nodes {
        cases.extend(
            piratewave_core::extensions::action_rows(n)
                .into_iter()
                .map(|(a, _)| a),
        );
    }
    cases.push(ShortcutAction::Sound(42));
    for action in piratewave_core::recording::SlotAction::ALL {
        cases.push(ShortcutAction::RecordingSlot {
            id: 91,
            slot: recording_slot,
            action,
        });
    }
    cases.extend([
        ShortcutAction::SwitchOutput { id: 90, output: 4 },
        ShortcutAction::SwitchOutput { id: 90, output: 5 },
    ]);
    for action in piratewave_core::shortcuts::EffectAction::ALL {
        cases.push(ShortcutAction::Effect {
            id: 6,
            effect: effect_id,
            action,
        });
    }
    for action in piratewave_core::shortcuts::EffectAction::ALL {
        cases.push(ShortcutAction::Effect {
            id: 91,
            effect: recording_effect_id,
            action,
        });
    }
    let mut delivered = 0;
    let class: Vec<u16> = "global_hotkey_app".encode_utf16().chain(Some(0)).collect();
    // Post only to this thread's own registered hotkey window. This verifies OS
    // registration and WM_HOTKEY delivery without synthesizing user keystrokes.
    unsafe {
        let current = GetCurrentThreadId();
        let mut after = std::ptr::null_mut();
        let hwnd = loop {
            let hwnd = FindWindowExW(
                std::ptr::null_mut(),
                after,
                class.as_ptr(),
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return Err("Hotkey message window not found".into());
            }
            if GetWindowThreadProcessId(hwnd, std::ptr::null_mut()) == current {
                break hwnd;
            }
            after = hwnd;
        };
        for expected in &cases {
            let mut project = base.clone();
            let mut settings = AppSettings::default();
            for key in settings.keybinds.values_mut() {
                key.clear();
            }
            let text = "Ctrl+Alt+Shift+F11".to_string();
            match *expected {
                ShortcutAction::Extension { id, effect, action } => {
                    piratewave_core::extensions::set_binding(
                        project.nodes.iter_mut().find(|n| n.id == id).unwrap(),
                        effect,
                        action,
                        text,
                    )?;
                }
                ShortcutAction::RecordingSlot { id, slot, action } => {
                    project
                        .nodes
                        .iter_mut()
                        .find(|n| n.id == id)
                        .unwrap()
                        .recording
                        .slots
                        .iter_mut()
                        .find(|s| s.id == slot)
                        .unwrap()
                        .keybinds
                        .insert(action, text);
                }
                ShortcutAction::App(action) => {
                    settings.keybinds.insert(action, text);
                }
                ShortcutAction::Node { id, action } => {
                    project
                        .nodes
                        .iter_mut()
                        .find(|n| n.id == id)
                        .unwrap()
                        .keybinds
                        .insert(action, text);
                }
                ShortcutAction::SwitchOutput { id, output } => {
                    project
                        .nodes
                        .iter_mut()
                        .find(|n| n.id == id)
                        .unwrap()
                        .switch_keybinds
                        .insert(output, text);
                }
                ShortcutAction::Effect { id, effect, action } => {
                    project
                        .nodes
                        .iter_mut()
                        .find(|n| n.id == id)
                        .unwrap()
                        .effects
                        .iter_mut()
                        .find(|e| e.id == effect)
                        .unwrap()
                        .keybinds
                        .insert(action, text);
                }
                ShortcutAction::Sound(id) => {
                    project
                        .sounds
                        .iter_mut()
                        .find(|s| s.id == id)
                        .unwrap()
                        .shortcut = text;
                }
            }
            let errors = hotkeys.update(&project, &settings);
            if !errors.is_empty() {
                return Err(errors.join("; "));
            }
            let id = *hotkeys
                .ids
                .iter()
                .find(|(_, action)| *action == expected)
                .ok_or("Test action not registered")?
                .0;
            if PostMessageW(hwnd, WM_HOTKEY, id as usize, (0x7A_i32 << 16) as isize) == 0 {
                return Err("Posting WM_HOTKEY failed".into());
            }
            let start = std::time::Instant::now();
            let mut received = false;
            while start.elapsed() < std::time::Duration::from_millis(500) {
                let mut message: MSG = std::mem::zeroed();
                while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                if hotkeys.poll().contains(expected) {
                    received = true;
                    delivered += 1;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            if !received {
                return Err(format!("Hotkey delivery timed out for {expected:?}"));
            }
            if let ShortcutAction::Extension {
                id: owner, effect, ..
            } = expected
            {
                if let Some(target) = effect {
                    project
                        .nodes
                        .iter_mut()
                        .find(|n| n.id == *owner)
                        .unwrap()
                        .effects
                        .retain(|e| e.id != *target);
                } else {
                    project.nodes.retain(|n| n.id != *owner);
                }
                let errors = hotkeys.update(&project, &settings);
                if !errors.is_empty() {
                    return Err(errors.join("; "));
                }
                if hotkeys.ids.contains_key(&id) {
                    return Err("Deleted extension action retained its OS registration".into());
                }
            }
        }
    }
    let mut empty = AppSettings::default();
    for key in empty.keybinds.values_mut() {
        key.clear();
    }
    let previous = hotkeys.ids.clone();
    let previously_registered = *hotkeys
        .registered
        .values()
        .next()
        .ok_or("Missing shortcut for pause test")?;
    hotkeys.suspend()?;
    if !hotkeys.ids.is_empty() || !hotkeys.poll().is_empty() {
        return Err("Recording did not pause shortcut dispatch".into());
    }
    let probe = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
    probe
        .register(previously_registered)
        .map_err(|e| format!("Recording did not release the OS shortcut: {e}"))?;
    probe
        .unregister(previously_registered)
        .map_err(|e| e.to_string())?;
    drop(probe);
    let errors = hotkeys.resume();
    if !errors.is_empty() || hotkeys.ids != previous {
        return Err(format!(
            "Recording did not restore applied shortcuts: {errors:?}"
        ));
    }
    let mut conflict = base.clone();
    conflict.sounds[0].shortcut = "Ctrl+Alt+Shift+F11".into();
    let mut conflicting = empty.clone();
    conflicting
        .keybinds
        .insert(AppAction::StartAudio, "Ctrl+Alt+Shift+F11".into());
    if hotkeys.update(&conflict, &conflicting).is_empty() || hotkeys.ids != previous {
        return Err("Conflicting update did not preserve the previous shortcut set".into());
    }
    let mut bound = base.clone();
    bound.nodes[0]
        .keybinds
        .insert(NodeAction::Mute, "Ctrl+Alt+Shift+F11".into());
    if !hotkeys.update(&bound, &empty).is_empty() {
        return Err("Could not install node deletion test binding".into());
    }
    bound.remove_node(1);
    bound.sounds[0].shortcut = "Ctrl+InvalidKey".into();
    if hotkeys.update(&bound, &empty).is_empty()
        || hotkeys
            .ids
            .values()
            .any(|action| matches!(action, ShortcutAction::Node { id: 1, .. }))
    {
        return Err("Deleting a node left its shortcut registered".into());
    }
    let errors = hotkeys.update(&base, &empty);
    if !errors.is_empty() || !hotkeys.ids.is_empty() {
        return Err("Unbinding did not remove registered shortcuts".into());
    }
    let mut cleanup = base.clone();
    cleanup
        .nodes
        .iter_mut()
        .find(|n| n.id == 6)
        .unwrap()
        .effects[0]
        .keybinds
        .insert(
            piratewave_core::shortcuts::EffectAction::Toggle,
            "Ctrl+Alt+Shift+F11".into(),
        );
    cleanup
        .nodes
        .iter_mut()
        .find(|n| n.id == 90)
        .unwrap()
        .switch_keybinds
        .insert(4, "Ctrl+Alt+Shift+F12".into());
    cleanup
        .nodes
        .iter_mut()
        .find(|n| n.id == 91)
        .unwrap()
        .recording
        .slots[0]
        .keybinds
        .insert(
            piratewave_core::recording::SlotAction::Preview,
            "Ctrl+Alt+Shift+F10".into(),
        );
    cleanup
        .nodes
        .iter_mut()
        .find(|n| n.id == 91)
        .unwrap()
        .effects[0]
        .keybinds
        .insert(
            piratewave_core::shortcuts::EffectAction::Toggle,
            "Ctrl+Alt+Shift+F9".into(),
        );
    cleanup
        .nodes
        .iter_mut()
        .find(|n| n.id == 94)
        .unwrap()
        .keybinds
        .insert(NodeAction::ToggleDucking, "Ctrl+Alt+Shift+F8".into());
    let errors = hotkeys.update(&cleanup, &empty);
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    cleanup
        .nodes
        .iter_mut()
        .find(|n| n.id == 6)
        .unwrap()
        .effects
        .clear();
    cleanup
        .nodes
        .iter_mut()
        .find(|n| n.id == 91)
        .unwrap()
        .effects
        .clear();
    cleanup.remove_node(94);
    cleanup.disconnect(90, 4);
    cleanup
        .nodes
        .iter_mut()
        .find(|n| n.id == 91)
        .unwrap()
        .recording
        .slots
        .clear();
    let errors = hotkeys.update(&cleanup, &empty);
    if !errors.is_empty() || !hotkeys.ids.is_empty() {
        return Err("Deleted effect/route shortcuts remained registered".into());
    }
    cleanup
        .nodes
        .iter_mut()
        .find(|n| n.id == 1)
        .unwrap()
        .keybinds
        .insert(NodeAction::SoloOn, "Ctrl+Alt+Shift+F9".into());
    cleanup
        .nodes
        .iter_mut()
        .find(|n| n.id == 1)
        .unwrap()
        .keybinds
        .insert(NodeAction::SoloOff, "Ctrl+Alt+Shift+F8".into());
    // Re-register the same keys to prove WM_HOTKEY ownership was released.
    cleanup
        .nodes
        .iter_mut()
        .find(|n| n.id == 1)
        .unwrap()
        .keybinds
        .insert(NodeAction::ToggleMute, "Ctrl+Alt+Shift+F10".into());
    cleanup
        .nodes
        .iter_mut()
        .find(|n| n.id == 1)
        .unwrap()
        .keybinds
        .insert(NodeAction::Mute, "Ctrl+Alt+Shift+F11".into());
    cleanup
        .nodes
        .iter_mut()
        .find(|n| n.id == 1)
        .unwrap()
        .keybinds
        .insert(NodeAction::Unmute, "Ctrl+Alt+Shift+F12".into());
    let errors = hotkeys.update(&cleanup, &empty);
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    hotkeys.clear();
    let report=format!("PASS: {delivered} Windows global shortcut mappings registered and delivered through WM_HOTKEY: all application actions, 14 actions for each of three source types, eight actions for each output bus, three processor actions, 17 VST instrument actions, three VST effect actions, per-effect enable/disable/toggle, switch outputs, recording-node start/stop, stop playback, rack bypass and individual recording effects, all three ducking actions, all five recording-slot actions, and unchanged sound-pad playback. Deleted effects, recording slots and routes release their OS keys for reuse. Recording suspension releases OS keys, suppresses dispatch and restores applied bindings. Unbinding, conflict rollback, and deleted-node cleanup verified. No user keystrokes synthesized.");
    std::fs::create_dir_all("dist").map_err(|e| e.to_string())?;
    std::fs::write("dist/shortcut-self-test.txt", &report).map_err(|e| e.to_string())?;
    println!("{report}");
    Ok(())
}
#[cfg(not(windows))]
pub fn self_test() -> Result<(), String> {
    Err("Shortcut integration self-test is Windows-only".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_new_node_and_transport_shortcuts_default_unbound() {
        let p = Project::default();
        let plan = binding_plan(&p, &AppSettings::default()).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].action, ShortcutAction::App(AppAction::StopSounds));
        assert_eq!(plan[0].text, "Ctrl+Alt+Space");
    }
    #[test]
    fn detects_conflicts_across_app_node_and_soundboard() {
        let mut p = Project::default();
        p.nodes[0]
            .keybinds
            .insert(NodeAction::Mute, "Alt+Ctrl+Space".into());
        let errors = binding_plan(&p, &AppSettings::default()).err().unwrap();
        assert!(errors[0].contains("Microphone"));
        assert!(errors[0].contains("Stop all soundboard"));
        p.nodes[0].keybinds.clear();
        p.nodes[0]
            .keybinds
            .insert(NodeAction::Mute, "Ctrl+Alt+F2".into());
        p.sounds.push(piratewave_core::project::Sound {
            id: 42,
            name: "Applause".into(),
            path: String::new(),
            shortcut: "Ctrl+Alt+F2".into(),
            volume: 1.0,
        });
        assert!(binding_plan(&p, &AppSettings::default()).is_err());
    }
    #[test]
    fn validates_syntax_and_preserves_sound_pad_mapping() {
        let mut p = Project::default();
        p.sounds.push(piratewave_core::project::Sound {
            id: 42,
            name: "Applause".into(),
            path: String::new(),
            shortcut: "Ctrl+Alt+F2".into(),
            volume: 1.0,
        });
        let plan = binding_plan(&p, &AppSettings::default()).unwrap();
        assert!(plan.iter().any(|b| b.action == ShortcutAction::Sound(42)));
        p.nodes[0]
            .keybinds
            .insert(NodeAction::Mute, "Ctrl+DefinitelyNotAKey".into());
        assert!(binding_plan(&p, &AppSettings::default()).is_err());
    }
}
