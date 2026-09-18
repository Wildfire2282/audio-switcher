//! Overlay geometry — where the card and its elements go, in physical pixels.
//!
//! Split from the window/painting half so the placement rules (against the tray
//! icon and the monitor work area) and the card metrics can be read without
//! scrolling through GDI code. `fit` and `layout` are pure; only the screen
//! queries above them touch Win32.

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    GetDC, GetDeviceCaps, GetMonitorInfoW, LOGPIXELSX, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromRect, ReleaseDC,
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

use super::{Anchor, BAR_H, CARD_H, NAME_H, NAME_TOP, PAD, THUMB_D, scale};
use crate::ui::osd::OsdContent;

/// Screen DPI, used to scale the card on high-DPI displays.
///
/// Read from the screen device caps rather than `GetDpiForWindow` so the
/// crate keeps its minimal `windows` feature set.
pub(super) fn screen_dpi() -> i32 {
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
pub(super) fn place(w: i32, h: i32, anchor: Option<Anchor>, gap: i32) -> (i32, i32) {
    let anchor = anchor.unwrap_or_else(screen_anchor);
    fit(anchor, w, h, gap, work_area(anchor))
}

/// Fit a `w`x`h` card a `gap` away from `anchor`, inside `work`.
///
/// Pure, so the placement rules are testable without a monitor: above the
/// icon, horizontally centred on it, flipped below when the taskbar leaves
/// no room above, then clamped into the work area.
pub(super) fn fit(
    anchor: Anchor,
    w: i32,
    h: i32,
    gap: i32,
    work: (i32, i32, i32, i32),
) -> (i32, i32) {
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
pub(super) struct Layout {
    pub(super) name: RECT,
    pub(super) bar_track: RECT,
    pub(super) bar_fill: RECT,
    /// Bounding box of the round slider thumb.
    pub(super) thumb: RECT,
    pub(super) state: RECT,
}

/// Lay the card out for a card of `w`x`h` and a state text `state_w` wide.
///
/// Pure: [`draw_card`] measures the text with GDI and then calls this, so
/// the geometry can be unit-tested without a window.
pub(super) fn layout(w: i32, h: i32, dpi: i32, state_w: i32, content: &OsdContent) -> Layout {
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
    let bar_right = (w - scale(PAD, dpi) - state_w - scale(10, dpi)).max(bar_left + scale(4, dpi));
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
