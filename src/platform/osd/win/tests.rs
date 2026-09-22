//! Overlay tests — the window, the painted pixels, and the geometry.
//!
//! These exercise the overlay as a whole (window + layout + paint) rather than
//! one module in isolation, which is why they sit beside `win`.

use super::draw::STYLE;
use super::layout::{fit, layout, place, window_dpi};
use super::*;
use crate::audio::AudioDevice;
use crate::config::Lang;
use crate::ui::osd::{Palette, format, palette};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{GetDC, GetPixel, ReleaseDC};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetSystemMetrics, GetWindowRect, IsWindowVisible, MSG, PM_REMOVE,
    PeekMessageW, SM_CXSCREEN, SM_CYSCREEN,
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
    /// X of the last column the slider painted, so its length can be compared
    /// across read-outs without hard-coding a pixel count.
    bar_right: i32,
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
    let (sw, sh) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
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
        let mut bar_right = -1i32;
        for x in scale(PAD, dpi)..(w - scale(PAD, dpi)) {
            let slider = GetPixel(dc, x, bar_mid).0 & 0x00FF_FFFF;
            if slider == fill_rgb {
                fill_px += 1;
                bar_right = x;
            } else if slider == track_rgb {
                track_px += 1;
                bar_right = x;
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
            bar_right,
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
    let (sw, sh) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
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

/// The card is sized and painted for the monitor it lands on.
///
/// `new()` can only read the primary screen's DPI, which is the wrong monitor
/// as soon as the tray icon sits on a differently-scaled display. The window is
/// therefore moved first and the scale re-read from the window — and the paint
/// has to use that same scale, or the `BitBlt` copies a card that does not fill
/// the window it was sized for.
#[test]
fn the_card_is_sized_and_painted_for_the_monitor_it_lands_on() {
    let tokens = palette(true, Some((0x00, 0x78, 0xD4)));
    STYLE.with(|slot| slot.set(Some(tokens)));
    let Some(mut osd) = OsdOverlay::new() else {
        panic!("OsdOverlay::new() returned None: window creation failed");
    };
    let monitor_dpi = window_dpi(osd.hwnd);
    // Stand in for "the tray icon is on the other monitor": whatever `new()`
    // cached must not survive a show.
    osd.dpi = if monitor_dpi == 96 { 144 } else { 96 };
    // SAFETY: both metrics take no arguments.
    let (sw, sh) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    osd.show(named(62), Some((sw / 2, sh / 2, 24, 24)));

    let (w, h) = (scale(CARD_W, monitor_dpi), scale(CARD_H, monitor_dpi));
    let mut rect = RECT::default();
    // SAFETY: plain out-parameter write on the live window.
    let rect_ok = unsafe { GetWindowRect(osd.hwnd, &mut rect) }.is_ok();
    // SAFETY: `GetDC`/`ReleaseDC` balance on the live window and the sample
    // lies inside the client area computed above.
    let inside = unsafe {
        let dc = GetDC(Some(osd.hwnd));
        // Right of the state column, vertically mid-card: inside the fill in
        // either theme, and outside the card entirely if the window was sized
        // for the monitor while the paint used another scale.
        let px = GetPixel(dc, w - scale(PAD, monitor_dpi) - 1, h / 2).0 & 0x00FF_FFFF;
        ReleaseDC(Some(osd.hwnd), dc);
        px
    };
    let shown_dpi = osd.dpi;
    // Hide before asserting so a failure cannot leave a card on screen.
    osd.hide();
    STYLE.with(|slot| slot.set(None));

    assert_eq!(
        shown_dpi, monitor_dpi,
        "show() kept the DPI cached at construction"
    );
    assert!(rect_ok, "GetWindowRect failed");
    assert_eq!(
        (rect.right - rect.left, rect.bottom - rect.top),
        (w, h),
        "window not sized for the monitor it is on"
    );
    assert_eq!(
        inside,
        rgb(tokens.card).0 & 0x00FF_FFFF,
        "the card was not painted at the window's scale"
    );
}

fn named(percent: u32) -> OsdContent {
    OsdContent {
        device: "Speaker".into(),
        state: format!("{percent}%"),
        muted_label: "Muted".into(),
        percent,
        muted: false,
    }
}

/// Regression: the bar used to stop one text-width short of the right-aligned
/// read-out, so it visibly shortened as the read-out gained a digit (`5%` →
/// `50%` → `100%`) or swapped to the mute word. Only the text that is drawn may
/// change; the bar's right edge may not move.
#[test]
fn slider_keeps_one_length_whatever_the_readout_says() {
    let tokens = palette(true, Some((0x00, 0x78, 0xD4)));
    let device = AudioDevice {
        id: "a".into(),
        name: "Speaker".into(),
    };
    let narrow = sample(tokens, &format(Some(&device), 5, false, Lang::Zh));
    let wide = sample(tokens, &format(Some(&device), 100, false, Lang::Zh));
    let muted = sample(tokens, &format(Some(&device), 62, true, Lang::Zh));
    assert_eq!(
        narrow.bar_right, wide.bar_right,
        "the bar resized for a wider read-out"
    );
    assert_eq!(
        muted.bar_right, wide.bar_right,
        "the bar resized for the mute word"
    );
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
    let (w, h, dpi, state_col) = (scale(CARD_W, 96), scale(CARD_H, 96), 96, 24);
    let content = named(72);
    let l = layout(w, h, dpi, state_col, &content);
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
    let (w, h, dpi, state_col) = (scale(CARD_W, 96), scale(CARD_H, 96), 96, 24);
    let radius = scale(THUMB_D, dpi) / 2;
    for percent in [0, 5, 50, 95, 100] {
        let l = layout(w, h, dpi, state_col, &named(percent));
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
