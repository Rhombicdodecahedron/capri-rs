//! Getting the composed pixmap onto the screen.
//!
//! - Windows: a layered window updated with `UpdateLayeredWindow`. That gives true per-pixel alpha,
//!   moves the window in the same call, and lets clicks on fully transparent pixels fall through to
//!   whatever is behind her.
//! - Elsewhere: softbuffer. Transparency depends on the compositor; good enough for development.

use std::sync::Arc;

use tiny_skia::Pixmap;
use winit::window::Window;

pub trait Presenter {
    /// Shows `pixmap` with its top-left corner at screen position `origin`.
    fn present(&mut self, pixmap: &Pixmap, origin: (i32, i32));

    /// Whether the OS already ignores clicks on transparent pixels.
    fn clicks_pass_through_transparency(&self) -> bool;

    /// Asks the OS to recompute where her window is clickable (it stopped sending her mouse events).
    fn repair_hit_test(&mut self) {}

    /// Which window holds the mouse capture, for diagnostics (Windows).
    fn capture_owner(&self) -> Option<u64> {
        None
    }
}

/// Platform id of the window (HWND on Windows, X11 window id), so the desktop probe can ignore it.
pub fn native_id(window: &Window) -> u64 {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get() as u64,
        #[allow(clippy::useless_conversion)] // c_ulong is u32 on Windows, u64 on Linux
        Ok(RawWindowHandle::Xlib(h)) => u64::from(h.window),
        Ok(RawWindowHandle::Xcb(h)) => u64::from(h.window.get()),
        _ => 0,
    }
}

