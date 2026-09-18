//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

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
fn the_mute_word_is_carried_even_when_the_percent_shows() {
    // The card reserves a column for the widest read-out it can show, so
    // the mute word has to be measurable while it is not on screen.
    let unmuted = format(Some(&dev("Speaker")), 5, false, Lang::Zh);
    assert_eq!(unmuted.state, "5%");
    assert_eq!(unmuted.muted_label, "静音");

    let muted = format(Some(&dev("Speaker")), 5, true, Lang::Zh);
    assert_eq!(muted.state, "静音");
    assert_eq!(muted.muted_label, "静音");

    let en = format(None, 100, false, Lang::En);
    assert_eq!(en.state, "100%");
    assert_eq!(en.muted_label, "Muted");
}

/// Regression: the slider used to be sized against the read-out on screen,
/// so it shortened as the text grew (`5%` → `50%` → `100%`). What the
/// painter measures must not depend on the volume or the mute state.
#[test]
fn the_reserved_column_inputs_do_not_move_with_the_readout() {
    let measured = |volume: u32, mute: bool| {
        let c = format(None, volume, mute, Lang::Zh);
        let mut inputs = vec![WIDEST_PERCENT.to_owned(), c.muted_label];
        inputs.sort();
        inputs
    };
    let baseline = measured(5, false);
    for volume in [0, 9, 10, 99, 100] {
        for mute in [true, false] {
            assert_eq!(measured(volume, mute), baseline, "{volume}% muted={mute}");
        }
    }
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
