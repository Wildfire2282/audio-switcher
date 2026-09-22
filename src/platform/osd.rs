//! Volume overlay (OSD) — a borderless, non-activating layered popup painted
//! with GDI next to the tray icon.
//!
//! It exists because `Shell_NotifyIcon(NIF_TIP)` only paints after the system
//! hover delay and never repaints a visible bubble, so the tray tooltip cannot
//! show per-notch wheel feedback; this window lands in the same frame as the
//! volume change.
//!
//! Three Win32 constraints shape it:
//! - Never activate (`WS_EX_NOACTIVATE` + `SW_SHOWNOACTIVATE`): activation would
//!   steal the tray icon's hover state and break the hover-roll gesture that
//!   gates wheel volume.
//! - Click-through (`WS_EX_TRANSPARENT`): it sits beside the icon and must not
//!   shadow it.
//! - Explicit layer attributes: a `WS_EX_LAYERED` window with none is fully
//!   transparent (the classic silent no-show), and the colour key is what makes
//!   the rounded corners transparent instead of showing the square backing.
//!
//! Styling mirrors the shell's own menus, not its XAML flyouts: measured on
//! Windows 11, a `TrackPopupMenu` menu is a `#32768` window that gets the small
//! corner (`DWMWCP_ROUNDSMALL`, 4px) and the system menu font, while a XAML
//! flyout gets 8px and a material. The card therefore uses 4px, opaque, system
//! menu font, themed palette.
//!
//! Best-effort by contract: every failure logs and degrades to "no overlay".

use crate::ui::osd::OsdContent;

/// `app` owns the hide deadline (loop policy lives there) and this module owns
/// only the window; the constant is shared so the two cannot drift apart.
pub(crate) const VISIBLE_MS: u64 = 2000;

/// Anchor rectangle in physical screen pixels: `(x, y, width, height)`.
pub(crate) type Anchor = (i32, i32, i32, i32);

