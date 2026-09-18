//! Overlay content formatting for the volume OSD.
//!
//! The overlay is the wheel-volume feedback channel: it is painted by this
//! process, so a change is visible in the same frame instead of after a system
//! hover delay. Formatting is pure so the layout contract stays testable
//! without a window.

use crate::audio::AudioDevice;
use crate::config::Lang;
use crate::ui::i18n::tr;
use crate::ui::text::truncate_label;

/// Device-name budget in the overlay.
///
/// Narrower than [`crate::ui::text::MAX_LABEL_CHARS`]: the card is a fixed
/// width and draws one line, but the name is still truncated so a hostile
/// endpoint name cannot widen it.
pub(crate) const OSD_NAME_CHARS: usize = 40;

/// Text the overlay paints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OsdContent {
    /// Device name; empty when no default device is known (the line is
    /// skipped by the painter rather than drawn blank).
    pub(crate) device: String,
    /// State text: `"72%"`, or the localized mute word.
    pub(crate) state: String,
    /// Bar fill, `0..=100`.
    pub(crate) percent: u32,
    /// Whether the bar paints in the muted (grey) colour.
    pub(crate) muted: bool,
}

/// Build the overlay content for the current volume/mute state.
pub(crate) fn format(
    device: Option<&AudioDevice>,
    volume: u32,
    mute: bool,
    lang: Lang,
) -> OsdContent {
    let percent = volume.min(100);
    let state = if mute {
        tr("muted", lang)
    } else {
        format!("{percent}%")
    };
    let device = device.map_or_else(String::new, |d| truncate_label(&d.name, OSD_NAME_CHARS));
    OsdContent {
        device,
        state,
        percent,
        muted: mute,
    }
}

// ---------------------------------------------------------------------------
// Design tokens
// ---------------------------------------------------------------------------

/// A colour as `(r, g, b)`. `platform` owns the `COLORREF` packing.
pub(crate) type Rgb = (u8, u8, u8);

/// Windows 11 flyout design tokens.
///
/// Values follow the shell's own ramps: `SolidBackgroundFillColorSecondary` for
/// the card, `SurfaceStrokeColorDefault` for the hairline,
/// `TextFillColorPrimary`/`Secondary` for the text, and
/// `ControlStrongFillColorDefault` for the slider track. The slider fill is the
/// user's accent, the way the shell's own volume flyout paints it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Palette {
    /// Card fill.
    pub(crate) card: Rgb,
    /// Hairline drawn around the card.
    pub(crate) border: Rgb,
    /// Primary text (the device name).
    pub(crate) text: Rgb,
    /// Secondary text (the state read-out).
    pub(crate) dim_text: Rgb,
    /// Slider track.
    pub(crate) track: Rgb,
    /// Slider fill and thumb.
    pub(crate) fill: Rgb,
    /// Slider fill and thumb while muted.
    pub(crate) muted_fill: Rgb,
}

/// Windows' default accent (`#0078D4`) for users who never picked one.
const DEFAULT_ACCENT: Rgb = (0x00, 0x78, 0xD4);

/// Mix roughly halfway between the light `#2C2C2C` and `#F9F9F9` cards, used
/// where an element has to read as "neutral" in both themes.
const MUTED_DARK: Rgb = (0x8A, 0x8A, 0x8A);
const MUTED_LIGHT: Rgb = (0x95, 0x95, 0x95);

/// Resolve the design tokens for a shell appearance.
pub(crate) fn palette(light: bool, accent: Option<Rgb>) -> Palette {
    let accent = accent.unwrap_or(DEFAULT_ACCENT);
    if light {
        Palette {
            card: (0xF9, 0xF9, 0xF9),
            border: (0xE5, 0xE5, 0xE5),
            text: (0x1A, 0x1A, 0x1A),
            dim_text: (0x5F, 0x5F, 0x5F),
            track: (0xE2, 0xE2, 0xE2),
            fill: accent,
            muted_fill: MUTED_LIGHT,
        }
    } else {
        Palette {
            card: (0x2C, 0x2C, 0x2C),
            border: (0x3D, 0x3D, 0x3D),
            text: (0xFF, 0xFF, 0xFF),
            dim_text: (0xC5, 0xC5, 0xC5),
            track: (0x4A, 0x4A, 0x4A),
            // On a dark surface the shell raises the accent's luminance rather
            // than using the light-surface accent directly.
            fill: lighten(accent, 35),
            muted_fill: MUTED_DARK,
        }
    }
}

