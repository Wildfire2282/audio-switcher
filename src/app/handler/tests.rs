//! Unit tests for the sibling module.
//!
//! Kept in its own file so a production read does not drag the tests along:
//! an agent changing this module reads the module, and one changing the
//! behaviour reads this file. `use super::*` still reaches every private item.

use super::*;

#[test]
fn parse_device() {
    assert_eq!(
        MenuAction::from_id("device_abc"),
        MenuAction::Device("abc".into())
    );
    assert_eq!(MenuAction::from_id("mute"), MenuAction::Mute);
    assert_eq!(MenuAction::from_id("refresh"), MenuAction::Refresh);
    assert_eq!(MenuAction::from_id("vol_25"), MenuAction::VolLimit(25));
    assert_eq!(MenuAction::from_id("vol_50"), MenuAction::VolLimit(50));
    assert_eq!(MenuAction::from_id("vol_75"), MenuAction::VolLimit(75));
    assert_eq!(MenuAction::from_id("lang_system"), MenuAction::LangSystem);
    assert!(matches!(
        MenuAction::from_id("unknown"),
        MenuAction::Unknown(_)
    ));
}

#[test]
fn parse_autostart_modes() {
    // Each mode is reachable from its own id: a rename here would make the item
    // parse as `Unknown`, and the click would silently do nothing.
    assert_eq!(
        MenuAction::from_id("autostart_off"),
        MenuAction::Autostart(AutostartMode::Off)
    );
    assert_eq!(
        MenuAction::from_id("autostart_user"),
        MenuAction::Autostart(AutostartMode::User)
    );
    assert_eq!(
        MenuAction::from_id("autostart_admin"),
        MenuAction::Autostart(AutostartMode::Admin)
    );
    // The submenu's own id is not an action.
    assert!(matches!(
        MenuAction::from_id("autostart"),
        MenuAction::Unknown(_)
    ));
}

#[test]
fn parse_hotkey_settings() {
    assert_eq!(
        MenuAction::from_id("open_hotkey_settings"),
        MenuAction::OpenHotkeySettings
    );
    // Retired submenu ids stay Unknown and never dispatch.
    for id in ["hotkeys", "hotkey_", "hotkey_bogus", "hotkey_mute"] {
        assert!(
            matches!(MenuAction::from_id(id), MenuAction::Unknown(_)),
            "{id}"
        );
    }
}

#[test]
fn parse_input_device() {
    assert_eq!(
        MenuAction::from_id("input_m1"),
        MenuAction::InputDevice("m1".into())
    );
    assert_eq!(
        MenuAction::from_id("device_abc"),
        MenuAction::Device("abc".into())
    );
    assert!(matches!(
        MenuAction::from_id("input_"),
        MenuAction::Unknown(_)
    ));
}

#[test]
fn parse_vol_preset_only_accepts_menu_presets() {
    // The menu only emits 25/50/75 — anything else stays Unknown.
    assert!(matches!(
        MenuAction::from_id("vol_30"),
        MenuAction::Unknown(_)
    ));
    assert!(matches!(
        MenuAction::from_id("vol_0"),
        MenuAction::Unknown(_)
    ));
    assert!(matches!(
        MenuAction::from_id("vol_101"),
        MenuAction::Unknown(_)
    ));
    assert!(matches!(
        MenuAction::from_id("vol_x"),
        MenuAction::Unknown(_)
    ));
}

#[test]
fn unknown_ids_stay_unknown_for_warn_path() {
    // The App dispatch warns (debug_assert + tracing::warn) and ignores
    // these; parsing must never coerce them into a real action.
    for id in ["bogus", "", "vol_", "device_", "DEVICE_abc", "About"] {
        assert!(
            matches!(MenuAction::from_id(id), MenuAction::Unknown(_)),
            "{id} must parse as Unknown"
        );
    }
}
