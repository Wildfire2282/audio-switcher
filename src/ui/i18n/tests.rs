//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::*;
use crate::config::Lang;

#[test]
fn i18n_zh_en() {
    assert_eq!(tr("mute", Lang::Zh), "全局静音");
    assert_eq!(tr("mute", Lang::En), "Mute");
    assert_eq!(tr("volume_limit", Lang::Zh), "音量上限");
    assert_eq!(tr("volume_limit", Lang::En), "Volume Limit");
    assert_eq!(tr("output_devices", Lang::Zh), "音频输出设备");
    assert_eq!(tr("output_devices", Lang::En), "Output Devices");
    assert_eq!(tr("input_devices", Lang::Zh), "音频输入设备");
    assert_eq!(tr("input_devices", Lang::En), "Input Devices");
}

#[test]
fn hotkey_settings_labels() {
    assert_eq!(tr("open_hotkey_settings", Lang::Zh), "打开快捷键设置");
    assert_eq!(tr("open_hotkey_settings", Lang::En), "Open Hotkey Settings");
    assert_ne!(tr("config_error", Lang::Zh), "config_error");
    assert_ne!(tr("config_error", Lang::En), "config_error");
    assert_ne!(tr("no_devices", Lang::Zh), "no_devices");
    assert_ne!(tr("no_devices", Lang::En), "no_devices");
}

/// Every key `tr` answers, spelled as the production call sites spell them.
const PRODUCTION_KEYS: &[&str] = &[
    "mute",
    "volume_limit",
    "enabled",
    "open_mixer",
    "open_sound",
    "open_hotkey_settings",
    "config_error",
    "autostart",
    "autostart_off",
    "autostart_user",
    "autostart_admin",
    "about",
    "exit",
    "chinese",
    "english",
    "input_devices",
    "output_devices",
    "muted",
    "refresh",
    "language",
    "system",
    "autostart_unknown",
    "no_devices",
    "device_error",
    "input_error",
    "mixer_error",
    "sound_error",
    "autostart_error",
    "link_error",
    "hotkey_conflict",
    "hotkey_conflict_hint",
];

/// An unknown key is returned verbatim, so an arm missing from one language
/// paints the key name itself into the menu — the one failure mode `tr` cannot
/// report at runtime. The count is asserted too: a new key must be added here
/// deliberately, or the missing translation stays invisible until a user sees
/// it.
#[test]
fn every_production_key_is_translated_in_both_languages() {
    for key in PRODUCTION_KEYS {
        for lang in [Lang::Zh, Lang::En] {
            let text = tr(key, lang);
            assert_ne!(text, *key, "{key} is untranslated in {lang:?}");
            assert!(
                text.contains(|c: char| !c.is_whitespace()),
                "{key}: {text:?}"
            );
        }
    }
    assert_eq!(
        PRODUCTION_KEYS.len(),
        31,
        "the key list must match the arms of `tr`"
    );
}