#[cfg(windows)]
mod win {
    use super::{Anchor, OsdContent};
    use std::cell::RefCell;
    use std::sync::Once;
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT, UpdateWindow,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, HTTRANSPARENT,
        LWA_COLORKEY, RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER,
        SetLayeredWindowAttributes, SetWindowPos, ShowWindow, WM_ERASEBKGND, WM_NCHITTEST,
        WM_PAINT, WM_SETTINGCHANGE, WM_THEMECHANGED, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };
    use windows::core::w;

    /// Gap between the tray icon and the overlay, logical pixels.
    const GAP: i32 = 10;
    /// Card size, logical pixels (scaled by the screen DPI at paint time).
    const CARD_W: i32 = 230;
    const CARD_H: i32 = 60;
    const PAD: i32 = 12;
    const NAME_TOP: i32 = 6;
    const NAME_H: i32 = 20;
    /// Slider track thickness.
    const BAR_H: i32 = 4;
    /// Slider thumb diameter, the shell's round grab handle.
    const THUMB_D: i32 = 12;
    /// Card corner radius. Windows gives legacy menus the *small* corner
    /// (`DWMWCP_ROUNDSMALL`) rather than the 8px flyout radius, so the card
    /// matches the application's own menus.
    const RADIUS: i32 = 4;
    /// Text size in logical pixels, used only when the shell will not report its
    /// menu font. Roughly the menu-font size on a default setup.
    const FALLBACK_TEXT_PX: i32 = 12;
    /// Colour keyed out of the layered window, so the rounded corners are truly
    /// transparent instead of showing the square backing behind the card.
    const KEY_RGB: (u8, u8, u8) = (0xFF, 0x00, 0xFF);

    /// What the window procedure paints. The procedure runs on the creating
    /// (UI) thread, so a thread-local is both sound and lock-free; the overlay
    /// is a per-thread singleton.
    struct PaintJob {
        content: OsdContent,
        dpi: i32,
    }

    /// `COLORREF` is `0x00BBGGRR`, not RGB.
    ///
    /// Not `const`: `u32::from` is not yet const-stable, and `From` is the
    /// idiomatic widening conversion.
    fn rgb((r, g, b): (u8, u8, u8)) -> COLORREF {
        COLORREF(u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16))
    }

    /// Logical pixels to physical, rounded.
    const fn scale(v: i32, dpi: i32) -> i32 {
        (v * dpi + 48) / 96
    }

    /// The process module handle, needed to own the window class.
    fn instance() -> Option<HINSTANCE> {
        // SAFETY: `GetModuleHandleW(None)` requests the current module; it
        // takes no pointer arguments and writes nothing.
        match unsafe { GetModuleHandleW(None) } {
            Ok(h) if !h.0.is_null() => Some(HINSTANCE(h.0)),
            _ => {
                tracing::warn!("osd: GetModuleHandleW failed; overlay disabled");
                None
            }
        }
    }

    /// Register the window class once per process.
    ///
    /// Returns `false` when registration failed; the caller degrades to no
    /// overlay instead of retrying on every volume change.
    fn ensure_class() -> bool {
        static REGISTER: Once = Once::new();
        static OK: AtomicBool = AtomicBool::new(false);
        REGISTER.call_once(|| {
            let Some(hinstance) = instance() else {
                return;
            };
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(osd_proc),
                hInstance: hinstance,
                lpszClassName: w!("AudioSwitcherVolumeOsd"),
                ..Default::default()
            };
            // SAFETY: `wc` is fully initialized and lives for the call; the
            // class name is a static NUL-terminated literal.
            let atom = unsafe { RegisterClassW(&raw const wc) };
            if atom == 0 {
                tracing::warn!("osd: RegisterClassW failed; overlay disabled");
            }
            OK.store(atom != 0, Ordering::Release);
        });
        OK.load(Ordering::Acquire)
    }

    mod draw;
    mod layout;
    use draw::draw_card;

    thread_local! {
        /// What the window procedure paints. The procedure runs on the creating
        /// (UI) thread, so a thread-local is both sound and lock-free; the
        /// overlay is a per-thread singleton.
        static PAINT: RefCell<Option<PaintJob>> = const { RefCell::new(None) };
    }

    /// Handle `WM_PAINT`, always pairing `BeginPaint`/`EndPaint`.
    ///
    /// The pairing is not optional. A window procedure that handles `WM_PAINT`
    /// without validating the update region (that is what `EndPaint` does)
    /// leaves Windows re-posting `WM_PAINT` immediately and forever, which
    /// starves the whole message loop: the app pegs a core and stops servicing
    /// the tray, the wheel, and the menu. So the pair is called even when there
    /// is nothing to draw.
    fn paint(hwnd: HWND) {
        let content = PAINT.with(|p| p.borrow().as_ref().map(|j| (j.content.clone(), j.dpi)));
        // SAFETY: `BeginPaint`/`EndPaint` bracket this call on the live window
        // and run on every path.
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if !hdc.0.is_null() {
                if let Some((content, dpi)) = content {
                    draw_card(hdc, &content, dpi);
                }
            }
            let _ = EndPaint(hwnd, &ps);
        }
    }

    /// Window procedure: paint, and never intercept input.
    ///
    /// # Safety
    ///
    /// Called by Windows with `hwnd` referring to a window created with this
    /// procedure, and `msg`/`wparam`/`lparam` from the OS message contract.
    unsafe extern "system" fn osd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            // The card covers the whole client area; erasing first only adds
            // flicker.
            WM_ERASEBKGND => LRESULT(1),
            // Belt and braces next to WS_EX_TRANSPARENT: a hit test over the
            // overlay must fall through to the tray icon underneath.
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            // The shell repaints its menus when the theme, the accent or the
            // system font changes; drop the cached appearance so the next paint
            // re-reads it.
            WM_THEMECHANGED | WM_SETTINGCHANGE => {
                draw::invalidate_appearance();
                LRESULT(0)
            }
            // SAFETY: default handling for every other message; always safe.
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    /// The overlay window. Construction failure yields `None` (no overlay).
    pub(crate) struct OsdOverlay {
        hwnd: HWND,
        dpi: i32,
    }

    impl OsdOverlay {
        /// Create the overlay window, or `None` when it cannot be created.
        #[must_use]
        pub(crate) fn new() -> Option<Self> {
            if !ensure_class() {
                return None;
            }
            let hinstance = instance()?;
            let dpi = layout::screen_dpi();
            // SAFETY: the class is registered above; the class name and window
            // name are static literals; no parent, menu, or param is used.
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_TOPMOST
                        | WS_EX_TOOLWINDOW
                        | WS_EX_NOACTIVATE
                        | WS_EX_LAYERED
                        | WS_EX_TRANSPARENT,
                    w!("AudioSwitcherVolumeOsd"),
                    w!(""),
                    WS_POPUP,
                    0,
                    0,
                    scale(CARD_W, dpi),
                    scale(CARD_H, dpi),
                    None,
                    None,
                    Some(hinstance),
                    None,
                )
            };
            let hwnd = match hwnd {
                Ok(h) if !h.0.is_null() => h,
                other => {
                    tracing::warn!("osd: CreateWindowExW failed: {other:?}");
                    return None;
                }
            };
            // SAFETY: `hwnd` is the live window just created. Only the colour
            // key is applied: the card stays fully opaque like a menu, and
            // everything outside its rounded outline is keyed out.
            let layered =
                unsafe { SetLayeredWindowAttributes(hwnd, rgb(KEY_RGB), 0, LWA_COLORKEY) };
            if let Err(e) = layered {
                tracing::warn!("osd: SetLayeredWindowAttributes failed: {e:?}");
                // SAFETY: destroys the window created above; balances the leak.
                let _ = unsafe { DestroyWindow(hwnd) };
                return None;
            }
            Some(Self { hwnd, dpi })
        }

        /// Move and size the window, reporting a failure once.
        fn move_to(&self, x: i32, y: i32, w: i32, h: i32) {
            // SAFETY: `hwnd` is live; SWP_NOACTIVATE keeps the gesture gate
            // intact and SWP_NOZORDER leaves the existing topmost z-order.
            let moved =
                unsafe { SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER) };
            if let Err(e) = moved {
                tracing::warn!("osd: SetWindowPos failed: {e:?}");
            }
        }

        /// Position the overlay next to `anchor` and show `content`.
        ///
        /// The position is recomputed every call: the icon moves with the
        /// taskbar, so a cached rectangle would drift.
        ///
        /// Two passes: the first puts the window on the anchor's monitor, which
        /// is what lets the second read that monitor's DPI. A single pass would
        /// scale the card for whichever screen `new()` happened to start on.
        pub(crate) fn show(&mut self, content: OsdContent, anchor: Option<Anchor>) {
            let (mut w, mut h) = (scale(CARD_W, self.dpi), scale(CARD_H, self.dpi));
            let (mut x, mut y) = layout::place(w, h, anchor, scale(GAP, self.dpi));
            self.move_to(x, y, w, h);

            let dpi = layout::window_dpi(self.hwnd);
            if dpi != self.dpi {
                self.dpi = dpi;
                (w, h) = (scale(CARD_W, dpi), scale(CARD_H, dpi));
                (x, y) = layout::place(w, h, anchor, scale(GAP, dpi));
            }

            PAINT.with(|p| {
                *p.borrow_mut() = Some(PaintJob {
                    content,
                    dpi: self.dpi,
                });
            });
            self.move_to(x, y, w, h);
            // SAFETY: `hwnd` is live for the whole block.
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                let _ = InvalidateRect(Some(self.hwnd), None, false);
                // Synchronous paint: the feedback must not wait for the pump.
                if !UpdateWindow(self.hwnd).as_bool() {
                    tracing::warn!("osd: UpdateWindow failed; the card may not be drawn");
                }
            }
        }

        /// Hide the overlay. Idempotent.
        pub(crate) fn hide(&self) {
            // SAFETY: `hwnd` is live for as long as `self`.
            let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
        }
    }

    impl Drop for OsdOverlay {
        fn drop(&mut self) {
            if !self.hwnd.0.is_null() {
                // SAFETY: `hwnd` came from CreateWindowExW and is destroyed
                // exactly once, here; the class is process-global and stays
                // registered for a later overlay.
                let _ = unsafe { DestroyWindow(self.hwnd) };
            }
        }
    }

    #[cfg(test)]
    mod tests;
}

#[cfg(windows)]
pub(crate) use win::OsdOverlay;

#[cfg(not(windows))]
pub(crate) struct OsdOverlay;

#[cfg(not(windows))]
impl OsdOverlay {
    /// Always `None` on non-Windows.
    #[must_use]
    pub(crate) fn new() -> Option<Self> {
        None
    }

    pub(crate) fn show(&mut self, _content: OsdContent, _anchor: Option<Anchor>) {}

    pub(crate) fn hide(&self) {}
}