/// Blend `color` toward white by `pct` percent.
fn lighten(color: Rgb, pct: u8) -> Rgb {
    let keep = u16::from(100 - pct);
    let add = u16::from(pct);
    let mix = |channel: u8| {
        let value = (u16::from(channel) * keep + 255 * add) / 100;
        u8::try_from(value).unwrap_or(255)
    };
    (mix(color.0), mix(color.1), mix(color.2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioDevice;

    fn dev(name: &str) -> AudioDevice {
        AudioDevice {
            id: "a".into(),
            name: name.into(),
        }
    }

    #[test]
    fn unmuted_shows_percent_and_fill() {
        let c = format(Some(&dev("Speaker")), 72, false, Lang::Zh);
        assert_eq!(c.device, "Speaker");
        assert_eq!(c.state, "72%");
        assert_eq!(c.percent, 72);
        assert!(!c.muted);
    }

    #[test]
    fn muted_swaps_state_for_the_word() {
        let zh = format(Some(&dev("Speaker")), 72, true, Lang::Zh);
        assert_eq!(zh.state, "静音");
        assert!(zh.muted);
        // The bar still reflects the volume under the mute.
        assert_eq!(zh.percent, 72);
        let en = format(Some(&dev("Speaker")), 72, true, Lang::En);
        assert_eq!(en.state, "Muted");
    }

    #[test]
    fn missing_device_leaves_the_line_empty() {
        let c = format(None, 50, false, Lang::En);
        assert!(c.device.is_empty());
        assert_eq!(c.state, "50%");
    }

    #[test]
    fn percent_is_clamped_to_the_bar_range() {
        assert_eq!(format(None, 150, false, Lang::En).percent, 100);
    }

    #[test]
    fn long_names_truncate_and_newlines_are_sanitized() {
        let long = format(Some(&dev(&"X".repeat(200))), 50, false, Lang::Zh);
        assert_eq!(long.device.chars().count(), OSD_NAME_CHARS - 1);
        assert!(long.device.ends_with('…'));

        let injected = format(Some(&dev("Speaker\nInjected")), 50, false, Lang::En);
        assert!(!injected.device.contains('\n'));
        assert_eq!(injected.device, "Speaker Injected");
    }

    #[test]
    fn light_and_dark_cards_are_opposite() {
        let light = palette(true, None).card;
        let dark = palette(false, None).card;
        assert!(
            light.0 > dark.0,
            "light card must be brighter than the dark one"
        );
        // The hairline has to stay distinguishable from the card in both themes.
        for light_theme in [true, false] {
            let p = palette(light_theme, None);
            assert_ne!(p.border, p.card, "hairline invisible on its own card");
            assert_ne!(p.track, p.card, "track invisible on its own card");
        }
    }

    #[test]
    fn light_theme_uses_the_accent_verbatim() {
        let accent = (0x11, 0x22, 0x33);
        assert_eq!(palette(true, Some(accent)).fill, accent);
    }

    #[test]
    fn dark_theme_raises_the_accent_luminance() {
        let accent = (0x00, 0x78, 0xD4);
        let raised = palette(false, Some(accent)).fill;
        assert_ne!(
            raised, accent,
            "dark surfaces must not reuse the light accent"
        );
        assert!(raised.0 >= accent.0 && raised.1 >= accent.1 && raised.2 >= accent.2);
        assert!(raised != (255, 255, 255), "must stay a colour, not white");
    }

    #[test]
    fn missing_accent_falls_back_to_the_windows_default() {
        assert_eq!(palette(true, None).fill, DEFAULT_ACCENT);
        assert_eq!(palette(false, None).fill, lighten(DEFAULT_ACCENT, 35));
    }

    #[test]
    fn muted_fill_is_neutral_in_both_themes() {
        for light in [true, false] {
            let muted = palette(light, Some((0x00, 0x78, 0xD4))).muted_fill;
            assert_eq!(muted.0, muted.1, "{muted:?} is tinted");
            assert_eq!(muted.1, muted.2, "{muted:?} is tinted");
        }
    }
}
