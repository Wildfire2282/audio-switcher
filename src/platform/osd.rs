//! Volume overlay (OSD) — a borderless, non-activating layered popup painted
//! with GDI next to the tray icon.
//!
//! Why it exists: `Shell_NotifyIcon(NIF_TIP)` only paints after the system
//! hover delay and does not repaint an already-visible bubble, so the tray
//! tooltip cannot show per-notch wheel feedback. This window belongs to the
//! process, so a repaint lands in the same frame as the volume change.
//!
//! Styling deliberately matches the application's own menus rather than the
//! Windows 11 XAML flyouts. Measured on Windows 11, a `TrackPopupMenu` menu is
//! a `#32768` window that the shell only themes: it gets the *small* corner
//! (`DWMWCP_ROUNDSMALL`, 4px), no acrylic, and the system menu font. A XAML
//! flyout (`Xaml_WindowedPopupClass`) gets 8px corners and a material — a
//! different generation of the design language. The card therefore mirrors the
//! menu: 4px radius, opaque, system menu font, themed palette.
//!
//! Three Win32 constraints shape the window:
//! - Never activate (`WS_EX_NOACTIVATE` + `SW_SHOWNOACTIVATE`): activation
//!   would steal the tray icon's hover state and break the hover-roll gesture
//!   that gates wheel volume.
//! - Click-through (`WS_EX_TRANSPARENT`): the overlay sits beside the icon and
//!   must not shadow it.
//! - Explicit layer attributes (`SetLayeredWindowAttributes`): a
//!   `WS_EX_LAYERED` window with no layer attributes is fully transparent — the
//!   classic silent no-show. The colour key is what keeps the rounded corners
//!   truly transparent, since a layered window without one shows the square
//!   backing behind the card.
//!
//! Best-effort by contract: every failure logs and degrades to "no overlay",
//! never to a broken volume path (the same policy `tray` runtime updates use).

use crate::ui::osd::OsdContent;

/// How long the overlay stays up after the last change, in milliseconds.
///
/// `app` owns the hide deadline (loop policy lives there) and this module owns
/// only the window; the constant is shared so the two cannot drift apart.
pub(crate) const VISIBLE_MS: u64 = 900;

/// Anchor rectangle in physical screen pixels: `(x, y, width, height)`.
pub(crate) type Anchor = (i32, i32, i32, i32);

