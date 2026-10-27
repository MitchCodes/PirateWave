//! Focused shortcut capture without a global keyboard hook.
use super::*;
use global_hotkey::hotkey::HotKey;

pub(super) struct Recorder {
    target: ShortcutAction,
    label: String,
    candidate: Option<String>,
    armed: bool,
    message: String,
}

fn chord(key: &str, modifiers: egui::Modifiers, super_key: bool) -> Result<String, String> {
    let mut parts = Vec::new();
    if modifiers.ctrl {
        parts.push("Ctrl");
    }
    if modifiers.alt {
        parts.push("Alt");
    }
    if modifiers.shift {
        parts.push("Shift");
    }
    if super_key || modifiers.mac_cmd {
        parts.push("Super");
    }
    parts.push(key);
    let text = parts.join("+");
    text.parse::<HotKey>()
        .map_err(|_| "This key is not supported. Try another key.".to_owned())?;
    Ok(text)
}

fn key_name(key: egui::Key) -> String {
    use egui::Key::*;
    match key {
        Num0 => "0".into(),
        Num1 => "1".into(),
        Num2 => "2".into(),
        Num3 => "3".into(),
        Num4 => "4".into(),
        Num5 => "5".into(),
        Num6 => "6".into(),
        Num7 => "7".into(),
        Num8 => "8".into(),
        Num9 => "9".into(),
        Backtick => "Backquote".into(),
        OpenBracket => "BracketLeft".into(),
        CloseBracket => "BracketRight".into(),
        Equals => "Equal".into(),
        _ => format!("{key:?}"),
    }
}

fn vk_name(vk: u8) -> Option<String> {
    Some(
        match vk {
            0x30..=0x39 | 0x41..=0x5a => return Some(char::from(vk).to_string()),
            0x60..=0x69 => return Some(format!("Numpad{}", vk - 0x60)),
            0x70..=0x87 => return Some(format!("F{}", vk - 0x70 + 1)),
            8 => "Backspace",
            9 => "Tab",
            13 => "Enter",
            19 => "Pause",
            20 => "CapsLock",
            27 => "Escape",
            32 => "Space",
            33 => "PageUp",
            34 => "PageDown",
            35 => "End",
            36 => "Home",
            37 => "ArrowLeft",
            38 => "ArrowUp",
            39 => "ArrowRight",
            40 => "ArrowDown",
            44 => "PrintScreen",
            45 => "Insert",
            46 => "Delete",
            0x6a => "NumpadMultiply",
            0x6b => "NumpadAdd",
            0x6d => "NumpadSubtract",
            0x6e => "NumpadDecimal",
            0x6f => "NumpadDivide",
            0x90 => "NumLock",
            0x91 => "ScrollLock",
            0xad => "AudioVolumeMute",
            0xae => "AudioVolumeDown",
            0xaf => "AudioVolumeUp",
            0xb0 => "MediaTrackNext",
            0xb1 => "MediaTrackPrevious",
            0xb2 => "MediaStop",
            0xb3 => "MediaPlayPause",
            0xba => "Semicolon",
            0xbb => "Equal",
            0xbc => "Comma",
            0xbd => "Minus",
            0xbe => "Period",
            0xbf => "Slash",
            0xc0 => "Backquote",
            0xdb => "BracketLeft",
            0xdc => "Backslash",
            0xdd => "BracketRight",
            0xde => "Quote",
            _ => return None,
        }
        .into(),
    )
}

#[derive(Default)]
struct NativeKeys {
    key: Option<String>,
    modifiers: egui::Modifiers,
    super_key: bool,
    any_down: bool,
}
fn native_keys(enabled: bool) -> NativeKeys {
    #[cfg(windows)]
    if enabled {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        // Only called during recording while this app has focus. No hooks,
        // injection, text logging or background keystroke collection.
        let down = |key: i32| unsafe { GetAsyncKeyState(key) < 0 };
        let modifiers = egui::Modifiers {
            ctrl: down(0x11),
            alt: down(0x12),
            shift: down(0x10),
            ..Default::default()
        };
        let super_key = down(0x5b) || down(0x5c);
        let mut result = NativeKeys {
            any_down: modifiers.ctrl || modifiers.alt || modifiers.shift || super_key,
            modifiers,
            super_key,
            ..Default::default()
        };
        for vk in 8..=254u8 {
            if matches!(vk, 0x10..=0x12 | 0x5b..=0x5c | 0xa0..=0xa5) {
                continue;
            }
            if down(vk as i32) {
                result.any_down = true;
                if result.key.is_none() {
                    result.key = vk_name(vk);
                }
            }
        }
        return result;
    }
    let _ = enabled;
    NativeKeys::default()
}

impl PirateWaveApp {
    pub(super) fn begin_shortcut_recording(
        &mut self,
        ctx: &egui::Context,
        target: ShortcutAction,
        label: String,
    ) {
        if self.recorder.is_some() {
            return;
        }
        if let Some(hotkeys) = &mut self.hotkeys {
            if let Err(error) = hotkeys.suspend() {
                self.notify(error);
                return;
            }
        }
        let idle = ctx.input(|i| i.keys_down.is_empty() && i.modifiers.is_none())
            && !native_keys(!self.smoke).any_down;
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        self.recorder = Some(Recorder {
            target,
            label,
            candidate: None,
            armed: idle,
            message: String::new(),
        });
        ctx.request_repaint();
    }

