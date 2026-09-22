//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::*;
use crate::config::clamp_volume;

/// The volume-limit menu entries: the preset is remembered independently of the
/// cap, and choosing a preset always switches the cap on.
#[test]
fn the_volume_limit_entries_store_the_preset_and_the_cap() {
    let mut cfg = AppConfig {
        volume_limit: 25,
        volume_limit_enabled: false,
        ..Default::default()
    };
    let mut ui = Lang::En;

    assert!(apply_menu_config(
        &mut cfg,
        &mut ui,
        &MenuAction::VolLimit(50)
    ));
    assert_eq!(cfg.volume_limit, 50);
    assert!(
        cfg.volume_limit_enabled,
        "picking a preset is how the cap is switched on"
    );
    assert_eq!(
        clamp_volume(80, &cfg),
        50,
        "the caller clamps with the limit just picked"
    );

    assert!(apply_menu_config(
        &mut cfg,
        &mut ui,
        &MenuAction::VolEnabled
    ));
    assert!(!cfg.volume_limit_enabled);
    assert_eq!(
        cfg.volume_limit, 50,
        "the preset survives while the cap is off"
    );
    assert_eq!(clamp_volume(80, &cfg), 80, "off means uncapped");
}

/// The three language entries store the mode and the resolved UI language
/// together: a stale `ui_lang` would render the menu in the previous language.
#[test]
fn the_language_entries_store_the_mode_and_the_ui_language() {
    let mut cfg = AppConfig {
        lang: Lang::System,
        ..Default::default()
    };
    let mut ui = Lang::System;

    for (action, expected) in [
        (MenuAction::LangZh, Lang::Zh),
        (MenuAction::LangEn, Lang::En),
    ] {
        assert!(apply_menu_config(&mut cfg, &mut ui, &action));
        assert_eq!(cfg.lang, expected);
        assert_eq!(ui, expected);
    }

    assert!(apply_menu_config(
        &mut cfg,
        &mut ui,
        &MenuAction::LangSystem
    ));
    assert_eq!(cfg.lang, Lang::System);
    assert_eq!(ui, cfg.effective_lang());
}
