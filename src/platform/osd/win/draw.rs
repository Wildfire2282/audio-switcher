//! The GDI drawing half of the overlay.
//!
//! Split from the window half in `win`: this file answers "what does the card
//! look like" (appearance tokens, the card font, the painted card), while
//! `win` answers "when does it exist and where" (class, creation, paint
//! entry, `WM_NCHITTEST`, show/hide). A colour or geometry change reads this
//! file alone; a "the overlay never appears" bug reads `win`.
//!
//! Every GDI object created here is released here: `Buffer`, `Selected` and
//! `PaintCache` own that bookkeeping so no call site repeats it, and Windows
//! refuses to delete an object that is still selected into a DC.
//!
//! Threading: paint only, on the message-loop thread. The caches below are
//! per-thread (`STYLE` is also seeded by `osd/win/tests.rs`).

use super::layout;
use super::{BAR_H, CARD_H, CARD_W, FALLBACK_TEXT_PX, KEY_RGB, RADIUS, rgb, scale};
use crate::platform::theme;
use crate::ui::osd::{OsdContent, Palette, WIDEST_PERCENT, palette};
use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap, CreateCompatibleDC,
    CreateFontIndirectW, CreateFontW, CreatePen, CreateSolidBrush, DEFAULT_CHARSET, DT_CALCRECT,
    DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DeleteDC,
    DeleteObject, DrawTextW, Ellipse, FW_NORMAL, FillRect, GetStockObject, HBITMAP, HBRUSH, HDC,
    HFONT, HGDIOBJ, HPEN, LOGFONTW, NULL_BRUSH, NULL_PEN, OUT_DEFAULT_PRECIS, PS_SOLID, RoundRect,
    SRCCOPY, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
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
    /// Scratch buffer for the UTF-16 form of the strings GDI is about to draw.
    ///
    /// Each `DrawTextW`/`DT_CALCRECT` call used to allocate its own `Vec`, four
    /// times per painted card, on a path that runs once per wheel notch.
    static WIDE_BUF: RefCell<Vec<u16>> = const { RefCell::new(Vec::new()) };
    /// Font, brushes and pen the card is drawn with.
    ///
    /// Every one of them is a pure function of the palette and the DPI, and the
    /// card is repainted once per wheel notch: creating them per paint spent
    /// eight GDI objects a notch to produce the same eight objects.
    static PAINT_CACHE: RefCell<Option<PaintCache>> = const { RefCell::new(None) };
}

/// Encode `text` into the shared buffer and hand it to `f`.
///
/// No NUL terminator: `DrawTextW` is told the length, and a terminator there
/// would be drawn as a stray glyph (see [`crate::platform::utf16`]). Not
/// re-entrant, and does not need to be: the paint path draws one string at a
/// time.
fn with_wide<R>(text: &str, f: impl FnOnce(&mut [u16]) -> R) -> R {
    WIDE_BUF.with(|buf| {
        let mut buf = buf.borrow_mut();
        crate::platform::utf16::wide_into(&mut buf, text);
        f(&mut buf)
    })
}

