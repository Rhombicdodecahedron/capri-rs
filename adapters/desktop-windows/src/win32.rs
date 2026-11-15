//! Win32 probe. All `unsafe` in the project is in this file: plain FFI calls with stack-allocated
//! out-parameters, no pointers kept beyond the call.
#![allow(unsafe_code)]

use std::ffi::c_void;

use capri_application::ports::{DesktopProbe, Tracking};
use capri_domain::{Ledge, Rect};
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, TRUE};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetClassNameW, GetWindowLongW, GetWindowRect, GetWindowTextLengthW, IsIconic,
    IsWindowVisible, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

use crate::edges::{WindowRect, visible_edges};

/// Shell windows that look like normal windows but are part of the desktop itself.
const SHELL_CLASSES: &[&str] =
    &["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "Windows.UI.Core.CoreWindow"];

#[derive(Debug, Clone, Copy)]
pub struct Win32Desktop {
    /// Window edges narrower than this are ignored (screen px).
    pub min_width: f32,
}

impl Default for Win32Desktop {
    fn default() -> Self {
        Self { min_width: 140.0 }
    }
}

fn to_rect(r: &RECT) -> Rect {
    Rect::new(r.left as f32, r.top as f32, (r.right - r.left) as f32, (r.bottom - r.top) as f32)
}

unsafe extern "system" fn on_monitor(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    // SAFETY: `data` is the `&mut Vec<Rect>` passed by `work_areas`, alive for the whole enumeration.
    let areas = unsafe { &mut *(data as *mut Vec<Rect>) };
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &raw mut info) } != 0 {
        areas.push(to_rect(&info.rcWork));
    }
    TRUE
}

unsafe extern "system" fn on_window(hwnd: HWND, data: LPARAM) -> BOOL {
    // SAFETY: `data` is the `&mut Vec<HWND>` passed by `ledges`.
    let list = unsafe { &mut *(data as *mut Vec<HWND>) };
    list.push(hwnd);
    TRUE
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    // SAFETY: buffer length is passed alongside the buffer.
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// A window she may sit on: visible, not minimised, not cloaked (other virtual desktop, suspended UWP),
/// not a tool window or overlay, has a title, and is not part of the shell.
fn usable(hwnd: HWND) -> Option<Rect> {
    // SAFETY: every call takes the handle by value or writes into a local out-parameter.
    unsafe {
        if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 || GetWindowTextLengthW(hwnd) == 0 {
            return None;
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) != 0 {
            return None;
        }
        let mut cloaked: u32 = 0;
        let hr = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED as u32, (&raw mut cloaked).cast::<c_void>(), 4);
        if hr == 0 && cloaked != 0 {
            return None;
        }
        if SHELL_CLASSES.contains(&class_name(hwnd).as_str()) {
            return None;
        }
        // the visible frame, without the invisible resize borders Windows 10/11 add around windows
        let mut r: RECT = std::mem::zeroed();
        let hr = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            (&raw mut r).cast::<c_void>(),
            std::mem::size_of::<RECT>() as u32,
        );
        if hr != 0 && GetWindowRect(hwnd, &raw mut r) == 0 {
            return None;
        }
        Some(to_rect(&r))
    }
}

impl DesktopProbe for Win32Desktop {
    fn work_areas(&self) -> Vec<Rect> {
        let mut areas: Vec<Rect> = Vec::new();
        // SAFETY: the callback only runs during this call and receives a pointer to `areas`.
        unsafe {
            EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(on_monitor), (&raw mut areas) as LPARAM);
        }
        areas
    }

    fn ledges(&self, ignore: &[u64]) -> Vec<Ledge> {
        let areas = self.work_areas();
        let mut handles: Vec<HWND> = Vec::new();
        // SAFETY: as above; EnumWindows walks top-level windows in z-order, topmost first.
        unsafe {
            EnumWindows(Some(on_window), (&raw mut handles) as LPARAM);
        }
        let windows: Vec<WindowRect> = handles
            .into_iter()
            .filter(|h| !ignore.contains(&(*h as u64)))
            .filter_map(|h| usable(h).map(|rect| WindowRect { id: h as u64, rect }))
            .collect();
        let mut ledges = visible_edges(&windows, &areas, self.min_width);
        ledges.extend(crate::floors(&areas));
        ledges
    }

    fn track(&self, window: u64) -> Tracking {
        match usable(window as HWND) {
            Some(rect) => Tracking::At(rect),
            None => Tracking::Gone,
        }
    }
}
