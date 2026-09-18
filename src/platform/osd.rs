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
        DEFAULT_CHARSET, DEFAULT_GUI_FONT, DT_CALCRECT, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX,
        DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, Ellipse, EndPaint,
        FW_NORMAL, FillRect, GetDC, GetDeviceCaps, GetMonitorInfoW, GetStockObject, GetTextFaceW,
        HBITMAP, HDC, HFONT, HGDIOBJ, InvalidateRect, LOGFONTW, LOGPIXELSX,
        MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect, NULL_BRUSH, NULL_PEN,
        OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, ReleaseDC, RoundRect, SRCCOPY, SelectObject,
        SetBkMode, SetTextColor, TRANSPARENT, UpdateWindow,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics,
        HTTRANSPARENT, LWA_COLORKEY, NONCLIENTMETRICSW, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN,
        SPI_GETNONCLIENTMETRICS, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER,
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetLayeredWindowAttributes, SetWindowPos, ShowWindow,
        SystemParametersInfoW, WM_ERASEBKGND, WM_NCHITTEST, WM_PAINT, WM_SETTINGCHANGE,
        WM_THEMECHANGED, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };
    use windows::core::w;

    use crate::platform::theme;
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

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
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

    /// Screen DPI, used to scale the card on high-DPI displays.
    ///
    /// Read from the screen device caps rather than `GetDpiForWindow` so the
    /// crate keeps its minimal `windows` feature set.
    fn screen_dpi() -> i32 {
        // SAFETY: `GetDC(None)` returns the screen DC or null; the matching
        // `ReleaseDC` runs below exactly once.
        let dc = unsafe { GetDC(None) };
        if dc.0.is_null() {
            return 96;
        }
        // SAFETY: `dc` is a live screen DC from `GetDC` above.
        let dpi = unsafe { GetDeviceCaps(Some(dc), LOGPIXELSX) };
        // SAFETY: balances the `GetDC` above (same null hwnd).
        unsafe { ReleaseDC(None, dc) };
        if dpi > 0 { dpi } else { 96 }
    }

    /// Work area `(left, top, right, bottom)` of the monitor holding the anchor.
    fn work_area(anchor: Anchor) -> (i32, i32, i32, i32) {
        let (ax, ay, aw, ah) = anchor;
        let rect = RECT {
            left: ax,
            top: ay,
            right: ax + aw,
            bottom: ay + ah,
        };
        // SAFETY: `MonitorFromRect` reads the `RECT` by pointer and returns a
        // monitor handle or null.
        let monitor = unsafe { MonitorFromRect(&raw const rect, MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: `info.cbSize` is set per the API contract; `info` is a valid
        // out-parameter.
        if !monitor.0.is_null() && unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            let r = info.rcWork;
            return (r.left, r.top, r.right, r.bottom);
        }
        // Fallback: the primary screen (no taskbar inset, but never off-screen).
        // SAFETY: both metrics take no arguments.
        (0, 0, unsafe { GetSystemMetrics(SM_CXSCREEN) }, unsafe {
            GetSystemMetrics(SM_CYSCREEN)
        })
    }

    /// Anchor to use when the tray icon rect is unavailable.
    fn screen_anchor() -> Anchor {
        // SAFETY: both metrics take no arguments.
        let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        (w / 2, h - 48, 0, 0)
    }

    /// Physical `(x, y)` for a card of `w`x`h` anchored to the tray icon.
    fn place(w: i32, h: i32, anchor: Option<Anchor>, gap: i32) -> (i32, i32) {
        let anchor = anchor.unwrap_or_else(screen_anchor);
        fit(anchor, w, h, gap, work_area(anchor))
    }

    /// Fit a `w`x`h` card a `gap` away from `anchor`, inside `work`.
    ///
    /// Pure, so the placement rules are testable without a monitor: above the
    /// icon, horizontally centred on it, flipped below when the taskbar leaves
    /// no room above, then clamped into the work area.
    fn fit(anchor: Anchor, w: i32, h: i32, gap: i32, work: (i32, i32, i32, i32)) -> (i32, i32) {
        let (ax, ay, aw, ah) = anchor;
        let (wl, wt, wr, wb) = work;
        let mut x = ax + aw / 2 - w / 2;
        let mut y = ay - h - gap;
        // Taskbar at the top: no room above, so flip below the icon.
        if y < wt {
            y = ay + ah + gap;
        }
        x = x.clamp(wl, (wr - w).max(wl));
        y = y.clamp(wt, (wb - h).max(wt));
        (x, y)
    }

    /// Where each element of the card goes, in physical pixels.
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Layout {
        name: RECT,
        bar_track: RECT,
        bar_fill: RECT,
        /// Bounding box of the round slider thumb.
        thumb: RECT,
        state: RECT,
    }

    /// Lay the card out for a card of `w`x`h` and a state text `state_w` wide.
    ///
    /// Pure: [`draw_card`] measures the text with GDI and then calls this, so
    /// the geometry can be unit-tested without a window.
    fn layout(w: i32, h: i32, dpi: i32, state_w: i32, content: &OsdContent) -> Layout {
        let has_name = !content.device.is_empty();
        // Without a device line the bar is centred instead of bottom-aligned.
        let bar_bottom = if has_name {
            scale(CARD_H - PAD, dpi)
        } else {
            h / 2 + scale(BAR_H, dpi) / 2
        };
        let bar_top = bar_bottom - scale(BAR_H, dpi);
        let bar_left = scale(PAD, dpi);
        // The bar stops short of the right-aligned state text rather than
        // running underneath it.
        let bar_right =
            (w - scale(PAD, dpi) - state_w - scale(10, dpi)).max(bar_left + scale(4, dpi));
        let fill_w = (bar_right - bar_left) * i32::try_from(content.percent).unwrap_or(0) / 100;
        // The thumb rides the fill head, kept inside the track so it cannot
        // overhang the card at either end.
        let radius = scale(THUMB_D, dpi) / 2;
        let head = bar_left + fill_w;
        let centre = head.clamp(
            bar_left + radius,
            (bar_right - radius).max(bar_left + radius),
        );
        let mid = (bar_top + bar_bottom) / 2;
        Layout {
            name: RECT {
                left: scale(PAD, dpi),
                top: scale(NAME_TOP, dpi),
                right: w - scale(PAD, dpi),
                bottom: scale(NAME_TOP + NAME_H, dpi),
            },
            bar_track: RECT {
                left: bar_left,
                top: bar_top,
                right: bar_right,
                bottom: bar_bottom,
            },
            bar_fill: RECT {
                left: bar_left,
                top: bar_top,
                right: head,
                bottom: bar_bottom,
            },
            thumb: RECT {
                left: centre - radius,
                top: mid - radius,
                right: centre + radius,
                bottom: mid + radius,
            },
            state: RECT {
                left: bar_right + scale(8, dpi),
                top: bar_top - scale(8, dpi),
                right: w - scale(PAD, dpi),
                bottom: bar_bottom + scale(8, dpi),
            },
        }
    }

    /// The face name currently selected into `dc`.
    fn selected_face(dc: HDC) -> String {
        let mut actual = [0u16; 64];
        // SAFETY: `actual` is a writable buffer for the face name; the call
        // reports how much it filled in.
        let len = unsafe { GetTextFaceW(dc, Some(&mut actual)) };
        let filled = usize::try_from(len).unwrap_or(0).min(actual.len());
        String::from_utf16_lossy(&actual[..filled.saturating_sub(1)])
    }

    /// Create the card's font and select it into `dc`.
    ///
    /// The shell's own menu font comes first, so the card reads as one more menu
    /// in the same language — and so a user-chosen menu font size is honoured
    /// automatically. It is accepted whatever face it names, because on a
    /// non-Latin system that face is the correct one.
    ///
    /// Without it, asks for Windows 11's UI face and then Segoe UI, checking
    /// what GDI actually granted rather than accepting a silent substitution,
    /// and finally falls back to the stock GUI font.
    ///
    /// Returns the font and the object it replaced in `dc`.
    fn select_card_font(dc: HDC, dpi: i32) -> (HFONT, HGDIOBJ) {
        if let Some(logfont) = menu_font() {
            // The returned LOGFONT is already sized for the system DPI, so it
            // must not be scaled again.
            // SAFETY: `logfont` is a fully initialized LOGFONTW, and `dc` is
            // live.
            let font = unsafe { CreateFontIndirectW(&raw const logfont) };
            // SAFETY: `dc` is live and `font` was just created.
            let previous = unsafe { SelectObject(dc, HGDIOBJ(font.0)) };
            return (font, previous);
        }
        for face in [w!("Segoe UI Variable Text"), w!("Segoe UI")] {
            // SAFETY: creates a font owned by this call; `dc` is live.
            let font = unsafe {
                CreateFontW(
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
                    face,
                )
            };
            // SAFETY: `dc` is live and `font` was just created.
            let previous = unsafe { SelectObject(dc, HGDIOBJ(font.0)) };
            let name = selected_face(dc);
            if name.starts_with("Segoe UI") {
                return (font, previous);
            }
            tracing::debug!("osd: '{name}' substituted for the requested UI face");
            // Wrong substitution: put the previous font back and try the next.
            // SAFETY: restores the object selected just above, then releases the
            // font this iteration created.
            unsafe {
                SelectObject(dc, previous);
                let _ = DeleteObject(HGDIOBJ(font.0));
            }
        }
        // Neither face is installed: the stock GUI font beats a substituted one.
        // SAFETY: stock objects are never deleted; `dc` is live.
        let stock = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
        // SAFETY: selecting a stock object into a live DC.
        let previous = unsafe { SelectObject(dc, stock) };
        (HFONT(stock.0), previous)
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

        let l = layout(w, h, dpi, buf.state_w, content);

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
            let dpi = screen_dpi();
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
            let (x, y) = place(w, h, anchor, scale(GAP, self.dpi));
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
    mod tests {
        use super::*;
        use windows::Win32::Graphics::Gdi::GetPixel;
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetWindowRect, IsWindowVisible, MSG, PM_REMOVE, PeekMessageW,
        };

        /// Regression: a window procedure that handles `WM_PAINT` without
        /// validating the update region makes Windows re-post `WM_PAINT`
        /// immediately and forever. That starves the message loop — the app
        /// pegs a core, never reaches its own polling, and the tray, wheel and
        /// menu all stop responding.
        #[test]
        fn window_does_not_flood_wm_paint() {
            let Some(osd) = OsdOverlay::new() else {
                panic!("OsdOverlay::new() returned None: window creation failed");
            };
            // Force a non-empty update region so the paint pairing is exercised
            // even where creation alone left the window clean.
            // SAFETY: invalidating the live window's own client area.
            let _ = unsafe { InvalidateRect(Some(osd.hwnd), None, false) };

            const LIMIT: u32 = 1000;
            let mut msg = MSG::default();
            let mut drained: u32 = 0;
            // SAFETY: draining this thread's queue, bounded so a regression
            // fails the test instead of hanging it.
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    DispatchMessageW(&raw const msg);
                    drained += 1;
                    assert!(
                        drained < LIMIT,
                        "message queue flooded: {drained} messages without the update region being \
                         validated (BeginPaint/EndPaint pairing broken)"
                    );
                }
            }
        }

        /// Pixel evidence read back from one rendered card.
        #[derive(Debug)]
        struct Sample {
            card: u32,
            corner: u32,
            fill_px: u32,
            track_px: u32,
            name_ink: u32,
        }

        /// Render `content` with `tokens` forced, then read the pixels back.
        ///
        /// The tokens are forced through the style cache so the assertions do
        /// not depend on this machine's theme; the top-left pixel is sampled
        /// because it lies outside the rounded corner and must stay keyed out.
        fn sample(tokens: Palette, content: &OsdContent) -> Sample {
            STYLE.with(|slot| slot.set(Some(tokens)));
            let Some(mut osd) = OsdOverlay::new() else {
                panic!("OsdOverlay::new() returned None: window creation failed");
            };
            // SAFETY: both metrics take no arguments.
            let (sw, sh) =
                unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
            osd.show(content.clone(), Some((sw / 2, sh / 2, 24, 24)));

            let dpi = osd.dpi;
            let (w, _) = (scale(CARD_W, dpi), scale(CARD_H, dpi));
            let bar_mid = scale(CARD_H - PAD, dpi) - scale(BAR_H, dpi) / 2;
            let name_mid = scale(NAME_TOP, dpi) + scale(NAME_H, dpi) / 2;
            let card_rgb = rgb(tokens.card).0 & 0x00FF_FFFF;
            let fill_rgb = rgb(tokens.fill).0 & 0x00FF_FFFF;
            let track_rgb = rgb(tokens.track).0 & 0x00FF_FFFF;
            // SAFETY: `GetDC`/`ReleaseDC` balance on the live window and every
            // `GetPixel` reads inside the client area computed above.
            let sample = unsafe {
                let dc = GetDC(Some(osd.hwnd));
                let card = GetPixel(dc, w / 2, scale(2, dpi)).0 & 0x00FF_FFFF;
                let corner = GetPixel(dc, 0, 0).0 & 0x00FF_FFFF;
                let mut fill_px = 0u32;
                let mut track_px = 0u32;
                let mut name_ink = 0u32;
                for x in scale(PAD, dpi)..(w - scale(PAD, dpi)) {
                    let slider = GetPixel(dc, x, bar_mid).0 & 0x00FF_FFFF;
                    if slider == fill_rgb {
                        fill_px += 1;
                    } else if slider == track_rgb {
                        track_px += 1;
                    }
                    // Text is whatever differs from the card behind it, so the
                    // check holds in either theme.
                    if GetPixel(dc, x, name_mid).0 & 0x00FF_FFFF != card_rgb {
                        name_ink += 1;
                    }
                }
                ReleaseDC(Some(osd.hwnd), dc);
                Sample {
                    card,
                    corner,
                    fill_px,
                    track_px,
                    name_ink,
                }
            };
            // Hide before returning so a failure cannot leave a card on screen.
            osd.hide();
            STYLE.with(|slot| slot.set(None));
            sample
        }

        /// The card paints in the dark tokens, the corners stay keyed out, and
        /// the slider and text are really drawn.
        #[test]
        fn dark_card_paints_tokens_and_keyed_corners() {
            let tokens = palette(false, Some((0x00, 0x78, 0xD4)));
            let s = sample(tokens, &named(62));
            assert_eq!(s.card, rgb(tokens.card).0 & 0x00FF_FFFF, "card fill");
            assert_eq!(
                s.corner,
                rgb(KEY_RGB).0 & 0x00FF_FFFF,
                "corner outside the radius must stay keyed out"
            );
            assert!(s.fill_px > 0, "slider fill not painted");
            assert!(s.track_px > 0, "slider track not painted");
            assert!(s.name_ink > 0, "device-name text not painted");
        }

        /// The light tokens are the risky path: a light card over a dark
        /// taskbar would show black corners if the key were not applied.
        #[test]
        fn light_card_paints_tokens_and_keyed_corners() {
            let tokens = palette(true, Some((0x00, 0x78, 0xD4)));
            let s = sample(tokens, &named(62));
            assert_eq!(s.card, rgb(tokens.card).0 & 0x00FF_FFFF, "light card fill");
            assert_eq!(
                s.corner,
                rgb(KEY_RGB).0 & 0x00FF_FFFF,
                "corner outside the radius must stay keyed out"
            );
            assert!(s.fill_px > 0, "slider fill not painted");
            assert!(s.track_px > 0, "slider track not painted");
            assert!(s.name_ink > 0, "device-name text not painted");
        }

        /// Exercises the real window: creation, computed position, and
        /// visibility. This is the layer the pure-formatting tests cannot
        /// reach, and the one that decides whether the overlay appears at all.
        #[test]
        fn overlay_creates_and_shows_visible_on_screen() {
            let Some(mut osd) = OsdOverlay::new() else {
                panic!("OsdOverlay::new() returned None: window creation failed");
            };
            // SAFETY: both metrics take no arguments.
            let (sw, sh) =
                unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
            // Anchor mid-screen so no clamp can push the card off the monitor.
            let anchor = (sw / 2, sh / 2, 24, 24);
            osd.show(named(62), Some(anchor));

            let (w, h) = (scale(CARD_W, osd.dpi), scale(CARD_H, osd.dpi));
            let expected = place(w, h, Some(anchor), scale(GAP, osd.dpi));
            // SAFETY: `osd.hwnd` is a live window owned by `osd`.
            let visible = unsafe { IsWindowVisible(osd.hwnd) }.as_bool();
            let mut rect = RECT::default();
            // SAFETY: plain out-parameter write on the live window.
            let rect_ok = unsafe { GetWindowRect(osd.hwnd, &mut rect) }.is_ok();
            // Hide before asserting so a failure cannot leave a card on screen.
            osd.hide();

            assert!(visible, "window not visible after show()");
            assert!(rect_ok, "GetWindowRect failed");
            assert_eq!(
                (rect.left, rect.top),
                expected,
                "window not at the computed position"
            );
            assert_eq!(
                (rect.right - rect.left, rect.bottom - rect.top),
                (w, h),
                "wrong window size"
            );
        }

        fn named(percent: u32) -> OsdContent {
            OsdContent {
                device: "Speaker".into(),
                state: format!("{percent}%"),
                percent,
                muted: false,
            }
        }

        /// Bottom taskbar: the card sits above the icon (which is inside the
        /// taskbar, below the work area) and is clamped so it cannot overflow
        /// the right edge of the screen.
        #[test]
        fn fit_places_the_card_above_the_icon() {
            let work = (0, 0, 1920, 1032);
            let icon = (1800, 1040, 24, 24);
            let (x, y) = fit(icon, 240, 64, GAP, work);
            assert_eq!(x, 1920 - 240, "centred on the icon, then clamped");
            assert_eq!(y, 1040 - 64 - GAP, "one gap above the icon");
        }

        /// Top taskbar: there is no room above the icon, so the card flips below
        /// it — and the clamp then pulls it back inside the work area.
        #[test]
        fn fit_flips_below_when_the_taskbar_leaves_no_room_above() {
            let work = (0, 48, 1920, 1080);
            let icon = (100, 10, 24, 24);
            assert_eq!(fit(icon, 240, 64, GAP, work), (0, 48));
        }

        /// Left taskbar: the card is pushed out of the taskbar strip entirely
        /// instead of overlapping it.
        #[test]
        fn fit_clamps_out_of_a_left_taskbar() {
            let work = (48, 0, 1920, 1080);
            let icon = (10, 500, 24, 24);
            assert_eq!(fit(icon, 240, 64, GAP, work), (48, 500 - 64 - GAP));
        }

        #[test]
        fn layout_keeps_every_element_inside_the_card() {
            let (w, h, dpi, state_w) = (scale(CARD_W, 96), scale(CARD_H, 96), 96, 24);
            let content = named(72);
            let l = layout(w, h, dpi, state_w, &content);
            for r in [l.name, l.bar_track, l.bar_fill, l.thumb, l.state] {
                assert!(
                    r.left >= 0 && r.top >= 0 && r.right <= w && r.bottom <= h,
                    "{r:?} escapes the card"
                );
            }
            assert!(l.bar_track.left < l.bar_track.right, "empty track");
            assert!(
                l.bar_fill.right <= l.bar_track.right,
                "fill overflows the track"
            );
            // The state text sits to the right of the bar, not on top of it.
            assert!(l.state.left >= l.bar_track.right);
            // The fill covers the requested share of the track (integer pixels).
            let track = l.bar_track.right - l.bar_track.left;
            let fill = l.bar_fill.right - l.bar_fill.left;
            assert!(fill > 0 && fill < track, "fill {fill} of track {track}");
            assert!((fill * 100 / track).abs_diff(72) <= 1);
        }

        /// The thumb rides the fill head and never overhangs the track, so it
        /// cannot poke out of the card at either end of the range.
        #[test]
        fn layout_keeps_the_thumb_on_the_track() {
            let (w, h, dpi, state_w) = (scale(CARD_W, 96), scale(CARD_H, 96), 96, 24);
            let radius = scale(THUMB_D, dpi) / 2;
            for percent in [0, 5, 50, 95, 100] {
                let l = layout(w, h, dpi, state_w, &named(percent));
                assert_eq!(l.thumb.right - l.thumb.left, radius * 2, "{percent}%");
                assert_eq!(l.thumb.bottom - l.thumb.top, radius * 2, "{percent}%");
                assert!(
                    l.thumb.left >= l.bar_track.left && l.thumb.right <= l.bar_track.right,
                    "{percent}%: thumb {:?} overhangs track {:?}",
                    l.thumb,
                    l.bar_track
                );
            }
        }

        #[test]
        fn layout_centres_the_bar_without_a_device_name() {
            let (w, h) = (scale(CARD_W, 96), scale(CARD_H, 96));
            let with_name = layout(w, h, 96, 24, &named(50));
            let mut no_name = named(50);
            no_name.device = String::new();
            let without = layout(w, h, 96, 24, &no_name);
            assert_eq!(with_name.bar_track.bottom, CARD_H - PAD);
            assert_eq!(without.bar_track.bottom, CARD_H / 2 + BAR_H / 2);
        }

        #[test]
        fn layout_zero_percent_draws_no_fill() {
            let (w, h) = (scale(CARD_W, 96), scale(CARD_H, 96));
            let l = layout(w, h, 96, 24, &named(0));
            assert_eq!(l.bar_fill.left, l.bar_fill.right, "0% must fill nothing");
            assert!(l.bar_track.left < l.bar_track.right, "track still drawn");
        }
    }
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