/// Drop the cached tokens, font and paint objects.
///
/// The shell reports theme, accent and settings changes through the same window
/// message, so all three are invalidated together — from the window half, which
/// owns that message. Keeping it a call instead of three pokes leaves the caches
/// owned by this file.
pub(super) fn invalidate_appearance() {
    STYLE.with(|slot| slot.set(None));
    MENU_FONT.with(|slot| slot.set(Cached::Pending));
    // Replacing the entry is what releases its GDI objects.
    PAINT_CACHE.with(|slot| *slot.borrow_mut() = None);
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

/// Create the card's font, owned by the caller.
///
/// Prefers the shell's own menu font, so the card reads as one more menu in
/// the same language — and so a user-chosen menu font size is honoured
/// automatically. That face is accepted whatever it names, because on a
/// non-Latin system it is the correct one.
fn card_font(dpi: i32) -> HFONT {
    // SAFETY: every `LOGFONTW` handed to GDI below is fully initialized.
    unsafe {
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
    }
}

/// Width of `text` in physical pixels under the font currently in `dc`.
///
/// `DT_CALCRECT` measures into the `RECT` instead of drawing.
fn text_width(dc: HDC, text: &str) -> i32 {
    with_wide(text, |buf| {
        let mut rect = RECT::default();
        // SAFETY: `dc` is live with the card font selected, `buf` is sized for
        // `DrawTextW`'s character count, and `DT_CALCRECT` makes `rect` the
        // measured box.
        let _ = unsafe {
            DrawTextW(
                dc,
                buf,
                &mut rect,
                DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
            )
        };
        rect.right - rect.left
    })
}

/// The per-card GDI objects: an off-screen device context and its bitmap.
///
/// The font, brushes and pen are [`PaintCache`]'s, because they outlive one
/// card. Created in one `unsafe` block and released on drop in another, so no
/// safe layout logic has to sit inside `unsafe` while every object is still
/// released exactly once — including when the paint returns early.
struct Buffer {
    dc: HDC,
    bmp: HBITMAP,
    old_bmp: HGDIOBJ,
}

impl Buffer {
    /// Create the buffer DC and its bitmap.
    ///
    /// `None` when GDI refuses one of them: painting a blank card and saying
    /// nothing left the failure invisible. Everything created before the
    /// refusal is released here, so a failed paint leaks no handle.
    ///
    /// # Safety
    ///
    /// `hdc` must be a live device context.
    unsafe fn new(hdc: HDC, w: i32, h: i32) -> Option<Self> {
        // SAFETY: the caller guarantees `hdc` is live; every handle is checked
        // for null before it is selected into the DC or released.
        unsafe {
            let dc = CreateCompatibleDC(Some(hdc));
            if dc.0.is_null() {
                tracing::warn!("osd: GDI allocation failed: CreateCompatibleDC");
                return None;
            }
            let bmp = CreateCompatibleBitmap(hdc, w, h);
            if bmp.0.is_null() {
                let _ = DeleteDC(dc);
                tracing::warn!("osd: GDI allocation failed: CreateCompatibleBitmap");
                return None;
            }
            let old_bmp = SelectObject(dc, HGDIOBJ(bmp.0));
            let _ = SetBkMode(dc, TRANSPARENT);
            Some(Self { dc, bmp, old_bmp })
        }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        // SAFETY: the handle came out of `Buffer::new` and is still live. The
        // bitmap is selected out before it is deleted, which Windows requires;
        // the restored stock object is not ours to delete.
        unsafe {
            SelectObject(self.dc, self.old_bmp);
            let _ = DeleteObject(HGDIOBJ(self.bmp.0));
            let _ = DeleteDC(self.dc);
        }
    }
}

/// The card's font, brushes and pen, created once per appearance and DPI.
///
/// Owns every handle it holds: dropping it releases them, which is also how a
/// theme change frees the objects for the old palette.
struct PaintCache {
    /// The palette these objects were built from; a different one rebuilds.
    tokens: Palette,
    /// The DPI the font size and pen width were scaled for.
    dpi: i32,
    font: HFONT,
    key: HBRUSH,
    card: HBRUSH,
    border: HPEN,
    track: HBRUSH,
    fill: HBRUSH,
    muted_fill: HBRUSH,
    /// The `muted_label` the reserved column was measured against, and its
    /// width: the measurement is per label, not per paint.
    state_label: String,
    state_col: i32,
}

impl PaintCache {
    /// Create the card's font, brushes and pen, or `None` when GDI refuses one.
    ///
    /// # Safety
    ///
    /// The returned cache owns every handle; each is released by [`Drop`].
    unsafe fn new(tokens: Palette, dpi: i32) -> Option<Self> {
        // SAFETY: plain GDI object creation; every handle is either stored here
        // or released by the `Drop` that runs as this value is discarded.
        unsafe {
            let cache = Self {
                tokens,
                dpi,
                font: card_font(dpi),
                key: CreateSolidBrush(rgb(KEY_RGB)),
                card: CreateSolidBrush(rgb(tokens.card)),
                border: CreatePen(PS_SOLID, scale(1, dpi).max(1), rgb(tokens.border)),
                track: CreateSolidBrush(rgb(tokens.track)),
                fill: CreateSolidBrush(rgb(tokens.fill)),
                muted_fill: CreateSolidBrush(rgb(tokens.muted_fill)),
                state_label: String::new(),
                state_col: 0,
            };
            let complete = !cache.font.0.is_null()
                && !cache.key.0.is_null()
                && !cache.card.0.is_null()
                && !cache.border.0.is_null()
                && !cache.track.0.is_null()
                && !cache.fill.0.is_null()
                && !cache.muted_fill.0.is_null();
            if !complete {
                tracing::warn!("osd: GDI allocation failed: card font or brush");
                return None;
            }
            Some(cache)
        }
    }

    /// Width reserved for the state read-out, measured once per label.
    ///
    /// Sized for the widest read-out the language can produce rather than for
    /// the text on screen: measuring that made the slider shorten as the
    /// read-out grew (`5%` → `50%` → `100%`). `dc` must have this cache's font
    /// selected.
    fn state_col(&mut self, dc: HDC, label: &str) -> i32 {
        if self.state_label != label {
            self.state_col = text_width(dc, WIDEST_PERCENT).max(text_width(dc, label));
            self.state_label.clear();
            self.state_label.push_str(label);
        }
        self.state_col
    }
}

impl Drop for PaintCache {
    fn drop(&mut self) {
        // SAFETY: each handle came from the matching create call in `new` and is
        // deleted exactly once, here. A null handle (a create that failed on the
        // way to the `None` above) is skipped.
        unsafe {
            for handle in [
                HGDIOBJ(self.font.0),
                HGDIOBJ(self.key.0),
                HGDIOBJ(self.card.0),
                HGDIOBJ(self.border.0),
                HGDIOBJ(self.track.0),
                HGDIOBJ(self.fill.0),
                HGDIOBJ(self.muted_fill.0),
            ] {
                if !handle.0.is_null() {
                    let _ = DeleteObject(handle);
                }
            }
        }
    }
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

    // SAFETY: `hdc` is a live DC from the caller; `Buffer` releases every
    // object it created when it goes out of scope on every path below.
    let Some(buf) = (unsafe { Buffer::new(hdc, w, h) }) else {
        return;
    };

    PAINT_CACHE.with(|slot| {
        let mut slot = slot.borrow_mut();
        // A theme change (new palette) or a move to another monitor (new DPI)
        // rebuilds: the old objects are released by the replacement.
        if slot
            .as_ref()
            .is_none_or(|cache| cache.tokens != tokens || cache.dpi != dpi)
        {
            // SAFETY: `PaintCache::new` only creates GDI objects and takes no
            // pointers; the returned cache owns them.
            *slot = unsafe { PaintCache::new(tokens, dpi) };
        }
        let Some(cache) = slot.as_mut() else {
            return;
        };

        // SAFETY: `buf.dc` is live and every handle passed to `Selected` belongs
        // to `cache`, which outlives the paint: `false` means "do not release".
        unsafe {
            let _font = Selected::new(buf.dc, HGDIOBJ(cache.font.0), false);
            let state_col = cache.state_col(buf.dc, &content.muted_label);
            let l = layout::layout(w, h, dpi, state_col, content);

            // Everything outside the rounded card stays the colour key, which
            // the layered window keys out, so the corners are transparent rather
            // than showing a square backing.
            FillRect(
                buf.dc,
                &RECT {
                    left: 0,
                    top: 0,
                    right: w,
                    bottom: h,
                },
                cache.key,
            );

            // Card fill, no outline.
            {
                let _pen = Selected::new(buf.dc, GetStockObject(NULL_PEN), false);
                let _brush = Selected::new(buf.dc, HGDIOBJ(cache.card.0), false);
                let _ = RoundRect(buf.dc, 0, 0, w, h, radius, radius);
            }

            // Hairline stroke, drawn one pixel inside the fill so the right and
            // bottom edges are not clipped away at the client boundary.
            {
                let _pen = Selected::new(buf.dc, HGDIOBJ(cache.border.0), false);
                let _brush = Selected::new(buf.dc, GetStockObject(NULL_BRUSH), false);
                let _ = RoundRect(buf.dc, 0, 0, w - 1, h - 1, radius, radius);
            }

            // Slider track.
            {
                let _brush = Selected::new(buf.dc, HGDIOBJ(cache.track.0), false);
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

            // Filled portion and thumb, both in the accent (neutral while
            // muted).
            {
                let level = if content.muted {
                    cache.muted_fill
                } else {
                    cache.fill
                };
                let _brush = Selected::new(buf.dc, HGDIOBJ(level.0), false);
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
                with_wide(&content.device, |name| {
                    let mut rect = l.name;
                    // SAFETY: `buf.dc` is live with the card font selected and
                    // `name` is sized for `DrawTextW`'s character count.
                    let _ = DrawTextW(
                        buf.dc,
                        name,
                        &mut rect,
                        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
                    );
                });
            }
            let _ = SetTextColor(buf.dc, rgb(tokens.dim_text));
            with_wide(&content.state, |state| {
                let mut rect = l.state;
                // SAFETY: as above, with the right-aligned state box.
                let _ = DrawTextW(
                    buf.dc,
                    state,
                    &mut rect,
                    DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                );
            });

            let _ = BitBlt(hdc, 0, 0, w, h, Some(buf.dc), 0, 0, SRCCOPY);
        }
    });
}
