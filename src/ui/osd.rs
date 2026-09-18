//! Overlay content formatting for the volume OSD.
//!
//! The overlay is the wheel-volume feedback channel: it is painted by this
//! process, so a change is visible in the same frame instead of after a system
//! hover delay. Formatting is pure so the layout contract stays testable
//! without a window.

use crate::audio::AudioDevice;
use crate::config::Lang;
use crate::ui::i18n::tr;
use crate::ui::label::truncate_label;

/// Device-name budget in the overlay.
///
/// Narrower than [`crate::ui::label::MAX_LABEL_CHARS`]: the card is a fixed
/// width and draws one line, but the name is still truncated so a hostile
/// endpoint name cannot widen it.
pub(crate) const OSD_NAME_CHARS: usize = 40;

/// Widest read-out a percentage can produce.
///
/// Reserved together with the mute word so the slider keeps one length whatever
/// is on screen: the read-out counts digits as the volume moves (`5%` → `50%` →
/// `100%`) and a bar sized to the text on screen visibly resizes with it.
pub(crate) const WIDEST_PERCENT: &str = "100%";

/// Text the overlay paints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OsdContent {
    /// Device name; empty when no default device is known (the line is
    /// skipped by the painter rather than drawn blank).
    pub(crate) device: String,
    /// State text: `"72%"`, or the localized mute word.
    pub(crate) state: String,
    /// The localized mute word, whether or not `state` is showing it.
    ///
    /// The card reserves a column sized for [`WIDEST_PERCENT`] or this word,
    /// whichever measures wider — both are measurable only while the painter
    /// holds the font, and neither may depend on the read-out on screen.
    pub(crate) muted_label: String,
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
    let muted_label = tr("muted", lang);
    let state = if mute {
        muted_label.clone()
    } else {
        format!("{percent}%")
    };
    let device = device.map_or_else(String::new, |d| truncate_label(&d.name, OSD_NAME_CHARS));
    OsdContent {
        device,
        state,
        muted_label,
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
mod tests;
