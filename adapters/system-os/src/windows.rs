//! Shell calls on Windows. FFI with null-terminated buffers that outlive each call.
#![allow(unsafe_code)]

use std::time::Duration;

use windows_sys::Win32::System::Shutdown::LockWorkStation;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn shell_execute(target: &str) -> Result<(), String> {
    let (verb, file) = (wide("open"), wide(target));
    // SAFETY: null-terminated UTF-16 strings that outlive the call.
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // values above 32 mean success
    if r as isize > 32 { Ok(()) } else { Err(format!("Windows could not open {target}")) }
}

/// Opens like a double-click. Starting an app cold can keep the shell busy for seconds, so the call
/// runs on its own thread and a slow start counts as started.
pub fn shell_open(target: &str) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let owned = target.to_string();
    std::thread::spawn(move || {
        let _ = tx.send(shell_execute(&owned));
    });
    rx.recv_timeout(Duration::from_secs(3)).unwrap_or(Ok(()))
}

pub fn lock() -> Result<(), String> {
    // SAFETY: no arguments; it only asks Windows to show the lock screen.
    if unsafe { LockWorkStation() } != 0 { Ok(()) } else { Err("Windows refused to lock".into()) }
}