    pub(super) fn record_shortcut_input(&mut self, ctx: &egui::Context) {
        if self.recorder.is_none() {
            return;
        }
        if !ctx.input(|i| i.focused) {
            self.finish_shortcut_recording(false);
            return;
        }
        let (events, idle) = ctx.input(|i| {
            (
                i.events.clone(),
                i.keys_down.is_empty() && i.modifiers.is_none(),
            )
        });
        let native = native_keys(!self.smoke);
        let idle = idle && !native.any_down;
        // Keep Space/Enter/Tab and text from activating or editing underlying UI.
        ctx.input_mut(|i| {
            i.events.retain(|event| {
                !matches!(
                    event,
                    egui::Event::Key { .. }
                        | egui::Event::Text(_)
                        | egui::Event::Copy
                        | egui::Event::Cut
                        | egui::Event::Paste(_)
                )
            })
        });
        if !native.super_key && events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::Escape, pressed: true, modifiers, .. } if modifiers.is_none())) {
            self.finish_shortcut_recording(false);
            return;
        }
        let recorder = self.recorder.as_mut().unwrap();
        if !recorder.armed {
            recorder.armed = idle;
            return;
        }
        if recorder.candidate.is_some() {
            if idle {
                self.finish_shortcut_recording(true);
            }
            return;
        }
        // Native keys preserve numpad and Windows modifier information. The
        // event fallback also captures very quick taps released between frames.
        let mut pressed = native
            .key
            .map(|key| (key, native.modifiers, native.super_key));
        if pressed.is_none() {
            pressed = events.iter().find_map(|event| match event {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                } => Some((
                    key_name(physical_key.unwrap_or(*key)),
                    *modifiers,
                    native.super_key,
                )),
                _ => None,
            });
        }
        if let Some((key, modifiers, super_key)) = pressed {
            if key == "Escape" && modifiers.is_none() && !super_key {
                self.finish_shortcut_recording(false);
                return;
            }
            match chord(&key, modifiers, super_key) {
                Ok(text) => {
                    recorder.candidate = Some(text);
                    if idle {
                        self.finish_shortcut_recording(true);
                    }
                }
                Err(error) => recorder.message = error,
            }
        }
    }

    fn finish_shortcut_recording(&mut self, commit: bool) {
        let Some(recorder) = self.recorder.take() else {
            return;
        };
        if commit {
            if let Some(text) = recorder.candidate {
                match recorder.target {
                    ShortcutAction::App(action) => {
                        self.settings_draft.keybinds.insert(action, text);
                    }
                    ShortcutAction::Node { id, action } => {
                        if self.view == View::NodeKeybinds(id) {
                            self.node_binding_draft.insert(action, text);
                        }
                    }
                    action @ (ShortcutAction::Extension { id, .. }
                    | ShortcutAction::Effect { id, .. }
                    | ShortcutAction::SwitchOutput { id, .. }
                    | ShortcutAction::RecordingSlot { id, .. }) => {
                        if (self.view == View::NodeKeybinds(id)
                            || matches!(self.view,View::RecordingSlotKeybinds(owner,_) if owner==id))
                            && action.exists(&self.project)
                        {
                            self.node_extra_binding_draft.insert(action, text);
                        }
                    }
                    ShortcutAction::Sound(id) => {
                        if let Some(sound) = self.project.sounds.iter_mut().find(|s| s.id == id) {
                            sound.shortcut = text;
                            self.changed(false);
                        }
                    }
                }
            }
        }
        let errors = self
            .hotkeys
            .as_mut()
            .map(|h| h.resume())
            .unwrap_or_default();
        if !errors.is_empty() {
            self.shortcut_errors = errors;
            self.notify("Some global shortcuts could not be restored. Check the keybind errors.");
        }
        self.refresh_shortcuts();
    }

    pub(super) fn shortcut_recording_popup(&mut self, ctx: &egui::Context) {
        let Some(recorder) = &self.recorder else {
            return;
        };
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("shortcut-recorder")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("Record a shortcut");
            ui.label(&recorder.label);
            ui.separator();
            if let Some(text) = &recorder.candidate { ui.heading(text); ui.label("Release all keys to finish."); }
            else if !recorder.armed { ui.label("Release any held keys, then press your shortcut."); }
            else { ui.label("Hold Ctrl, Alt, Shift or Windows (Super), then press one key. You can also use a key without modifiers."); ui.strong("Waiting for a key…"); }
            ui.label("Existing global shortcuts are paused while recording. Press Escape or click Cancel to keep the previous binding.");
            if !recorder.message.is_empty() { ui.colored_label(theme::palette().peach, &recorder.message); }
            cancel = ui.button("Cancel recording").clicked();
        });
        if cancel {
            self.finish_shortcut_recording(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recorded_key_names_parse_using_the_same_global_shortcut_parser() {
        let modifiers = egui::Modifiers {
            ctrl: true,
            alt: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(chord("F3", modifiers, false).unwrap(), "Ctrl+Alt+Shift+F3");
        for key in egui::Key::ALL {
            let name = key_name(*key);
            if let Ok(text) = chord(&name, modifiers, true) {
                assert!(text.parse::<HotKey>().is_ok());
            }
        }
        for vk in 0..=255 {
            if let Some(key) = vk_name(vk) {
                chord(&key, modifiers, true).unwrap();
            }
        }
        assert_eq!(
            chord("Equal", egui::Modifiers::SHIFT, false).unwrap(),
            "Shift+Equal"
        );
        assert!(chord("F35", modifiers, false).is_err());
        assert!(chord("Ctrl", modifiers, false).is_err());
    }
}
