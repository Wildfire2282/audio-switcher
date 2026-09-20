//! The GDI drawing half of the overlay.
//!
//! Split from the window half in `win`: this file answers "what does the card
//! look like" (appearance tokens, the card font, the painted card), while
//! `win` answers "when does it exist and where" (class, creation, paint
//! entry, `WM_NCHITTEST`, show/hide). A colour or geometry change reads this
//! file alone; a "the overlay never appears" bug reads `win`.
//!
//! Every GDI object created here is released here: `Buffer` and `Selected`
//! own that bookkeeping so no call site repeats it, and Windows refuses to
//! delete an object that is still selected into a DC.
//!
//! Threading: paint only, on the message-loop thread. The caches below are
//! per-thread (`STYLE` is also seeded by `osd/win/tests.rs`).

use super::layout;
use super::{BAR_H, CARD_H, CARD_W, FALLBACK_TEXT_PX, KEY_RGB, RADIUS, rgb, scale};
use crate::platform::theme;
use crate::platform::utf16::wide;
use crate::ui::osd::{OsdContent, Palette, WIDEST_PERCENT, palette};
use std::cell::Cell;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap, CreateCompatibleDC,
    CreateFontIndirectW, CreateFontW, CreatePen, CreateSolidBrush, DEFAULT_CHARSET, DT_CALCRECT,
    DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DeleteDC,
    DeleteObject, DrawTextW, Ellipse, FW_NORMAL, FillRect, GetStockObject, HBITMAP, HDC, HFONT,
    HGDIOBJ, LOGFONTW, NULL_BRUSH, NULL_PEN, OUT_DEFAULT_PRECIS, PS_SOLID, RoundRect, SRCCOPY,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SystemParametersInfoW,
};
use windows::core::w;

/// A cached shell lookup whose answer may itself be "unavailable".
///
/// The failure is cached alongside the success on purpose: re-querying on
/// every paint would log the same warning several times a second while the
/// user scrolls.
#[derive(Clone, Copy)]
enum Cached<T> {
    Pending,
    Ready(Option<T>),
}

thread_local! {
    /// Resolved design tokens, cached per thread and cleared when the shell
    /// reports a theme or settings change.
    pub(super) static STYLE: Cell<Option<Palette>> = const { Cell::new(None) };
    /// The shell's menu font, cached alongside the tokens: it changes with
    /// the same notification.
    static MENU_FONT: Cell<Cached<LOGFONTW>> = const { Cell::new(Cached::Pending) };
}

/// Drop the cached tokens and font.
///
/// The shell reports theme, accent and settings changes through the same window
/// message, so both are invalidated together — from the window half, which owns
/// that message. Keeping it a call instead of two pokes leaves the caches owned
/// by this file.
pub(super) fn invalidate_appearance() {
    STYLE.with(|slot| slot.set(None));
    MENU_FONT.with(|slot| slot.set(Cached::Pending));
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

/// Width of `text` in physical pixels under the font currently in `dc`.
///
/// `DT_CALCRECT` measures into the `RECT` instead of drawing.
fn text_width(dc: HDC, text: &str) -> i32 {
    let mut buf = wide(text);
    let mut rect = RECT::default();
    // SAFETY: `dc` is live with the card font selected, `buf` is
    // NUL-terminated, and `DT_CALCRECT` makes `rect` the measured box.
    let _ = unsafe {
        DrawTextW(
            dc,
            &mut buf,
            &mut rect,
            DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
        )
    };
    rect.right - rect.left
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
    /// Width reserved for the state read-out, in physical pixels.
    ///
    /// Sized for the widest read-out the language can produce rather than
    /// for the text on screen, so the slider keeps one length.
    state_col: i32,
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
pub(super) fn draw_card(hdc: HDC, content: &OsdContent, dpi: i32) {
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
        // Reserve the column for the widest read-out this language can
        // show, never for the text on screen: measuring that made the
        // slider shorten as the read-out grew (`5%` → `50%` → `100%`).
        let state_col = text_width(dc, WIDEST_PERCENT).max(text_width(dc, &content.muted_label));
        Buffer {
            dc,
            bmp,
            old_bmp,
            font,
            old_font,
            state_col,
        }
    };

    let l = layout::layout(w, h, dpi, buf.state_col, content);

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