// ---------------------------------------------------------------------------
// Windows implementation
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod win {
    use super::{Anchor, OsdContent};
    use std::cell::{Cell, RefCell};
    use std::sync::Once;
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap,
        CreateCompatibleDC, CreateFontIndirectW, CreateFontW, CreatePen, CreateSolidBrush,
        DEFAULT_CHARSET, DT_CALCRECT, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT,
        DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, Ellipse, EndPaint, FW_NORMAL,
        FillRect, GetStockObject, HBITMAP, HDC, HFONT, HGDIOBJ, InvalidateRect, LOGFONTW,
        NULL_BRUSH, NULL_PEN, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, RoundRect, SRCCOPY,
        SelectObject, SetBkMode, SetTextColor, TRANSPARENT, UpdateWindow,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, HTTRANSPARENT,
        LWA_COLORKEY, NONCLIENTMETRICSW, RegisterClassW, SPI_GETNONCLIENTMETRICS, SW_HIDE,
        SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
        SetLayeredWindowAttributes, SetWindowPos, ShowWindow, SystemParametersInfoW, WM_ERASEBKGND,
        WM_NCHITTEST, WM_PAINT, WM_SETTINGCHANGE, WM_THEMECHANGED, WNDCLASSW, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };
    use windows::core::w;

    use crate::platform::theme;
    use crate::platform::wide::wide;
    use crate::ui::osd::{Palette, palette};

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

    /// A cached shell lookup whose answer may itself be "unavailable".
    ///
    /// The failure is cached alongside the success on purpose: re-querying on
    /// every paint would log the same warning several times a second while the
    /// user scrolls.
    #[derive(Clone, Copy)]
    enum Cached<T> {
        /// Not looked up yet.
        Pending,
        /// Looked up, with the answer.
        Ready(Option<T>),
    }

    thread_local! {
        static PAINT: RefCell<Option<PaintJob>> = const { RefCell::new(None) };
        /// Resolved design tokens, cached per thread and cleared when the shell
        /// reports a theme or settings change.
        static STYLE: Cell<Option<Palette>> = const { Cell::new(None) };
        /// The shell's menu font, cached alongside the tokens: it changes with
        /// the same notification.
        static MENU_FONT: Cell<Cached<LOGFONTW>> = const { Cell::new(Cached::Pending) };
    }

    /// Design tokens for the current shell appearance.
    fn style() -> Palette {
        STYLE.with(|slot| {
            if let Some(tokens) = slot.get() {
                return tokens;
            }
            let appearance = theme::appearance();
            let tokens = palette(appearance.light, appearance.accent);
            slot.set(Some(tokens));
            tokens
        })
    }

    /// The shell's own menu font, or `None` when it cannot be read.
    fn menu_font() -> Option<LOGFONTW> {
        MENU_FONT.with(|slot| {
            if let Cached::Ready(answer) = slot.get() {
                return answer;
            }
            let answer = query_menu_font();
            slot.set(Cached::Ready(answer));
            answer
        })
    }

    /// Ask the shell for its menu font.
    fn query_menu_font() -> Option<LOGFONTW> {
        let mut metrics = NONCLIENTMETRICSW {
            cbSize: u32::try_from(std::mem::size_of::<NONCLIENTMETRICSW>()).unwrap_or(0),
            ..Default::default()
        };
        // SAFETY: `uiParam` carries the size the action requires and `metrics`
        // is a valid out-parameter for `SPI_GETNONCLIENTMETRICS`.
        let queried = unsafe {
            SystemParametersInfoW(
                SPI_GETNONCLIENTMETRICS,
                metrics.cbSize,
                Some((&raw mut metrics).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        };
        match queried {
            Ok(()) => Some(metrics.lfMenuFont),
            Err(e) => {
                tracing::warn!("osd: SPI_GETNONCLIENTMETRICS failed: {e:?}");
                None
            }
        }
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

    mod layout;
    /// Create the card's font and select it into `dc`.
    ///
    /// Prefers the shell's own menu font, so the card reads as one more menu in
    /// the same language — and so a user-chosen menu font size is honoured
    /// automatically. That face is accepted whatever it names, because on a
    /// non-Latin system it is the correct one.
    ///
    /// Returns the font, always owned by the caller, and the object it replaced
    /// in `dc`.
    fn select_card_font(dc: HDC, dpi: i32) -> (HFONT, HGDIOBJ) {
        // SAFETY: `dc` is live, and every `LOGFONTW` handed to GDI below is
        // fully initialized.
        let font = unsafe {
            match menu_font() {
                // Already sized for the system DPI, so not scaled again.
                Some(logfont) => CreateFontIndirectW(&raw const logfont),
                // `SPI_GETNONCLIENTMETRICS` does not fail in a normal session. A
                // named face is a better guess than a zeroed `LOGFONTW`, and GDI
                // substitutes by family when the face is not installed.
                None => CreateFontW(
                    -scale(FALLBACK_TEXT_PX, dpi),
                    0,
                    0,
                    0,
                    i32::try_from(FW_NORMAL.0).unwrap_or(400),
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    CLEARTYPE_QUALITY,
                    0,
                    w!("Segoe UI"),
                ),
            }
        };
        // SAFETY: `dc` is live and `font` was just created.
        let previous = unsafe { SelectObject(dc, HGDIOBJ(font.0)) };
        (font, previous)
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

    /// GDI objects backing one card.
    ///
    /// Created in one `unsafe` block and restored/released in another, so no
    /// safe layout logic has to sit inside `unsafe` while every object is still
    /// released exactly once.
    struct Buffer {
        dc: HDC,
        bmp: HBITMAP,
        old_bmp: HGDIOBJ,
        font: HFONT,
        old_font: HGDIOBJ,
        /// Measured width of the state text, in physical pixels.
        state_w: i32,
    }

    /// A GDI object selected into a DC, restored and released on drop.
    ///
    /// Keeps the select / draw / restore / release cycle in one place instead of
    /// repeating the bookkeeping at every call site — and makes it impossible to
    /// release an object while it is still selected, which Windows refuses,
    /// leaking the object for the process lifetime.
    struct Selected {
        dc: HDC,
        previous: HGDIOBJ,
        /// The object to release, or `None` for stock objects, which must never
        /// be deleted.
        owned: Option<HGDIOBJ>,
    }

    impl Selected {
        /// Select `object` into `dc`.
        ///
        /// # Safety
        ///
        /// `dc` must be a live device context and `object` a valid GDI object
        /// that is not currently selected into another DC.
        unsafe fn new(dc: HDC, object: HGDIOBJ, owned: bool) -> Self {
            // SAFETY: the caller guarantees both handles are valid.
            let previous = unsafe { SelectObject(dc, object) };
            Self {
                dc,
                previous,
                owned: owned.then_some(object),
            }
        }
    }

    impl Drop for Selected {
        fn drop(&mut self) {
            // SAFETY: `self.dc` was live when the guard was created and the guard
            // cannot outlive the buffer DC owned by `draw_card`.
            unsafe {
                SelectObject(self.dc, self.previous);
                if let Some(object) = self.owned {
                    let _ = DeleteObject(object);
                }
            }
        }
    }

    /// Draw the card into `hdc` through an off-screen buffer.
    ///
    /// One `BitBlt` per paint keeps the layered window from ever showing a
    /// half-drawn frame. The styling follows Windows 11's flyouts: a rounded
    /// card with a hairline stroke, Segoe UI text, and a thin slider whose fill
    /// and thumb carry the accent colour.
    fn draw_card(hdc: HDC, content: &OsdContent, dpi: i32) {
        let tokens = style();
        let w = scale(CARD_W, dpi);
        let h = scale(CARD_H, dpi);
        let radius = scale(RADIUS, dpi) * 2;
        let pill = scale(BAR_H, dpi);
        let hairline = scale(1, dpi).max(1);

        // SAFETY: creates the buffer DC, its bitmap and the card's font. The
        // objects selected here are restored and released in the drawing block
        // below, which runs on every path; `hdc` is a live DC from the caller.
        let buf = unsafe {
            let dc = CreateCompatibleDC(Some(hdc));
            let bmp = CreateCompatibleBitmap(hdc, w, h);
            let old_bmp = SelectObject(dc, HGDIOBJ(bmp.0));
            let (font, old_font) = select_card_font(dc, dpi);
            let _ = SetBkMode(dc, TRANSPARENT);
            // Measure the state text so the slider can leave room for it.
            let mut state = wide(&content.state);
            let mut measured = RECT::default();
            let _ = DrawTextW(
                dc,
                &mut state,
                &mut measured,
                DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
            );
            Buffer {
                dc,
                bmp,
                old_bmp,
                font,
                old_font,
                state_w: measured.right - measured.left,
            }
        };

        let l = layout::layout(w, h, dpi, buf.state_w, content);

        // SAFETY: `buf` owns live GDI objects created above. Each `Selected`
        // guard restores and releases exactly the object it selected, so nothing
        // is released while still selected; the buffer's own objects are restored
        // and released exactly once below.
        unsafe {
            // Everything outside the rounded card stays the colour key, which
            // the layered window keys out, so the corners are transparent rather
            // than showing a square backing.
            let key = CreateSolidBrush(rgb(KEY_RGB));
            FillRect(
                buf.dc,
                &RECT {
                    left: 0,
                    top: 0,
                    right: w,
                    bottom: h,
                },
                key,
            );
            let _ = DeleteObject(HGDIOBJ(key.0));

            // Card fill, no outline.
            {
                let card = CreateSolidBrush(rgb(tokens.card));
                let _pen = Selected::new(buf.dc, GetStockObject(NULL_PEN), false);
                let _brush = Selected::new(buf.dc, HGDIOBJ(card.0), true);
                let _ = RoundRect(buf.dc, 0, 0, w, h, radius, radius);
            }

            // Hairline stroke, drawn one pixel inside the fill so the right and
            // bottom edges are not clipped away at the client boundary.
            {
                let border = CreatePen(PS_SOLID, hairline, rgb(tokens.border));
                let _pen = Selected::new(buf.dc, HGDIOBJ(border.0), true);
                let _brush = Selected::new(buf.dc, GetStockObject(NULL_BRUSH), false);
                let _ = RoundRect(buf.dc, 0, 0, w - 1, h - 1, radius, radius);
            }

            // Slider track.
            {
                let track = CreateSolidBrush(rgb(tokens.track));
                let _brush = Selected::new(buf.dc, HGDIOBJ(track.0), true);
                let _ = RoundRect(
                    buf.dc,
                    l.bar_track.left,
                    l.bar_track.top,
                    l.bar_track.right,
                    l.bar_track.bottom,
                    pill,
                    pill,
                );
            }

            // Filled portion and thumb, both in the accent (neutral while muted).
            {
                let level = CreateSolidBrush(rgb(if content.muted {
                    tokens.muted_fill
                } else {
                    tokens.fill
                }));
                let _brush = Selected::new(buf.dc, HGDIOBJ(level.0), true);
                if l.bar_fill.right > l.bar_fill.left {
                    let _ = RoundRect(
                        buf.dc,
                        l.bar_fill.left,
                        l.bar_fill.top,
                        l.bar_fill.right,
                        l.bar_fill.bottom,
                        pill,
                        pill,
                    );
                }
                let _ = Ellipse(
                    buf.dc,
                    l.thumb.left,
                    l.thumb.top,
                    l.thumb.right,
                    l.thumb.bottom,
                );
            }

            // Text. The device line is skipped outright (not drawn blank) when
            // no default device is known.
            let _ = SetTextColor(buf.dc, rgb(tokens.text));
            if !content.device.is_empty() {
                let mut name = wide(&content.device);
                let mut rect = l.name;
                let _ = DrawTextW(
                    buf.dc,
                    &mut name,
                    &mut rect,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
                );
            }
            let mut state = wide(&content.state);
            let mut rect = l.state;
            let _ = SetTextColor(buf.dc, rgb(tokens.dim_text));
            let _ = DrawTextW(
                buf.dc,
                &mut state,
                &mut rect,
                DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );

            let _ = BitBlt(hdc, 0, 0, w, h, Some(buf.dc), 0, 0, SRCCOPY);

            SelectObject(buf.dc, buf.old_font);
            SelectObject(buf.dc, buf.old_bmp);
            let _ = DeleteObject(HGDIOBJ(buf.font.0));
            let _ = DeleteObject(HGDIOBJ(buf.bmp.0));
            let _ = DeleteDC(buf.dc);
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
                STYLE.with(|slot| slot.set(None));
                MENU_FONT.with(|slot| slot.set(Cached::Pending));
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
            if let Err(e) =
                unsafe { SetLayeredWindowAttributes(hwnd, rgb(KEY_RGB), 0, LWA_COLORKEY) }
            {
                tracing::warn!("osd: SetLayeredWindowAttributes failed: {e:?}");
                // SAFETY: destroys the window created above; balances the leak.
                let _ = unsafe { DestroyWindow(hwnd) };
                return None;
            }
            Some(Self { hwnd, dpi })
        }

        /// Position the overlay next to `anchor` and show `content`.
        ///
        /// The position is recomputed every call: the icon moves with the
        /// taskbar, so a cached rectangle would drift.
        pub(crate) fn show(&mut self, content: OsdContent, anchor: Option<Anchor>) {
            let (w, h) = (scale(CARD_W, self.dpi), scale(CARD_H, self.dpi));
            PAINT.with(|p| {
                *p.borrow_mut() = Some(PaintJob {
                    content,
                    dpi: self.dpi,
                });
            });
            let (x, y) = layout::place(w, h, anchor, scale(GAP, self.dpi));
            // SAFETY: `hwnd` is live; SWP_NOACTIVATE keeps the gesture gate
            // intact and SWP_NOZORDER leaves the existing topmost z-order.
            unsafe {
                let _ = SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER);
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                let _ = InvalidateRect(Some(self.hwnd), None, false);
                // Synchronous paint: the feedback must not wait for the pump.
                let _ = UpdateWindow(self.hwnd);
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

// ---------------------------------------------------------------------------
// Non-Windows stub (compilation parity; no overlay)
// ---------------------------------------------------------------------------

#[cfg(not(windows))]
/// Non-Windows stub: never creates a window.
pub(crate) struct OsdOverlay;

#[cfg(not(windows))]
impl OsdOverlay {
    /// Always `None` on non-Windows.
    #[must_use]
    pub(crate) fn new() -> Option<Self> {
        None
    }

    /// No-op on non-Windows.
    pub(crate) fn show(&mut self, _content: OsdContent, _anchor: Option<Anchor>) {}

    /// No-op on non-Windows.
    pub(crate) fn hide(&self) {}
}