#[allow(clippy::needless_pass_by_value)] // consumed by softbuffer, only borrowed on Windows
/// The pointer position in screen pixels, where the OS can tell (Windows).
pub fn os_cursor() -> Option<(f32, f32)> {
    #[cfg(windows)]
    {
        layered::cursor()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub fn create(window: Arc<Window>) -> Box<dyn Presenter> {
    #[cfg(windows)]
    {
        Box::new(layered::Layered::new(&window))
    }
    #[cfg(not(windows))]
    {
        Box::new(soft::Soft::new(window))
    }
}

#[cfg(not(windows))]
mod soft {
    use std::num::NonZeroU32;
    use std::sync::Arc;

    use tiny_skia::Pixmap;
    use winit::dpi::PhysicalPosition;
    use winit::window::Window;

    pub struct Soft {
        window: Arc<Window>,
        surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
        last_origin: Option<(i32, i32)>,
    }

    impl Soft {
        pub fn new(window: Arc<Window>) -> Self {
            let surface = softbuffer::Context::new(window.clone())
                .and_then(|ctx| softbuffer::Surface::new(&ctx, window.clone()))
                .map_err(|e| tracing::error!(error = %e, "softbuffer unavailable"))
                .ok();
            Self { window, surface, last_origin: None }
        }
    }

    impl super::Presenter for Soft {
        fn present(&mut self, pixmap: &Pixmap, origin: (i32, i32)) {
            if self.last_origin != Some(origin) {
                self.window.set_outer_position(PhysicalPosition::new(origin.0, origin.1));
                self.last_origin = Some(origin);
            }
            let Some(surface) = self.surface.as_mut() else { return };
            let (Some(w), Some(h)) = (NonZeroU32::new(pixmap.width()), NonZeroU32::new(pixmap.height())) else {
                return;
            };
            if surface.resize(w, h).is_err() {
                return;
            }
            let Ok(mut buf) = surface.buffer_mut() else { return };
            for (dst, p) in buf.iter_mut().zip(pixmap.pixels()) {
                *dst = (u32::from(p.alpha()) << 24)
                    | (u32::from(p.red()) << 16)
                    | (u32::from(p.green()) << 8)
                    | u32::from(p.blue());
            }
            let _ = buf.present();
        }

        fn clicks_pass_through_transparency(&self) -> bool {
            false
        }
    }
}

#[cfg(windows)]
mod layered {
    //! The only `unsafe` in this crate: GDI calls on handles this struct owns.
    #![allow(unsafe_code)]

    use std::ffi::c_void;

    use tiny_skia::Pixmap;
    use windows_sys::Win32::Foundation::{HWND, POINT, SIZE};
    use windows_sys::Win32::Graphics::Gdi::{
        AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
        CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP, HDC, HGDIOBJ, ReleaseDC,
        SelectObject,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetCursorPos, GetWindowLongW, HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowLongW, SetWindowPos, ULW_ALPHA, UpdateLayeredWindow,
        WS_EX_LAYERED,
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;

    pub fn cursor() -> Option<(f32, f32)> {
        let mut p = POINT { x: 0, y: 0 };
        // SAFETY: writes into a local POINT.
        (unsafe { GetCursorPos(&raw mut p) } != 0).then_some((p.x as f32, p.y as f32))
    }

    pub struct Layered {
        hwnd: HWND,
        dc: HDC,
        bitmap: HBITMAP,
        old: HGDIOBJ,
        bits: *mut u8,
        size: (u32, u32),
        warned: bool,
        topmost_at: std::time::Instant,
    }

    impl Layered {
        pub fn new(window: &Window) -> Self {
            let hwnd = match window.window_handle().map(|h| h.as_raw()) {
                Ok(RawWindowHandle::Win32(h)) => h.hwnd.get() as HWND,
                _ => std::ptr::null_mut(),
            };
            let me = Self {
                hwnd,
                dc: std::ptr::null_mut(),
                bitmap: std::ptr::null_mut(),
                old: std::ptr::null_mut(),
                bits: std::ptr::null_mut(),
                size: (0, 0),
                warned: false,
                topmost_at: std::time::Instant::now(),
            };
            me.ensure_layered();
            me
        }

        /// Other always-on-top windows (the taskbar, overlays) can end up above her after they are
        /// clicked: put her back on top about once a second, without taking the focus.
        fn keep_on_top(&mut self) {
            if self.topmost_at.elapsed() < std::time::Duration::from_secs(1) {
                return;
            }
            self.topmost_at = std::time::Instant::now();
            // SAFETY: z-order change of our own window only; no move, no resize, no activation.
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
            }
        }

        /// winit rewrites the window's extended style whenever it updates its own flags (focus, size,
        /// level...), and that drops WS_EX_LAYERED. Without it `UpdateLayeredWindow` fails silently:
        /// the window stops following her and stops hit-testing per pixel. So it is checked every frame.
        fn ensure_layered(&self) {
            // SAFETY: `hwnd` is our live window; reading and setting one style bit.
            unsafe {
                let ex = GetWindowLongW(self.hwnd, GWL_EXSTYLE) as u32;
                if ex & WS_EX_LAYERED == 0 {
                    tracing::debug!("winit dropped the layered style, putting it back");
                    SetWindowLongW(self.hwnd, GWL_EXSTYLE, (ex | WS_EX_LAYERED) as i32);
                }
            }
        }

        fn release(&mut self) {
            // SAFETY: handles were created by `ensure` and are released exactly once.
            unsafe {
                if !self.dc.is_null() {
                    SelectObject(self.dc, self.old);
                    DeleteObject(self.bitmap);
                    DeleteDC(self.dc);
                }
            }
            self.dc = std::ptr::null_mut();
            self.bitmap = std::ptr::null_mut();
            self.bits = std::ptr::null_mut();
            self.size = (0, 0);
        }

        /// A top-down 32-bit DIB of `w` x `h`, reused while the canvas size stays the same.
        fn ensure(&mut self, w: u32, h: u32) -> bool {
            if self.size == (w, h) && !self.bits.is_null() {
                return true;
            }
            self.release();
            // SAFETY: plain GDI object creation; every out-pointer is a local.
            unsafe {
                let screen = GetDC(std::ptr::null_mut());
                let dc = CreateCompatibleDC(screen);
                ReleaseDC(std::ptr::null_mut(), screen);
                let mut info: BITMAPINFO = std::mem::zeroed();
                info.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w as i32,
                    biHeight: -(h as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..std::mem::zeroed()
                };
                let mut bits: *mut c_void = std::ptr::null_mut();
                let bitmap =
                    CreateDIBSection(dc, &raw const info, DIB_RGB_COLORS, &raw mut bits, std::ptr::null_mut(), 0);
                if bitmap.is_null() || bits.is_null() {
                    DeleteDC(dc);
                    return false;
                }
                self.old = SelectObject(dc, bitmap);
                self.dc = dc;
                self.bitmap = bitmap;
                self.bits = bits.cast::<u8>();
                self.size = (w, h);
            }
            true
        }
    }

    impl Drop for Layered {
        fn drop(&mut self) {
            self.release();
        }
    }

    impl super::Presenter for Layered {
        fn present(&mut self, pixmap: &Pixmap, origin: (i32, i32)) {
            let (w, h) = (pixmap.width(), pixmap.height());
            if self.hwnd.is_null() || !self.ensure(w, h) {
                return;
            }
            self.ensure_layered();
            self.keep_on_top();
            // SAFETY: `bits` points to a w*h*4 byte DIB owned by `self`, alive until `release`.
            let dst = unsafe { std::slice::from_raw_parts_mut(self.bits, (w * h * 4) as usize) };
            // tiny-skia stores premultiplied RGBA, GDI wants premultiplied BGRA
            for (d, s) in dst.chunks_exact_mut(4).zip(pixmap.data().chunks_exact(4)) {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
                d[3] = s[3];
            }
            let pos = POINT { x: origin.0, y: origin.1 };
            let size = SIZE { cx: w as i32, cy: h as i32 };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            // SAFETY: all pointers are to locals; `dc` holds the selected DIB.
            let ok = unsafe {
                let screen = GetDC(std::ptr::null_mut());
                let ok = UpdateLayeredWindow(
                    self.hwnd,
                    screen,
                    &raw const pos,
                    &raw const size,
                    self.dc,
                    &raw const src,
                    0,
                    &raw const blend,
                    ULW_ALPHA,
                );
                ReleaseDC(std::ptr::null_mut(), screen);
                ok
            };
            if ok == 0 && !self.warned {
                self.warned = true;
                tracing::warn!("UpdateLayeredWindow failed: her window may not follow her");
            }
        }

        fn clicks_pass_through_transparency(&self) -> bool {
            true
        }

        fn repair_hit_test(&mut self) {
            // SAFETY: our own window; re-show it on top and make the system re-read its frame.
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW,
                );
            }
            self.size = (0, 0); // rebuild the bitmap on the next frame too
        }

        fn capture_owner(&self) -> Option<u64> {
            // SAFETY: reads which window of this thread has the capture.
            let h = unsafe { GetCapture() };
            (!h.is_null()).then_some(h as u64)
        }
    }
}
