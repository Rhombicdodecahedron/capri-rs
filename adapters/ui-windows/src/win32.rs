//! The few plain Win32 calls UI Automation doesn't cover: a process's file name, bringing a window to
//! the front, and pressing a key chord. FFI only, with stack buffers; nothing is kept past a call.
#![allow(unsafe_code)]

use capri_domain::ui::{Key, KeyChord, Modifier};
use windows_sys::Win32::Foundation::{CloseHandle, HWND};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_HOME, VK_LEFT, VK_LWIN, VK_MENU,
    VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP, VkKeyScanW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow};

/// `C:\...\Spotify.exe` -> `Spotify.exe`; empty when the process can't be opened (elevated apps).
pub fn process_name(pid: u32) -> String {
    // SAFETY: plain call; the handle is closed below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return String::new();
    }
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` holds `len` u16s and outlives the call; `len` is updated with the length written.
    let ok = unsafe { QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &raw mut len) };
    // SAFETY: the handle came from OpenProcess above.
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return String::new();
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    path.rsplit(['\\', '/']).next().unwrap_or_default().to_string()
}

fn hwnd(id: u64) -> HWND {
    id as usize as HWND
}

pub fn is_minimized(id: u64) -> bool {
    // SAFETY: IsIconic only reads; a stale handle just answers false.
    unsafe { IsIconic(hwnd(id)) != 0 }
}

/// Restores the window if it is minimized and puts it in front. Windows only lets the foreground app
/// do that: tapping Alt first lifts the lock when she isn't the one in front.
pub fn bring_to_front(id: u64) {
    let h = hwnd(id);
    // SAFETY: plain calls on a window handle; a stale one makes them fail harmlessly.
    unsafe {
        if IsIconic(h) != 0 {
            ShowWindow(h, SW_RESTORE);
        }
        if SetForegroundWindow(h) == 0 {
            send(&[key(VK_MENU, false, false), key(VK_MENU, true, false)]);
            SetForegroundWindow(h);
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(120));
}

fn key(vk: VIRTUAL_KEY, up: bool, extended: bool) -> INPUT {
    let mut flags: KEYBD_EVENT_FLAGS = 0;
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

fn send(inputs: &[INPUT]) -> bool {
    // SAFETY: a slice of initialised INPUT structs, with its exact length and element size.
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    sent as usize == inputs.len()
}

/// The virtual key for `k`, and whether it is an extended key (arrows and the navigation block).
fn virtual_key(k: Key) -> Option<(VIRTUAL_KEY, bool)> {
    Some(match k {
        Key::Char(c) => {
            let mut units = [0u16; 2];
            let encoded = c.encode_utf16(&mut units);
            if encoded.len() != 1 {
                return None;
            }
            // SAFETY: plain lookup in the current keyboard layout.
            let scan = unsafe { VkKeyScanW(encoded[0]) };
            if scan == -1 {
                return None;
            }
            ((scan as u16) & 0xFF, false)
        }
        Key::Enter => (VK_RETURN, false),
        Key::Tab => (VK_TAB, false),
        Key::Escape => (VK_ESCAPE, false),
        Key::Space => (VK_SPACE, false),
        Key::Backspace => (VK_BACK, false),
        Key::Delete => (VK_DELETE, true),
        Key::Up => (VK_UP, true),
        Key::Down => (VK_DOWN, true),
        Key::Left => (VK_LEFT, true),
        Key::Right => (VK_RIGHT, true),
        Key::Home => (VK_HOME, true),
        Key::End => (VK_END, true),
        Key::PageUp => (VK_PRIOR, true),
        Key::PageDown => (VK_NEXT, true),
        Key::F(n) if (1..=24).contains(&n) => (VK_F1 + u16::from(n) - 1, false),
        Key::F(_) => return None,
    })
}

/// Presses the chord: modifiers down, the key down and up, modifiers up in reverse.
pub fn send_chord(chord: &KeyChord) -> Result<(), String> {
    let (vk, extended) = virtual_key(chord.key).ok_or_else(|| format!("no key for {chord} on this keyboard layout"))?;
    let mods: Vec<VIRTUAL_KEY> = chord
        .modifiers
        .iter()
        .map(|m| match m {
            Modifier::Ctrl => VK_CONTROL,
            Modifier::Alt => VK_MENU,
            Modifier::Shift => VK_SHIFT,
            Modifier::Win => VK_LWIN,
        })
        .collect();
    let mut inputs: Vec<INPUT> = mods.iter().map(|&m| key(m, false, m == VK_LWIN)).collect();
    inputs.push(key(vk, false, extended));
    inputs.push(key(vk, true, extended));
    inputs.extend(mods.iter().rev().map(|&m| key(m, true, m == VK_LWIN)));
    if send(&inputs) { Ok(()) } else { Err("Windows refused the key presses".into()) }
}
