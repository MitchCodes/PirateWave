//! Native parent window for plugin editors. All methods run on its creating thread.
use anyhow::Result;
use std::{ffi::c_void, ptr};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow},
    UI::Input::KeyboardAndMouse::{
        GetFocus, GetKeyState, GetKeyboardState, SetFocus, ToUnicode, VK_CONTROL, VK_MENU, VK_SHIFT,
    },
    UI::WindowsAndMessaging::*,
};

/// VST2 and VST3 share Steinberg's virtual key and modifier numbering.
pub struct EditorKey {
    pub down: bool,
    pub character: u16,
    pub code: i16,
    pub modifiers: i16,
}
pub fn editor_key(message: &MSG) -> Option<EditorKey> {
    if !matches!(
        message.message,
        WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP
    ) {
        return None;
    }
    unsafe {
        let mut modifiers = 0;
        if GetKeyState(VK_SHIFT as i32) < 0 {
            modifiers |= 1;
        }
        if GetKeyState(VK_MENU as i32) < 0 {
            modifiers |= 2;
        }
        if GetKeyState(0x5b) < 0 || GetKeyState(0x5c) < 0 {
            modifiers |= 4;
        }
        if GetKeyState(VK_CONTROL as i32) < 0 {
            modifiers |= 8;
        }
        let mut state = [0u8; 256];
        let mut chars = [0u16; 4];
        // Do not alter dead-key state; TranslateMessage may still need it if the
        // plugin declines the event.
        let character = if GetKeyboardState(state.as_mut_ptr()) != 0
            && ToUnicode(
                message.wParam as u32,
                ((message.lParam as usize >> 16) & 0xff) as u32,
                state.as_ptr(),
                chars.as_mut_ptr(),
                chars.len() as i32,
                4,
            ) > 0
        {
            chars[0]
        } else {
            0
        };
        let key = message.wParam as u32;
        let code = match key {
            0x08 => 1,
            0x09 => 2,
            0x0c => 3,
            0x0d => 4,
            0x13 => 5,
            0x1b => 6,
            0x20 => 7,
            0x23 => 9,
            0x24 => 10,
            0x25 => 11,
            0x26 => 12,
            0x27 => 13,
            0x28 => 14,
            0x21 => 15,
            0x22 => 16,
            0x2c => 20,
            0x2d => 21,
            0x2e => 22,
            0x60..=0x69 => 24 + (key - 0x60) as i16,
            0x6a => 34,
            0x6b => 35,
            0x6c => 36,
            0x6d => 37,
            0x6e => 38,
            0x6f => 39,
            0x70..=0x7b => 40 + (key - 0x70) as i16,
            0x7c..=0x87 => 65 + (key - 0x7c) as i16,
            0x90 => 52,
            0x91 => 53,
            0x10 => 54,
            0x11 => 55,
            0x12 => 56,
            0x5b | 0x5c => 77,
            _ => 0,
        };
        Some(EditorKey {
            down: matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN),
            character,
            code,
            modifiers,
        })
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_CLOSE {
        // The owner closes the plugin view before destroying the native window.
        ShowWindow(hwnd, SW_HIDE);
        return 0;
    }
    if message == WM_DPICHANGED && lparam != 0 {
        let suggested = &*(lparam as *const RECT);
        SetWindowPos(
            hwnd,
            ptr::null_mut(),
            suggested.left,
            suggested.top,
            suggested.right - suggested.left,
            suggested.bottom - suggested.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        return 0;
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

pub struct EditorWindow(HWND);
impl EditorWindow {
    pub fn new(title: &str, width: i32, height: i32, resizable: bool) -> Result<Self> {
        anyhow::ensure!(
            (1..=8192).contains(&width) && (1..=8192).contains(&height),
            "Invalid plugin editor size"
        );
        unsafe {
            let instance = GetModuleHandleW(ptr::null());
            let class: Vec<u16> = "PirateWavePluginEditor\0".encode_utf16().collect();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassW(&wc);
            let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            let style = if resizable {
                WS_OVERLAPPEDWINDOW
            } else {
                WS_OVERLAPPEDWINDOW & !(WS_THICKFRAME | WS_MAXIMIZEBOX)
            };
            if AdjustWindowRectExForDpi(&mut rect, style, 0, 0, GetDpiForSystem()) == 0 {
                AdjustWindowRectEx(&mut rect, style, 0, 0);
            }
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                rect.right - rect.left,
                rect.bottom - rect.top,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            );
            anyhow::ensure!(
                !hwnd.is_null(),
                "Could not create plugin editor window: {}",
                std::io::Error::last_os_error()
            );
            let window = Self(hwnd);
            window.resize(width, height)?;
            Ok(window)
        }
    }
    pub fn handle(&self) -> *mut c_void {
        self.0.cast()
    }
    pub fn show(&self) {
        unsafe {
            ShowWindow(
                self.0,
                if IsIconic(self.0) != 0 {
                    SW_RESTORE
                } else {
                    SW_SHOW
                },
            );
            SetForegroundWindow(self.0);
            let focus = GetFocus();
            if focus.is_null() || (focus != self.0 && IsChild(self.0, focus) == 0) {
                SetFocus(self.0);
            }
        }
    }
    pub fn visible(&self) -> bool {
        unsafe { IsWindowVisible(self.0) != 0 }
    }
    pub fn scale(&self) -> f32 {
        unsafe { GetDpiForWindow(self.0) as f32 / 96.0 }
    }
    pub fn focused(&self) -> bool {
        unsafe {
            let focus = GetFocus();
            focus == self.0 || (!focus.is_null() && IsChild(self.0, focus) != 0)
        }
    }
    pub fn client_size(&self) -> (i32, i32) {
        unsafe {
            let mut rect: RECT = std::mem::zeroed();
            if GetClientRect(self.0, &mut rect) == 0 {
                return (0, 0);
            }
            (rect.right - rect.left, rect.bottom - rect.top)
        }
    }
    pub fn resize(&self, width: i32, height: i32) -> Result<()> {
        anyhow::ensure!(
            (1..=8192).contains(&width) && (1..=8192).contains(&height),
            "Invalid plugin editor size"
        );
        unsafe {
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            let style = GetWindowLongW(self.0, GWL_STYLE) as u32;
            let ex_style = GetWindowLongW(self.0, GWL_EXSTYLE) as u32;
            if AdjustWindowRectExForDpi(&mut rect, style, 0, ex_style, GetDpiForWindow(self.0)) == 0
            {
                AdjustWindowRectEx(&mut rect, style, 0, ex_style);
            }
            anyhow::ensure!(
                SetWindowPos(
                    self.0,
                    ptr::null_mut(),
                    0,
                    0,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE
                ) != 0,
                "Could not resize plugin editor window"
            );
        }
        Ok(())
    }
}
impl Drop for EditorWindow {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_editor_window_tracks_client_size_and_rejects_invalid_requests() {
        let window = EditorWindow::new("PirateWave editor test", 360, 200, true).unwrap();
        assert_eq!(window.client_size(), (360, 200));
        window.resize(420, 240).unwrap();
        assert_eq!(window.client_size(), (420, 240));
        assert!(window.resize(0, 240).is_err());
        assert!(window.resize(8193, 240).is_err());
        assert_eq!(window.client_size(), (420, 240));
    }
    #[test]
    fn dpi_change_applies_windows_suggested_bounds() {
        let window = EditorWindow::new("PirateWave DPI test", 360, 200, true).unwrap();
        let suggested = RECT {
            left: 120,
            top: 140,
            right: 760,
            bottom: 620,
        };
        unsafe {
            SendMessageW(
                window.0,
                WM_DPICHANGED,
                144 | (144 << 16),
                (&suggested as *const RECT) as isize,
            );
            let mut actual: RECT = std::mem::zeroed();
            assert_ne!(GetWindowRect(window.0, &mut actual), 0);
            assert_eq!(
                (actual.left, actual.top, actual.right, actual.bottom),
                (120, 140, 760, 620)
            );
        }
    }
    #[test]
    fn translates_parent_window_function_keys_for_both_plugin_formats() {
        let mut message: MSG = unsafe { std::mem::zeroed() };
        message.message = WM_KEYDOWN;
        message.wParam = 0x71;
        let key = editor_key(&message).unwrap();
        assert!(key.down);
        assert_eq!(key.code, 41); // F2 in both VST key code tables.
        message.message = WM_KEYUP;
        assert!(!editor_key(&message).unwrap().down);
        message.message = WM_PAINT;
        assert!(editor_key(&message).is_none());
    }
}
